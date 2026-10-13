//! Public-API oracles derived from planes, line capsules, and point spheres.
//! No expected contact is obtained from the production intersection routines.
#![allow(clippy::float_arithmetic)]

use orr_terrain::Terrain;
use orr_terrain_camera::{
    BlockedReason, BoomHit, BoomQuery, BoomResolution, CameraBoomError, CameraPose,
    TerrainCameraObstacles, TriangleId, ARITHMETIC_ENVELOPE, CONTACT_BACKOFF, NUMERIC_PAD,
    ROOT_REFINEMENTS,
};

const Q16: i64 = 65_536;
const ROUNDING: f64 = 2.0e-8;

fn terrain(
    width: u32,
    depth: u32,
    origin: [i64; 2],
    spacing: i64,
    heights: &[i64],
    holes: &[bool],
) -> Terrain {
    let id = b"tests/camera.orrt";
    let mut bytes = b"ORRTHF\x01\0".to_vec();
    bytes.extend((id.len() as u16).to_le_bytes());
    bytes.extend(id);
    bytes.extend(width.to_le_bytes());
    bytes.extend(depth.to_le_bytes());
    for value in [origin[0], origin[1], spacing] {
        bytes.extend(value.to_le_bytes());
    }
    for value in heights {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(holes.iter().map(|hole| u8::from(*hole)));
    Terrain::load(&bytes).expect("independently encoded canonical fixture")
}

fn square(side: i64) -> Terrain {
    terrain(2, 2, [0; 2], side * Q16, &[0; 4], &[false])
}

fn boom(obstacles: &TerrainCameraObstacles, pivot: [f64; 3], eye: [f64; 3]) -> BoomQuery {
    BoomQuery {
        expected_revision: obstacles.revision(),
        pivot,
        desired_eye: eye,
        radius: 1.0 - NUMERIC_PAD,
        margin: 0.0,
        min_distance: 1.0e-4,
    }
}

fn length(query: BoomQuery) -> f64 {
    query
        .desired_eye
        .iter()
        .zip(query.pivot)
        .map(|(eye, pivot)| (eye - pivot).powi(2))
        .sum::<f64>()
        .sqrt()
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual {actual:.17e}, expected {expected:.17e}, tolerance {tolerance:.3e}"
    );
}

fn retracted(obstacles: &TerrainCameraObstacles, query: BoomQuery) -> (CameraPose, BoomHit) {
    match obstacles.resolve(query).expect("valid finite query") {
        BoomResolution::Retracted { pose, hit } => (pose, hit),
        other => panic!("expected retraction, got {other:?}"),
    }
}

/// A monotone closed-form contact oracle is evaluated at the documented
/// geometrical radius and at its maximum arithmetic expansion. This handles
/// tangent and nearly parallel roots without a misleading fixed distance epsilon.
fn analytic_contact(
    obstacles: &TerrainCameraObstacles,
    query: BoomQuery,
    root: impl Fn(f64) -> f64,
) -> (CameraPose, BoomHit) {
    let (pose, hit) = retracted(obstacles, query);
    let radius = query.radius + query.margin + NUMERIC_PAD;
    let latest = root(radius);
    let earliest = root(radius + ARITHMETIC_ENVELOPE);
    assert!(
        hit.distance >= earliest - ROUNDING && hit.distance <= latest + ROUNDING,
        "contact {} outside independent [{earliest}, {latest}]",
        hit.distance
    );
    close(
        pose.distance,
        (hit.distance - CONTACT_BACKOFF).max(0.0),
        ROUNDING,
    );
    let length = length(query);
    for (axis, actual) in pose.eye.iter().enumerate() {
        close(
            *actual,
            query.pivot[axis]
                + (query.desired_eye[axis] - query.pivot[axis]) * pose.distance / length,
            ROUNDING,
        );
        assert!(actual.is_finite());
    }
    assert!(pose.distance >= query.min_distance && pose.distance <= length);
    // Compile-time checks that the consumer never silently receives f32 output.
    let _: f64 = pose.distance;
    let _: [f64; 3] = pose.eye;
    let _: f64 = hit.distance;
    (pose, hit)
}

#[test]
fn numerical_policy_is_explicit_and_independent_of_fixture_scale() {
    assert_eq!(NUMERIC_PAD, 1.0e-5);
    assert_eq!(CONTACT_BACKOFF, 1.0e-5);
    assert_eq!(ARITHMETIC_ENVELOPE, 1.0e-6);
}

#[test]
fn interior_face_entry_has_closed_form_and_stable_diagonal_tie() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let query = boom(&obstacles, [2.0, 3.0, 2.0], [2.0, -3.0, 2.0]);
    let (pose, hit) = analytic_contact(&obstacles, query, |radius| 3.0 - radius);
    assert_eq!(
        hit.triangle,
        TriangleId {
            cell: [0, 0],
            triangle: 0
        }
    );
    assert!(pose.eye[1] >= query.radius + query.margin);
    assert_eq!(
        obstacles.resolve(query).unwrap(),
        obstacles.resolve(query).unwrap()
    );
}

#[test]
fn edge_entry_is_not_a_height_sample_or_center_ray() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    // The sphere first touches the exterior z=0 edge while its center is outside.
    let query = boom(&obstacles, [2.0, 0.6, -2.0], [2.0, 0.6, 2.0]);
    let (_, hit) = analytic_contact(&obstacles, query, |r| 2.0 - (r * r - 0.36).sqrt());
    assert_eq!(
        hit.triangle,
        TriangleId {
            cell: [0, 0],
            triangle: 1
        }
    );
}

#[test]
fn vertex_entry_has_independently_derived_diagonal_root() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let query = boom(&obstacles, [-2.0, 0.6, -2.0], [2.0, 0.6, 2.0]);
    let (_, hit) = analytic_contact(&obstacles, query, |r| {
        2.0 * 2.0_f64.sqrt() - (r * r - 0.36).sqrt()
    });
    assert_eq!(
        hit.triangle,
        TriangleId {
            cell: [0, 0],
            triangle: 0
        }
    );
}

#[test]
fn isolated_edge_tangent_cannot_be_rescued_by_a_face_or_endpoint_sample() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    // Both endpoints are clear. The nearest point stays on the open z=0 edge;
    // its squared distance is 1 + y^2, with an isolated minimum at y=0.
    let query = boom(&obstacles, [2.0, -2.0, -1.0], [2.0, 2.0, -1.0]);
    analytic_contact(&obstacles, query, |r| 2.0 - (r * r - 1.0).max(0.0).sqrt());
}

#[test]
fn isolated_vertex_tangent_cannot_be_rescued_by_any_edge_interior() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    // x,z are strictly outside both boundaries. The sole closest point is the
    // corner, at exact squared transverse distance .75^2 + 1^2 = 1.25^2.
    let mut query = boom(&obstacles, [-0.75, -2.0, -1.0], [-0.75, 2.0, -1.0]);
    query.radius = 1.25 - NUMERIC_PAD;
    analytic_contact(&obstacles, query, |r| {
        2.0 - (r * r - 1.5625).max(0.0).sqrt()
    });
}

#[test]
fn face_parallel_edge_grazing_detects_the_first_contact() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let query = boom(&obstacles, [2.0, 1.0, -2.0], [2.0, 1.0, 2.0]);
    analytic_contact(&obstacles, query, |r| 2.0 - (r * r - 1.0).max(0.0).sqrt());
}

#[test]
fn a_genuine_grazing_miss_outside_the_entire_envelope_stays_clear() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let query = boom(&obstacles, [2.0, -2.0, -1.0001], [2.0, 2.0, -1.0001]);
    assert_eq!(
        obstacles.resolve(query).unwrap(),
        BoomResolution::Clear(CameraPose {
            eye: query.desired_eye,
            distance: 4.0
        })
    );
}

#[test]
fn near_parallel_interior_face_is_not_discarded_as_parallel() {
    let obstacles = TerrainCameraObstacles::new(&square(1024)).unwrap();
    let delta = 2.0_f64.powi(-18);
    for side in [-1.0, 1.0] {
        let query = boom(
            &obstacles,
            [1.0, side * (1.0 + delta), 512.0],
            [1023.0, side * (1.0 - delta), 512.0],
        );
        let travel = length(query);
        // The root is strictly inside the left triangle, far from all edges.
        // Either side of the two-sided face has the same algebraic slab root.
        analytic_contact(&obstacles, query, |r| {
            (1.0 + delta - r) * travel / (2.0 * delta)
        });
    }
}

#[test]
fn nearest_hit_wins_even_when_its_cell_is_visited_last() {
    let source = terrain(4, 2, [0; 2], 4 * Q16, &[0; 8], &[false, true, false]);
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    let query = boom(&obstacles, [14.0, 0.6, 2.0], [-2.0, 0.6, 2.0]);
    let (_, hit) = analytic_contact(&obstacles, query, |r| 2.0 - (r * r - 0.36).sqrt());
    assert_eq!(hit.triangle.cell, [2, 0]);
}

#[test]
fn shared_cell_corner_ties_use_canonical_z_x_triangle_order() {
    let source = terrain(3, 3, [0; 2], 2 * Q16, &[0; 9], &[false; 4]);
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    let query = boom(&obstacles, [2.0, 3.0, 2.0], [2.0, -3.0, 2.0]);
    let (_, hit) = analytic_contact(&obstacles, query, |r| 3.0 - r);
    assert_eq!(
        hit.triangle,
        TriangleId {
            cell: [0, 0],
            triangle: 0
        }
    );
}

#[test]
fn holes_remove_surfaces_but_do_not_remove_neighboring_sphere_contacts() {
    let source = terrain(4, 2, [0; 2], 4 * Q16, &[0; 8], &[false, true, false]);
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    let through_hole = boom(&obstacles, [6.0, 3.0, 2.0], [6.0, -3.0, 2.0]);
    assert!(matches!(
        obstacles.resolve(through_hole),
        Ok(BoomResolution::Clear(_))
    ));
    let near_edge = boom(&obstacles, [4.5, 3.0, 2.0], [4.5, -3.0, 2.0]);
    let (_, hit) = analytic_contact(&obstacles, near_edge, |r| 3.0 - (r * r - 0.25).sqrt());
    assert_eq!(hit.triangle.cell, [0, 0]);
}

#[test]
fn all_holes_are_empty_obstacles_even_for_a_cross_surface_boom() {
    let source = terrain(2, 2, [0; 2], 4 * Q16, &[0; 4], &[true]);
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    let query = boom(&obstacles, [2.0, 3.0, 2.0], [2.0, -3.0, 2.0]);
    assert_eq!(obstacles.triangle_count(), 0);
    assert!(obstacles.triangle(0).is_none());
    assert_eq!(
        obstacles.resolve(query).unwrap(),
        BoomResolution::Clear(CameraPose {
            eye: query.desired_eye,
            distance: 6.0
        })
    );
}

#[test]
fn minimum_spacing_thin_ridge_has_an_analytic_face_contact() {
    let s = 1.0 / 256.0;
    let source = terrain(3, 2, [0; 2], 256, &[0, Q16, 0, 0, Q16, 0], &[false; 2]);
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    for half_span in [2.0, 500_000.0] {
        let mut query = boom(
            &obstacles,
            [-half_span, 0.5, s / 2.0],
            [half_span, 0.5, s / 2.0],
        );
        query.radius = 1.0e-4;
        // On y=x/s, perpendicular distance is (s*y-x)/sqrt(1+s²).
        // The contact projection is inside the triangle. The million-unit
        // variant makes the narrow ridge much smaller than a sampling step.
        analytic_contact(&obstacles, query, |r| {
            half_span + 0.5 * s - r * (1.0 + s * s).sqrt()
        });
    }
}

#[test]
fn million_unit_boom_preserves_sub_f32_contact_position() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let query = boom(&obstacles, [-500_000.0, 0.6, 2.0], [500_000.0, 0.6, 2.0]);
    let (pose, _) = analytic_contact(&obstacles, query, |r| 500_000.0 - (r * r - 0.36).sqrt());
    assert!(pose.eye[0] < -0.8 && pose.eye[0] > -0.800_02);
    assert!((pose.eye[0] - f64::from(pose.eye[0] as f32)).abs() > 1.0e-10);
}

#[test]
fn collision_geometry_is_a_two_sided_surface_not_a_filled_half_space() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let away = boom(&obstacles, [2.0, -3.0, 2.0], [2.0, -5.0, 2.0]);
    assert!(matches!(
        obstacles.resolve(away),
        Ok(BoomResolution::Clear(_))
    ));
    let upward = boom(&obstacles, [2.0, -3.0, 2.0], [2.0, 3.0, 2.0]);
    let (pose, _) = analytic_contact(&obstacles, upward, |r| 3.0 - r);
    assert!(pose.eye[1] <= -upward.radius);
}

#[test]
fn start_overlap_and_exact_start_tangent_never_return_a_camera_pose() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    for y in [0.0, 0.5, 1.0, -0.5, -1.0] {
        let query = boom(&obstacles, [2.0, y, 2.0], [2.0, 4.0, 2.0]);
        assert!(
            matches!(
                obstacles.resolve(query),
                Ok(BoomResolution::Blocked {
                    reason: BlockedReason::StartOverlap,
                    hit: Some(_)
                })
            ),
            "start height {y}"
        );
    }
}

#[test]
fn minimum_usable_distance_is_a_closed_lower_bound_without_forced_extension() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let mut query = boom(&obstacles, [2.0, 3.0, 2.0], [2.0, -3.0, 2.0]);
    let (pose, _) = retracted(&obstacles, query);
    // Reusing only the reported safe distance tests API boundary policy; it is
    // deliberately not the geometric collision oracle used by the tests above.
    query.min_distance = pose.distance;
    assert!(matches!(
        obstacles.resolve(query),
        Ok(BoomResolution::Retracted { .. })
    ));
    query.min_distance = pose.distance - 1.0e-7;
    assert!(matches!(
        obstacles.resolve(query),
        Ok(BoomResolution::Retracted { .. })
    ));
    query.min_distance = pose.distance + 1.0e-7;
    assert!(matches!(
        obstacles.resolve(query),
        Ok(BoomResolution::Blocked {
            reason: BlockedReason::InsufficientDistance,
            hit: Some(_)
        })
    ));
    let mut clear = boom(&obstacles, [2.0, 3.0, 2.0], [2.0, 4.0, 2.0]);
    clear.min_distance = 1.0;
    assert!(matches!(
        obstacles.resolve(clear),
        Ok(BoomResolution::Clear(_))
    ));
    clear.min_distance = 1.01;
    assert!(matches!(
        obstacles.resolve(clear),
        Ok(BoomResolution::Blocked {
            reason: BlockedReason::InsufficientDistance,
            ..
        })
    ));
}

#[test]
fn radius_and_margin_are_additive_for_every_contact_feature() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    for (pivot, eye) in [
        ([2.0, 3.0, 2.0], [2.0, -3.0, 2.0]),
        ([2.0, 0.6, -2.0], [2.0, 0.6, 2.0]),
        ([-2.0, 0.6, -2.0], [2.0, 0.6, 2.0]),
    ] {
        let mut base = boom(&obstacles, pivot, eye);
        base.radius = 0.75;
        let mut split = base;
        split.radius = 0.25;
        split.margin = 0.5;
        assert_eq!(
            obstacles.resolve(base).unwrap(),
            obstacles.resolve(split).unwrap()
        );
    }
}

#[test]
fn nonfinite_values_in_every_query_field_are_rejected() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let base = boom(&obstacles, [2.0, 3.0, 2.0], [2.0, 4.0, 2.0]);
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for axis in 0..3 {
            for is_pivot in [true, false] {
                let mut query = base;
                if is_pivot {
                    query.pivot[axis] = invalid;
                } else {
                    query.desired_eye[axis] = invalid;
                }
                assert!(matches!(
                    obstacles.resolve(query),
                    Err(CameraBoomError::InvalidQuery(_))
                ));
            }
        }
        for scalar in 0..3 {
            let mut query = base;
            match scalar {
                0 => query.radius = invalid,
                1 => query.margin = invalid,
                _ => query.min_distance = invalid,
            }
            assert!(matches!(
                obstacles.resolve(query),
                Err(CameraBoomError::InvalidQuery(_))
            ));
        }
    }
}

#[test]
fn finite_out_of_range_values_and_zero_or_overlong_booms_are_rejected() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let base = boom(&obstacles, [2.0, 3.0, 2.0], [2.0, 4.0, 2.0]);
    for radius in [-1.0, 0.0, 0.000_099, 10_000.001] {
        let mut query = base;
        query.radius = radius;
        assert!(matches!(
            obstacles.resolve(query),
            Err(CameraBoomError::InvalidQuery(_))
        ));
    }
    for margin in [-0.000_001, 10_000.001] {
        let mut query = base;
        query.margin = margin;
        assert!(matches!(
            obstacles.resolve(query),
            Err(CameraBoomError::InvalidQuery(_))
        ));
    }
    for min_distance in [-1.0, 0.0, 0.000_099, 1_000_000.001] {
        let mut query = base;
        query.min_distance = min_distance;
        assert!(matches!(
            obstacles.resolve(query),
            Err(CameraBoomError::InvalidQuery(_))
        ));
    }
    for axis in 0..3 {
        for coordinate in [-1_000_000.001, 1_000_000.001] {
            let mut query = base;
            query.pivot[axis] = coordinate;
            assert!(matches!(
                obstacles.resolve(query),
                Err(CameraBoomError::InvalidQuery(_))
            ));
            query = base;
            query.desired_eye[axis] = coordinate;
            assert!(matches!(
                obstacles.resolve(query),
                Err(CameraBoomError::InvalidQuery(_))
            ));
        }
    }
    for (pivot, eye) in [
        ([0.0, 3.0, 0.0], [0.0, 3.0, 0.0]),
        ([0.0, 3.0, 0.0], [0.000_099, 3.0, 0.0]),
        ([-500_001.0, 3.0, 0.0], [500_001.0, 3.0, 0.0]),
    ] {
        let mut query = base;
        query.pivot = pivot;
        query.desired_eye = eye;
        assert!(matches!(
            obstacles.resolve(query),
            Err(CameraBoomError::InvalidQuery(_))
        ));
    }
}

#[test]
fn inclusive_query_limits_are_admitted_on_empty_geometry() {
    let source = terrain(2, 2, [0; 2], Q16, &[0; 4], &[true]);
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    let mut query = boom(&obstacles, [0.0, 0.0, 0.0], [0.0001, 0.0, 0.0]);
    query.radius = 0.0001;
    query.min_distance = 0.0001;
    assert!(matches!(
        obstacles.resolve(query),
        Ok(BoomResolution::Clear(_))
    ));
    query.pivot = [-1_000_000.0, 1_000_000.0, -1_000_000.0];
    query.desired_eye = [0.0, 1_000_000.0, -1_000_000.0];
    query.radius = 10_000.0;
    query.margin = 10_000.0;
    query.min_distance = 1_000_000.0;
    assert!(matches!(
        obstacles.resolve(query),
        Ok(BoomResolution::Clear(_))
    ));
}

#[test]
fn stale_revision_is_rejected_before_any_pose_is_returned() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let mut query = boom(&obstacles, [2.0, 3.0, 2.0], [2.0, 4.0, 2.0]);
    query.expected_revision[0] ^= 1;
    assert!(matches!(
        obstacles.resolve(query),
        Err(CameraBoomError::RevisionMismatch)
    ));
}

#[test]
fn reported_work_is_bounded_for_clear_hit_tangent_and_blocked_queries() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    for (pivot, eye) in [
        ([2.0, 3.0, 2.0], [2.0, 4.0, 2.0]),
        ([2.0, 3.0, 2.0], [2.0, -3.0, 2.0]),
        ([2.0, -2.0, -1.0], [2.0, 2.0, -1.0]),
        ([2.0, 0.5, 2.0], [2.0, 4.0, 2.0]),
    ] {
        let query = boom(&obstacles, pivot, eye);
        let (resolution, work) = obstacles.resolve_with_work(query).unwrap();
        assert_eq!(resolution, obstacles.resolve(query).unwrap());
        assert!(work.triangles > 0 && work.triangles <= obstacles.triangle_count());
        assert!(work.features > 0 && work.features <= work.triangles * 7);
        assert!(work.root_refinements <= work.triangles * 6 * ROOT_REFINEMENTS);
        assert!(work.bracket_probes <= work.triangles * 6 * 2);
        assert!(work.brackets_narrowed <= work.bracket_probes / 2);
    }
}

#[test]
fn contact_at_the_desired_endpoint_still_retracts_by_the_safety_backoff() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let query = boom(&obstacles, [2.0, 3.0, 2.0], [2.0, 1.0, 2.0]);
    let (pose, _) = analytic_contact(&obstacles, query, |r| 3.0 - r);
    assert!(pose.distance < length(query));
    assert!(pose.eye[1] > query.desired_eye[1]);
}

#[test]
fn maximum_slope_at_minimum_spacing_keeps_the_factored_face_contact() {
    let s = 1.0 / 256.0;
    let h = 1_000_000 * Q16;
    let source = terrain(2, 2, [0; 2], 256, &[-h, h, -h, h], &[false]);
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    let mut query = boom(&obstacles, [-1.0, 0.0, s / 2.0], [1.0, 0.0, s / 2.0]);
    query.radius = 1.0e-4;
    // The plane is x=s/2 + y*s/(2e6). This formulation does not obtain a
    // normal by subtracting nearly parallel, very long triangle edges.
    analytic_contact(&obstacles, query, |r| {
        1.0 + s / 2.0 - r * (1.0 + (s / 2_000_000.0).powi(2)).sqrt()
    });
}

#[test]
fn nearly_axial_edge_motion_is_not_discarded_as_parallel() {
    let obstacles = TerrainCameraObstacles::new(&square(1_000_000)).unwrap();
    let radius = 1.0e-4 + NUMERIC_PAD;
    let delta = 4.0e-6;
    let mut query = boom(
        &obstacles,
        [-(radius + delta), 0.0, 1.0],
        [-(radius - delta), 0.0, 999_999.0],
    );
    query.radius = 1.0e-4;
    let travel = length(query);
    // Both sphere centers are outside the face prism, far from vertices. The
    // nearest edge point has the same z; its distance is exactly -center.x.
    analytic_contact(&obstacles, query, |r| {
        (-query.pivot[0] - r) * travel / (query.desired_eye[0] - query.pivot[0])
    });
}

#[test]
fn certified_hint_saves_refinement_without_changing_the_edge_oracle() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let query = boom(&obstacles, [2.0, 0.6, -2.0], [2.0, 0.6, 2.0]);
    analytic_contact(&obstacles, query, |r| 2.0 - (r * r - 0.36).sqrt());
    let (_, work) = obstacles.resolve_with_work(query).unwrap();
    assert!(work.bracket_probes >= 2);
    assert!(work.brackets_narrowed > 0);
    // The two candidate edge entries need fewer than two full 48-step brackets.
    assert!(work.root_refinements < 2 * ROOT_REFINEMENTS);
}

#[test]
fn rounded_outer_tangent_rejects_an_uncertified_hint_and_keeps_full_bracket() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let mut query = boom(&obstacles, [2.0, -500_000.0, 0.0], [2.0, 500_000.0, 0.0]);
    query.radius = 10_000.0;
    query.margin = 10_000.0;
    let outer_radius = query.radius + query.margin + NUMERIC_PAD + ARITHMETIC_ENVELOPE;
    query.pivot[2] = -outer_radius;
    query.desired_eye[2] = -outer_radius;
    let (resolution, work) = obstacles.resolve_with_work(query).unwrap();
    let BoomResolution::Retracted { pose, hit } = resolution else {
        panic!("tangent must retain a conservative contact: {resolution:?}");
    };
    // A near-minimum trial endpoint's tiny squared displacement rounds away
    // beside radius^2. Its claimed outside status cannot be certified, so the
    // full bracket must survive. No expectation is derived from the hint.
    assert!(work.bracket_probes >= 2);
    assert_eq!(work.brackets_narrowed, 0);
    // The untouched [0,0.5] bracket takes47 bisections to reach2^-48.
    assert!(work.root_refinements >= ROOT_REFINEMENTS - 1);
    assert!(hit.distance > 499_999.0 && hit.distance <= 500_000.0);
    assert!(pose.eye.into_iter().all(f64::is_finite));
    assert!(pose.distance < hit.distance);
}

#[test]
fn long_boom_discriminant_cancellation_falls_back_to_the_corner_oracle() {
    let obstacles = TerrainCameraObstacles::new(&square(4)).unwrap();
    let radius = 1.0e-4 + NUMERIC_PAD;
    let mut query = boom(
        &obstacles,
        [-0.6 * radius, -500_000.0, -0.8 * radius],
        [-0.6 * radius, 500_000.0, -0.8 * radius],
    );
    query.radius = 1.0e-4;
    // The transverse radius term is lost beside the long-boom b² and a*c.
    // Geometry still has a continuous corner contact, independently derived
    // from the point sphere. The hint may not reject it or become the result.
    analytic_contact(&obstacles, query, |r| {
        500_000.0 - (r * r - radius * radius).max(0.0).sqrt()
    });
    let (_, work) = obstacles.resolve_with_work(query).unwrap();
    assert!(work.bracket_probes >= 2);
    assert_eq!(work.brackets_narrowed, 0);
    // The untouched [0,0.5] bracket takes47 bisections to reach2^-48.
    assert!(work.root_refinements >= ROOT_REFINEMENTS - 1);
}
