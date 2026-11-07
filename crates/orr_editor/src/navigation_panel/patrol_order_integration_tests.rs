//! CPU widget authoring -> one host transaction -> saved source -> immutable
//! public simulation. No rendered pixels, native window, or OS input claim.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_remote::navigation_yard3d::{self as remote, patrol};
use std::time::{Duration, Instant};
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
        .with_size([1600.0, 4600.0])
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
fn click(h: &mut Harness<'_, (NavigationPanel, Editor, TerrainPanel)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
fn ordered_draft(h: &mut Harness<'_, (NavigationPanel, Editor, TerrainPanel)>) {
    click(h, "Enable terrain waypoint patrol");
    click(h, "Add terrain patrol waypoint");
    click(h, "Add terrain patrol waypoint");
    for (i, x) in [(1, "-2"), (2, "0"), (3, "2")] {
        text(h, &format!("Terrain patrol waypoint {i} X"), x);
        text(h, &format!("Terrain patrol waypoint {i} Z"), "0");
    }
    // A/B/C -> A/B/B/C -> A/B/C -> C/A/B, using actual structural buttons.
    click(h, "Insert before patrol waypoint 2");
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Terrain patrol waypoint 3 X")
            .value()
            .as_deref(),
        Some("0")
    );
    click(h, "Remove terrain patrol waypoint 3");
    click(h, "Move patrol waypoint 3 up");
    click(h, "Move patrol waypoint 2 up");
    for (i, x) in [(1, "2"), (2, "-2"), (3, "0")] {
        assert_eq!(
            h.get_by_role_and_label(Role::TextInput, &format!("Terrain patrol waypoint {i} X"))
                .value()
                .as_deref(),
            Some(x)
        );
    }
    text(h, "Terrain patrol wait ticks", "2");
    click(h, "Loop terrain patrol");
}
fn config(editor: &Editor) -> patrol::TerrainPatrol {
    patrol::from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .config
}
#[test]
fn structural_order_applies_as_one_edit_and_saved_public_simulation_follows_order_and_wait() {
    let (dir, mut h) = fixture();
    let scene = h.state().1.path().unwrap();
    let original = std::fs::read(&scene).unwrap();
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().entries.len();
    ordered_draft(&mut h);
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history().entries.len(), history);
    assert_eq!(std::fs::read(&scene).unwrap(), original);
    click(&mut h, "Apply terrain waypoint patrol");
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
    let expected = config(&h.state().1);
    assert_eq!(
        (expected.count, expected.looped, expected.wait_ticks),
        (3, 1, 2)
    );
    assert_eq!(
        expected.points[..3],
        [
            [FP::from_int(2), FP::ZERO],
            [FP::from_int(-2), FP::ZERO],
            [FP::ZERO; 2]
        ]
    );
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
    let pin = *doc.frame().singleton::<NavigationScenePin>();
    let mut sim = orr_sim::Simulation::<remote::NavigationYard3D>::from_frame(
        doc.frame(),
        remote::YARD_TICK_RATE,
        remote::NAVIGATION_BUILD_ID,
    )
    .unwrap();
    let source = dir.path().join("route.orrt");
    let sourcebytes = std::fs::read(&source).unwrap();
    let hidden = dir.path().join("terrain-hidden");
    std::fs::rename(&source, &hidden).unwrap();
    let mut arrivals = Vec::new();
    let mut waits = Vec::new();
    for _ in 0..512 {
        sim.step(&orr_sim::TickInputs::new(
            sim.tick() + 1,
            remote::YARD_PLAYERS,
        ));
        assert_eq!(
            sim.frame()
                .singleton::<remote::NavigationStepStatus>()
                .failed,
            0
        );
        assert_eq!(*sim.frame().singleton::<NavigationScenePin>(), pin);
        let frame = orr_bridge::FrameView::of(sim.frame());
        let p = patrol::from_view(frame).unwrap().unwrap();
        assert_eq!(p.config, expected);
        if p.run.arrived == 1 {
            let position = remote::navigation_from_view(frame)
                .unwrap()
                .navigator
                .position();
            let target = expected.points[p.run.cursor as usize];
            assert_eq!([position[0], position[2]], target);
            if arrivals.last() != Some(&p.run.cursor) {
                arrivals.push(p.run.cursor);
            }
            if arrivals.len() == 1 {
                waits.push(p.run.wait_remaining);
            }
        }
        if arrivals == [0, 1, 2, 0] {
            break;
        }
    }
    assert_eq!(arrivals, [0, 1, 2, 0]);
    assert!(waits.windows(3).any(|w| w == [2, 1, 0]), "{waits:?}");
    assert_eq!(std::fs::read(&hidden).unwrap(), sourcebytes);
    assert_eq!(std::fs::read(scene).unwrap(), saved);
}
#[test]
fn invalid_reordered_late_leg_rejects_without_frame_history_or_file_changes() {
    let (_dir, mut h) = fixture();
    let scene = h.state().1.path().unwrap();
    let original = std::fs::read(&scene).unwrap();
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().clone();
    ordered_draft(&mut h);
    text(&mut h, "Terrain patrol waypoint 3 X", "99");
    click(&mut h, "Move patrol waypoint 3 up");
    click(&mut h, "Move patrol waypoint 2 down");
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Terrain patrol waypoint 3 X")
            .value()
            .as_deref(),
        Some("99")
    );
    click(&mut h, "Apply terrain waypoint patrol");
    assert!(h.state().0.error.is_some());
    assert!(
        patrol::from_view(h.state().1.snapshot().unwrap().predicted())
            .unwrap()
            .is_none()
    );
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(std::fs::read(scene).unwrap(), original);
}
