use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{fp, FP};
use orr_navigation::{AgentProfile, NavigationStatus, TerrainGraph};
use orr_navigation_runtime::{
    AgentSpec, NavigationRuntime, RuntimeError, RuntimeState, MAX_GRAPH_BYTES, MAX_NAVIGATOR_BYTES,
    MAX_TERRAIN_BYTES,
};
use orr_terrain::Terrain;

fn setup() -> (Frame, Terrain, TerrainGraph, AgentSpec) {
    let mut builder = ComponentRegistryBuilder::new();
    NavigationRuntime::register(&mut builder);
    let frame = Frame::new(builder.build());
    let terrain = Terrain::new(
        "runtime/flat".into(),
        4,
        4,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 16],
        vec![false; 9],
    )
    .unwrap();
    let spec = AgentSpec {
        start: [fp!(0.125), fp!(0.125)],
        goal: [fp!(2.875), fp!(2.875)],
        max_slope: FP::ONE,
        distance_per_tick: fp!(0.25),
    };
    let graph = TerrainGraph::build(
        &terrain,
        AgentProfile {
            max_slope: spec.max_slope,
            ..Default::default()
        },
    )
    .unwrap();
    (frame, terrain, graph, spec)
}

#[test]
fn route_step_clone_restore_and_checksum_replay() {
    let (mut frame, terrain, graph, spec) = setup();
    let entity = NavigationRuntime::admit(&mut frame, &terrain, &graph, spec).unwrap();
    NavigationRuntime::validate(&frame).unwrap();
    let before = frame.clone();
    let baseline = frame.checksum();
    let mut other = frame.clone();
    let first = NavigationRuntime::agent(&frame, entity).unwrap();
    assert_eq!(first.status, NavigationStatus::Moving);
    for tick in 1..=24 {
        NavigationRuntime::step(&mut frame).unwrap();
        NavigationRuntime::step(&mut other).unwrap();
        frame.set_tick(tick);
        other.set_tick(tick);
        assert_eq!(frame.checksum(), other.checksum());
        NavigationRuntime::validate(&frame).unwrap();
    }
    let after = frame.checksum();
    assert_ne!(after, baseline);
    frame.copy_from(&before);
    assert_eq!(frame.checksum(), baseline);
    for tick in 1..=24 {
        NavigationRuntime::step(&mut frame).unwrap();
        frame.set_tick(tick);
    }
    assert_eq!(frame.checksum(), after);
}

#[test]
fn agent_deletion_fails_closed_and_preserves_bytes() {
    let (mut frame, terrain, graph, spec) = setup();
    let entity = NavigationRuntime::admit(&mut frame, &terrain, &graph, spec).unwrap();
    frame.despawn(entity);
    let before = frame.checksum();
    assert_eq!(
        NavigationRuntime::step(&mut frame),
        Err(RuntimeError::MissingAgent)
    );
    assert_eq!(before, frame.checksum());
    assert_eq!(
        NavigationRuntime::validate(&frame),
        Err(RuntimeError::MissingAgent)
    );
    let next = frame.spawn();
    assert_ne!(next, entity);
    let before = frame.checksum();
    assert_eq!(
        NavigationRuntime::step(&mut frame),
        Err(RuntimeError::MissingAgent)
    );
    assert_eq!(before, frame.checksum());
}

#[test]
fn admission_rejection_and_tamper_are_atomic() {
    let (mut frame, terrain, graph, spec) = setup();
    let before = frame.checksum();
    let mut bad = spec;
    bad.distance_per_tick = fp!(-1);
    assert!(NavigationRuntime::admit(&mut frame, &terrain, &graph, bad).is_err());
    assert_eq!(before, frame.checksum());
    let entity = NavigationRuntime::admit(&mut frame, &terrain, &graph, spec).unwrap();
    assert_eq!(
        NavigationRuntime::admit(&mut frame, &terrain, &graph, spec),
        Err(RuntimeError::AlreadyActive)
    );
    let handle = frame.singleton::<RuntimeState>().navigator_bytes;
    frame.list_mut(handle)[0] ^= 0xff;
    let before = frame.checksum();
    assert!(NavigationRuntime::step(&mut frame).is_err());
    assert_eq!(before, frame.checksum());
    assert!(NavigationRuntime::agent(&frame, entity).is_err());
}

#[test]
fn removal_reclaims_single_agent_then_readmits() {
    let (mut frame, terrain, graph, spec) = setup();
    let entity = NavigationRuntime::admit(&mut frame, &terrain, &graph, spec).unwrap();
    NavigationRuntime::remove(&mut frame, entity).unwrap();
    assert!(!frame.exists(entity));
    assert!(!frame.singleton::<RuntimeState>().is_active());
    let new_entity = NavigationRuntime::admit(&mut frame, &terrain, &graph, spec).unwrap();
    assert_ne!(new_entity, entity);
    NavigationRuntime::validate(&frame).unwrap();
}

#[test]
fn oversized_terrain_and_blocked_routes_reject_atomically() {
    let (mut frame, _, _, spec) = setup();
    let oversized = Terrain::new(
        "runtime/oversized".into(),
        18,
        2,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 36],
        vec![false; 17],
    )
    .unwrap();
    let graph = TerrainGraph::build(&oversized, AgentProfile::default()).unwrap();
    let before = frame.checksum();
    assert_eq!(
        NavigationRuntime::admit(&mut frame, &oversized, &graph, spec),
        Err(RuntimeError::Limit)
    );
    assert_eq!(frame.checksum(), before);

    let hole = Terrain::new(
        "runtime/hole".into(),
        2,
        2,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 4],
        vec![true],
    )
    .unwrap();
    let graph = TerrainGraph::build(&hole, AgentProfile::default()).unwrap();
    let mut short = spec;
    short.goal = [fp!(0.875), fp!(0.875)];
    assert!(NavigationRuntime::admit(&mut frame, &hole, &graph, short).is_err());
    assert_eq!(frame.checksum(), before);

    let steep = Terrain::new(
        "runtime/steep".into(),
        2,
        2,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO, FP::ONE, FP::ONE, fp!(2)],
        vec![false],
    )
    .unwrap();
    let graph = TerrainGraph::build(
        &steep,
        AgentProfile {
            max_slope: FP::ZERO,
            ..Default::default()
        },
    )
    .unwrap();
    short.max_slope = FP::ZERO;
    assert!(NavigationRuntime::admit(&mut frame, &steep, &graph, short).is_err());
    assert_eq!(frame.checksum(), before);

    let islands = Terrain::new(
        "runtime/islands".into(),
        3,
        3,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 9],
        vec![false, true, true, false],
    )
    .unwrap();
    let graph = TerrainGraph::build(&islands, AgentProfile::default()).unwrap();
    short.max_slope = FP::ONE;
    short.goal = [fp!(1.875), fp!(1.875)];
    assert!(NavigationRuntime::admit(&mut frame, &islands, &graph, short).is_err());
    assert_eq!(frame.checksum(), before);
}

#[test]
fn oversized_snapshot_is_rejected_before_graph_rebuild() {
    let (mut frame, terrain, graph, spec) = setup();
    let _ = NavigationRuntime::admit(&mut frame, &terrain, &graph, spec).unwrap();
    let oversized = Terrain::new(
        "runtime/oversized".into(),
        18,
        2,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 36],
        vec![false; 17],
    )
    .unwrap();
    let handle = frame.singleton::<RuntimeState>().terrain_bytes;
    frame.list_clear(handle);
    for byte in oversized.cook() {
        frame.list_push(handle, byte);
    }
    let before = frame.checksum();
    assert_eq!(
        NavigationRuntime::step(&mut frame),
        Err(RuntimeError::Limit)
    );
    assert_eq!(frame.checksum(), before);
}

#[test]
fn overlarge_frame_byte_lists_fail_before_unbounded_decode() {
    let (mut frame, terrain, graph, spec) = setup();
    let _ = NavigationRuntime::admit(&mut frame, &terrain, &graph, spec).unwrap();
    let baseline = frame.clone();
    let state = *frame.singleton::<RuntimeState>();
    for (handle, maximum) in [
        (state.terrain_bytes, MAX_TERRAIN_BYTES),
        (state.graph_bytes, MAX_GRAPH_BYTES),
        (state.navigator_bytes, MAX_NAVIGATOR_BYTES),
    ] {
        frame.copy_from(&baseline);
        while frame.list(handle).len() <= maximum {
            frame.list_push(handle, 0);
        }
        let before = frame.checksum();
        assert_eq!(
            NavigationRuntime::step(&mut frame),
            Err(RuntimeError::Limit)
        );
        assert_eq!(frame.checksum(), before);
    }
}
