use super::*;
fn orbit() -> OrbitCamera {
    OrbitCamera::new([0., 2., 0.], 0.7, 0.4, 20.)
}
fn points() -> Vec<[f32; 3]> {
    vec![[-8., 0., -2.], [4., 3., -7.], [9., 1., 6.], [0., 7., 1.]]
}
fn check(camera: &Camera3D, points: &[[f32; 3]], size: (u32, u32)) {
    for p in points {
        let pixel = camera.world_to_screen(*p, size).unwrap();
        assert!(pixel[0] > 0. && pixel[0] < size.0 as f32);
        assert!(pixel[1] > 0. && pixel[1] < size.1 as f32);
        let c = camera
            .view_proj(size.0 as f32 / size.1 as f32)
            .transform_point4(*p);
        assert!(c[2] / c[3] > 0. && c[2] / c[3] < 1.);
    }
}
#[test]
fn perspective_complete_route_fits_portrait_and_landscape_with_margin_preserving_angles() {
    let original = orbit();
    let route = points();
    for size in [(800, 600), (300, 900)] {
        let fit = fit_route(original, None, &route, size).unwrap();
        assert_eq!(
            (fit.yaw, fit.pitch, fit.fov_y_degrees),
            (original.yaw, original.pitch, original.fov_y_degrees)
        );
        let (center, radius) = enclosing(&route).unwrap();
        assert_eq!(fit.target, center);
        math::validate_fit(&fit.camera(), center, radius, size).unwrap();
        check(&fit.camera(), &route, size);
    }
    assert_eq!(original.target, orbit().target);
}
#[test]
fn static_orthographic_uses_actual_authored_scale_projection_and_clipping() {
    let doc = crate::mesh_camera::Document::from_orbit(orbit())
        .unwrap()
        .with_projection(
            crate::mesh_camera::Projection::Orthographic { half_height: 5. },
            0.2,
            500.,
        )
        .unwrap();
    let before = doc.clone();
    for size in [(900, 400), (300, 900)] {
        let fit = fit_route(orbit(), Some(&doc), &points(), size).unwrap();
        let c = doc.camera(&fit, size).unwrap();
        check(&c, &points(), size);
        let orr_render::Projection::Orthographic { near, far, .. } = c.projection else {
            panic!("expected orthographic projection")
        };
        assert_eq!((near, far), (0.2, 500.));
        assert_eq!((fit.yaw, fit.pitch), (orbit().yaw, orbit().pitch));
    }
    assert_eq!(doc, before);
}
#[test]
fn enclosing_sphere_uses_every_point_including_off_diagonal_outlier() {
    let stationary = [[4., 2., -3.]; 3];
    let original = orbit();
    let framed = fit_route(original, None, &stationary, (800, 600)).unwrap();
    assert_eq!(framed.target, stationary[0]);
    assert_eq!(framed.distance, original.distance);
    check(&framed.camera(), &stationary, (800, 600));

    let route = vec![[-3., 0., 0.], [3., 0., 0.], [0., 8., 6.], [0., -2., -4.]];
    let (center, radius) = enclosing(&route).unwrap();
    assert_eq!(center, [0., 3., 1.]);
    for p in route {
        let d = (0..3)
            .map(|i| (f64::from(p[i]) - f64::from(center[i])).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!(d <= f64::from(radius));
    }
}
#[test]
fn invalid_counts_coordinates_and_viewports_reject_without_clamping() {
    for route in [
        vec![],
        vec![[0.; 3]; MAX_POINTS + 1],
        vec![[f32::NAN, 0., 0.]],
        vec![[257., 0., 0.]],
    ] {
        assert!(fit_route(orbit(), None, &route, (800, 600)).is_err());
    }
    for size in [(0, 600), (800, 0), (16385, 600)] {
        assert!(fit_route(orbit(), None, &points(), size).is_err());
    }
    let mut invalid = orbit();
    invalid.yaw = f32::INFINITY;
    assert!(fit_route(invalid, None, &points(), (800, 600)).is_err());
}
#[test]
fn follow_far_clipping_and_distance_overflow_fail_instead_of_truncating_route() {
    let d = crate::mesh_camera::Document::from_orbit(orbit()).unwrap();
    let follow = d.clone().with_follow_offset(Some([0.; 3])).unwrap();
    assert!(fit_route(orbit(), Some(&follow), &points(), (800, 600)).is_err());
    let clipped = d
        .clone()
        .with_projection(crate::mesh_camera::Projection::Perspective, 0.1, 10.)
        .unwrap();
    assert!(fit_route(orbit(), Some(&clipped), &points(), (800, 600)).is_err());
    let wide = [[-256., -256., -256.], [256., 256., 256.]];
    assert!(fit_route(orbit(), None, &wide, (800, 600)).is_err());
    assert_eq!(
        d,
        crate::mesh_camera::Document::from_orbit(orbit()).unwrap()
    );
}
