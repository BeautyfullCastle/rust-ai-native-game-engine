use super::*;
use orr_sim::{PlayerSlot, Simulation, TickInputs};
fn p(x: i32, z: i32) -> [FP; 3] {
    [FP::from_int(x), FP::ZERO, FP::from_int(z)]
}
fn graph() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[p(0, 0), p(0, 4), p(4, 0), p(4, 4)],
        &[[0, 1, 2], [1, 3, 2]],
        orr_navigation::AgentProfile::default(),
    )
    .unwrap()
}
fn simulation(looped: u32, wait_ticks: u32) -> Simulation<MeshNavigation> {
    let g = graph();
    let spec = AgentSpec {
        start: p(0, 0),
        goal: p(4, 0),
        max_vertical: FP::ZERO,
        distance_per_tick: FP::ONE,
    };
    let mut pin = MeshScenePin::new(&g, spec).unwrap();
    let mut sim = Simulation::<MeshNavigation>::new((), 60, 77);
    sim.frame_mut().set_singleton(pin);
    MeshRuntime::admit(sim.frame_mut(), &g, pin.binding(), spec).unwrap();
    let mut c = MeshPatrol {
        enabled: 1,
        count: 2,
        looped,
        wait_ticks,
        graph_revision: g.revision(),
        ..MeshPatrol::zeroed()
    };
    c.points[0] = p(1, 0);
    c.points[1] = p(2, 0);
    sim.frame_mut().set_singleton(c);
    admit(sim.frame_mut(), &mut pin).unwrap();
    sim
}
fn tick(sim: &mut Simulation<MeshNavigation>) {
    let t = TickInputs::new(sim.tick() + 1, 1);
    sim.step(&t);
    assert_eq!(sim.frame().singleton::<MeshStepStatus>().failed, 0);
}
fn pos(sim: &Simulation<MeshNavigation>) -> [FP; 3] {
    let pin = *sim.frame().singleton::<MeshScenePin>();
    MeshRuntime::agent(sim.frame(), pin.binding())
        .unwrap()
        .position
}
#[test]
fn arrival_waits_exact_full_ticks_before_next_goal_then_completes() {
    let mut s = simulation(0, 2);
    tick(&mut s);
    assert_eq!(pos(&s), p(1, 0));
    assert_eq!(read_run(s.frame()).unwrap().wait_remaining, 2);
    for expected in [1, 0] {
        tick(&mut s);
        assert_eq!(pos(&s), p(1, 0));
        assert_eq!(read_run(s.frame()).unwrap().wait_remaining, expected);
    }
    tick(&mut s);
    assert_eq!(pos(&s), p(2, 0));
    for _ in 0..3 {
        tick(&mut s);
    }
    assert_eq!(read_run(s.frame()).unwrap().completed, 1);
    let before = pos(&s);
    tick(&mut s);
    assert_eq!(pos(&s), before);
}
#[test]
fn loop_and_rollback_replay_are_frame_owned_and_deterministic() {
    let mut a = simulation(1, 0);
    let mut b = simulation(1, 0);
    for _ in 0..9 {
        tick(&mut a);
        tick(&mut b);
        assert_eq!(a.checksum(), b.checksum());
    }
    let snap = a.frame().clone();
    let mut expected = Vec::new();
    for _ in 0..5 {
        tick(&mut a);
        expected.push(a.checksum());
    }
    a.restore(&snap);
    for hash in expected {
        tick(&mut a);
        assert_eq!(a.checksum(), hash);
    }
    assert_eq!(read_run(a.frame()).unwrap().completed, 0);
}
#[test]
fn config_rejects_disabled_garbage_tail_bounds_revision_and_unreachable_segments() {
    let g = graph();
    let mut c = MeshPatrol::zeroed();
    validate_config(&c, &g, p(0, 0), FP::ZERO).unwrap();
    c.wait_ticks = 1;
    assert!(validate_config(&c, &g, p(0, 0), FP::ZERO).is_err());
    c = MeshPatrol {
        enabled: 1,
        count: 2,
        graph_revision: g.revision(),
        ..MeshPatrol::zeroed()
    };
    c.points[0] = p(1, 0);
    c.points[1] = p(9, 0);
    assert!(validate_config(&c, &g, p(0, 0), FP::ZERO).is_err());
    c.points[1] = p(2, 0);
    c.points[2] = p(1, 1);
    assert!(validate_config(&c, &g, p(0, 0), FP::ZERO).is_err());
    c.points[2] = p(0, 0);
    c.graph_revision = [0; 32];
    assert!(validate_config(&c, &g, p(0, 0), FP::ZERO).is_err());
    c.graph_revision = g.revision();
    c.points[1] = p(257, 0);
    assert!(validate_config(&c, &g, p(0, 0), FP::ZERO).is_err());
}
#[test]
fn successful_manual_goal_pauses_until_explicit_readmission() {
    let mut s = simulation(1, 2);
    tick(&mut s);
    for buttons in [MESH_SET_GOAL, MESH_SET_GOAL | 1] {
        let mut rejected = TickInputs::new(s.tick() + 1, 1);
        rejected.set_input(
            PlayerSlot(0),
            YardInput {
                buttons,
                origin: p(99, 0).map(|v| v.raw() as i32),
                ..YardInput::default()
            },
        );
        s.step(&rejected);
        assert_eq!(read_run(s.frame()).unwrap().paused, 0);
        assert_eq!(s.frame().singleton::<MeshStepStatus>().failed, 0);
    }
    let input = YardInput {
        buttons: MESH_SET_GOAL,
        _pad: 0,
        origin: p(4, 0).map(|v| v.raw() as i32),
        dir: [0; 3],
    };
    let mut t = TickInputs::new(s.tick() + 1, 1);
    t.set_input(PlayerSlot(0), input);
    s.step(&t);
    assert_eq!(read_run(s.frame()).unwrap().paused, 1);
    for _ in 0..8 {
        tick(&mut s);
    }
    assert_eq!(pos(&s), p(4, 0));
    assert_eq!(read_run(s.frame()).unwrap().paused, 1);
}
#[test]
fn malformed_runtime_fails_closed_without_movement_and_marks_failure() {
    let mut s = simulation(1, 0);
    let before = pos(&s);
    let mut run = read_run(s.frame()).unwrap();
    run.reserved[0] = 1;
    s.frame_mut().set_singleton(run);
    let t = TickInputs::new(s.tick() + 1, 1);
    s.step(&t);
    assert_eq!(pos(&s), before);
    assert_eq!(*s.frame().singleton::<MeshPatrolRun>(), run);
    assert_eq!(s.frame().singleton::<MeshStepStatus>().failed, 1);
}
#[test]
fn admission_failure_does_not_mutate_frame_or_pin() {
    let mut s = simulation(0, 0);
    let mut c = read_config(s.frame()).unwrap();
    c.points[1] = p(9, 0);
    s.frame_mut().set_singleton(c);
    let bytes = s.frame().to_bytes();
    let mut pin = *s.frame().singleton::<MeshScenePin>();
    let before = pin;
    assert!(admit(s.frame_mut(), &mut pin).is_err());
    assert_eq!(pin, before);
    assert_eq!(s.frame().to_bytes(), bytes);
}
#[test]
fn runtime_config_and_padding_bounds_are_strict() {
    let c = MeshPatrol {
        enabled: 1,
        count: 1,
        ..MeshPatrol::zeroed()
    };
    for r in [
        MeshPatrolRun {
            cursor: 1,
            ..MeshPatrolRun::zeroed()
        },
        MeshPatrolRun {
            paused: 2,
            ..MeshPatrolRun::zeroed()
        },
        MeshPatrolRun {
            wait_remaining: 1,
            ..MeshPatrolRun::zeroed()
        },
        MeshPatrolRun {
            completed: 1,
            ..MeshPatrolRun::zeroed()
        },
    ] {
        assert!(validate_run(&c, &r).is_err());
    }
    assert_eq!(std::mem::size_of::<MeshPatrol>(), 240);
    assert_eq!(std::mem::size_of::<MeshPatrolRun>(), 32);
}
