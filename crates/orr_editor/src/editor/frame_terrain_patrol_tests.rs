use super::*;
fn orbit() -> OrbitCamera {
    OrbitCamera::new([1., 2., -3.], 0.8, 0.45, 24.)
}
fn route() -> Vec<[f32; 3]> {
    vec![
        [-7., -2., -4.],
        [-2., 6., -1.],
        [3., 9., 2.],
        [8., 1., 7.],
        [-7., -2., -4.],
    ]
}
fn verify(camera: &orr_render::Camera3D, points: &[[f32; 3]], size: (u32, u32)) {
    for point in points {
        let clip = camera
            .view_proj(size.0 as f32 / size.1 as f32)
            .transform_point4(*point);
        assert!(clip[3] > 0.);
        let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
        assert!(ndc[0].abs() < 1. && ndc[1].abs() < 1.);
        assert!((0.0..1.0).contains(&ndc[2]));
    }
}
#[test]
fn ramp_approach_bends_and_closure_fit_landscape_portrait_preserving_perspective() {
    let original = orbit();
    let points = route();
    for size in [(800, 600), (300, 900), (1600, 400)] {
        let fitted = fit_route(original, &points, size).unwrap();
        assert_eq!(
            (fitted.yaw, fitted.pitch, fitted.fov_y_degrees),
            (original.yaw, original.pitch, original.fov_y_degrees)
        );
        verify(&fitted.camera(), &points, size);
        let (_, radius) =
            super::super::frame_route_points::enclosing(&points, MAX_POINTS, 256.).unwrap();
        super::super::frame_selection::math::validate_fit(
            &fitted.camera(),
            fitted.target,
            radius,
            size,
        )
        .unwrap();
        assert!(matches!(
            fitted.camera().projection,
            orr_render::Projection::Perspective { .. }
        ));
    }
    assert_eq!(original, orbit());
}
#[test]
fn full_three_dimensional_height_outlier_sets_center_and_enclosing_radius() {
    let points = [[0., -12., 0.], [-5., 0., -3.], [5., 0., 3.], [0., 20., 0.]];
    let (center, radius) =
        super::super::frame_route_points::enclosing(&points, MAX_POINTS, 256.).unwrap();
    assert_eq!(center, [0., 4., 0.]);
    assert!(radius >= 16.);
    let result = fit_route(orbit(), &points, (800, 600)).unwrap();
    assert_eq!(result.target, center);
    verify(&result.camera(), &points, (800, 600));
}
#[test]
fn stationary_patrol_centers_only_and_preserves_distance_and_authored_view_angles() {
    let points = [[12., 7., -9.]; 4];
    let original = orbit();
    for size in [(800, 600), (300, 900)] {
        let result = fit_route(original, &points, size).unwrap();
        assert_eq!(result.target, points[0]);
        assert_eq!(
            (
                result.distance,
                result.yaw,
                result.pitch,
                result.fov_y_degrees
            ),
            (
                original.distance,
                original.yaw,
                original.pitch,
                original.fov_y_degrees
            )
        );
        verify(&result.camera(), &points, size);
    }
    assert_eq!(original, orbit());
}
#[test]
fn bounds_empty_oversized_nonfinite_and_bad_viewport_reject_without_clamping() {
    for points in [
        vec![],
        vec![[0.; 3]; MAX_POINTS + 1],
        vec![[f32::NAN, 0., 0.]],
        vec![[0., f32::INFINITY, 0.]],
        vec![[256.01, 0., 0.]],
    ] {
        assert!(fit_route(orbit(), &points, (800, 600)).is_err());
    }
    for size in [(0, 600), (800, 0), (16385, 600)] {
        assert!(fit_route(orbit(), &route(), size).is_err());
    }
    let at_limit = [[256., 0., 0.], [255., 1., 0.]];
    assert!(fit_route(orbit(), &at_limit, (800, 600)).is_ok());
}
#[test]
fn camera_limit_clipping_and_nonfinite_inputs_fail_atomically_instead_of_partial_fit() {
    let original = orbit();
    let wide = [[-256., -256., -256.], [256., 256., 256.]];
    assert!(fit_route(original, &wide, (800, 600)).is_err());
    let mut wide_angle = original;
    wide_angle.fov_y_degrees = 150.;
    assert!(fit_route(
        wide_angle,
        &[[-144., -144., -144.], [144., 144., 144.]],
        (800, 600)
    )
    .is_err());
    let mut invalid = original;
    invalid.yaw = f32::NAN;
    assert!(fit_route(invalid, &route(), (800, 600)).is_err());
    let mut invalid = original;
    invalid.distance = OrbitCamera::MAX_DISTANCE + 1.;
    assert!(fit_route(invalid, &[[1., 2., 3.]; 2], (800, 600)).is_err());
    assert_eq!(original, orbit());
}
