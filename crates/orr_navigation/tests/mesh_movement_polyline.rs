use orr_fp::FP;
use orr_navigation::{
    mesh::{MeshPath, StaticMeshGraph},
    AgentProfile, NavigationError, SearchBudget,
};

fn raw(x: i64, y: i64, z: i64) -> [FP; 3] {
    [FP::from_raw(x), FP::from_raw(y), FP::from_raw(z)]
}

fn corner_path() -> (StaticMeshGraph, MeshPath) {
    let graph = StaticMeshGraph::bake(
        &[raw(0, 0, 0), raw(0, 0, 10), raw(10, 0, 10), raw(10, 0, 0)],
        &[[0, 1, 2], [0, 2, 3]],
        AgentProfile::default(),
    )
    .unwrap();
    let path = graph
        .find_path(
            raw(0, 0, 8),
            raw(8, 0, 2),
            FP::ZERO,
            SearchBudget::default(),
        )
        .unwrap();
    assert_eq!(path.points, vec![raw(0, 0, 8), raw(5, 0, 5), raw(8, 0, 2)]);
    (graph, path)
}

#[test]
fn full_and_partial_sweeps_keep_the_corner_instead_of_a_chord() {
    let (graph, path) = corner_path();
    let length = graph.path_length(&path).unwrap();
    // ceil(sqrt(34)) + ceil(sqrt(18)), in raw fixed-point units.
    assert_eq!(length.raw(), 11);
    assert_eq!(
        graph.movement_polyline(&path, FP::ZERO, length).unwrap(),
        path.points
    );
    assert_eq!(
        graph
            .movement_polyline(&path, FP::from_raw(3), FP::from_raw(8))
            .unwrap(),
        vec![raw(2, 0, 7), raw(5, 0, 5), raw(6, 0, 4)]
    );
    assert_eq!(
        graph
            .movement_polyline(&path, FP::from_raw(6), FP::from_raw(8))
            .unwrap(),
        vec![path.points[1], raw(6, 0, 4)]
    );
    assert_eq!(
        graph
            .movement_polyline(&path, FP::from_raw(3), FP::from_raw(6))
            .unwrap(),
        vec![raw(2, 0, 7), path.points[1]]
    );
}

#[test]
fn every_small_interval_agrees_with_existing_endpoint_sampling() {
    let (graph, path) = corner_path();
    for from_raw in 0..=14 {
        for to_raw in from_raw..=14 {
            let from = FP::from_raw(from_raw);
            let to = FP::from_raw(to_raw);
            let sweep = graph.movement_polyline(&path, from, to).unwrap();
            assert_eq!(
                sweep.first(),
                Some(&graph.point_at_distance(&path, from).unwrap())
            );
            assert_eq!(
                sweep.last(),
                Some(&graph.point_at_distance(&path, to).unwrap())
            );
            assert!(sweep.len() <= path.points.len());
            assert!(sweep.windows(2).all(|pair| pair[0] != pair[1]));
            if from_raw < 6 && to_raw > 6 {
                assert!(sweep.contains(&path.points[1]));
            }
            if from_raw == to_raw || from_raw >= 11 {
                assert_eq!(sweep.len(), 1);
            }
        }
    }
}

#[test]
fn zero_length_segments_and_stationary_paths_are_deduplicated() {
    let (graph, path) = corner_path();
    let portal = path.points[1];
    for position in [path.points[0], portal, path.points[2]] {
        let stationary = graph
            .find_path(position, position, FP::ZERO, SearchBudget::default())
            .unwrap();
        assert_eq!(
            graph
                .movement_polyline(&stationary, FP::ZERO, FP::ONE)
                .unwrap(),
            vec![position]
        );
    }
    let mut zero_first = path.clone();
    zero_first.points[0] = portal;
    let mut zero_last = path.clone();
    zero_last.points[2] = portal;
    for route in [zero_first, zero_last] {
        let mut expected = route.points.clone();
        expected.dedup();
        assert_eq!(
            graph.movement_polyline(&route, FP::ZERO, FP::ONE).unwrap(),
            expected
        );
    }
}

#[test]
fn fractional_negative_height_uses_the_existing_lattice_projection() {
    let graph = StaticMeshGraph::bake(
        &[raw(0, 0, 0), raw(0, -1, 65_536), raw(65_536, 0, 0)],
        &[[0, 1, 2]],
        AgentProfile::default(),
    )
    .unwrap();
    let route = graph
        .find_path(
            raw(0, 0, 0),
            raw(0, -1, 65_536),
            FP::ZERO,
            SearchBudget::default(),
        )
        .unwrap();
    let sweep = graph
        .movement_polyline(&route, FP::from_raw(16_384), FP::from_raw(32_768))
        .unwrap();
    assert_eq!(sweep, vec![raw(0, -1, 16_383), raw(0, -1, 32_767)]);
    for (position, distance) in sweep.iter().zip([16_384, 32_768]) {
        assert_eq!(
            *position,
            graph
                .point_at_distance(&route, FP::from_raw(distance))
                .unwrap()
        );
        assert_eq!(
            graph.project(*position, FP::ZERO).unwrap().position,
            *position
        );
    }
}

#[test]
fn invalid_distances_and_forged_paths_fail_without_bypassing_validation() {
    let (graph, path) = corner_path();
    for (from, to) in [(-1, 0), (0, -1), (8, 3)] {
        assert_eq!(
            graph.movement_polyline(&path, FP::from_raw(from), FP::from_raw(to)),
            Err(NavigationError::InvalidDistance)
        );
    }
    let mut stale = path.clone();
    stale.revision[0] ^= 1;
    assert_eq!(
        graph.movement_polyline(&stale, FP::ZERO, FP::ZERO),
        Err(NavigationError::StaleGraph)
    );
    let mut forged = path.clone();
    forged.points[1] = raw(4, 0, 4);
    assert_eq!(
        graph.movement_polyline(&forged, FP::ZERO, FP::ZERO),
        Err(NavigationError::InvalidSegment)
    );
    let mut empty = path;
    empty.corridor.clear();
    empty.points.clear();
    assert_eq!(
        graph.movement_polyline(&empty, FP::ZERO, FP::ZERO),
        Err(NavigationError::InvalidSegment)
    );
}

#[test]
fn maximum_adapter_corridor_stays_within_the_movement_point_budget() {
    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    for column in 0..=256 {
        vertices.push(raw(column * 10, 0, 0));
        vertices.push(raw(column * 10, 0, 10));
    }
    for column in 0..256u32 {
        let base = column * 2;
        faces.push([base, base + 1, base + 3]);
        faces.push([base, base + 3, base + 2]);
    }
    let graph = StaticMeshGraph::bake(&vertices, &faces, AgentProfile::default()).unwrap();
    let path = graph
        .find_path(
            raw(0, 0, 9),
            raw(2_560, 0, 1),
            FP::ZERO,
            SearchBudget {
                max_expansions: 512,
            },
        )
        .unwrap();
    assert_eq!(path.corridor.len(), 512);
    let length = graph.path_length(&path).unwrap();
    let full = graph.movement_polyline(&path, FP::ZERO, length).unwrap();
    assert_eq!(full, path.points);
    assert_eq!(full.len(), 513);
    let partial = graph
        .movement_polyline(&path, FP::from_raw(1), FP::from_raw(length.raw() - 1))
        .unwrap();
    assert!(partial.len() <= 514);
    assert_eq!(
        &partial[1..partial.len() - 1],
        &path.points[1..path.points.len() - 1]
    );
}
