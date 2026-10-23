use super::{from_degrees, to_degrees, validate_normalize};

fn assert_rotation_matches(left: [f32; 4], right: [f32; 4]) {
    for basis in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
        let left_result = orr_render::math3::quat_rotate(left, basis);
        let right_result = orr_render::math3::quat_rotate(right, basis);
        for axis in 0..3 {
            assert!((left_result[axis] - right_result[axis]).abs() < 1.0e-5);
        }
    }
}

#[test]
fn axis_rotations_follow_renderer_xyzw_convention() {
    let rotation_x = from_degrees([90.0, 0.0, 0.0]).unwrap();
    let rotation_y = from_degrees([0.0, 90.0, 0.0]).unwrap();
    let rotation_z = from_degrees([0.0, 0.0, 90.0]).unwrap();
    let rotated_y = orr_render::math3::quat_rotate(rotation_x, [0.0, 1.0, 0.0]);
    let rotated_x = orr_render::math3::quat_rotate(rotation_y, [1.0, 0.0, 0.0]);
    let rotated_x_by_z = orr_render::math3::quat_rotate(rotation_z, [1.0, 0.0, 0.0]);

    assert!((rotated_y[2] - 1.0).abs() < 1.0e-5);
    assert!((rotated_x[2] + 1.0).abs() < 1.0e-5);
    assert!((rotated_x_by_z[1] - 1.0).abs() < 1.0e-5);
}

#[test]
fn euler_composition_applies_x_then_y_then_z() {
    let combined = from_degrees([90.0, 90.0, 0.0]).unwrap();
    let rotated_y = orr_render::math3::quat_rotate(combined, [0.0, 1.0, 0.0]);
    assert!((rotated_y[0] - 1.0).abs() < 1.0e-5);
    assert!(rotated_y[1].abs() < 1.0e-5);
    assert!(rotated_y[2].abs() < 1.0e-5);
}

#[test]
fn identity_and_quaternion_sign_are_canonical() {
    assert_eq!(from_degrees([0.0; 3]).unwrap(), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(
        validate_normalize([0.0, 0.0, 0.0, -2.0]).unwrap(),
        [0.0, 0.0, 0.0, 1.0]
    );
    assert_eq!(
        validate_normalize([0.0, 0.0, -2.0, 0.0]).unwrap(),
        [0.0, 0.0, 1.0, 0.0]
    );
    assert_eq!(
        validate_normalize([-0.0, 0.0, 0.0, 2.0]).unwrap(),
        [0.0, 0.0, 0.0, 1.0]
    );
}

#[test]
fn nonunit_quaternions_normalize_and_preserve_orientation() {
    let source = from_degrees([23.0, -47.0, 81.0]).unwrap();
    let scaled = source.map(|component| component * 12.0);
    let normalized = validate_normalize(scaled).unwrap();
    let round_trip = from_degrees(to_degrees(normalized).unwrap()).unwrap();

    assert_rotation_matches(source, normalized);
    assert_rotation_matches(source, round_trip);
    let length = normalized
        .iter()
        .map(|component| component * component)
        .sum::<f32>();
    assert!((length - 1.0).abs() < 1.0e-6);
}

#[test]
fn gimbal_singularities_have_stable_zero_roll_representations() {
    for pitch in [-90.0, 90.0] {
        let source = from_degrees([35.0, pitch, 70.0]).unwrap();
        let angles = to_degrees(source).unwrap();
        let round_trip = from_degrees(angles).unwrap();

        assert_eq!(angles[0], 0.0);
        assert_eq!(angles[1], pitch);
        assert_rotation_matches(source, round_trip);
    }
}

#[test]
fn near_gimbal_round_trips_preserve_orientation_without_snapping_to_the_pole() {
    for pitch in [-90.02, -89.98, 89.98, 90.02] {
        let source = from_degrees([35.0, pitch, 70.0]).unwrap();
        let angles = to_degrees(source).unwrap();
        assert!(angles[0].abs() > 1.0);
        assert_rotation_matches(source, from_degrees(angles).unwrap());
    }
}

#[test]
fn nonfinite_zero_and_unbounded_inputs_are_rejected() {
    assert!(from_degrees([f32::NAN, 0.0, 0.0]).is_err());
    assert!(from_degrees([f32::INFINITY, 0.0, 0.0]).is_err());
    assert!(from_degrees([360_001.0, 0.0, 0.0]).is_err());
    assert!(validate_normalize([0.0; 4]).is_err());
    assert!(validate_normalize([f32::NAN, 0.0, 0.0, 1.0]).is_err());
    assert!(validate_normalize([f32::MAX, 0.0, 0.0, 0.0]).is_err());
    assert!(validate_normalize([1_000_000.0, 1_000_000.0, 0.0, 0.0]).is_err());
}
