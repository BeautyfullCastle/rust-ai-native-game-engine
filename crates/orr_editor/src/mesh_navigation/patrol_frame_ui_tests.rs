use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
const INSPECT: &str = "Inspect patrol draft route";
const SHOW: &str = "Show inspected patrol route";
fn click(h: &mut Harness<'_, (Panel, Editor, bool)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
fn text(h: &mut Harness<'_, (Panel, Editor, bool)>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}
fn fixture() -> (tempfile::TempDir, Harness<'static, (Panel, Editor, bool)>) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("patrol-preview.scene.yaml");
    fs::write(&path, "schema: orr.scene/1\nsingletons: {}\nentities: {}\n").unwrap();
    let mut editor = Editor::open_game(&path, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.sync(&editor);
    panel.create_ramp_and_bridge().unwrap();
    panel.place_agent(&mut editor).unwrap();
    editor.sync();
    assert!(editor.save());
    panel.sync(&editor);
    let mut h = Harness::builder().with_size([1600., 4400.]).build_ui_state(
        |ui, state: &mut (Panel, Editor, bool)| {
            state.0.sync_patrol_inspection_focus(&state.1, ui.ctx());
            if state.2 {
                state.0.show(ui, &mut state.1);
            }
        },
        (panel, editor, true),
    );
    h.run_steps(3);
    click(&mut h, "Static-mesh navigation");
    click(&mut h, "Waypoint patrol");
    click(&mut h, "Enable waypoint patrol");
    click(&mut h, "Add patrol waypoint");
    for (label, value) in [
        ("Patrol waypoint 1 X", "3"),
        ("Patrol waypoint 1 Y", "2"),
        ("Patrol waypoint 1 Z", "1"),
        ("Patrol waypoint 2 X", "1"),
        ("Patrol waypoint 2 Y", "0"),
        ("Patrol waypoint 2 Z", "1"),
    ] {
        text(&mut h, label, value);
    }
    click(&mut h, "Loop patrol");
    (temp, h)
}

#[test]
fn actual_button_queues_complete_current_visible_loop_once_without_frame_history_or_source_writes()
{
    let (dir, mut h) = fixture();
    click(&mut h, INSPECT);
    // Framing does not require displaying the overlay.
    let scene_bytes = fs::read(dir.path().join("patrol-preview.scene.yaml")).unwrap();
    let frame = h.state().1.checksum();
    let history = h.state().1.history().clone();
    let camera = format!("{:?}", h.state().1.camera3d);
    let expected = {
        let s = h.state();
        s.0.inspected_patrol_frame_points(&s.1).unwrap()
    };
    click(&mut h, "Frame inspected patrol route");
    let s = h.state_mut();
    assert_eq!(s.0.take_patrol_frame_request(&s.1).unwrap(), Some(expected));
    assert!(s.0.take_patrol_frame_request(&s.1).unwrap().is_none());
    assert_eq!(s.1.checksum(), frame);
    assert_eq!(s.1.history(), &history);
    assert_eq!(format!("{:?}", s.1.camera3d), camera);
    assert_eq!(
        fs::read(dir.path().join("patrol-preview.scene.yaml")).unwrap(),
        scene_bytes
    );
}
#[test]
fn overlay_visibility_is_independent_but_form_aba_failed_and_busy_requests_fail_closed() {
    let (_dir, mut h) = fixture();
    click(&mut h, INSPECT);
    assert!(!h
        .get_by_label("Frame inspected patrol route")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Frame inspected patrol route");
    click(&mut h, SHOW);
    click(&mut h, SHOW);
    {
        let state = h.state_mut();
        assert!(state
            .0
            .take_patrol_frame_request(&state.1)
            .unwrap()
            .is_some());
    }
    click(&mut h, "Frame inspected patrol route");
    text(&mut h, "Patrol waypoint 1 X", "2");
    text(&mut h, "Patrol waypoint 1 X", "3");
    {
        let s = h.state_mut();
        assert!(s.0.take_patrol_frame_request(&s.1).unwrap().is_none());
    }
    assert!(h
        .get_by_label("Frame inspected patrol route")
        .accesskit_node()
        .is_disabled());
    click(&mut h, INSPECT);
    click(&mut h, SHOW);
    h.state_mut().0.set_patrol_frame_busy(true);
    h.run_steps(2);
    assert!(h
        .get_by_label("Frame inspected patrol route")
        .accesskit_node()
        .is_disabled());
    h.state_mut().0.set_patrol_frame_busy(false);
    text(&mut h, "Patrol waypoint 2 X", "99999");
    click(&mut h, INSPECT);
    assert!(h
        .get_by_label("Frame inspected patrol route")
        .accesskit_node()
        .is_disabled());
    {
        let state = h.state_mut();
        assert!(state
            .0
            .take_patrol_frame_request(&state.1)
            .unwrap()
            .is_none());
    }
}
