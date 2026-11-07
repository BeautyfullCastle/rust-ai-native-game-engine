//! CPU matrix algebra only. Arbitrary body rotation tests this general helper;
//! admitted Room PLAYER gameplay bodies retain their existing identity rotation.
#![allow(clippy::float_arithmetic)]
use super::*;
use orr_view::{Quat, Vec3};
fn point(m: orr_model::animation::Matrix4, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}
fn close(a: [f32; 3], b: [f32; 3]) {
    for i in 0..3 {
        assert!((a[i] - b[i]).abs() < 0.00002, "{a:?} != {b:?}");
    }
}
#[test]
fn body_yaw_local_order_matches_independent_scaled_rotated_point_oracle() {
    let body = Transform3::new(
        Vec3::new(7.0, -2.0, 5.0),
        Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), std::f32::consts::FRAC_PI_2),
    );
    let local = LocalTransform {
        translation: [2.0, 3.0, -4.0],
        rotation: Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), std::f32::consts::FRAC_PI_2)
            .to_array(),
        scale: [2.0, 3.0, 0.5],
    };
    let before = local;
    let matrix = facing_world_matrix(body, local, std::f32::consts::FRAC_PI_2).unwrap();
    for p in [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, -2.0, 3.0],
    ] {
        // Scale, local Rz(+90), local translation, facing Ry(+90),
        // body Rx(+90), body translation. No matrix/quat helper in this oracle.
        let local = [-p[1] * 3.0 + 2.0, p[0] * 2.0 + 3.0, p[2] * 0.5 - 4.0];
        let facing = [local[2], local[1], -local[0]];
        let expected = [facing[0] + 7.0, -facing[2] - 2.0, facing[1] + 5.0];
        close(point(matrix, p), expected);
    }
    assert_eq!(local, before);
}
#[test]
fn zero_heading_is_bit_exact_legacy_and_nonfinite_yaw_is_rejected() {
    let body = Transform3::new(
        Vec3::new(-0.0, 2.0, 3.0),
        Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), 0.37),
    );
    let local = LocalTransform {
        translation: [1.0, -0.0, 2.0],
        rotation: Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), 0.29).to_array(),
        scale: [0.5, 2.0, 1.25],
    };
    let expected = world_matrix(body, local);
    for yaw in [0.0, -0.0] {
        let actual = facing_world_matrix(body, local, yaw).unwrap();
        for c in 0..4 {
            for r in 0..4 {
                assert_eq!(actual[c][r].to_bits(), expected[c][r].to_bits());
            }
        }
    }
    for yaw in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(facing_world_matrix(body, local, yaw).is_err());
    }
}
#[test]
fn yaw_rotates_offsets_about_body_origin_and_does_not_commute_with_local_rotation() {
    let body = Transform3::new(Vec3::new(5.0, 2.0, 3.0), Quat::IDENTITY);
    let local = LocalTransform {
        translation: [2.0, 0.0, 0.0],
        rotation: Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), std::f32::consts::FRAC_PI_2)
            .to_array(),
        scale: [1.0; 3],
    };
    let m = facing_world_matrix(body, local, std::f32::consts::FRAC_PI_2).unwrap();
    close(point(m, [0.0; 3]), [5.0, 2.0, 1.0]);
    close(point(m, [0.0, 0.0, 1.0]), [6.0, 2.0, 1.0]);
    // Swapping local Rz with facing Ry sends +Z toward +Y instead.
    let swapped = [5.0, 3.0, 1.0];
    let correct = point(m, [0.0, 0.0, 1.0]);
    assert!((correct[0] - swapped[0]).abs() > 0.5);
    assert!((correct[1] - swapped[1]).abs() > 0.5);
}
