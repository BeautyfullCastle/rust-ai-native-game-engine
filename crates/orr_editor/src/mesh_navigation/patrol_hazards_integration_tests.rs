//! Actual CPU egui authoring and the public Hazards simulation. No OS window,
//! GPU rendering, physical input, or managed Editor Play stepping is exercised.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_remote::mesh_navigation::{hazards, patrol, YARD_PLAYERS, YARD_TICK_RATE};
fn p(x: i32) -> [FP; 3] {
    [FP::from_int(x), FP::ZERO, FP::ZERO]
}
fn fixture(collision: bool) -> (tempfile::TempDir, Harness<'static, (Panel, Editor)>) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("patrol-hazards.scene.yaml");
    let point = |x, z| [FP::from_int(x), FP::ZERO, FP::from_int(z)];
    let graph = StaticMeshGraph::bake(
        &[point(0, 0), point(0, 4), point(4, 0), point(4, 4)],
        &[[0, 1, 2], [2, 1, 3]],
        AgentProfile::default(),
    )
    .unwrap();
    let mut base = orr_remote::mesh_navigation::authoring::create_mesh_scene(
        &graph,
        AgentSpec {
            start: p(0),
            goal: p(4),
            max_vertical: FP::ZERO,
            distance_per_tick: FP::ONE,
        },
    )
    .unwrap();
    let mut doc = hazards::hazards_doc_from_yaml(&base.save_yaml()).unwrap();
    if collision {
        let mut config = hazards::MeshHazards::default();
        config.enabled = 1;
        config.count = 1;
        config.graph_revision = graph.revision();
        config.spheres[0] = hazards::HazardSphere {
            center: p(2),
            radius: FP::from_raw(FP::SCALE / 4),
        };
        let value = doc
            .types()
            .get(hazards::HAZARDS_NAME)
            .unwrap()
            .read(orr_reflect::bytemuck::bytes_of(&config));
        doc.apply(
            orr_edit::Op::SetSingletonField {
                singleton: hazards::HAZARDS_NAME.into(),
                path: String::new(),
                value,
            },
            orr_edit::Origin::User,
        )
        .unwrap();
    }
    fs::write(&path, doc.save_yaml()).unwrap();
    let mut editor = Editor::open_game(&path, EditorGame::MeshHazardsV1).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.sync(&editor);
    let mut h = Harness::builder()
        .with_size([1600.0, 3000.0])
        .build_ui_state(
            |ui, state: &mut (Panel, Editor)| state.0.show(ui, &mut state.1),
            (panel, editor),
        );
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
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, label)
            .value()
            .as_deref(),
        Some(value)
    );
}
fn stage(h: &mut Harness<'_, (Panel, Editor)>) {
    assert!(!h
        .get_by_label("Enable waypoint patrol")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Enable waypoint patrol").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Add patrol waypoint").click_accesskit();
    h.run_steps(3);
    for (label, value) in [
        ("Patrol waypoint 1 X", "1"),
        ("Patrol waypoint 1 Y", "0"),
        ("Patrol waypoint 1 Z", "0"),
        ("Patrol waypoint 2 X", "3"),
        ("Patrol waypoint 2 Y", "0"),
        ("Patrol waypoint 2 Z", "0"),
        ("Patrol wait ticks", "1"),
    ] {
        text(h, label, value);
    }
    h.get_by_label("Loop patrol").click_accesskit();
    h.run_steps(3);
}
fn config(editor: &Editor) -> patrol::MeshPatrol {
    patrol::from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .config
}
fn sim(path: &std::path::Path) -> orr_sim::Simulation<hazards::MeshHazardNavigation> {
    let doc = hazards::hazards_doc_from_path(path).unwrap();
    orr_sim::Simulation::<hazards::MeshHazardNavigation>::from_frame(
        doc.frame(),
        YARD_TICK_RATE,
        hazards::MESH_HAZARDS_BUILD_ID,
    )
    .unwrap()
}
#[test]
fn hazards_actual_patrol_widgets_apply_history_save_reopen_and_public_simulation_visits_multiple_waypoints(
) {
    let (_temp, mut h) = fixture(false);
    let path = h.state().1.path().unwrap();
    let before = fs::read(&path).unwrap();
    let checksum = h.state().1.checksum();
    let history = h.state().1.history().entries.len();
    stage(&mut h);
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history().entries.len(), history);
    assert_eq!(fs::read(&path).unwrap(), before);
    h.get_by_label("Apply waypoint patrol").click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
    let expected = config(&h.state().1);
    assert_eq!(
        (expected.count, expected.looped, expected.wait_ticks),
        (2, 1, 1)
    );
    assert_eq!(expected.points[0], p(1));
    assert_eq!(expected.points[1], p(3));
    assert_eq!(h.state().1.history().entries.len(), history + 1);
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
    let saved = fs::read(&path).unwrap();
    let mut reopened = Editor::open_game(&path, EditorGame::MeshHazardsV1).unwrap();
    reopened.sync();
    assert_eq!(reopened.game(), EditorGame::MeshHazardsV1);
    assert_eq!(config(&reopened), expected);
    let mut simulation = sim(&path);
    let mut visited = std::collections::BTreeSet::new();
    let mut saw_second = false;
    let mut looped_back = false;
    for _ in 0..12 {
        simulation.step(&orr_sim::TickInputs::new(
            simulation.tick() + 1,
            YARD_PLAYERS,
        ));
        assert_eq!(
            simulation
                .frame()
                .singleton::<orr_remote::mesh_navigation::MeshStepStatus>()
                .failed,
            0,
            "simulation step must succeed before checking movement or terminal freeze"
        );
        let frame = orr_bridge::FrameView::of(simulation.frame());
        let view = mesh_from_view(frame).unwrap();
        visited.insert(view.agent.position[0].raw());
        let progress = patrol::from_view(frame).unwrap().unwrap().run;
        saw_second |= progress.cursor == 1;
        looped_back |= saw_second && progress.cursor == 0 && view.agent.position == p(1);
        assert_eq!(
            *simulation.frame().singleton::<hazards::MeshHazardRun>(),
            Default::default()
        );
        assert_eq!(patrol::from_view(frame).unwrap().unwrap().config, expected);
    }
    assert!(visited.contains(&FP::from_int(1).raw()));
    assert!(visited.contains(&FP::from_int(3).raw()));
    assert!(
        saw_second && looped_back,
        "patrol must advance to waypoint two and loop back to waypoint one"
    );
    assert_eq!(fs::read(path).unwrap(), saved);
}
#[test]
fn hazards_patrol_collision_on_later_segment_rolls_back_and_terminal_ticks_freeze_route_state() {
    let (_temp, mut h) = fixture(true);
    let path = h.state().1.path().unwrap();
    stage(&mut h);
    h.get_by_label("Apply waypoint patrol").click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
    assert!(h.state_mut().1.save());
    let saved = fs::read(&path).unwrap();
    let mut simulation = sim(&path);
    let mut visited_first = false;
    let mut before = None;
    for _ in 0..12 {
        let old_frame = orr_bridge::FrameView::of(simulation.frame());
        let old_mesh = mesh_from_view(old_frame).unwrap();
        let old_patrol = patrol::from_view(old_frame).unwrap().unwrap().run;
        simulation.step(&orr_sim::TickInputs::new(
            simulation.tick() + 1,
            YARD_PLAYERS,
        ));
        assert_eq!(
            simulation
                .frame()
                .singleton::<orr_remote::mesh_navigation::MeshStepStatus>()
                .failed,
            0,
            "simulation step must succeed before checking movement or terminal freeze"
        );
        let frame = orr_bridge::FrameView::of(simulation.frame());
        let mesh = mesh_from_view(frame).unwrap();
        if mesh.agent.position == p(1) {
            visited_first = true;
        }
        if simulation
            .frame()
            .singleton::<hazards::MeshHazardRun>()
            .lost
            == 1
        {
            assert!(visited_first);
            assert_eq!(mesh.agent.position, old_mesh.agent.position);
            assert_eq!(mesh.pin, old_mesh.pin);
            assert_eq!(patrol::from_view(frame).unwrap().unwrap().run, old_patrol);
            before = Some((
                mesh.pin,
                mesh.agent.position,
                old_patrol,
                *simulation.frame().singleton::<hazards::MeshHazardRun>(),
            ));
            break;
        }
    }
    let (pin, position, run, loss) =
        before.expect("later patrol segment must hit the authored sphere");
    assert_eq!(loss.hit_index, 0);
    assert!(loss.lost_tick > 1);
    for _ in 0..5 {
        simulation.step(&orr_sim::TickInputs::new(
            simulation.tick() + 1,
            YARD_PLAYERS,
        ));
        assert_eq!(
            simulation
                .frame()
                .singleton::<orr_remote::mesh_navigation::MeshStepStatus>()
                .failed,
            0,
            "simulation step must succeed before checking movement or terminal freeze"
        );
        let frame = orr_bridge::FrameView::of(simulation.frame());
        let mesh = mesh_from_view(frame).unwrap();
        assert_eq!(mesh.pin, pin);
        assert_eq!(mesh.agent.position, position);
        assert_eq!(patrol::from_view(frame).unwrap().unwrap().run, run);
        assert_eq!(
            *simulation.frame().singleton::<hazards::MeshHazardRun>(),
            loss
        );
    }
    assert_eq!(fs::read(path).unwrap(), saved);
}
