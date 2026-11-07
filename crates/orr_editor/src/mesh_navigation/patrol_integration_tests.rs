//! Actual UI -> atomic scene authoring -> saved scene -> deterministic Play.
#![allow(clippy::disallowed_types)]
use super::*;
use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use orr_remote::mesh_navigation::patrol;
use std::time::{Duration, Instant};
fn fixture() -> (tempfile::TempDir, Harness<'static, (Panel, Editor)>) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("patrol.scene.yaml");
    let p = |x, z| [FP::from_int(x), FP::ZERO, FP::from_int(z)];
    let graph = StaticMeshGraph::bake(
        &[p(0, 0), p(0, 4), p(4, 0), p(4, 4)],
        &[[0, 1, 2], [2, 1, 3]],
        AgentProfile::default(),
    )
    .unwrap();
    let mut document = orr_remote::mesh_navigation::authoring::create_mesh_scene(
        &graph,
        AgentSpec {
            start: p(0, 0),
            goal: p(4, 0),
            max_vertical: FP::ZERO,
            distance_per_tick: FP::ONE,
        },
    )
    .unwrap();
    fs::write(&path, document.save_yaml()).unwrap();
    let mut editor = Editor::open_game(&path, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.sync(&editor);
    let mut h = Harness::builder()
        .with_size([1600.0, 4000.0])
        .build_ui_state(
            |ui, state: &mut (Panel, Editor)| state.0.show(ui, &mut state.1),
            (panel, editor),
        );
    h.run_steps(3);
    h.get_by_label("Static-mesh navigation").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Waypoint patrol").click_accesskit();
    h.run_steps(3);
    (temp, h)
}
fn text(h: &mut Harness<'_, (Panel, Editor)>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}
fn stage(h: &mut Harness<'_, (Panel, Editor)>, bad: bool) {
    h.get_by_label("Enable waypoint patrol").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Add patrol waypoint").click_accesskit();
    h.run_steps(3);
    for (label, value) in [
        ("Patrol waypoint 1 X", "1"),
        ("Patrol waypoint 1 Y", "0"),
        ("Patrol waypoint 1 Z", "0"),
        ("Patrol waypoint 2 X", if bad { "99" } else { "2" }),
        ("Patrol waypoint 2 Y", "0"),
        ("Patrol waypoint 2 Z", "0"),
        ("Patrol wait ticks", "2"),
    ] {
        text(h, label, value);
    }
    h.get_by_label("Loop patrol").click_accesskit();
    h.run_steps(3);
}
fn read(editor: &Editor) -> patrol::PatrolView {
    patrol::from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
}
fn advance(editor: &mut Editor) -> orr_remote::mesh_navigation::MeshView {
    let target = editor.snapshot().unwrap().predicted().tick() + 1;
    editor.step(1);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.sync();
        if editor
            .snapshot()
            .is_some_and(|s| s.predicted().tick() == target && s.timeline().is_some())
        {
            return mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "Play tick {target} not observed: {:?}",
            editor.down()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn authored_waypoints_apply_undo_redo_save_reopen_and_play_loop_then_manual_goal_pauses() {
    let (_temp, mut h) = fixture();
    let path = h.state().1.path().unwrap();
    let bytes = fs::read(&path).unwrap();
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().entries.len();
    stage(&mut h, false);
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history().entries.len(), history);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    h.get_by_label("Apply waypoint patrol").click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
    let config = read(&h.state().1).config;
    assert_eq!(
        (
            config.enabled,
            config.count,
            config.looped,
            config.wait_ticks
        ),
        (1, 2, 1, 2)
    );
    assert_eq!(config.points[0], [fp!(1), FP::ZERO, FP::ZERO]);
    assert_eq!(config.points[1], [fp!(2), FP::ZERO, FP::ZERO]);
    assert_eq!(h.state().1.history().entries.len(), history + 1);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(h.state_mut().1.undo());
    h.state_mut().1.sync();
    assert!(
        patrol::from_view(h.state().1.snapshot().unwrap().predicted())
            .unwrap()
            .is_none()
    );
    assert!(h.state_mut().1.redo());
    h.state_mut().1.sync();
    assert_eq!(read(&h.state().1).config, config);
    assert!(h.state_mut().1.save());
    let saved = fs::read(&path).unwrap();
    let loaded = orr_remote::mesh_navigation::mesh_doc_from_path(&path).unwrap();
    assert_eq!(
        patrol::from_view(orr_bridge::FrameView::of(loaded.frame()))
            .unwrap()
            .unwrap()
            .config,
        config
    );
    let mut editor = Editor::open_game(&path, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    assert_eq!(read(&editor).config, config);
    for (expected_x, remaining, cursor) in [
        (1, 2, 0),
        (1, 1, 0),
        (1, 0, 0),
        (2, 2, 1),
        (2, 1, 1),
        (2, 0, 1),
        (1, 2, 0),
    ] {
        let view = advance(&mut editor);
        assert_eq!(
            view.agent.position,
            [FP::from_int(expected_x), FP::ZERO, FP::ZERO]
        );
        let run = read(&editor).run;
        assert_eq!(
            (run.wait_remaining, run.cursor, run.paused),
            (remaining, cursor, 0)
        );
    }
    let grant = editor
        .host_call(
            "sim.input_claim",
            serde_json::json!({"player":0,"replace_held":true}),
        )
        .unwrap();
    editor.host_call("sim.input_value",serde_json::json!({"player":0,"grant":grant["grant"],"generation":grant["generation"],"sequence":"1","value":{"buttons":orr_remote::mesh_navigation::MESH_SET_GOAL,"_pad":0,"origin":[0,0,0],"dir":[0,0,0]}})).unwrap();
    advance(&mut editor);
    assert_eq!(read(&editor).run.paused, 1);
    for _ in 0..5 {
        let view = advance(&mut editor);
        assert_eq!(read(&editor).run.paused, 1);
        assert_eq!(view.pin.goal, [FP::ZERO; 3]);
    }
    assert_eq!(fs::read(path).unwrap(), saved);
}
#[test]
fn unreachable_later_waypoint_rejects_actual_apply_without_history_frame_or_disk_changes() {
    let (_temp, mut h) = fixture();
    let path = h.state().1.path().unwrap();
    let bytes = fs::read(&path).unwrap();
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().clone();
    stage(&mut h, true);
    h.get_by_label("Apply waypoint patrol").click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.error.is_some());
    assert!(
        patrol::from_view(h.state().1.snapshot().unwrap().predicted())
            .unwrap()
            .is_none()
    );
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(fs::read(path).unwrap(), bytes);
}
