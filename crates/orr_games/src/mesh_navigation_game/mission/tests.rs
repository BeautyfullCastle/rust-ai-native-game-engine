use super::{
    admit, read_run, update, validate_config, validate_run, MeshMission, MeshMissionRun,
    MissionPoint, MAX_MISSION_POINTS,
};
use crate::mesh_navigation_game::{MeshNavigation, MeshScenePin};
use bytemuck::Zeroable;
use orr_ecs::Frame;
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
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

fn spec(start: [FP; 3], goal: [FP; 3], speed: FP) -> AgentSpec {
    AgentSpec {
        start,
        goal,
        max_vertical: FP::ZERO,
        distance_per_tick: speed,
    }
}

fn mission(graph: &StaticMeshGraph, count: u32, point: [FP; 3], exit: [FP; 3]) -> MeshMission {
    let mut config = MeshMission::zeroed();
    config.enabled = 1;
    config.count = count;
    config.points[0] = MissionPoint {
        position: point,
        radius: FP::HALF,
    };
    config.exit = MissionPoint {
        position: exit,
        radius: FP::HALF,
    };
    config.graph_revision = graph.revision();
    config
}

fn admitted_frame(
    config: MeshMission,
    graph: &StaticMeshGraph,
    agent_spec: AgentSpec,
) -> (Simulation<MeshNavigation>, MeshScenePin) {
    let mut simulation = Simulation::<MeshNavigation>::new((), 60, 17);
    let pin = MeshScenePin::new(graph, agent_spec).unwrap();
    let frame = simulation.frame_mut();
    frame.set_singleton(pin);
    frame.set_singleton(config);
    MeshRuntime::admit(frame, graph, pin.binding(), agent_spec).unwrap();
    (simulation, pin)
}

fn assert_all_run_zero(frame: &Frame) {
    assert_eq!(read_run(frame).unwrap(), MeshMissionRun::zeroed());
}

#[test]
fn disabled_mission_is_canonical_zero_and_enabled_config_is_graph_bound() {
    let graph = flat_graph();
    validate_config(&MeshMission::zeroed(), &graph, FP::ZERO).unwrap();
    let mut invalid_disabled = MeshMission::zeroed();
    invalid_disabled.exit.radius = FP::EPSILON;
    assert!(validate_config(&invalid_disabled, &graph, FP::ZERO).is_err());

    let mut config = mission(
        &graph,
        1,
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [FP::from_int(2), FP::ZERO, FP::from_int(2)],
    );
    validate_config(&config, &graph, FP::ZERO).unwrap();
    config.graph_revision[0] ^= 1;
    assert!(validate_config(&config, &graph, FP::ZERO).is_err());
}

#[test]
fn config_rejects_bad_count_radius_projection_coordinates_and_unused_points() {
    let graph = flat_graph();
    let baseline = mission(
        &graph,
        1,
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [FP::from_int(2), FP::ZERO, FP::from_int(2)],
    );
    for count in [0, MAX_MISSION_POINTS as u32 + 1] {
        let mut invalid = baseline;
        invalid.count = count;
        assert!(validate_config(&invalid, &graph, FP::ZERO).is_err());
    }
    let mut invalid = baseline;
    invalid.points[0].radius = FP::ZERO;
    assert!(validate_config(&invalid, &graph, FP::ZERO).is_err());
    invalid = baseline;
    invalid.points[0].radius = FP::from_int(5);
    assert!(validate_config(&invalid, &graph, FP::ZERO).is_err());
    invalid = baseline;
    invalid.points[0].position[1] = FP::ONE;
    assert!(validate_config(&invalid, &graph, FP::ZERO).is_err());
    invalid = baseline;
    invalid.points[1] = baseline.exit;
    assert!(validate_config(&invalid, &graph, FP::ZERO).is_err());
    invalid = baseline;
    invalid.exit.position[0] = FP::from_int(257);
    assert!(validate_config(&invalid, &graph, FP::ZERO).is_err());
    assert!(validate_config(&baseline, &graph, FP::from_raw(-1)).is_err());
}

#[test]
fn run_mask_and_win_flag_are_strictly_validated() {
    let graph = flat_graph();
    let config = mission(
        &graph,
        2,
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [FP::from_int(2), FP::ZERO, FP::from_int(2)],
    );
    validate_run(&config, &MeshMissionRun::zeroed()).unwrap();
    validate_run(
        &config,
        &MeshMissionRun {
            collected: 0b11,
            won: 1,
        },
    )
    .unwrap();
    for invalid in [
        MeshMissionRun {
            collected: 0b100,
            won: 0,
        },
        MeshMissionRun {
            collected: 0b01,
            won: 1,
        },
        MeshMissionRun {
            collected: 0,
            won: 2,
        },
    ] {
        assert!(validate_run(&config, &invalid).is_err());
    }
}

#[test]
fn authored_run_must_be_zero_and_successful_step_collects_and_wins() {
    let graph = flat_graph();
    let start = [FP::ZERO, FP::ZERO, FP::ZERO];
    let goal = [FP::from_int(2), FP::ZERO, FP::from_int(2)];
    let config = mission(&graph, 1, goal, goal);
    let spec = spec(start, goal, FP::from_int(10));
    let (mut simulation, pin) = admitted_frame(config, &graph, spec);
    let frame = simulation.frame_mut();
    admit(frame, pin).unwrap();
    frame.set_singleton(MeshMissionRun {
        collected: 1,
        won: 0,
    });
    assert!(admit(frame, pin).is_err());
    frame.set_singleton(MeshMissionRun::zeroed());

    simulation.step(&TickInputs::new(1, 1));
    assert_eq!(
        read_run(simulation.frame()).unwrap(),
        MeshMissionRun {
            collected: 1,
            won: 1,
        }
    );
    let frozen = read_run(simulation.frame()).unwrap();
    update(simulation.frame_mut(), &pin).unwrap();
    assert_eq!(read_run(simulation.frame()).unwrap(), frozen);
}

#[test]
fn mission_update_does_not_mutate_frame_for_disabled_config() {
    let graph = flat_graph();
    let spec = spec(
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [FP::from_int(2), FP::ZERO, FP::from_int(2)],
        FP::ONE,
    );
    let (mut simulation, pin) = admitted_frame(MeshMission::zeroed(), &graph, spec);
    let frame = simulation.frame_mut();
    let before = frame.checksum();
    update(frame, &pin).unwrap();
    assert_eq!(frame.checksum(), before);
    assert_all_run_zero(frame);
}

#[test]
fn mission_frame_setup_registers_the_runtime_only_run_state() {
    let simulation = Simulation::<MeshNavigation>::new((), 60, 19);
    assert_all_run_zero(simulation.frame());
}
