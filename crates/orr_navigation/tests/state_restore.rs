use orr_fp::{fp, FP};
use orr_navigation::{
    AgentProfile, NavigationError, NavigationStatus, Navigator, NavigatorSnapshot, SearchBudget,
    TerrainGraph,
};
use orr_terrain::Terrain;

fn fixture() -> (Terrain, TerrainGraph) {
    let terrain = Terrain::new(
        "state/terrain".into(),
        3,
        3,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 9],
        vec![false; 4],
    )
    .unwrap();
    let graph = TerrainGraph::build(&terrain, AgentProfile::default()).unwrap();
    (terrain, graph)
}

#[test]
fn moving_and_arrived_roundtrip_exact() {
    let (terrain, graph) = fixture();
    let mut navigator = Navigator::new(&graph, &terrain, [fp!(0.125), fp!(0.125)]).unwrap();
    navigator
        .replan(
            &graph,
            &terrain,
            [fp!(1.875), fp!(1.875)],
            SearchBudget::default(),
        )
        .unwrap();
    let snapshot = navigator.snapshot();
    let mut loaded = Navigator::new(&graph, &terrain, [fp!(0.125), fp!(0.125)]).unwrap();
    loaded.restore(&graph, &terrain, &snapshot).unwrap();
    assert_eq!(loaded, navigator);
    assert_eq!(loaded.snapshot().as_bytes(), snapshot.as_bytes());
    navigator.advance(&graph, &terrain, fp!(0.5)).unwrap();
    loaded
        .restore(&graph, &terrain, &navigator.snapshot())
        .unwrap();
    assert_eq!(loaded, navigator);
    navigator.advance(&graph, &terrain, fp!(100)).unwrap();
    assert_eq!(navigator.status(), NavigationStatus::Arrived);
    loaded
        .restore(&graph, &terrain, &navigator.snapshot())
        .unwrap();
    assert_eq!(loaded, navigator);
    assert_eq!(loaded.checksum(), navigator.checksum());
}

#[test]
fn stopped_new_and_stopped_after_movement_roundtrip() {
    let (terrain, graph) = fixture();
    let start = [fp!(0.125), fp!(0.125)];
    let mut navigator = Navigator::new(&graph, &terrain, start).unwrap();
    let initial = navigator.snapshot();
    let mut loaded = Navigator::new(&graph, &terrain, [fp!(0.25), fp!(0.25)]).unwrap();
    loaded.restore(&graph, &terrain, &initial).unwrap();
    assert_eq!(navigator, loaded);
    assert_eq!(loaded.status(), NavigationStatus::Stopped);
    navigator
        .replan(
            &graph,
            &terrain,
            [fp!(1.875), fp!(1.875)],
            SearchBudget::default(),
        )
        .unwrap();
    navigator.advance(&graph, &terrain, fp!(0.5)).unwrap();
    navigator.stop();
    loaded
        .restore(&graph, &terrain, &navigator.snapshot())
        .unwrap();
    assert_eq!(navigator, loaded);
    assert!(loaded.path().is_none());
    assert_eq!(loaded.next_waypoint(), 0);
}

#[test]
fn stale_or_malformed_never_mutates() {
    let (terrain, graph) = fixture();
    let mut navigator = Navigator::new(&graph, &terrain, [fp!(0.125), fp!(0.125)]).unwrap();
    navigator
        .replan(
            &graph,
            &terrain,
            [fp!(1.875), fp!(1.875)],
            SearchBudget::default(),
        )
        .unwrap();
    let before = navigator.clone();
    let original = navigator.snapshot();
    let different = Terrain::new(
        "state/different".into(),
        3,
        3,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 9],
        vec![false; 4],
    )
    .unwrap();
    let other_graph = TerrainGraph::build(&different, AgentProfile::default()).unwrap();
    assert_eq!(
        navigator.restore(&other_graph, &different, &original),
        Err(NavigationError::StaleTerrain)
    );
    assert_eq!(navigator, before);
    for offset in [0usize, 8, 40, 72, 96, 97, 98, original.as_bytes().len() - 1] {
        let mut bytes = original.as_bytes().to_vec();
        bytes[offset] ^= 0xff;
        let snapshot = NavigatorSnapshot::from_bytes(&bytes).unwrap();
        assert!(
            navigator.restore(&graph, &terrain, &snapshot).is_err(),
            "offset {offset}"
        );
        assert_eq!(navigator, before);
    }
    for length in [0usize, 8, 73, original.as_bytes().len() - 1] {
        let result = NavigatorSnapshot::from_bytes(&original.as_bytes()[..length]);
        if let Ok(snapshot) = result {
            assert!(navigator.restore(&graph, &terrain, &snapshot).is_err());
        }
        assert_eq!(navigator, before);
    }
}

#[test]
fn rejects_forged_corridor_and_cursor() {
    let (terrain, graph) = fixture();
    let mut navigator = Navigator::new(&graph, &terrain, [fp!(0.125), fp!(0.125)]).unwrap();
    navigator
        .replan(
            &graph,
            &terrain,
            [fp!(1.875), fp!(1.875)],
            SearchBudget::default(),
        )
        .unwrap();
    let before = navigator.clone();
    // Cursor is at byte 97: magic + two digests + 3 coordinates + status.
    for cursor in [0u32, u32::MAX] {
        let mut bytes = navigator.snapshot().as_bytes().to_vec();
        bytes[97..101].copy_from_slice(&cursor.to_le_bytes());
        let snapshot = NavigatorSnapshot::from_bytes(&bytes).unwrap();
        assert!(navigator.restore(&graph, &terrain, &snapshot).is_err());
        assert_eq!(navigator, before);
    }
    // Corridor count after path revisions and cost, with no room for giant allocation.
    let mut bytes = navigator.snapshot().as_bytes().to_vec();
    bytes[174..178].copy_from_slice(&u32::MAX.to_le_bytes());
    let snapshot = NavigatorSnapshot::from_bytes(&bytes).unwrap();
    assert!(navigator.restore(&graph, &terrain, &snapshot).is_err());
    assert_eq!(navigator, before);
}

#[test]
fn status_path_counts_revisions_and_tail_are_validated() {
    let (terrain, graph) = fixture();
    let mut navigator = Navigator::new(&graph, &terrain, [fp!(0.125), fp!(0.125)]).unwrap();
    navigator
        .replan(
            &graph,
            &terrain,
            [fp!(1.875), fp!(1.875)],
            SearchBudget::default(),
        )
        .unwrap();
    let baseline = navigator.clone();
    let original = navigator.snapshot();
    let mut cases = Vec::new();
    for offset in [8usize, 40, 102, 134] {
        let mut bytes = original.as_bytes().to_vec();
        bytes[offset] ^= 1;
        cases.push(bytes);
    }
    for (offset, value) in [(96usize, 3u8), (96, 0), (101, 0), (101, 2)] {
        let mut bytes = original.as_bytes().to_vec();
        bytes[offset] = value;
        cases.push(bytes);
    }
    for offset in [174usize, 178, original.as_bytes().len() - 28] {
        let mut bytes = original.as_bytes().to_vec();
        bytes[offset] ^= 1;
        cases.push(bytes);
    }
    let mut tail = original.as_bytes().to_vec();
    tail.push(0);
    cases.push(tail);
    for bytes in cases {
        let snapshot = NavigatorSnapshot::from_bytes(&bytes).unwrap();
        assert!(navigator.restore(&graph, &terrain, &snapshot).is_err());
        assert_eq!(navigator, baseline);
    }
}

#[test]
fn current_point_is_continuation_safe_but_not_a_replay_proof() {
    let terrain = Terrain::new(
        "state/one-cell".into(),
        2,
        2,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 4],
        vec![false],
    )
    .unwrap();
    let graph = TerrainGraph::build(&terrain, AgentProfile::default()).unwrap();
    let mut navigator = Navigator::new(&graph, &terrain, [fp!(0.125), fp!(0.75)]).unwrap();
    navigator
        .replan(
            &graph,
            &terrain,
            [fp!(0.625), fp!(0.75)],
            SearchBudget::default(),
        )
        .unwrap();
    let original = navigator.clone();
    let mut invalid = navigator.snapshot().as_bytes().to_vec();
    invalid[72..80].copy_from_slice(&fp!(0.875).raw().to_le_bytes());
    invalid[88..96].copy_from_slice(&fp!(0.125).raw().to_le_bytes());
    assert!(navigator
        .restore(
            &graph,
            &terrain,
            &NavigatorSnapshot::from_bytes(&invalid).unwrap()
        )
        .is_err());
    assert_eq!(navigator, original);
    // This point can safely continue inside the corridor triangle, although
    // a snapshot alone does not prove a prior tick actually moved there.
    let mut safe = navigator.snapshot().as_bytes().to_vec();
    safe[72..80].copy_from_slice(&fp!(0.125).raw().to_le_bytes());
    safe[88..96].copy_from_slice(&fp!(0.875).raw().to_le_bytes());
    navigator
        .restore(
            &graph,
            &terrain,
            &NavigatorSnapshot::from_bytes(&safe).unwrap(),
        )
        .unwrap();
    assert_eq!(navigator.position(), [fp!(0.125), FP::ZERO, fp!(0.875)]);
}
