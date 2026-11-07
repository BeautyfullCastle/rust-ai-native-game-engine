//! CPU-only assertions against the actual Room camera consumer.
use super::*;
fn rich() -> Document {
    let mut d = Document::readable_default();
    d.schema = 3;
    d.target = [-0.0, 1.25, -2.0];
    d.yaw = 0.7;
    d.pitch = 0.6;
    d.distance = 24.0;
    d.follow_player = Some([1.0, -0.0, 2.0]);
    d.target_bounds = Some(orr_sample::room_camera::TargetBounds {
        min: [-8.0, -2.0, -9.0],
        max: [9.0, 5.0, 11.0],
    });
    d
}
fn compare_plane(a: &Document, b: &Document, size: (u32, u32), zoom: f32) {
    let mut oa = a.orbit();
    oa.distance *= zoom;
    let mut ob = b.orbit();
    ob.distance *= zoom;
    let ca = a.camera(&oa, size).unwrap();
    let cb = b.camera(&ob, size).unwrap();
    let right = ca.right();
    let up = ca.camera_up();
    for (x, y) in [(-2.0, -1.0), (0.0, 0.0), (2.0, 1.0), (-1.0, 2.0)] {
        let point = std::array::from_fn(|i| a.target[i] + right[i] * x + up[i] * y);
        let pa = ca.world_to_screen(point, size).unwrap();
        let pb = cb.world_to_screen(point, size).unwrap();
        for i in 0..2 {
            assert!(
                (pa[i] - pb[i]).abs() < 0.015,
                "{size:?} {zoom}: {pa:?} != {pb:?}"
            );
        }
    }
}
#[test]
fn actual_target_plane_pixels_match_both_directions_landscape_portrait_square() {
    for size in [(1600, 900), (900, 1600), (1024, 1024)] {
        for projection in [
            Projection::Perspective {
                fov_y_degrees: 50.0,
            },
            Projection::Orthographic { half_height: 8.0 },
        ] {
            let mut source = rich();
            source.projection = projection;
            let original = source.to_bytes().unwrap();
            let changed = match_projection(&source, size).unwrap();
            compare_plane(&source, &changed, size, 1.0);
            let back = match_projection(&changed, size).unwrap();
            compare_plane(&source, &back, size, 1.0);
            assert_eq!(source.to_bytes().unwrap(), original);
        }
    }
}
#[test]
fn independent_formula_accounts_for_shorter_axis_and_roundtrip() {
    let mut source = rich();
    source.projection = Projection::Perspective {
        fov_y_degrees: 60.0,
    };
    for (size, aspect_factor) in [((1600, 900), 1.0), ((900, 1600), 0.5625), ((800, 800), 1.0)] {
        let changed = match_projection(&source, size).unwrap();
        let Projection::Orthographic { half_height } = changed.projection else {
            panic!("expected ortho")
        };
        let expected = 24.0_f64 * (std::f64::consts::PI / 6.0).tan() * aspect_factor;
        assert!((f64::from(half_height) - expected).abs() < 0.00001);
        let back = match_projection(&changed, size).unwrap();
        let Projection::Perspective { fov_y_degrees } = back.projection else {
            panic!("expected perspective")
        };
        assert!((fov_y_degrees - 60.0).abs() < 0.0001);
    }
}
#[test]
fn only_projection_changes_preserving_schema_tracking_bounds_and_signed_zero_bits() {
    for schema in [1, 2, 3] {
        let mut source = rich();
        source.schema = schema;
        if schema == 1 {
            source.follow_player = None;
        }
        if schema < 3 {
            source.target_bounds = None;
        }
        let changed = match_projection(&source, (1600, 900)).unwrap();
        let mut restored = changed;
        restored.projection = source.projection;
        assert_eq!(
            serde_json::to_vec(&restored).unwrap(),
            serde_json::to_vec(&source).unwrap()
        );
        assert_eq!(
            restored.target.map(f32::to_bits),
            source.target.map(f32::to_bits)
        );
    }
}
#[test]
fn invalid_source_and_viewport_reject_without_mutation() {
    let source = rich();
    let before = source.to_bytes().unwrap();
    for size in [(0, 1), (1, 0), (16385, 1), (1, 16385)] {
        assert!(match_projection(&source, size).is_err());
    }
    assert_eq!(source.to_bytes().unwrap(), before);
    for field in 0..6 {
        let mut bad = source.clone();
        match field {
            0 => bad.distance = 0.0,
            1 => bad.pitch = f32::NAN,
            2 => bad.target[0] = f32::INFINITY,
            3 => bad.schema = 0,
            4 => bad.follow_player = None,
            _ => bad.projection = Projection::Orthographic { half_height: 0.5 },
        };
        let snapshot = format!("{bad:?}");
        assert!(match_projection(&bad, (1600, 900)).is_err());
        assert_eq!(format!("{bad:?}"), snapshot);
    }
}
#[test]
fn unrepresentable_destination_rejects_instead_of_clamping_fov_or_half_height() {
    let mut source = rich();
    source.distance = 1.0;
    source.projection = Projection::Perspective {
        fov_y_degrees: 20.0,
    };
    assert!(source.validate().is_ok());
    assert!(match_projection(&source, (100, 100)).is_err());
    source.distance = 128.0;
    source.projection = Projection::Perspective {
        fov_y_degrees: 100.0,
    };
    assert!(source.validate().is_ok());
    assert!(match_projection(&source, (100, 100)).is_err());
    source.distance = 1.0;
    source.projection = Projection::Orthographic { half_height: 64.0 };
    assert!(source.validate().is_ok());
    assert!(match_projection(&source, (100, 100)).is_err());
    source.distance = 128.0;
    source.projection = Projection::Orthographic { half_height: 1.0 };
    assert!(source.validate().is_ok());
    assert!(match_projection(&source, (100, 100)).is_err());
}
#[test]
fn ordinary_session_zoom_uses_same_authored_distance_after_switch() {
    let source = rich();
    for size in [(1600, 900), (900, 1600)] {
        let changed = match_projection(&source, size).unwrap();
        for zoom in [0.75, 1.0, 1.5] {
            compare_plane(&source, &changed, size, zoom);
        }
        assert_eq!(changed.distance, source.distance);
    }
}
