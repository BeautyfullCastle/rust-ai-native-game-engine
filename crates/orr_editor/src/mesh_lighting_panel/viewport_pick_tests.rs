use super::*;
fn camera() -> Camera3D {
    Camera3D {
        eye: [0.0, 10.0, 10.0],
        target: [0.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        projection: Projection::Perspective {
            fov_y: 60.0_f32.to_radians(),
            near: 0.1,
            far: 500.0,
        },
    }
}
fn approx(actual: [f32; 2], expected: [f32; 2]) {
    for i in 0..2 {
        assert!(
            (actual[i] - expected[i]).abs() < 0.0001,
            "{actual:?} != {expected:?}"
        );
    }
}
#[test]
fn perspective_and_orthographic_projected_ground_points_roundtrip() {
    let perspective = camera();
    let orthographic = Camera3D::orthographic(perspective.eye, perspective.target, 10.0);
    for camera in [perspective, orthographic] {
        for size in [(800, 600), (300, 900)] {
            for world in [[0.0, 0.0, 0.0], [1.5, 0.0, -2.0], [-1.0, 0.0, 3.0]] {
                let pixel = camera.world_to_screen(world, size).unwrap();
                approx(
                    ground_point(&camera, size, pixel).unwrap(),
                    [world[0], world[2]],
                );
            }
        }
    }
}
#[test]
fn parallel_behind_plane_and_degenerate_or_nonfinite_cameras_miss() {
    let mut parallel = camera();
    parallel.eye = [0.0, 5.0, 10.0];
    parallel.target = [0.0, 5.0, 0.0];
    assert!(ground_point(&parallel, (800, 600), [400.0, 300.0]).is_none());
    let mut behind = camera();
    behind.eye = [0.0, 5.0, 10.0];
    behind.target = [0.0, 10.0, 0.0];
    assert!(ground_point(&behind, (800, 600), [400.0, 300.0]).is_none());
    let mut same = camera();
    same.target = same.eye;
    let mut zero_up = camera();
    zero_up.up = [0.0; 3];
    let mut invalid = camera();
    invalid.eye[0] = f32::NAN;
    let mut invalid_projection = camera();
    invalid_projection.projection = Projection::Perspective {
        fov_y: f32::INFINITY,
        near: 0.1,
        far: 500.0,
    };
    for camera in [same, zero_up, invalid, invalid_projection] {
        assert!(ground_point(&camera, (800, 600), [400.0, 300.0]).is_none());
    }
}
#[test]
fn viewport_rejects_edges_nonfinite_and_zero_size_without_clamping() {
    for (size, pixel) in [
        ((0, 600), [0., 0.]),
        ((800, 0), [0., 0.]),
        ((800, 600), [-1., 300.]),
        ((800, 600), [800., 300.]),
        ((800, 600), [400., 600.]),
        ((800, 600), [f32::NAN, 0.]),
    ] {
        assert!(ground_point(&camera(), size, pixel).is_none());
    }
}
fn document() -> Document {
    let mut d = Document::from_lighting(orr_render::Lighting::default()).unwrap();
    d.point_light = Some(orr_render::PointLight {
        position: [1., 7., 3.],
        color: [0.2, 0.4, 0.8],
        intensity: 9.,
        range: 17.,
    });
    d
}
#[test]
fn staging_only_replaces_xz_preserving_height_appearance_and_all_other_fields() {
    let original = document();
    let next = stage_point(&original, [-8., 19.]).unwrap();
    let mut expected = original.clone();
    expected.point_light.as_mut().unwrap().position = [-8., 7., 19.];
    assert_eq!(next, expected);
    for p in [[f32::NAN, 0.], [0., f32::INFINITY], [1.1e9, 0.]] {
        assert!(stage_point(&original, p).is_err());
        assert_eq!(original, document());
    }
    let mut absent = original.clone();
    absent.point_light = None;
    assert!(stage_point(&absent, [0., 0.]).is_err());
}
#[test]
fn source_raw_history_generation_and_host_pins_reject_stale_state_and_aba() {
    let document = document();
    let raw = vec!["raw 1.00".to_owned()];
    let pin = Pin {
        raw: raw.clone(),
        document: document.clone(),
        generation: 4,
        scene_epoch: 5,
        host_epoch: 6,
        checksum: 7,
    };
    assert!(pin.matches(&raw, &document, 4, (5, 6, Some(7))));
    assert!(!pin.matches(&["raw 1".into()], &document, 4, (5, 6, Some(7))));
    for (g, s, h, c) in [
        (5, 5, 6, Some(7)),
        (4, 8, 6, Some(7)),
        (4, 5, 8, Some(7)),
        (4, 5, 6, Some(8)),
        (4, 5, 6, None),
    ] {
        assert!(!pin.matches(&raw, &document, g, (s, h, c)));
    }
    let changed = stage_point(&document, [2., 3.]).unwrap();
    assert!(!pin.matches(&raw, &changed, 4, (5, 6, Some(7))));
    let mut panel = Panel::default();
    panel.viewport_pick.pending = Some(pin);
    panel.pick_generation = 4;
    panel.invalidate_viewport_pick();
    assert!(!panel.pick_pending());
    assert_eq!(panel.pick_generation, 5);
    panel.pick_generation = u64::MAX;
    panel.invalidate_viewport_pick();
    assert_eq!(panel.pick_generation, u64::MAX);
}
