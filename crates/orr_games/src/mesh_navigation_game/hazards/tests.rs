use super::*;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_navigation_runtime::mesh::AgentSpec;
use orr_sim::{PlayerSlot, Simulation, TickInputs};

fn p(x: i32, z: i32) -> [FP; 3] {
    [FP::from_int(x), FP::ZERO, FP::from_int(z)]
}

fn graph() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[p(0, 0), p(0, 8), p(8, 0), p(8, 8)],
        &[[0, 1, 2], [1, 3, 2]],
        AgentProfile::default(),
    )
    .unwrap()
}

fn sphere(center: [FP; 3]) -> HazardSphere {
    HazardSphere {
        center,
        radius: FP::HALF,
    }
}

fn simulation(spheres: &[HazardSphere], speed: FP) -> Simulation<MeshHazardNavigation> {
    let graph = graph();
    let spec = AgentSpec {
        start: p(0, 0),
        goal: p(8, 8),
        max_vertical: FP::ZERO,
        distance_per_tick: speed,
    };
    let pin = MeshScenePin::new(&graph, spec).unwrap();
    let mut simulation = Simulation::<MeshHazardNavigation>::new((), 60, 37);
    let frame = simulation.frame_mut();
    frame.set_singleton(pin);
    MeshRuntime::admit(frame, &graph, pin.binding(), spec).unwrap();
    if !spheres.is_empty() {
        let mut config = MeshHazards {
            enabled: 1,
            count: spheres.len() as u32,
            graph_revision: graph.revision(),
            ..MeshHazards::zeroed()
        };
        config.spheres[..spheres.len()].copy_from_slice(spheres);
        frame.set_singleton(config);
    }
    admit(frame, pin).unwrap();
    simulation
}

fn tick(simulation: &mut Simulation<MeshHazardNavigation>, goal: Option<[FP; 3]>) {
    let mut inputs = TickInputs::new(simulation.tick() + 1, 1);
    if let Some(goal) = goal {
        inputs.set_input(
            PlayerSlot(0),
            YardInput {
                buttons: super::super::MESH_SET_GOAL,
                origin: goal.map(|v| v.raw() as i32),
                ..YardInput::zeroed()
            },
        );
    }
    simulation.step(&inputs);
}

fn add_mission(simulation: &mut Simulation<MeshHazardNavigation>, point: [FP; 3]) {
    let pin = *simulation.frame().singleton::<MeshScenePin>();
    let mut config = mission::MeshMission {
        enabled: 1,
        count: 1,
        graph_revision: pin.graph_revision,
        ..mission::MeshMission::zeroed()
    };
    config.points[0] = mission::MissionPoint {
        position: point,
        radius: FP::HALF,
    };
    config.exit = config.points[0];
    simulation.frame_mut().set_singleton(config);
    admit(simulation.frame(), pin).unwrap();
}

#[test]
fn swept_hit_rejects_whole_tick_then_freezes_input_movement_and_mission() {
    let mut sim = simulation(&[sphere(p(0, 4))], FP::from_int(32));
    add_mission(&mut sim, p(0, 8));
    let initial_pin = *sim.frame().singleton::<MeshScenePin>();
    let original = MeshRuntime::agent(sim.frame(), initial_pin.binding()).unwrap();
    tick(&mut sim, Some(p(0, 8)));
    assert_eq!(
        read_run(sim.frame()).unwrap(),
        MeshHazardRun {
            lost: 1,
            hit_index: 0,
            lost_tick: 1
        }
    );
    let stopped = MeshRuntime::agent(sim.frame(), initial_pin.binding()).unwrap();
    assert_eq!(stopped.position, original.position);
    assert_eq!(stopped.spec, original.spec);
    assert_eq!(stopped.distance, original.distance);
    assert_eq!(stopped.status, NavigationStatus::Stopped);
    assert_eq!(*sim.frame().singleton::<MeshScenePin>(), initial_pin);
    assert_eq!(
        mission::read_run(sim.frame()).unwrap(),
        mission::MeshMissionRun::zeroed()
    );
    for _ in 0..3 {
        tick(&mut sim, Some(p(8, 0)));
    }
    assert_eq!(
        MeshRuntime::agent(sim.frame(), initial_pin.binding()).unwrap(),
        stopped
    );
    assert_eq!(read_run(sim.frame()).unwrap().lost_tick, 1);
    assert_eq!(
        mission::read_run(sim.frame()).unwrap(),
        mission::MeshMissionRun::zeroed()
    );
    assert_eq!(sim.frame().singleton::<MeshStepStatus>().failed, 0);
    validate_frame(sim.frame(), initial_pin).unwrap();
}

#[test]
fn hazard_has_precedence_over_exit_and_collection_at_same_tick() {
    let mut sim = simulation(&[sphere(p(8, 8))], FP::from_int(32));
    add_mission(&mut sim, p(8, 8));
    tick(&mut sim, None);
    assert_eq!(read_run(sim.frame()).unwrap().lost, 1);
    assert_eq!(
        mission::read_run(sim.frame()).unwrap(),
        mission::MeshMissionRun::zeroed()
    );
}

#[test]
fn initial_overlap_and_multiple_hazards_choose_lowest_authored_index() {
    let mut sim = simulation(&[sphere(p(7, 7)), sphere(p(0, 0))], FP::from_int(32));
    tick(&mut sim, None);
    assert_eq!(read_run(sim.frame()).unwrap().hit_index, 0);
    let mut stationary = simulation(&[sphere(p(0, 0))], FP::ONE);
    let pin = *stationary.frame().singleton::<MeshScenePin>();
    MeshRuntime::stop(stationary.frame_mut(), pin.binding()).unwrap();
    tick(&mut stationary, None);
    assert_eq!(read_run(stationary.frame()).unwrap().lost, 1);
}

#[test]
fn replay_restore_and_authored_reset_restore_all_authoritative_state() {
    let mut sim = simulation(&[sphere(p(4, 4))], FP::from_int(2));
    let authored = sim.frame().clone();
    let mut checksums = Vec::new();
    for _ in 0..6 {
        tick(&mut sim, None);
        checksums.push(sim.checksum());
    }
    assert_eq!(read_run(sim.frame()).unwrap().lost, 1);
    sim.restore(&authored);
    assert_eq!(read_run(sim.frame()).unwrap(), MeshHazardRun::zeroed());
    admit(sim.frame(), *sim.frame().singleton::<MeshScenePin>()).unwrap();
    for checksum in checksums {
        tick(&mut sim, None);
        assert_eq!(sim.checksum(), checksum);
    }
    sim.restore(&authored);
    for _ in 0..6 {
        tick(&mut sim, Some(p(0, 8)));
    }
    assert_eq!(read_run(sim.frame()).unwrap().lost, 0);
    assert_eq!(
        MeshRuntime::agent(sim.frame(), authored.singleton::<MeshScenePin>().binding())
            .unwrap()
            .position,
        p(0, 8)
    );
}

#[test]
fn no_hit_updates_mission_and_win_is_terminal() {
    let mut sim = simulation(&[sphere(p(0, 8))], FP::from_int(32));
    add_mission(&mut sim, p(8, 8));
    tick(&mut sim, None);
    assert_eq!(mission::read_run(sim.frame()).unwrap().won, 1);
    assert_eq!(read_run(sim.frame()).unwrap().lost, 0);
    let pin = *sim.frame().singleton::<MeshScenePin>();
    let agent = MeshRuntime::agent(sim.frame(), pin.binding()).unwrap();
    tick(&mut sim, Some(p(0, 8)));
    assert_eq!(
        MeshRuntime::agent(sim.frame(), pin.binding()).unwrap(),
        agent
    );
    assert_eq!(read_run(sim.frame()).unwrap().lost, 0);
}

#[test]
fn invalid_authored_or_runtime_state_is_engine_failure_not_loss_and_is_atomic() {
    let mut sim = simulation(&[sphere(p(4, 4))], FP::from_int(32));
    let pin = *sim.frame().singleton::<MeshScenePin>();
    let baseline = read_config(sim.frame()).unwrap();
    for config in [
        MeshHazards {
            enabled: 2,
            ..baseline
        },
        MeshHazards {
            count: 9,
            ..baseline
        },
        MeshHazards {
            count: 0,
            ..baseline
        },
        MeshHazards {
            graph_revision: [0; 32],
            ..baseline
        },
        MeshHazards {
            enabled: 0,
            ..baseline
        },
    ] {
        assert!(validate_config(&config, &pin).is_err());
    }
    let mut invalid = baseline;
    invalid.spheres[1] = sphere(p(0, 0));
    assert!(validate_config(&invalid, &pin).is_err());
    invalid = baseline;
    invalid.spheres[0].radius = FP::ZERO;
    assert!(validate_config(&invalid, &pin).is_err());
    invalid = baseline;
    invalid.spheres[0].center[0] = FP::from_raw(i64::MIN);
    assert!(validate_config(&invalid, &pin).is_err());
    for run in [
        MeshHazardRun {
            lost: 2,
            hit_index: 0,
            lost_tick: 0,
        },
        MeshHazardRun {
            lost: 0,
            hit_index: 1,
            lost_tick: 0,
        },
        MeshHazardRun {
            lost: 0,
            hit_index: 0,
            lost_tick: 1,
        },
        MeshHazardRun {
            lost: 1,
            hit_index: 1,
            lost_tick: 1,
        },
    ] {
        assert!(validate_run(&baseline, &run).is_err());
    }
    let original = MeshRuntime::agent(sim.frame(), pin.binding()).unwrap();
    sim.frame_mut().set_singleton(invalid);
    tick(&mut sim, Some(p(0, 8)));
    assert_eq!(sim.frame().singleton::<MeshStepStatus>().failed, 1);
    assert_eq!(read_run(sim.frame()).unwrap(), MeshHazardRun::zeroed());
    assert_eq!(
        MeshRuntime::agent(sim.frame(), pin.binding()).unwrap(),
        original
    );
    assert_eq!(*sim.frame().singleton::<MeshScenePin>(), pin);
    assert_eq!(
        mission::read_run(sim.frame()).unwrap(),
        mission::MeshMissionRun::zeroed()
    );
}

#[test]
fn authored_admission_rejects_loss_even_when_runtime_state_is_consistent() {
    let mut sim = simulation(&[sphere(p(0, 0))], FP::ONE);
    tick(&mut sim, None);
    let pin = *sim.frame().singleton::<MeshScenePin>();
    validate_frame(sim.frame(), pin).unwrap();
    assert!(admit(sim.frame(), pin).is_err());
    sim.frame_mut()
        .singleton_mut::<mission::MeshMissionRun>()
        .won = 1;
    assert!(validate_frame(sim.frame(), pin).is_err());
}

#[test]
fn hazard_profile_never_registers_into_the_legacy_game() {
    let legacy = Simulation::<MeshNavigation>::new((), 60, 37);
    assert!(legacy.registry().singleton_id::<MeshHazards>().is_none());
    assert!(legacy.registry().singleton_id::<MeshHazardRun>().is_none());
    let hazard = Simulation::<MeshHazardNavigation>::new((), 60, 37);
    assert!(hazard.registry().singleton_id::<MeshHazards>().is_some());
    assert!(hazard.registry().singleton_id::<MeshHazardRun>().is_some());
    assert_ne!(legacy.checksum(), hazard.checksum());
}

#[test]
fn corner_sweep_uses_route_portal_not_tick_endpoint_chord() {
    // The route turns at (4,4). Its endpoint chord goes through (4,3).
    let mut safe = simulation(&[sphere(p(4, 3))], FP::from_int(32));
    tick(&mut safe, Some(p(8, 6)));
    assert_eq!(read_run(safe.frame()).unwrap().lost, 0);
    let pin = *safe.frame().singleton::<MeshScenePin>();
    assert_eq!(
        MeshRuntime::agent(safe.frame(), pin.binding())
            .unwrap()
            .position,
        p(8, 6)
    );
    let mut hit = simulation(&[sphere(p(4, 4))], FP::from_int(32));
    tick(&mut hit, Some(p(8, 6)));
    assert_eq!(read_run(hit.frame()).unwrap().lost, 1);
}

#[test]
fn exact_tangent_is_loss_and_off_surface_sphere_is_admitted() {
    let mut center = p(4, 0);
    center[1] = FP::ONE;
    let mut sim = simulation(
        &[HazardSphere {
            center,
            radius: FP::ONE,
        }],
        FP::from_int(32),
    );
    tick(&mut sim, Some(p(8, 0)));
    assert_eq!(read_run(sim.frame()).unwrap().lost, 1);
    assert_eq!(sim.frame().singleton::<MeshStepStatus>().failed, 0);
}

#[test]
fn disabled_hazards_tick_matches_legacy_movement_and_mission() {
    let mut hazard = simulation(&[], FP::from_int(32));
    add_mission(&mut hazard, p(8, 8));
    let pin = *hazard.frame().singleton::<MeshScenePin>();
    let mission_config = mission::read_config(hazard.frame()).unwrap();
    let mut legacy = Simulation::<MeshNavigation>::new((), 60, 37);
    legacy.frame_mut().set_singleton(pin);
    legacy.frame_mut().set_singleton(mission_config);
    MeshRuntime::admit(
        legacy.frame_mut(),
        &pin.graph().unwrap(),
        pin.binding(),
        pin.spec(),
    )
    .unwrap();

    let inputs = TickInputs::new(1, 1);
    hazard.step(&inputs);
    legacy.step(&inputs);

    let actual = MeshRuntime::agent(hazard.frame(), pin.binding()).unwrap();
    let expected = MeshRuntime::agent(legacy.frame(), pin.binding()).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.position, p(8, 8));
    assert_eq!(actual.status, NavigationStatus::Arrived);
    assert_eq!(
        mission::read_run(hazard.frame()).unwrap(),
        mission::read_run(legacy.frame()).unwrap()
    );
    assert_eq!(mission::read_run(hazard.frame()).unwrap().won, 1);
    assert_eq!(read_config(hazard.frame()).unwrap(), MeshHazards::zeroed());
    assert_eq!(read_run(hazard.frame()).unwrap(), MeshHazardRun::zeroed());
    assert_eq!(
        *hazard.frame().singleton::<MeshStepStatus>(),
        *legacy.frame().singleton::<MeshStepStatus>()
    );
    validate_frame(hazard.frame(), pin).unwrap();
}

#[test]
fn disabled_hazards_reject_nonzero_run_before_movement_or_mission() {
    let mut sim = simulation(&[], FP::from_int(32));
    add_mission(&mut sim, p(8, 8));
    let authored = sim.frame().clone();
    let pin = *authored.singleton::<MeshScenePin>();
    let before = MeshRuntime::agent(&authored, pin.binding()).unwrap();
    for invalid in [
        MeshHazardRun {
            lost: 0,
            hit_index: 1,
            lost_tick: 0,
        },
        MeshHazardRun {
            lost: 0,
            hit_index: 0,
            lost_tick: 1,
        },
        MeshHazardRun {
            lost: 1,
            hit_index: 0,
            lost_tick: 1,
        },
    ] {
        assert!(validate_run(&MeshHazards::zeroed(), &invalid).is_err());
        sim.restore(&authored);
        sim.frame_mut().set_singleton(invalid);
        assert!(admit(sim.frame(), pin).is_err());
        tick(&mut sim, Some(p(0, 8)));
        assert_eq!(sim.frame().singleton::<MeshStepStatus>().failed, 1);
        assert_eq!(
            MeshRuntime::agent(sim.frame(), pin.binding()).unwrap(),
            before
        );
        assert_eq!(
            mission::read_run(sim.frame()).unwrap(),
            mission::MeshMissionRun::zeroed()
        );
        assert_eq!(read_run(sim.frame()).unwrap(), invalid);
        assert_eq!(*sim.frame().singleton::<MeshScenePin>(), pin);
    }
}
