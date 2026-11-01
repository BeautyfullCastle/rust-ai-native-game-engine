use super::*;
use orr_render::camera3d::OrbitCamera;

fn document() -> Document {
    Document::from_orbit(OrbitCamera::new([2.0, 3.0, -1.0], 0.4, 0.3, 16.0))
        .unwrap()
        .with_follow_offset(Some([1.0, 2.0, 3.0]))
        .unwrap()
}

#[test]
fn projection_match_target_plane_screen_coordinates_and_roundtrip() {
    for size in [(1600, 900), (900, 1600), (800, 800), (320, 240)] {
        let original = document();
        let matched = match_projection(&original, size).unwrap();
        assert!(matches!(
            matched.projection,
            Projection::Orthographic { .. }
        ));
        let a = original.camera(&original.orbit(), size).unwrap();
        let b = matched.camera(&matched.orbit(), size).unwrap();
        let right = a.right();
        let up = a.camera_up();
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (-2.0, 1.5)] {
            let point = std::array::from_fn(|i| original.target[i] + right[i] * x + up[i] * y);
            let pa = a.world_to_screen(point, size).unwrap();
            let pb = b.world_to_screen(point, size).unwrap();
            assert!((pa[0] - pb[0]).abs() < 0.002, "{size:?}: {pa:?} {pb:?}");
            assert!((pa[1] - pb[1]).abs() < 0.002, "{size:?}: {pa:?} {pb:?}");
        }
        let restored = match_projection(&matched, size).unwrap();
        assert_eq!(restored.projection, Projection::Perspective);
        assert!((restored.fov_y_degrees - original.fov_y_degrees).abs() < 0.0001);
        for result in [&matched, &restored] {
            assert_eq!(result.target, original.target);
            assert_eq!(result.yaw, original.yaw);
            assert_eq!(result.pitch, original.pitch);
            assert_eq!(result.distance, original.distance);
            assert_eq!(result.near, original.near);
            assert_eq!(result.far, original.far);
            assert_eq!(result.follow_offset, original.follow_offset);
        }
    }
}

#[test]
fn projection_match_rejects_invalid_inputs_and_unrepresentable_outputs_without_clamping() {
    let source = document();
    let before = source.clone();
    for size in [(0, 9), (16, 0), (16385, 9), (16, 16385)] {
        assert!(match_projection(&source, size).is_err());
    }
    let mut invalid = source.clone();
    invalid.fov_y_degrees = f32::NAN;
    assert!(match_projection(&invalid, (16, 9)).is_err());
    let too_wide = source
        .clone()
        .with_projection(
            Projection::Orthographic {
                half_height: 10000.0,
            },
            source.near,
            source.far,
        )
        .unwrap();
    assert!(match_projection(&too_wide, (16, 9)).is_err());
    let too_narrow = source
        .clone()
        .with_projection(
            Projection::Orthographic { half_height: 0.01 },
            source.near,
            source.far,
        )
        .unwrap();
    assert!(match_projection(&too_narrow, (16, 9)).is_err());
    assert!(match_projection(&source, (1, 16384)).is_err());
    assert_eq!(source, before);
}

#[test]
fn projection_match_preserves_scale_only_at_target_plane_not_other_depths() {
    let source = document();
    let matched = match_projection(&source, (1600, 900)).unwrap();
    let a = source.camera(&source.orbit(), (1600, 900)).unwrap();
    let b = matched.camera(&matched.orbit(), (1600, 900)).unwrap();
    let right = a.right();
    let forward = a.forward();
    let point = std::array::from_fn(|i| source.target[i] + right[i] * 2.0 + forward[i] * 5.0);
    let pa = a.world_to_screen(point, (1600, 900)).unwrap();
    let pb = b.world_to_screen(point, (1600, 900)).unwrap();
    assert!((pa[0] - pb[0]).abs() > 1.0);
}
