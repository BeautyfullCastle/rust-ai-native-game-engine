//! Actual CPU widgets -> ordered saved route -> public simulation; no GPU/window.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_remote::mesh_navigation::{self as remote, patrol};
fn fixture(game: EditorGame) -> (tempfile::TempDir, Harness<'static, (Panel, Editor)>) {
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
    let yaml = document.save_yaml();
    #[cfg(feature = "mesh-hazards")]
    let yaml = if game.is_mesh_hazards() {
        orr_remote::mesh_navigation::hazards::hazards_doc_from_yaml(&yaml)
            .unwrap()
            .save_yaml()
    } else {
        yaml
    };
    fs::write(&path, yaml).unwrap();
    let mut editor = Editor::open_game(&path, game).unwrap();
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
    if game.is_mesh_navigation() {
        h.get_by_label("Static-mesh navigation").click_accesskit();
        h.run_steps(3);
    }
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

fn click(h: &mut Harness<'_, (Panel, Editor)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}
fn games() -> Vec<EditorGame> {
    let games = vec![EditorGame::MeshNavigation];
    #[cfg(feature = "mesh-hazards")]
    let games = {
        let mut games = games;
        games.push(EditorGame::MeshHazardsV1);
        games
    };
    games
}
fn stage(h: &mut Harness<'_, (Panel, Editor)>, bad: bool) {
    click(h, "Enable waypoint patrol");
    click(h, "Add patrol waypoint");
    click(h, "Add patrol waypoint");
    for (index, x) in [(1, "1"), (2, if bad { "99" } else { "2" }), (3, "3")] {
        for (axis, value) in [("X", x), ("Y", "0"), ("Z", "0")] {
            text(h, &format!("Patrol waypoint {index} {axis}"), value);
        }
    }
    // Exercise InsertBefore without changing authored values, then remove its
    // duplicate; move the final point twice to produce C,A,B from A,B,C.
    click(h, "Insert before patrol waypoint 2");
    click(h, "Remove patrol waypoint 2");
    click(h, "Move patrol waypoint 3 up");
    click(h, "Move patrol waypoint 2 up");
}
macro_rules! prove_order {
    ($doc:expr,$game:ty,$build:expr,$expected:expr) => {{
        let mut sim =
            orr_sim::Simulation::<$game>::from_frame($doc.frame(), remote::YARD_TICK_RATE, $build)
                .unwrap();
        let mut visited = Vec::new();
        for tick in 1..=160 {
            sim.step(&orr_sim::TickInputs::new(tick, 1));
            let frame = orr_bridge::FrameView::of(sim.frame());
            assert_eq!(
                frame.singleton::<remote::MeshStepStatus>().failed,
                0,
                "runtime must not fail while checking arrival order"
            );
            let run = patrol::from_view(frame).unwrap().unwrap();
            let view = remote::mesh_from_view(frame).unwrap();
            let cursor = run.run.cursor as usize;
            if run.run.arrived == 1
                && view.agent.position == $expected.points[cursor]
                && !visited.contains(&cursor)
            {
                visited.push(cursor);
            }
            if run.run.completed == 1 {
                break;
            }
        }
        assert_eq!(
            visited,
            vec![0, 1, 2],
            "saved reordered points must be visited in their new order"
        );
        assert_eq!(
            remote::mesh_from_view(orr_bridge::FrameView::of(sim.frame()))
                .unwrap()
                .agent
                .position,
            $expected.points[2]
        );
    }};
}
#[test]
fn reordered_mesh_and_hazards_widgets_apply_history_save_reopen_and_runtime_arrival_order() {
    for game in games() {
        let (_temp, mut h) = fixture(game);
        let path = h.state().1.path().unwrap();
        let saved = fs::read(&path).unwrap();
        let checksum = h.state().1.checksum();
        let history = h.state().1.history().entries.len();
        stage(&mut h, false);
        assert_eq!(h.state().1.checksum(), checksum);
        assert_eq!(h.state().1.history().entries.len(), history);
        assert_eq!(fs::read(&path).unwrap(), saved);
        click(&mut h, "Pick patrol waypoint 1");
        let ctx = h.ctx.clone();
        {
            let (panel, editor) = h.state_mut();
            assert_eq!(
                panel.take_patrol_pick_request(),
                Some(super::PatrolPickRequest::Pick(0))
            );
            panel.sync_patrol_picking(editor, &ctx, false);
            panel.begin_patrol_pick(editor, 0).unwrap();
        }
        h.run_steps(2);
        assert!(
            h.get_by_label("Move patrol waypoint 2 up")
                .accesskit_node()
                .is_disabled()
        );
        click(&mut h, "Move patrol waypoint 2 up");
        h.state_mut().0.cancel_patrol_pick();
        h.run_steps(2);
        assert!(!h.state().0.patrol_pick_busy());
        assert!(
            h.state().0.patrol_pick_owns_input(),
            "cancellation still owns the release fence"
        );
        assert!(
            h.get_by_label("Move patrol waypoint 2 up")
                .accesskit_node()
                .is_disabled()
        );
        click(&mut h, "Move patrol waypoint 2 up");
        let ctx = h.ctx.clone();
        {
            let (panel, editor) = h.state_mut();
            panel.sync_patrol_picking(editor, &ctx, false);
        }
        h.run_steps(2);
        assert!(!h.state().0.patrol_pick_owns_input());
        assert!(
            !h.get_by_label("Move patrol waypoint 2 up")
                .accesskit_node()
                .is_disabled()
        );
        click(&mut h, "Apply waypoint patrol");
        assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
        let config = patrol::from_view(h.state().1.snapshot().unwrap().predicted())
            .unwrap()
            .unwrap()
            .config;
        assert_eq!(
            config.points[..3],
            [
                [FP::from_int(3), FP::ZERO, FP::ZERO],
                [FP::ONE, FP::ZERO, FP::ZERO],
                [FP::from_int(2), FP::ZERO, FP::ZERO]
            ]
        );
        assert_eq!(h.state().1.history().entries.len(), history + 1);
        assert_eq!(fs::read(&path).unwrap(), saved);
        assert!(h.state_mut().1.undo());
        h.state_mut().1.sync();
        assert!(
            patrol::from_view(h.state().1.snapshot().unwrap().predicted())
                .unwrap()
                .is_none()
        );
        assert!(h.state_mut().1.redo());
        h.state_mut().1.sync();
        assert_eq!(
            patrol::from_view(h.state().1.snapshot().unwrap().predicted())
                .unwrap()
                .unwrap()
                .config,
            config
        );
        assert!(h.state_mut().1.save());
        if game.is_mesh_navigation() {
            let doc = remote::mesh_doc_from_path(&path).unwrap();
            assert_eq!(
                patrol::from_view(orr_bridge::FrameView::of(doc.frame()))
                    .unwrap()
                    .unwrap()
                    .config,
                config
            );
            prove_order!(
                doc,
                remote::MeshNavigation,
                remote::MESH_NAVIGATION_BUILD_ID,
                config
            );
        }
        #[cfg(feature = "mesh-hazards")]
        if game.is_mesh_hazards() {
            let doc = remote::hazards::hazards_doc_from_path(&path).unwrap();
            assert_eq!(
                patrol::from_view(orr_bridge::FrameView::of(doc.frame()))
                    .unwrap()
                    .unwrap()
                    .config,
                config
            );
            prove_order!(
                doc,
                remote::hazards::MeshHazardNavigation,
                remote::hazards::MESH_HAZARDS_BUILD_ID,
                config
            );
        }
    }
}
#[test]
fn invalid_reordered_leg_leaves_frame_history_files_unchanged() {
    for game in games() {
        let (_temp, mut h) = fixture(game);
        let path = h.state().1.path().unwrap();
        let saved = fs::read(&path).unwrap();
        let checksum = h.state().1.checksum();
        let history = h.state().1.history().clone();
        stage(&mut h, true);
        click(&mut h, "Apply waypoint patrol");
        assert!(h.state().0.error.is_some());
        assert_eq!(h.state().1.checksum(), checksum);
        assert_eq!(h.state().1.history(), &history);
        assert_eq!(fs::read(&path).unwrap(), saved);
        // Current draft still contains the invalid point; structural commands
        // cannot become a back door around full route admission.
        assert!(
            patrol::from_view(h.state().1.snapshot().unwrap().predicted())
                .unwrap()
                .is_none()
        );
    }
}
