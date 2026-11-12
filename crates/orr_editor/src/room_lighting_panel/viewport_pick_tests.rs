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
fn viewport_and_world_bounds_reject_without_clamping() {
    for (size, pixel) in [
        ((0, 600), [0.0, 0.0]),
        ((800, 0), [0.0, 0.0]),
        ((800, 600), [-1.0, 300.0]),
        ((800, 600), [800.0, 300.0]),
        ((800, 600), [400.0, 600.0]),
        ((800, 600), [f32::NAN, 0.0]),
        ((800, 600), [0.0, f32::INFINITY]),
    ] {
        assert!(ground_point(&camera(), size, pixel).is_none());
    }
    let size = (800, 600);
    let mut far = camera();
    far.eye = [40.0, 10.0, 10.0];
    far.target = [40.0, 0.0, 0.0];
    assert!(ground_point(&far, size, [400.0, 300.0]).is_none());
    let mut edge = camera();
    edge.eye = [32.0, 10.0, 10.0];
    edge.target = [32.0, 0.0, 0.0];
    approx(
        ground_point(&edge, size, [400.0, 300.0]).unwrap(),
        [32.0, 0.0],
    );
}
#[test]
fn staged_pick_changes_only_xz_and_preserves_height_appearance_and_activation() {
    let mut source = Document::enabled_default();
    source.point_light.as_mut().unwrap().position = [7.0, 6.25, -4.0];
    source.point_light.as_mut().unwrap().color = [0.25, 0.5, 0.75];
    source.activation = Some(orr_sample::room_lighting::Activation::Escaped);
    let original = source.clone();
    let next = stage_point(&source, [-12.0, 9.0]).unwrap();
    let mut expected = source.clone();
    expected.point_light.as_mut().unwrap().position = [-12.0, 6.25, 9.0];
    assert_eq!(next, expected);
    assert_eq!(source, original);
    for point in [[33.0, 0.0], [0.0, -33.0], [f32::NAN, 0.0]] {
        assert!(stage_point(&source, point).is_err());
        assert_eq!(source, original);
    }
    let mut carried = source.clone();
    carried.follow_player = Some([0.0, 2.0, 0.0]);
    assert!(stage_point(&carried, [0.0, 0.0]).is_err());
    assert!(stage_point(&Document::default(), [0.0, 0.0]).is_err());
}
#[test]
fn source_pointer_frame_identity_and_full_draft_changes_reject_stale_pin() {
    let source = Arc::new(());
    let document = Document::enabled_default();
    let pin = Pin {
        source: source.clone(),
        identity: (1, 2, 3),
        candidate: document.clone(),
    };
    assert!(pin.matches(&source, (1, 2, 3), &document));
    assert!(!pin.matches(&Arc::new(()), (1, 2, 3), &document));
    for identity in [(9, 2, 3), (1, 9, 3), (1, 2, 9)] {
        assert!(!pin.matches(&source, identity, &document));
    }
    let mut edited = document.clone();
    edited.point_light.as_mut().unwrap().intensity += 1.0;
    assert!(!pin.matches(&source, (1, 2, 3), &edited));
    let mut changed = document.clone();
    changed.activation = Some(orr_sample::room_lighting::Activation::KeyCollected);
    assert!(!pin.matches(&source, (1, 2, 3), &changed));
    assert_eq!(pin.candidate, document);
    // Construct an inert panel without RootPin/File/TempDir access. These direct
    // operations must cancel an armed pick even when host identity stays fixed.
    let mut panel = Panel {
        path: "unused.light".into(),
        scene_path: "unused.scene".into(),
        manifest_path: "unused.manifest".into(),
        root: Err("no filesystem in this fixture".into()),
        document: document.clone(),
        candidate: document.clone(),
        saved: document.clone(),
        saved_bytes: Vec::new(),
        manifest_bytes: Vec::new(),
        undo: Vec::new(),
        redo: Vec::new(),
        source: super::super::SourcePin::Pinned(source.clone()),
        error: None,
        viewport_pick: State::default(),
    };
    let arm = |panel: &mut Panel| {
        panel.viewport_pick.pending = Some(Pin {
            source: source.clone(),
            identity: (1, 2, 3),
            candidate: panel.candidate.clone(),
        });
    };
    arm(&mut panel);
    let mut applied = document.clone();
    applied.point_light.as_mut().unwrap().intensity += 1.0;
    panel.apply_document(applied).unwrap();
    assert!(!panel.pick_pending());
    arm(&mut panel);
    assert!(panel.undo());
    assert!(!panel.pick_pending());
    arm(&mut panel);
    assert!(panel.redo());
    assert!(!panel.pick_pending());
    arm(&mut panel);
    panel.discard_draft();
    assert!(!panel.pick_pending());
    arm(&mut panel);
    assert!(panel.save().is_err());
    assert!(!panel.pick_pending());
}
