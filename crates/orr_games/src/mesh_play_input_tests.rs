use super::{MeshNavigation, MeshScenePin, MeshStepStatus, MESH_SET_GOAL};
use crate::yard3d_game::YardInput;
use orr_fp::FP;
use orr_navigation::mesh::StaticMeshGraph;
use orr_navigation::{AgentProfile, NavigationStatus};
use orr_navigation_runtime::mesh::{AgentSpec, MeshRuntime};
use orr_sim::{PlayerSlot, Simulation, TickInputs};

fn flat_graph() -> StaticMeshGraph {
    let vertices = [
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [FP::ZERO, FP::ZERO, FP::from_int(4)],
        [FP::from_int(4), FP::ZERO, FP::ZERO],
        [FP::from_int(4), FP::ZERO, FP::from_int(4)],
    ];
    StaticMeshGraph::bake(&vertices, &[[0, 1, 2], [1, 3, 2]], AgentProfile::default()).unwrap()
}

fn specification() -> AgentSpec {
    AgentSpec {
        start: [FP::ZERO; 3],
        goal: [FP::from_int(4), FP::ZERO, FP::from_int(4)],
        max_vertical: FP::ZERO,
        distance_per_tick: FP::ONE,
    }
}

fn simulation(seed: u64) -> Simulation<MeshNavigation> {
    let graph = flat_graph();
    let agent_spec = specification();
    let pin = MeshScenePin::new(&graph, agent_spec).unwrap();
    let binding = pin.binding();
    let mut simulation = Simulation::<MeshNavigation>::new((), 60, seed);
    simulation.frame_mut().set_singleton(pin);
    MeshRuntime::admit(simulation.frame_mut(), &graph, binding, agent_spec).unwrap();
    simulation
}

fn request(goal: [FP; 3]) -> YardInput {
    YardInput {
        buttons: MESH_SET_GOAL,
        _pad: 0,
        origin: goal.map(|coordinate| i32::try_from(coordinate.raw()).unwrap()),
        dir: [0; 3],
    }
}

fn tick(simulation: &mut Simulation<MeshNavigation>, player_count: u8, inputs: &[(u8, YardInput)]) {
    let mut tick_inputs = TickInputs::new(simulation.tick() + 1, player_count);
    for (player, input) in inputs {
        tick_inputs.set_input(PlayerSlot(*player), *input);
    }
    simulation.step(&tick_inputs);
}

#[test]
fn goal_input_replans_once_keeps_pin_coherent_and_does_not_stall_when_held() {
    let mut simulation = simulation(41);
    let pin = *simulation.frame().singleton::<MeshScenePin>();
    let binding = pin.binding();
    let initial_position = MeshRuntime::agent(simulation.frame(), binding)
        .unwrap()
        .position;
    let requested = [FP::ZERO, FP::ZERO, FP::from_int(4)];

    tick(&mut simulation, 1, &[(0, request(requested))]);

    let updated_pin = *simulation.frame().singleton::<MeshScenePin>();
    let after_first = MeshRuntime::agent(simulation.frame(), binding).unwrap();
    assert_eq!(updated_pin.start, initial_position);
    assert_eq!(updated_pin.goal, requested);
    assert_eq!(after_first.spec, updated_pin.spec());
    assert_eq!(after_first.status, NavigationStatus::Moving);
    assert_eq!(after_first.distance, FP::ONE);

    tick(&mut simulation, 1, &[(0, request(requested))]);

    let after_second = MeshRuntime::agent(simulation.frame(), binding).unwrap();
    assert_eq!(after_second.distance, FP::from_int(2));
    assert_eq!(
        after_second.spec,
        simulation.frame().singleton::<MeshScenePin>().spec()
    );
    assert_eq!(simulation.frame().singleton::<MeshStepStatus>().failed, 0);
}

#[test]
fn malformed_and_off_graph_requests_leave_the_route_unchanged_while_it_advances() {
    let mut simulation = simulation(43);
    let pin_before = *simulation.frame().singleton::<MeshScenePin>();
    let binding = pin_before.binding();
    let runtime_before = MeshRuntime::agent(simulation.frame(), binding).unwrap();
    let mut malformed = request([FP::ZERO, FP::ZERO, FP::from_int(4)]);
    malformed._pad = 1;
    tick(&mut simulation, 1, &[(0, malformed)]);

    let pin_after_malformed = *simulation.frame().singleton::<MeshScenePin>();
    let runtime_after_malformed = MeshRuntime::agent(simulation.frame(), binding).unwrap();
    assert_eq!(pin_after_malformed, pin_before);
    assert_eq!(runtime_after_malformed.spec, runtime_before.spec);
    assert_eq!(runtime_after_malformed.distance, FP::ONE);

    let off_graph = [FP::from_int(8), FP::ZERO, FP::from_int(8)];
    tick(&mut simulation, 1, &[(0, request(off_graph))]);

    let pin_after_off_graph = *simulation.frame().singleton::<MeshScenePin>();
    let runtime_after_off_graph = MeshRuntime::agent(simulation.frame(), binding).unwrap();
    assert_eq!(pin_after_off_graph, pin_before);
    assert_eq!(runtime_after_off_graph.spec, runtime_before.spec);
    assert_eq!(runtime_after_off_graph.distance, FP::from_int(2));
    assert_eq!(simulation.frame().singleton::<MeshStepStatus>().failed, 0);
}

#[test]
fn the_lowest_connected_player_with_a_valid_goal_wins_deterministically() {
    let mut simulation = simulation(47);
    let first_goal = [FP::ZERO, FP::ZERO, FP::from_int(4)];
    let second_goal = [FP::from_int(4), FP::ZERO, FP::ZERO];
    let mut inputs = TickInputs::new(1, 2);
    inputs.set_input(PlayerSlot(0), request(first_goal));
    inputs.set_input(PlayerSlot(1), request(second_goal));
    simulation.step(&inputs);

    assert_eq!(
        simulation.frame().singleton::<MeshScenePin>().goal,
        first_goal
    );
    let binding = simulation.frame().singleton::<MeshScenePin>().binding();
    assert_eq!(
        MeshRuntime::agent(simulation.frame(), binding)
            .unwrap()
            .spec,
        simulation.frame().singleton::<MeshScenePin>().spec()
    );
}

#[test]
fn replaying_the_same_goal_input_sequence_has_identical_frame_checksums() {
    let mut first = simulation(53);
    let mut second = simulation(53);
    let first_goal = [FP::ZERO, FP::ZERO, FP::from_int(4)];
    let second_goal = [FP::from_int(4), FP::ZERO, FP::ZERO];

    for (tick_index, goal) in [first_goal, first_goal, second_goal, second_goal]
        .into_iter()
        .enumerate()
    {
        let mut first_inputs = TickInputs::new(tick_index as u64 + 1, 1);
        let mut second_inputs = TickInputs::new(tick_index as u64 + 1, 1);
        first_inputs.set_input(PlayerSlot(0), request(goal));
        second_inputs.set_input(PlayerSlot(0), request(goal));
        first.step(&first_inputs);
        second.step(&second_inputs);
        assert_eq!(first.checksum(), second.checksum());
    }
}

#[cfg(feature = "mesh-mission")]
#[test]
fn legacy_mesh_mission_checksum_is_unchanged_by_hazard_feature() {
    let mut sim = simulation(53);
    let mut actual = vec![sim.checksum()];
    for goal in [
        [FP::ZERO, FP::ZERO, FP::from_int(4)],
        [FP::from_int(4), FP::ZERO, FP::ZERO],
    ] {
        tick(&mut sim, 1, &[(0, request(goal))]);
        actual.push(sim.checksum());
    }
    // Captured from e1ba3ea with mesh-mission, before hazard implementation.
    assert_eq!(
        actual,
        [
            14079670917705819130,
            15298626416119841259,
            11243942499133049614
        ]
    );
}
