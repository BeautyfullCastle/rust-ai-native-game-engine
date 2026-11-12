use super::*;
use orr_sim::{Simulation, TickInputs};
fn p(x: i32, z: i32) -> [FP; 2] {
    [FP::from_int(x), FP::from_int(z)]
}
fn assets() -> (Terrain, TerrainGraph) {
    let t = Terrain::new(
        "patrol-test".into(),
        2,
        2,
        p(0, 0),
        FP::from_int(8),
        vec![FP::ZERO; 4],
        vec![false],
    )
    .unwrap();
    let g = TerrainGraph::build(&t, AgentProfile::default()).unwrap();
    (t, g)
}
fn simulation(looped: u32, wait: u32) -> Simulation<NavigationYard3D> {
    let (t, g) = assets();
    let spec = AgentSpec {
        start: p(0, 0),
        goal: p(8, 0),
        max_slope: FP::ONE,
        distance_per_tick: FP::ONE,
    };
    let pin = NavigationScenePin::new("terrain.orrterrain", &t, &g, spec.into()).unwrap();
    let mut s = Simulation::<NavigationYard3D>::new((), 60, 51);
    s.frame_mut().set_singleton(pin);
    NavigationRuntime::admit(s.frame_mut(), &t, &g, spec).unwrap();
    let mut c = TerrainPatrol {
        enabled: 1,
        count: 3,
        looped,
        wait_ticks: wait,
        graph_revision: g.revision(),
        ..TerrainPatrol::zeroed()
    };
    c.points[0] = p(1, 0);
    c.points[1] = p(3, 0);
    c.points[2] = p(5, 0);
    s.frame_mut().set_singleton(c);
    admit(s.frame_mut(), pin).unwrap();
    s
}
fn tick(s: &mut Simulation<NavigationYard3D>) {
    let input = TickInputs::new(s.tick() + 1, 1);
    s.step(&input);
    assert_eq!(s.frame().singleton::<NavigationStepStatus>().failed, 0);
}
fn position(s: &Simulation<NavigationYard3D>) -> [FP; 3] {
    let state = *s.frame().singleton::<RuntimeState>();
    NavigationRuntime::agent(s.frame(), state.agent)
        .unwrap()
        .position
}
#[test]
fn exact_wait_ticks_and_nonloop_completion_preserve_authored_pin() {
    let mut s = simulation(0, 2);
    let pin = *s.frame().singleton::<NavigationScenePin>();
    // Terrain routing follows validated triangle portals, not an assumed straight line.
    for _ in 0..100 {
        if read_run(s.frame()).unwrap().arrived == 1 {
            break;
        }
        tick(&mut s);
    }
    assert_eq!(read_run(s.frame()).unwrap().arrived, 1);
    assert_eq!(position(&s), [FP::ONE, FP::ZERO, FP::ZERO]);
    for wait in [1, 0] {
        tick(&mut s);
        assert_eq!(position(&s)[0], FP::ONE);
        assert_eq!(read_run(s.frame()).unwrap().wait_remaining, wait);
    }
    tick(&mut s);
    assert_eq!(read_run(s.frame()).unwrap().cursor, 1);
    assert_ne!(position(&s), [FP::ONE, FP::ZERO, FP::ZERO]);
    for _ in 0..100 {
        if read_run(s.frame()).unwrap().completed == 1 {
            break;
        }
        tick(&mut s);
    }
    assert_eq!(position(&s)[0], FP::from_int(5));
    assert_eq!(read_run(s.frame()).unwrap().completed, 1);
    assert_eq!(*s.frame().singleton::<NavigationScenePin>(), pin);
}
#[test]
fn many_loops_keep_entity_list_handles_and_serialized_size_stable() {
    let mut s = simulation(1, 0);
    let state = *s.frame().singleton::<RuntimeState>();
    let size = s.frame().to_bytes().len();
    let mut high_water = size;
    let mut saw_last = false;
    let mut saw_cycle = false;
    for index in 0..200 {
        tick(&mut s);
        assert_eq!(*s.frame().singleton::<RuntimeState>(), state);
        assert_eq!(s.frame().alive_count(), 1);
        let current = s.frame().to_bytes().len();
        if index < 50 {
            high_water = high_water.max(current);
        } else {
            assert!(current <= high_water);
        }
        let r = read_run(s.frame()).unwrap();
        saw_last |= r.cursor == 2;
        saw_cycle |= r.cycled == 1 && r.cursor == 0;
    }
    assert!(saw_last && saw_cycle);
}
#[test]
fn snapshot_restore_replays_frame_checksums() {
    let mut s = simulation(1, 1);
    for _ in 0..5 {
        tick(&mut s);
    }
    let snap = s.frame().clone();
    let mut hashes = Vec::new();
    for _ in 0..12 {
        tick(&mut s);
        hashes.push(s.checksum());
    }
    s.restore(&snap);
    for hash in hashes {
        tick(&mut s);
        assert_eq!(s.checksum(), hash);
    }
}
#[test]
fn invalid_route_tail_revision_flags_and_bounds_reject() {
    let (t, g) = assets();
    let mut c = TerrainPatrol {
        enabled: 1,
        count: 2,
        graph_revision: g.revision(),
        ..TerrainPatrol::zeroed()
    };
    c.points[0] = p(1, 0);
    c.points[1] = p(9, 0);
    assert!(validate_config(&c, &t, &g, p(0, 0)).is_err());
    c.points[1] = p(3, 0);
    c.points[2] = p(1, 1);
    assert!(validate_config(&c, &t, &g, p(0, 0)).is_err());
    c.points[2] = p(0, 0);
    c.graph_revision = [0; 32];
    assert!(validate_config(&c, &t, &g, p(0, 0)).is_err());
    c.graph_revision = g.revision();
    c.wait_ticks = 3601;
    assert!(validate_config(&c, &t, &g, p(0, 0)).is_err());
    c = TerrainPatrol::zeroed();
    c.wait_ticks = 1;
    assert!(validate_config(&c, &t, &g, p(0, 0)).is_err());
}
#[test]
fn invalid_runtime_fails_without_movement_then_records_failure() {
    let mut s = simulation(1, 0);
    let before = position(&s);
    let mut r = read_run(s.frame()).unwrap();
    r.reserved[0] = 1;
    s.frame_mut().set_singleton(r);
    let input = TickInputs::new(s.tick() + 1, 1);
    s.step(&input);
    assert_eq!(position(&s), before);
    assert_eq!(read_run(s.frame()).unwrap(), r);
    assert_eq!(s.frame().singleton::<NavigationStepStatus>().failed, 1);
}
#[test]
fn derived_leg_goal_and_speed_cannot_diverge_from_immutable_pin() {
    let mut s = simulation(1, 0);
    let pin = *s.frame().singleton::<NavigationScenePin>();
    assert_eq!(pin.agent.goal, p(8, 0));
    let state = *s.frame().singleton::<RuntimeState>();
    let a = *s.frame().get::<RuntimeAgent>(state.agent).unwrap();
    assert_eq!(AgentSpec::from(a).goal, p(1, 0));
    let (t, g, n, mut spec) = decoded(s.frame()).unwrap();
    let c = read_config(s.frame()).unwrap();
    let r = read_run(s.frame()).unwrap();
    spec.distance_per_tick = FP::from_int(2);
    assert!(validate_runtime_parts(&c, &r, &pin, &t, &g, &n, &spec).is_err());
    for _ in 0..6 {
        tick(&mut s);
    }
    assert_eq!(*s.frame().singleton::<NavigationScenePin>(), pin);
}
#[test]
fn blank_disabled_admits_and_runtime_is_unreflectable_canonical() {
    let mut s = Simulation::<NavigationYard3D>::new((), 60, 1);
    admit(s.frame_mut(), NavigationScenePin::zeroed()).unwrap();
    let mut c = TerrainPatrol::zeroed();
    c.enabled = 1;
    s.frame_mut().set_singleton(c);
    assert!(admit(s.frame_mut(), NavigationScenePin::zeroed()).is_err());
    assert_eq!(std::mem::size_of::<TerrainPatrol>(), 176);
    assert_eq!(std::mem::size_of::<TerrainPatrolRun>(), 32);
}
#[test]
fn invalid_admission_preserves_all_frame_bytes() {
    let mut s = simulation(0, 0);
    let mut c = read_config(s.frame()).unwrap();
    c.points[2] = p(9, 0);
    s.frame_mut().set_singleton(c);
    let before = s.frame().to_bytes();
    let pin = *s.frame().singleton::<NavigationScenePin>();
    assert!(admit(s.frame_mut(), pin).is_err());
    assert_eq!(s.frame().to_bytes(), before);
}

#[test]
fn authored_xz_projection_is_exact_and_just_outside_points_reject_before_play() {
    let (terrain, graph) = assets();
    let inside = [FP::from_raw(FP::SCALE / 3), FP::from_raw(FP::SCALE / 7)];
    let projected = graph.project(&terrain, inside).unwrap();
    assert_eq!([projected.position[0], projected.position[2]], inside);
    let outside = [FP::from_raw(-1), FP::ZERO];
    assert!(graph
        .find_path(&terrain, inside, outside, SearchBudget::default())
        .is_err());
    for looped in [0, 1] {
        let mut config = TerrainPatrol {
            enabled: 1,
            count: 2,
            looped,
            graph_revision: graph.revision(),
            ..TerrainPatrol::zeroed()
        };
        config.points[0] = inside;
        config.points[1] = p(3, 0);
        validate_config(&config, &terrain, &graph, p(0, 0)).unwrap();
        assert!(validate_config(&config, &terrain, &graph, outside)
            .unwrap_err()
            .contains("exact authored XZ"));
        config.points[1] = outside;
        assert!(validate_config(&config, &terrain, &graph, p(0, 0))
            .unwrap_err()
            .contains("exact authored XZ"));
    }
}
