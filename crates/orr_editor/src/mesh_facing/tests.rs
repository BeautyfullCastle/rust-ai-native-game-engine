use super::{is_fixed, multiply, rotation_with_yaw, route_yaw_on_path, Facing};
use orr_fp::FP;

fn point(coordinate_x: i64, coordinate_y: i64, coordinate_z: i64) -> [FP; 3] {
    [coordinate_x, coordinate_y, coordinate_z].map(|value| FP::from_raw(value * FP::SCALE))
}

fn distance(units: i64) -> FP {
    FP::from_raw(units * FP::SCALE)
}

fn raw_point(coordinate_x: i64, coordinate_y: i64, coordinate_z: i64) -> [FP; 3] {
    [coordinate_x, coordinate_y, coordinate_z].map(FP::from_raw)
}

#[test]
fn fixed_is_default_and_route_serialization_is_strict() {
    assert_eq!(Facing::default(), Facing::Fixed);
    assert!(is_fixed(&Facing::Fixed));
    assert!(!is_fixed(&Facing::Route));
    assert_eq!(serde_json::to_string(&Facing::Fixed).unwrap(), "\"fixed\"");
    assert_eq!(serde_json::to_string(&Facing::Route).unwrap(), "\"route\"");
    assert!(serde_json::from_str::<Facing>("\"other\"").is_err());
    assert!(serde_json::from_str::<Facing>("{}").is_err());
}

#[test]
fn route_yaw_uses_segment_at_distance_and_falls_back_for_vertical_routes() {
    let north = [point(0, 0, 0), point(0, 0, 10)];
    assert_eq!(route_yaw_on_path(&north, distance(3)), Some(0.0));

    let east = [point(0, 0, 0), point(10, 0, 0)];
    let east_yaw = route_yaw_on_path(&east, distance(3)).unwrap();
    assert!((east_yaw - std::f32::consts::FRAC_PI_2).abs() < 1.0e-6);

    let turn_after_vertical = [
        point(0, 0, 0),
        point(0, 0, 2),
        point(0, 2, 2),
        point(4, 2, 2),
    ];
    let after_turn = route_yaw_on_path(&turn_after_vertical, distance(3)).unwrap();
    assert!((after_turn - std::f32::consts::FRAC_PI_2).abs() < 1.0e-6);

    let vertical = [point(0, 0, 0), point(0, 4, 0)];
    assert_eq!(route_yaw_on_path(&vertical, distance(1)), None);
    assert_eq!(route_yaw_on_path(&[point(0, 0, 0)], FP::ZERO), None);
}

#[test]
fn route_yaw_uses_exact_ceil_distance_before_and_at_non_square_boundary() {
    let points = [raw_point(0, 0, 0), raw_point(1, 0, 1), raw_point(6, 0, 1)];
    let before_boundary = route_yaw_on_path(&points, FP::from_raw(1)).unwrap();
    let at_boundary = route_yaw_on_path(&points, FP::from_raw(2)).unwrap();
    assert!((before_boundary - std::f32::consts::FRAC_PI_4).abs() < 1.0e-6);
    assert!((at_boundary - std::f32::consts::FRAC_PI_2).abs() < 1.0e-6);
}

#[test]
fn world_yaw_is_premultiplied_with_authored_rotation() {
    let authored = crate::mesh_rotation::from_degrees([30.0, 0.0, 0.0]).unwrap();
    let yaw = std::f32::consts::FRAC_PI_2;
    let route = [0.0, (yaw * 0.5).sin(), 0.0, (yaw * 0.5).cos()];
    let combined = rotation_with_yaw(Facing::Route, authored, Some(yaw)).unwrap();
    let expected = crate::mesh_rotation::validate_normalize(multiply(route, authored)).unwrap();
    let reversed = crate::mesh_rotation::validate_normalize(multiply(authored, route)).unwrap();
    for (actual, expected) in combined.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1.0e-6);
    }
    assert!(combined
        .into_iter()
        .zip(reversed)
        .any(|(left, right)| (left - right).abs() > 1.0e-3));
    assert_eq!(
        rotation_with_yaw(Facing::Route, authored, None).unwrap(),
        authored
    );
}

#[test]
fn fixed_and_degenerate_route_facing_preserve_authored_quaternion_bits() {
    let authored = [0.123_456_7, -0.234_567_8, 0.345_678_9, 0.901_234_5];
    crate::mesh_rotation::validate_normalize(authored).unwrap();
    assert_eq!(
        rotation_with_yaw(Facing::Fixed, authored, Some(1.25))
            .unwrap()
            .map(f32::to_bits),
        authored.map(f32::to_bits)
    );
    assert_eq!(
        rotation_with_yaw(Facing::Route, authored, None)
            .unwrap()
            .map(f32::to_bits),
        authored.map(f32::to_bits)
    );
}
