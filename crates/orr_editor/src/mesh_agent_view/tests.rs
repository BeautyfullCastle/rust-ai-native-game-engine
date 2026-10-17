use super::*;
use orr_navigation::AgentProfile;

fn point(x_coordinate: i32, y_coordinate: i32, z_coordinate: i32) -> [FP; 3] {
    [
        FP::from_int(x_coordinate),
        FP::from_int(y_coordinate),
        FP::from_int(z_coordinate),
    ]
}

fn raw_point(x_coordinate: i64, y_coordinate: i64, z_coordinate: i64) -> [FP; 3] {
    [
        FP::from_raw(x_coordinate),
        FP::from_raw(y_coordinate),
        FP::from_raw(z_coordinate),
    ]
}

fn floor() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(0, 0, 1),
            point(1, 0, 0),
            point(1, 0, 1),
        ],
        &[[0, 1, 3], [0, 3, 2]],
        AgentProfile::default(),
    )
    .unwrap()
}

fn horizontal_triangle(y_coordinate: i32, x_offset: i32) -> [[FP; 3]; 3] {
    [
        point(x_offset, y_coordinate, 0),
        point(x_offset, y_coordinate, 1),
        point(x_offset + 1, y_coordinate, 0),
    ]
}

fn one_triangle(points: [[FP; 3]; 3]) -> StaticMeshGraph {
    StaticMeshGraph::bake(&points, &[[0, 1, 2]], AgentProfile::default()).unwrap()
}

#[test]
fn picks_floor_and_returns_the_authoritative_projected_fixed_point() {
    let graph = floor();
    let hit = pick(&graph, [0.25, 3.0, 0.25], [0.0, -1.0, 0.0], FP::ZERO)
        .unwrap()
        .unwrap();
    let projected = graph
        .project(
            [FP::from_raw(16_384), FP::from_int(3), FP::from_raw(16_384)],
            FP::from_int(3),
        )
        .unwrap();
    assert_eq!(hit.position, projected.position);
    assert_eq!(hit.triangle, projected.triangle);
    let distant_camera = pick(&graph, [0.25, 512.0, 0.25], [0.0, -1.0, 0.0], FP::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(distant_camera, hit);
}

#[test]
fn ramps_quantize_then_require_projection_to_the_hit_triangle() {
    let graph = StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(1, 1, 0),
            point(1, 1, 1),
            point(2, 1, 0),
        ],
        &[[0, 2, 1], [1, 2, 3]],
        AgentProfile::default(),
    )
    .unwrap();
    let hit = pick(&graph, [0.5, 2.0, 0.25], [0.0, -2.0, 0.0], FP::ONE)
        .unwrap()
        .unwrap();
    assert_eq!(hit.position[0], FP::HALF);
    assert_eq!(hit.position[1], FP::HALF);
    assert_eq!(hit.position[2], FP::from_raw(16_384));
    assert_eq!(
        graph.project(hit.position, FP::ZERO).unwrap().triangle,
        hit.triangle
    );
}

#[test]
fn nearest_layer_and_round_trip_keep_the_same_surface_hit() {
    let lower = horizontal_triangle(0, 0);
    let upper = horizontal_triangle(2, 0);
    let graph = StaticMeshGraph::bake(
        &[lower[0], lower[1], lower[2], upper[0], upper[1], upper[2]],
        &[[0, 1, 2], [3, 4, 5]],
        AgentProfile::default(),
    )
    .unwrap();
    let loaded = StaticMeshGraph::from_bytes(&graph.to_bytes()).unwrap();
    let hit = pick(&loaded, [0.25, 3.0, 0.25], [0.0, -1.0, 0.0], FP::ONE)
        .unwrap()
        .unwrap();
    assert_eq!(hit.position[1], FP::from_int(2));
    assert_eq!(
        hit.triangle,
        loaded
            .project(
                [FP::from_raw(16_384), FP::from_int(3), FP::from_raw(16_384)],
                FP::ONE,
            )
            .unwrap()
            .triangle
    );
}

#[test]
fn seam_ties_are_resolved_by_stable_triangle_key() {
    let graph = floor();
    let hit = pick(&graph, [0.5, 2.0, 0.5], [0.0, -1.0, 0.0], FP::ZERO)
        .unwrap()
        .unwrap();
    let expected = graph
        .project([FP::HALF, FP::ZERO, FP::HALF], FP::ZERO)
        .unwrap();
    assert_eq!(hit.triangle, expected.triangle);
    assert_eq!(hit.position, expected.position);
}

#[test]
fn disconnected_geometry_picks_only_the_surface_under_the_ray() {
    let left = horizontal_triangle(0, 0);
    let right = horizontal_triangle(0, 3);
    let graph = StaticMeshGraph::bake(
        &[left[0], left[1], left[2], right[0], right[1], right[2]],
        &[[0, 1, 2], [3, 4, 5]],
        AgentProfile::default(),
    )
    .unwrap();
    assert!(pick(&graph, [3.25, 2.0, 0.25], [0.0, -1.0, 0.0], FP::ZERO)
        .unwrap()
        .is_some());
    assert_eq!(
        pick(&graph, [2.0, 2.0, 0.25], [0.0, -1.0, 0.0], FP::ZERO).unwrap(),
        None
    );
}

#[test]
fn rejects_misses_parallel_and_behind_surface_rays() {
    let graph = floor();
    assert_eq!(
        pick(&graph, [2.0, 2.0, 2.0], [0.0, -1.0, 0.0], FP::ZERO).unwrap(),
        None
    );
    assert_eq!(
        pick(&graph, [0.5, 2.0, 0.5], [1.0, 0.0, 0.0], FP::ZERO).unwrap(),
        None
    );
    assert_eq!(
        pick(&graph, [0.5, -1.0, 0.5], [0.0, -1.0, 0.0], FP::ZERO).unwrap(),
        None
    );
}

#[test]
fn rejects_invalid_rays_vertical_tolerances_and_display_outliers() {
    let graph = floor();
    assert!(pick(&graph, [0.0; 3], [0.0; 3], FP::ZERO).is_err());
    assert!(pick(&graph, [0.0, 1.0, 0.0], [f32::NAN, -1.0, 0.0], FP::ZERO).is_err());
    assert!(pick(&graph, [1.0e6 + 1.0, 1.0, 0.0], [0.0, -1.0, 0.0], FP::ZERO).is_err());
    assert!(pick(&graph, [0.0, 1.0, 0.0], [1.0e6 + 1.0, -1.0, 0.0], FP::ZERO).is_err());
    assert!(pick(&graph, [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], FP::from_raw(-1)).is_err());

    let boundary_graph = one_triangle([point(255, 0, 0), point(255, 0, 1), point(256, 0, 0)]);
    assert!(pick(
        &boundary_graph,
        [255.25, 1.0, 0.25],
        [0.0, -1.0, 0.0],
        FP::ZERO
    )
    .unwrap()
    .is_some());

    let outlier_graph = one_triangle([point(255, 0, 0), point(255, 0, 1), point(257, 0, 0)]);
    assert!(pick(
        &outlier_graph,
        [255.25, 1.0, 0.25],
        [0.0, -1.0, 0.0],
        FP::ZERO
    )
    .is_err());
}

#[test]
fn subunit_raw_geometry_is_preserved_through_projection() {
    let graph = one_triangle([
        raw_point(0, 0, 0),
        raw_point(0, 0, 65_536),
        raw_point(65_536, 0, 0),
    ]);
    let hit = pick(&graph, [0.25, 1.0, 0.25], [0.0, -1.0, 0.0], FP::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(
        hit.position,
        [FP::from_raw(16_384), FP::ZERO, FP::from_raw(16_384)]
    );
}
