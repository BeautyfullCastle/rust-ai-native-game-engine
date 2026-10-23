use super::goal;
use crate::mesh_navigation::geometry_grid::{create, Grid};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};

fn profile() -> AgentProfile {
    AgentProfile {
        max_slope: fp!(4),
        ..AgentProfile::default()
    }
}

fn flat_graph(origin_x: i32, origin_z: i32, width: u32, depth: u32) -> StaticMeshGraph {
    create(
        Grid {
            origin: [
                FP::from_raw(i64::from(origin_x) * FP::SCALE),
                FP::ZERO,
                FP::from_raw(i64::from(origin_z) * FP::SCALE),
            ],
            width: FP::from_raw(i64::from(width) * FP::SCALE),
            depth: FP::from_raw(i64::from(depth) * FP::SCALE),
            columns: 1,
            rows: 1,
            excluded_cells: 0,
        },
        profile(),
    )
    .unwrap()
}

fn grid_graph(origin: [FP; 3], width: FP, depth: FP) -> StaticMeshGraph {
    create(
        Grid {
            origin,
            width,
            depth,
            columns: 1,
            rows: 1,
            excluded_cells: 0,
        },
        profile(),
    )
    .unwrap()
}

#[test]
fn cardinal_diagonal_and_negative_steps_are_exact_half_units() {
    let graph = flat_graph(-2, -2, 4, 4);
    let half = FP::from_raw(FP::SCALE / 2);
    for (direction, expected) in [
        ([1, 0], [half, FP::ZERO, FP::ZERO]),
        ([-1, 0], [-half, FP::ZERO, FP::ZERO]),
        ([0, 1], [FP::ZERO, FP::ZERO, half]),
        ([0, -1], [FP::ZERO, FP::ZERO, -half]),
        ([1, -1], [half, FP::ZERO, -half]),
    ] {
        assert_eq!(
            goal(&graph, [FP::ZERO; 3], FP::ZERO, direction).unwrap(),
            expected
        );
    }
}

#[test]
fn invalid_direction_vectors_are_rejected() {
    let graph = flat_graph(-2, -2, 4, 4);
    for direction in [[0, 0], [2, 0], [0, -2], [-1, 2]] {
        assert!(goal(&graph, [FP::ZERO; 3], FP::ZERO, direction).is_err());
    }
}

#[test]
fn projection_tracks_the_walkable_surface_height() {
    let vertices = [
        [fp!(-1), fp!(0), fp!(-1)],
        [fp!(-1), fp!(2), fp!(1)],
        [fp!(1), fp!(2), fp!(1)],
        [fp!(1), fp!(0), fp!(-1)],
    ];
    let graph = StaticMeshGraph::bake(&vertices, &[[0, 1, 2], [0, 2, 3]], profile()).unwrap();
    assert_eq!(
        goal(&graph, [FP::ZERO, fp!(1), FP::ZERO], fp!(1), [0, 1]).unwrap(),
        [FP::ZERO, fp!(1.5), fp!(0.5)]
    );
}

#[test]
fn off_mesh_and_out_of_range_steps_are_rejected() {
    let graph = grid_graph([fp!(-1), FP::ZERO, fp!(-1)], fp!(2), fp!(2));
    assert!(goal(&graph, [fp!(0.75), FP::ZERO, FP::ZERO], FP::ZERO, [1, 0]).is_err());
    assert!(goal(&graph, [fp!(256), FP::ZERO, FP::ZERO], FP::ZERO, [1, 0]).is_err());
    assert!(goal(
        &graph,
        [FP::ZERO, FP::from_raw(256 * FP::SCALE + 1), FP::ZERO],
        FP::ZERO,
        [0, 1]
    )
    .is_err());
    assert!(goal(&graph, [FP::ZERO; 3], FP::from_raw(-1), [0, 1]).is_err());
}

#[test]
fn projected_goal_height_must_stay_within_managed_bounds() {
    let vertices = [
        [fp!(-1), fp!(257), fp!(-1)],
        [fp!(-1), fp!(257), fp!(1)],
        [fp!(1), fp!(257), fp!(1)],
        [fp!(1), fp!(257), fp!(-1)],
    ];
    let graph = StaticMeshGraph::bake(&vertices, &[[0, 1, 2], [0, 2, 3]], profile()).unwrap();
    assert!(goal(&graph, [FP::ZERO; 3], fp!(300), [0, 1]).is_err());
}

#[test]
fn candidate_on_a_disconnected_island_is_rejected() {
    let vertices = [
        [fp!(-1), FP::ZERO, fp!(-1)],
        [fp!(-1), FP::ZERO, fp!(1)],
        [FP::ZERO, FP::ZERO, fp!(1)],
        [FP::ZERO, FP::ZERO, fp!(-1)],
        [fp!(0.25), FP::ZERO, fp!(-1)],
        [fp!(0.25), FP::ZERO, fp!(1)],
        [fp!(1.25), FP::ZERO, fp!(1)],
        [fp!(1.25), FP::ZERO, fp!(-1)],
    ];
    let graph = StaticMeshGraph::bake(
        &vertices,
        &[[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]],
        profile(),
    )
    .unwrap();
    assert!(goal(&graph, [fp!(-0.25), FP::ZERO, FP::ZERO], FP::ZERO, [1, 0]).is_err());
}

#[test]
fn half_step_coordinate_overflow_is_not_saturated() {
    let graph = flat_graph(-2, -2, 4, 4);
    for position in [
        [FP::from_raw(i64::MAX), FP::ZERO, FP::ZERO],
        [FP::from_raw(i64::MIN), FP::ZERO, FP::ZERO],
    ] {
        assert!(goal(&graph, position, FP::ZERO, [1, 0]).is_err());
    }
}
