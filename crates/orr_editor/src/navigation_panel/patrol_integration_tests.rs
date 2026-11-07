//! CPU widget authoring -> one host transaction -> saved source -> immutable
//! public simulation. No rendered pixels, native window, or OS input claim.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_remote::navigation_yard3d::{self as remote, patrol};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
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
    Harness<'static, (NavigationPanel, Editor, TerrainPanel)>,
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
    terrain.save_for_editor(&editor).unwrap();
    let mut panel = NavigationPanel::default();
    panel.start = ["-3".into(), "0".into()];
    panel.goal = ["3".into(), "0".into()];
    panel.distance_per_tick = "1".into();
    panel.build_route(&mut editor, &terrain).unwrap();
    wait(&mut editor);
    assert!(editor.save());
    panel.sync_from_editor(&editor);
    let mut h = Harness::builder()
        .with_size([1500.0, 2000.0])
        .build_ui_state(
            |ui, state: &mut (NavigationPanel, Editor, TerrainPanel)| {
                state.0.show(ui, &mut state.1, &state.2);
            },
            (panel, editor, terrain),
        );
    h.run_steps(3);
    h.get_by_label("Point navigation").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Terrain waypoint patrol").click_accesskit();
    h.run_steps(3);
    (dir, h)
}
fn text(h: &mut Harness<'_, (NavigationPanel, Editor, TerrainPanel)>, label: &str, value: &str) {
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
fn stage(h: &mut Harness<'_, (NavigationPanel, Editor, TerrainPanel)>, bad: bool) {
    assert!(!h
        .get_by_label("Enable terrain waypoint patrol")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Enable terrain waypoint patrol")
        .click_accesskit();
    h.run_steps(3);
    h.get_by_label("Add terrain patrol waypoint")
        .click_accesskit();
    h.run_steps(3);
    for (label, value) in [
        ("Terrain patrol waypoint 1 X", "-2"),
        ("Terrain patrol waypoint 1 Z", "0"),
        ("Terrain patrol waypoint 2 X", if bad { "99" } else { "2" }),
        ("Terrain patrol waypoint 2 Z", "0"),
        ("Terrain patrol wait ticks", "1"),
    ] {
        text(h, label, value);
    }
    h.get_by_label("Loop terrain patrol").click_accesskit();
    h.run_steps(3);
}
fn config(editor: &Editor) -> patrol::TerrainPatrol {
    patrol::from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .config
}
#[test]
fn terrain_patrol_widgets_apply_undo_redo_save_reopen_and_public_simulation_runs_without_source_reads(
) {
    let (dir, mut h) = fixture();
    let scene = h.state().1.path().unwrap();
    let original = std::fs::read(&scene).unwrap();
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().entries.len();
    stage(&mut h, false);
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history().entries.len(), history);
    assert_eq!(std::fs::read(&scene).unwrap(), original);
    assert!(h
        .state()
        .0
        .play_blocker(&h.state().1, &h.state().2)
        .is_some());
    h.get_by_label("Apply terrain waypoint patrol")
        .click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
    let expected = config(&h.state().1);
    assert_eq!(
        (expected.count, expected.looped, expected.wait_ticks),
        (2, 1, 1)
    );
    assert_eq!(expected.points[0], [FP::from_int(-2), FP::ZERO]);
    assert_eq!(expected.points[1], [FP::from_int(2), FP::ZERO]);
    assert_eq!(h.state().1.history().entries.len(), history + 1);
    assert_eq!(std::fs::read(&scene).unwrap(), original);
    assert!(h.state_mut().1.undo());
    h.state_mut().1.sync();
    assert!(
        patrol::from_view(h.state().1.snapshot().unwrap().predicted())
            .unwrap()
            .is_none()
    );
    assert!(h.state_mut().1.redo());
    h.state_mut().1.sync();
    assert_eq!(config(&h.state().1), expected);
    assert!(h.state_mut().1.save());
    let saved = std::fs::read(&scene).unwrap();
    let mut fresh = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
    wait(&mut fresh);
    assert_eq!(config(&fresh), expected);
    let doc = remote::navigation_yard3d_doc_from_path(&scene).unwrap();
    let authored = *doc.frame().singleton::<NavigationScenePin>();
    let mut sim = orr_sim::Simulation::<remote::NavigationYard3D>::from_frame(
        doc.frame(),
        remote::YARD_TICK_RATE,
        remote::NAVIGATION_BUILD_ID,
    )
    .unwrap();
    let mut repeat = orr_sim::Simulation::<remote::NavigationYard3D>::from_frame(
        doc.frame(),
        remote::YARD_TICK_RATE,
        remote::NAVIGATION_BUILD_ID,
    )
    .unwrap();
    let source = dir.path().join("route.orrt");
    let source_bytes = std::fs::read(&source).unwrap();
    let hidden = dir.path().join("route.hidden");
    std::fs::rename(source, &hidden).unwrap();
    let mut saw_second = false;
    let mut looped_back = false;
    for _ in 0..128 {
        let inputs = orr_sim::TickInputs::new(sim.tick() + 1, remote::YARD_PLAYERS);
        sim.step(&inputs);
        repeat.step(&inputs);
        assert_eq!(
            sim.frame()
                .singleton::<remote::NavigationStepStatus>()
                .failed,
            0
        );
        assert_eq!(sim.frame().checksum(), repeat.frame().checksum());
        assert_eq!(*sim.frame().singleton::<NavigationScenePin>(), authored);
        let frame = orr_bridge::FrameView::of(sim.frame());
        let progress = patrol::from_view(frame).unwrap().unwrap();
        assert_eq!(progress.config, expected);
        let view = remote::navigation_from_view(frame).unwrap();
        saw_second |= progress.run.cursor == 1;
        looped_back |= saw_second
            && progress.run.cursor == 0
            && view.navigator.position() == [FP::from_int(-2), FP::ZERO, FP::ZERO];
        if looped_back {
            break;
        }
    }
    assert!(saw_second && looped_back);
    assert_eq!(std::fs::read(&hidden).unwrap(), source_bytes);
    assert_eq!(std::fs::read(scene).unwrap(), saved);
}
#[test]
fn terrain_patrol_unreachable_later_point_rejects_atomically_and_same_path_new_epoch_retires_dirty_form(
) {
    let (_dir, mut h) = fixture();
    let scene = h.state().1.path().unwrap();
    let bytes = std::fs::read(&scene).unwrap();
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().clone();
    stage(&mut h, true);
    h.get_by_label("Apply terrain waypoint patrol")
        .click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.error.is_some());
    assert!(
        patrol::from_view(h.state().1.snapshot().unwrap().predicted())
            .unwrap()
            .is_none()
    );
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(std::fs::read(&scene).unwrap(), bytes);
    let mut replacement = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
    wait(&mut replacement);
    h.state_mut().1 = replacement;
    h.run_steps(3);
    assert!(h
        .get_by_label("Apply terrain waypoint patrol")
        .accesskit_node()
        .is_disabled());
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Terrain patrol waypoint 2 X")
            .value()
            .as_deref(),
        Some("99")
    );
    assert_eq!(std::fs::read(PathBuf::from(scene)).unwrap(), bytes);
}
