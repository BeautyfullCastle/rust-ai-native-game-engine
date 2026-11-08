//! Actual CPU widgets and production one-shot frame tickets. No window or GPU.
#![allow(clippy::disallowed_types)]
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_fp::FP;
use std::time::{Duration, Instant};
const INSPECT: &str = "Inspect terrain patrol draft route";
const SHOW: &str = "Show inspected terrain patrol route";
fn wait(editor: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.sync();
        if editor.can_mutate()
            && editor.yard_rows_coherent()
            && editor.admitted_navigation().admitted
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn fixture() -> (
    tempfile::TempDir,
    Harness<'static, (NavigationPanel, Editor, TerrainPanel, bool)>,
) {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("terrain-patrol.scene.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../../scenes/navigation_blank.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.sync();
        if editor.can_mutate() && editor.yard_rows_coherent() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut terrain = TerrainPanel::default();
    terrain.local_path = "route.orrt".into();
    terrain.asset_id = "navigation/patrol-fixture".into();
    terrain.dimensions = [9, 9];
    terrain.origin = ["-4".into(), "-4".into()];
    terrain.spacing = "1".into();
    terrain.initial_height = "0".into();
    terrain.create_for_editor(&editor).unwrap();
    let mut edits = Vec::new();
    for z in 0..9 {
        for x in 0..9 {
            edits.push(orr_terrain::Edit::SetHeight {
                x,
                z,
                height: FP::from_raw(i64::from(x) * 16384),
            });
        }
    }
    edits.push(orr_terrain::Edit::SetHole {
        x: 4,
        z: 6,
        hole: true,
    });
    terrain.document.apply(&edits).unwrap();
    terrain.save_for_editor(&editor).unwrap();
    let mut panel = NavigationPanel::default();
    panel.start = ["-3".into(), "0".into()];
    panel.goal = ["3".into(), "0".into()];
    panel.distance_per_tick = "1".into();
    panel.build_route(&mut editor, &terrain).unwrap();
    wait(&mut editor);
    assert!(editor.save());
    wait(&mut editor);
    panel.sync_from_editor(&editor);
    let mut h = Harness::builder()
        .with_size([1600.0, 3000.0])
        .build_ui_state(
            |ui, state: &mut (NavigationPanel, Editor, TerrainPanel, bool)| {
                state
                    .0
                    .sync_patrol_inspection_focus(&state.1, &state.2, ui.ctx());
                if state.3 {
                    state.0.show(ui, &mut state.1, &state.2);
                }
            },
            (panel, editor, terrain, true),
        );
    h.run_steps(3);
    h.get_by_label("Point navigation").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Terrain waypoint patrol").click_accesskit();
    h.run_steps(3);
    (dir, h)
}

fn click(h: &mut Harness<'_, (NavigationPanel, Editor, TerrainPanel, bool)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
fn text(
    h: &mut Harness<'_, (NavigationPanel, Editor, TerrainPanel, bool)>,
    label: &str,
    value: &str,
) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, label)
            .value()
            .as_deref(),
        Some(value)
    );
}
fn stage(h: &mut Harness<'_, (NavigationPanel, Editor, TerrainPanel, bool)>) {
    click(h, "Enable terrain waypoint patrol");
    click(h, "Add terrain patrol waypoint");
    for (label, value) in [
        ("Terrain patrol waypoint 1 X", "-2"),
        ("Terrain patrol waypoint 1 Z", "0"),
        ("Terrain patrol waypoint 2 X", "2"),
        ("Terrain patrol waypoint 2 Z", "0"),
    ] {
        text(h, label, value);
    }
    click(h, "Loop terrain patrol");
}

#[test]
fn actual_frame_button_uses_current_inspection_without_overlay_and_consumes_once_without_authoring_writes(
) {
    let (dir, mut h) = fixture();
    stage(&mut h);
    click(&mut h, INSPECT);
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().clone();
    let camera = format!("{:?}", h.state().1.camera3d);
    let scene = dir.path().join("terrain-patrol.scene.yaml");
    let bytes = std::fs::read(&scene).unwrap();
    let expected = {
        let s = h.state();
        s.0.inspected_patrol_frame_points(&s.1, &s.2).unwrap()
    };
    assert!(expected.iter().any(|p| p[1] != 0.0));
    click(&mut h, "Frame inspected patrol route");
    click(&mut h, SHOW);
    click(&mut h, SHOW);
    {
        let s = h.state_mut();
        assert_eq!(
            s.0.take_patrol_frame_request(&s.1, &s.2).unwrap(),
            Some(expected)
        );
        assert!(s.0.take_patrol_frame_request(&s.1, &s.2).unwrap().is_none());
    }
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(format!("{:?}", h.state().1.camera3d), camera);
    assert_eq!(std::fs::read(scene).unwrap(), bytes);
}
#[test]
fn form_aba_terrain_arc_change_busy_and_failed_inspection_refuse_framing() {
    let (_dir, mut h) = fixture();
    stage(&mut h);
    click(&mut h, INSPECT);
    click(&mut h, "Frame inspected patrol route");
    text(&mut h, "Terrain patrol waypoint 2 X", "2.5");
    text(&mut h, "Terrain patrol waypoint 2 X", "2");
    {
        let s = h.state_mut();
        assert!(s.0.take_patrol_frame_request(&s.1, &s.2).unwrap().is_none());
    }
    assert!(h
        .get_by_label("Frame inspected patrol route")
        .accesskit_node()
        .is_disabled());
    click(&mut h, INSPECT);
    click(&mut h, "Frame inspected patrol route");
    h.state_mut().3 = false;
    h.state_mut()
        .2
        .document
        .apply(&[orr_terrain::Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::ONE,
        }])
        .unwrap();
    h.step();
    h.state_mut().2.document.undo().unwrap();
    h.step();
    {
        let s = h.state_mut();
        assert!(s.0.take_patrol_frame_request(&s.1, &s.2).is_err());
    }
    h.state_mut().3 = true;
    h.run_steps(3);
    assert!(h
        .get_by_label("Frame inspected patrol route")
        .accesskit_node()
        .is_disabled());
    click(&mut h, INSPECT);
    h.state_mut().0.set_patrol_frame_busy(true);
    h.run_steps(2);
    assert!(h
        .get_by_label("Frame inspected patrol route")
        .accesskit_node()
        .is_disabled());
    h.state_mut().0.set_patrol_frame_busy(false);
    text(&mut h, "Terrain patrol waypoint 2 X", "999");
    click(&mut h, INSPECT);
    assert!(h
        .get_by_label("Frame inspected patrol route")
        .accesskit_node()
        .is_disabled());
}
