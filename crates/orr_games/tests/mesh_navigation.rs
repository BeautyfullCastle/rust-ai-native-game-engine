#![cfg(feature = "mesh-navigation")]

use bytemuck::Zeroable;
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::FP;
use orr_games::mesh_navigation_game::{
    validate_scene_frame, MeshNavigation, MeshScenePin, MeshStepStatus,
};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile, NavigationStatus};
use orr_navigation_runtime::mesh::{AgentSpec, MeshRuntime};
use orr_sim::{Simulation, TickInputs};

fn flat_graph() -> StaticMeshGraph {
    let vertices = [
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [FP::ZERO, FP::ZERO, FP::from_int(2)],
        [FP::from_int(2), FP::ZERO, FP::ZERO],
        [FP::from_int(2), FP::ZERO, FP::from_int(2)],
    ];
    StaticMeshGraph::bake(&vertices, &[[0, 1, 2], [1, 3, 2]], AgentProfile::default()).unwrap()
}

fn spec(start: [FP; 3], goal: [FP; 3]) -> AgentSpec {
    AgentSpec {
        start,
        goal,
        max_vertical: FP::ZERO,
        distance_per_tick: FP::ONE,
    }
}

#[test]
fn empty_pin_is_canonical_and_valid_pin_roundtrips() {
    let empty = MeshScenePin::zeroed();
    assert!(empty.is_empty());
    assert!(empty.graph().is_err());

    let graph = flat_graph();
    let pin = MeshScenePin::new(
        &graph,
        spec(
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [FP::from_int(2), FP::ZERO, FP::from_int(2)],
        ),
    )
    .unwrap();
    assert!(!pin.is_empty());
    assert_eq!(pin.graph().unwrap().to_bytes(), graph.to_bytes());
    assert_eq!(pin.graph_revision, graph.revision());
    assert_eq!(pin.binding().world_revision, pin.binding().graph_revision);
}

#[test]
fn pin_rejects_reserved_tail_hash_and_count_corruption() {
    let graph = flat_graph();
    let mut pin = MeshScenePin::new(
        &graph,
        spec(
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [FP::from_int(2), FP::ZERO, FP::from_int(2)],
        ),
    )
    .unwrap();

    pin.reserved = 1;
    assert!(pin.graph().is_err());
    pin.reserved = 0;

    let graph_length = pin.graph_len as usize;
    pin.graph_bytes[graph_length] = 1;
    assert!(pin.graph().is_err());
    pin.graph_bytes[graph_length] = 0;

    pin.graph_revision[0] ^= 1;
    assert!(pin.graph().is_err());
    pin.graph_revision = graph.revision();

    pin.graph_bytes[40..44].copy_from_slice(&1025u32.to_le_bytes());
    assert!(pin.graph().is_err());
}

#[test]
fn pin_rejects_out_of_bounds_graph_and_invalid_or_unreachable_specs() {
    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    for face_index in 0..300u32 {
        let x = FP::from_int((face_index * 3) as i32);
        let first = vertices.len() as u32;
        vertices.extend([
            [x, FP::ZERO, FP::ZERO],
            [x, FP::ZERO, FP::from_int(2)],
            [x + FP::from_int(2), FP::ZERO, FP::ZERO],
        ]);
        faces.push([first, first + 1, first + 2]);
    }
    let large_graph = StaticMeshGraph::bake(&vertices, &faces, AgentProfile::default()).unwrap();
    assert!(MeshScenePin::new(
        &large_graph,
        spec(
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [FP::from_int(2), FP::ZERO, FP::ZERO],
        ),
    )
    .is_err());

    let graph = flat_graph();
    let mut invalid_speed = spec(
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [FP::from_int(2), FP::ZERO, FP::from_int(2)],
    );
    invalid_speed.distance_per_tick = FP::ZERO;
    assert!(MeshScenePin::new(&graph, invalid_speed).is_err());
    let mut invalid_vertical = invalid_speed;
    invalid_vertical.distance_per_tick = FP::ONE;
    invalid_vertical.max_vertical = FP::from_raw(-1);
    assert!(MeshScenePin::new(&graph, invalid_vertical).is_err());

    let separated_vertices = [
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [FP::ZERO, FP::ZERO, FP::from_int(2)],
        [FP::from_int(2), FP::ZERO, FP::ZERO],
        [FP::from_int(10), FP::ZERO, FP::from_int(10)],
        [FP::from_int(10), FP::ZERO, FP::from_int(12)],
        [FP::from_int(12), FP::ZERO, FP::from_int(10)],
    ];
    let separated_graph = StaticMeshGraph::bake(
        &separated_vertices,
        &[[0, 1, 2], [3, 4, 5]],
        AgentProfile::default(),
    )
    .unwrap();
    assert!(MeshScenePin::new(
        &separated_graph,
        spec(
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [FP::from_int(10), FP::ZERO, FP::from_int(10)],
        ),
    )
    .is_err());
}

#[test]
fn admitted_mesh_agent_advances_in_simulation_frame_and_arrives() {
    let graph = flat_graph();
    let agent_spec = spec(
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [FP::from_int(2), FP::ZERO, FP::from_int(2)],
    );
    let pin = MeshScenePin::new(&graph, agent_spec).unwrap();
    let binding = pin.binding();
    let mut simulation = Simulation::<MeshNavigation>::new((), 60, 7);
    simulation.frame_mut().set_singleton::<MeshScenePin>(pin);
    MeshRuntime::admit(simulation.frame_mut(), &graph, binding, agent_spec).unwrap();
    validate_scene_frame(simulation.frame()).unwrap();

    for _ in 0..16 {
        let inputs = TickInputs::new(simulation.tick() + 1, 0);
        simulation.step(&inputs);
    }

    validate_scene_frame(simulation.frame()).unwrap();
    let readback = MeshRuntime::agent(simulation.frame(), binding).unwrap();
    assert_eq!(readback.status, NavigationStatus::Arrived);
    assert_eq!(readback.position, agent_spec.goal);
    assert_eq!(simulation.frame().singleton::<MeshStepStatus>().failed, 0);
}

#[test]
fn malformed_success_status_fails_closed_and_latches() {
    let graph = flat_graph();
    let agent_spec = spec(
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [FP::from_int(2), FP::ZERO, FP::from_int(2)],
    );
    let pin = MeshScenePin::new(&graph, agent_spec).unwrap();
    let binding = pin.binding();
    let mut simulation = Simulation::<MeshNavigation>::new((), 60, 9);
    simulation.frame_mut().set_singleton(pin);
    MeshRuntime::admit(simulation.frame_mut(), &graph, binding, agent_spec).unwrap();
    let position_before = MeshRuntime::agent(simulation.frame(), binding)
        .unwrap()
        .position;

    let mut malformed_status = MeshStepStatus::zeroed();
    malformed_status.error_len = 1;
    malformed_status.error[0] = b'x';
    simulation.frame_mut().set_singleton(malformed_status);
    simulation.step(&TickInputs::new(simulation.tick() + 1, 0));

    let failed_status = *simulation.frame().singleton::<MeshStepStatus>();
    assert_eq!(failed_status.failed, 1);
    assert_eq!(
        failed_status.message(),
        "MeshNavigation step status is malformed"
    );
    assert_eq!(
        MeshRuntime::agent(simulation.frame(), binding)
            .unwrap()
            .position,
        position_before
    );
    assert!(validate_scene_frame(simulation.frame()).is_err());

    simulation.step(&TickInputs::new(simulation.tick() + 1, 0));
    assert_eq!(
        simulation.frame().singleton::<MeshStepStatus>().failed_tick,
        1
    );
    assert_eq!(
        MeshRuntime::agent(simulation.frame(), binding)
            .unwrap()
            .position,
        position_before
    );
}

#[test]
fn scene_validation_rejects_a_wrong_frame_registry_without_panicking() {
    let registry = ComponentRegistryBuilder::new().build();
    let frame = Frame::new(registry);
    assert!(validate_scene_frame(&frame).is_err());
}
