//! Actual CPU widgets -> inspected paths -> production render-list adapter.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_render::RenderList3D;
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
fn paint(h: &Harness<'_, (Panel, Editor, bool)>) -> RenderList3D {
    let mut list = RenderList3D::default();
    h.state().0.paint_draft_route(&h.state().1, &mut list);
    list
}
#[test]
fn actual_all_leg_inspection_matches_bent_elevated_paths_then_apply_save_reopens() {
    let (_temp, mut h) = fixture();
    let path = h.state().1.path().unwrap();
    let bytes = fs::read(&path).unwrap();
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().clone();
    let selected = h.state().1.selection().cloned();
    click(&mut h, INSPECT);
    assert!(paint(&h).is_empty());
    click(&mut h, SHOW);
    let graph = h.state().0.graph.as_ref().unwrap();
    let p = |x, y, z| [FP::from_int(x), FP::from_int(y), FP::from_int(z)];
    let endpoints = [
        (p(1, 0, 1), p(3, 2, 1)),
        (p(3, 2, 1), p(1, 0, 1)),
        (p(1, 0, 1), p(3, 2, 1)),
    ];
    let paths = endpoints
        .iter()
        .map(|(a, b)| {
            graph
                .find_path(
                    *a,
                    *b,
                    fp!(2),
                    SearchBudget {
                        max_expansions: graph.triangles().len() as u32,
                    },
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(paths.iter().any(|path| path.points.len() > 2));
    assert!(
        paths
            .iter()
            .flat_map(|path| &path.points)
            .any(|p| p[1] > FP::ZERO)
    );
    let report = h.state().0.inspected_patrol_route(&h.state().1).unwrap();
    let mut total = FP::ZERO;
    for (leg, path) in report.legs.iter().zip(&paths) {
        assert_eq!(leg.points, path.points);
        assert_eq!(leg.length, graph.path_length(path).unwrap());
        total += leg.length;
    }
    assert_eq!(report.total_length, total);
    assert_eq!(report.legs.len(), 3);
    let list = paint(&h);
    let segments = paths
        .iter()
        .map(|path| path.points.len() - 1)
        .sum::<usize>();
    assert_eq!(list.lines.len(), segments + 36);
    let mut index = 0;
    for (leg, path) in paths.iter().enumerate() {
        for pair in path.points.windows(2) {
            assert_eq!(list.lines[index].a, pair[0].map(FP::to_f32));
            assert_eq!(list.lines[index].b, pair[1].map(FP::to_f32));
            assert_eq!(
                list.lines[index].color,
                if leg == 2 {
                    [0.85, 0.35, 1., 1.]
                } else {
                    [0., 0.8, 1., 1.]
                }
            );
            index += 1;
        }
    }
    assert_eq!(list.instance_count(), 0);
    assert_eq!(paint(&h).lines, list.lines);
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(h.state().1.selection(), selected.as_ref());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    click(&mut h, "Apply waypoint patrol");
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
    let config =
        orr_remote::mesh_navigation::patrol::from_view(h.state().1.snapshot().unwrap().predicted())
            .unwrap()
            .unwrap()
            .config;
    assert_eq!((config.count, config.looped), (2, 1));
    assert!(paint(&h).is_empty());
    assert!(h.state_mut().1.undo());
    h.state_mut().1.sync();
    assert!(h.state_mut().1.redo());
    h.state_mut().1.sync();
    assert!(h.state_mut().1.save());
    let reopened = orr_remote::mesh_navigation::mesh_doc_from_path(&path).unwrap();
    assert_eq!(
        orr_remote::mesh_navigation::patrol::from_view(orr_bridge::FrameView::of(reopened.frame()))
            .unwrap()
            .unwrap()
            .config,
        config
    );
}
#[test]
fn invalid_late_waypoint_reinspection_never_keeps_partial_or_previous_overlay() {
    let (_temp, mut h) = fixture();
    let path = h.state().1.path().unwrap();
    let bytes = fs::read(&path).unwrap();
    let history = h.state().1.history().clone();
    let checksum = h.state().1.checksum();
    click(&mut h, INSPECT);
    click(&mut h, SHOW);
    assert!(!paint(&h).is_empty());
    text(&mut h, "Patrol waypoint 2 X", "250");
    assert!(paint(&h).is_empty());
    click(&mut h, INSPECT);
    assert!(h.get_by_label(SHOW).accesskit_node().is_disabled());
    assert!(paint(&h).is_empty());
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(fs::read(&path).unwrap(), bytes);
}
#[test]
fn changed_draft_aba_and_focus_loss_with_panel_closed_irreversibly_retire_inspection() {
    let (_temp, mut h) = fixture();
    click(&mut h, INSPECT);
    click(&mut h, SHOW);
    assert!(!paint(&h).is_empty());
    text(&mut h, "Patrol waypoint 2 X", "1.5");
    assert!(paint(&h).is_empty());
    text(&mut h, "Patrol waypoint 2 X", "1");
    assert!(paint(&h).is_empty());
    click(&mut h, INSPECT);
    click(&mut h, SHOW);
    assert!(!paint(&h).is_empty());
    // The frame-start adapter must also observe source changes without a panel draw.
    h.state_mut().2 = false;
    let graph = h.state_mut().0.graph.take();
    h.step();
    h.state_mut().0.graph = graph;
    h.step();
    assert!(
        paint(&h).is_empty(),
        "hidden-panel A-B-A must not revive inspection"
    );
    h.state_mut().2 = true;
    h.run_steps(3);
    click(&mut h, INSPECT);
    click(&mut h, SHOW);
    assert!(!paint(&h).is_empty());
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().clone();
    h.state_mut().2 = false;
    // One RawInput batch, not Harness.event (which schedules separate frames).
    h.input_mut().focused = true;
    h.input_mut().events.extend([
        egui::Event::WindowFocused(false),
        egui::Event::WindowFocused(true),
    ]);
    h.step();
    assert!(paint(&h).is_empty());
    h.run_steps(2);
    assert!(paint(&h).is_empty());
    h.state_mut().2 = true;
    h.run_steps(3);
    assert!(paint(&h).is_empty());
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
}
