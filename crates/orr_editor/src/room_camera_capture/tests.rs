use super::*;

fn context() -> Context {
    Context {
        scene: PathBuf::from("camera.scene.yaml"),
        epoch: 7,
        checksum: 42,
        applied: Document::readable_default(),
    }
}

#[test]
fn perspective_capture_preserves_exact_camera_matrices_rays_and_applied_fov() {
    let mut applied = Document::readable_default();
    applied.projection = Projection::Perspective {
        fov_y_degrees: 63.0,
    };
    let mut live = applied.orbit();
    live.target = [3.0, 1.0, -2.0];
    live.yaw = -0.8;
    live.pitch = 0.75;
    live.distance = 17.5;
    live.fov_y_degrees = 80.0; // Room's effective FOV comes from the document.
    let captured = from_view(&applied, &live).unwrap();
    assert_eq!(captured.projection, applied.projection);
    assert_eq!(captured.target, live.target);
    assert_eq!(captured.yaw, live.yaw);
    assert_eq!(captured.pitch, live.pitch);
    assert_eq!(captured.distance, live.distance);
    assert_projection_parity(&applied, &live, &captured);
}

fn assert_projection_parity(applied: &Document, live: &OrbitCamera, captured: &Document) {
    for size in [(800, 600), (600, 800), (777, 777)] {
        let before = applied.camera(live, size).unwrap();
        let after = captured.camera(&captured.orbit(), size).unwrap();
        assert_eq!(before, after);
        assert_eq!(
            before.view_proj(size.0 as f32 / size.1 as f32).0,
            after.view_proj(size.0 as f32 / size.1 as f32).0
        );
        for pixel in [[0.0, 0.0], [200.0, 300.0], [size.0 as f32, size.1 as f32]] {
            assert_eq!(
                before.screen_ray(pixel, size),
                after.screen_ray(pixel, size)
            );
        }
    }
}

#[test]
fn orthographic_capture_normalizes_zoom_for_landscape_and_portrait() {
    let applied = Document::readable_default();
    for distance in [3.0, 12.0, 31.0, 96.0] {
        let mut live = applied.orbit();
        live.target = [-3.0, 2.0, 4.0];
        live.yaw = -1.0;
        live.pitch = 0.9;
        live.distance = distance;
        let captured = from_view(&applied, &live).unwrap();
        assert_eq!(
            captured.projection,
            Projection::Orthographic {
                half_height: 8.0 * (distance / 24.0)
            }
        );
        assert_projection_parity(&applied, &live, &captured);
    }
}

#[test]
fn rejects_invalid_or_unrepresentable_capture_without_clamping() {
    let applied = Document::readable_default();
    for bad in [f32::NAN, f32::INFINITY, -f32::INFINITY] {
        for field in 0..6 {
            let mut live = applied.orbit();
            match field {
                0 => live.target[0] = bad,
                1 => live.yaw = bad,
                2 => live.pitch = bad,
                3 => live.distance = bad,
                4 => live.fov_y_degrees = bad,
                _ => live.target[2] = bad,
            }
            assert!(from_view(&applied, &live).is_err());
        }
    }
    let mut live = applied.orbit();
    live.distance = 1.0; // Effective half-height 1/3 is outside authored 1..64.
    assert!(applied.camera(&live, (800, 600)).is_ok());
    assert!(from_view(&applied, &live).is_err());
    let mut wide = applied.clone();
    wide.projection = Projection::Orthographic { half_height: 64.0 };
    live.distance = 48.0;
    assert!(wide.camera(&live, (800, 600)).is_ok());
    assert!(from_view(&wide, &live).is_err());
    live.target[0] = 65.0;
    assert!(from_view(&applied, &live).is_err());
}

#[test]
fn stage_and_cancel_preserve_context_and_candidate_is_not_authoritative() {
    let context = context();
    let original = context.applied.to_bytes().unwrap();
    let mut live = context.applied.orbit();
    live.target[0] = 2.0;
    let mut state = State::default();
    state.stage(context.clone(), &live).unwrap();
    assert!(state.busy());
    assert_eq!(state.candidate(&context).unwrap().target, live.target);
    assert_eq!(context.applied.to_bytes().unwrap(), original);
    let accepted = state.candidate(&context).unwrap().clone();
    live.distance = f32::NAN;
    assert!(state.stage(context.clone(), &live).is_err());
    assert_eq!(state.candidate(&context), Some(&accepted));
    assert!(!state.sync(Some(&context)));
    state.cancel();
    assert!(!state.busy());
    assert!(state.candidate(&context).is_none());
    assert_eq!(context.applied.to_bytes().unwrap(), original);
}

#[test]
fn every_context_change_hides_and_clears_staged_camera() {
    let baseline = context();
    for field in 0..5 {
        let mut state = State::default();
        state
            .stage(baseline.clone(), &baseline.applied.orbit())
            .unwrap();
        let mut changed = baseline.clone();
        match field {
            0 => changed.scene = PathBuf::from("other.scene.yaml"),
            1 => changed.epoch += 1,
            2 => changed.checksum += 1,
            3 => changed.applied.target[0] = 1.0,
            _ => {
                changed.applied.projection = Projection::Perspective {
                    fov_y_degrees: 60.0,
                }
            }
        }
        assert!(state.candidate(&changed).is_none());
        assert!(state.sync(Some(&changed)));
        assert!(!state.busy());
    }
    let mut state = State::default();
    state
        .stage(baseline.clone(), &baseline.applied.orbit())
        .unwrap();
    assert!(state.sync(None)); // Play, dirty form, missing camera or busy caller.
    assert!(!state.busy());
}

#[test]
fn failed_initial_stage_keeps_state_empty() {
    let mut context = context();
    context.applied.schema = 99;
    let mut state = State::default();
    assert!(state
        .stage(context.clone(), &context.applied.orbit())
        .is_err());
    assert!(!state.busy());
}
