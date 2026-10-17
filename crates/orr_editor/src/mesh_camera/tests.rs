use super::*;

fn orbit() -> OrbitCamera {
    OrbitCamera::new([1.0, 2.0, 3.0], 0.7, 0.4, 12.0)
}

#[test]
fn perspective_camera_uses_saved_projection_and_session_pose() {
    let saved = Document::from_orbit(orbit()).unwrap();
    let mut session = orbit();
    session.yaw += 0.5;
    session.distance = 20.0;
    let camera = saved.camera(&session, (1200, 800)).unwrap();
    assert_eq!(camera.target, session.target);
    assert_eq!(camera.eye, session.eye());
    let center = camera.world_to_screen(session.target, (1200, 800)).unwrap();
    assert!((center[0] - 600.0).abs() < 0.1);
    assert!((center[1] - 400.0).abs() < 0.1);
}

#[test]
fn orthographic_camera_is_finite_at_extreme_viewports() {
    let saved = Document::from_orbit(orbit())
        .unwrap()
        .with_projection(
            Projection::Orthographic {
                half_height: 10_000.0,
            },
            0.001,
            1_000_000.0,
        )
        .unwrap();
    for size in [(1, 16_384), (16_384, 1), (16_384, 16_384)] {
        let camera = saved.camera(&orbit(), size).unwrap();
        assert!(camera
            .view_proj(size.0 as f32 / size.1 as f32)
            .0
            .iter()
            .flatten()
            .all(|value| value.is_finite()));
        assert!(camera
            .screen_ray([0.0, 0.0], size)
            .0
            .iter()
            .all(|value| value.is_finite()));
    }
}

#[test]
fn scene_and_graph_binding_roundtrips_and_rejects_other_identity() {
    let saved = Document::from_orbit(orbit()).unwrap();
    let revision = [17; 32];
    let bytes = saved.encode("room.mesh.scene.yaml", revision).unwrap();
    assert_eq!(
        Document::decode(&bytes, "room.mesh.scene.yaml", revision).unwrap(),
        saved
    );
    assert!(Document::decode(&bytes, "other.mesh.scene.yaml", revision).is_err());
    assert!(Document::decode(&bytes, "room.mesh.scene.yaml", [18; 32]).is_err());
}

#[test]
fn strict_schema_rejects_duplicate_unknown_wrong_shaped_and_oversize_json() {
    let saved = Document::from_orbit(orbit()).unwrap();
    let bytes = saved.encode("room.scene.yaml", [9; 32]).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    for invalid in [
        text.replace("\"schema\": 1", "\"schema\": 1, \"extra\": 0"),
        text.replace("\"schema\": 1", "\"schema\": 1, \"schema\": 1"),
        text.replace("\"far\": 500.0", "\"far\": 500.0, \"far\": 500.0"),
        format!("[{text}]"),
    ] {
        assert!(Document::decode(invalid.as_bytes(), "room.scene.yaml", [9; 32]).is_err());
    }
    assert!(Document::decode(&vec![b' '; MAX_BYTES + 1], "room.scene.yaml", [9; 32]).is_err());
    assert!(Document::decode(&bytes, "../room.scene.yaml", [9; 32]).is_err());
}

#[test]
fn pose_projection_and_frustum_bounds_reject_nonfinite_or_out_of_range_values() {
    for (target, yaw, pitch, distance, fov) in [
        ([0.0, 0.0, 0.0], f32::NAN, 0.0, 12.0, 50.0),
        ([1_000_001.0, 0.0, 0.0], 0.0, 0.0, 12.0, 50.0),
        ([0.0, 0.0, 0.0], 0.0, 1.501, 12.0, 50.0),
        ([0.0, 0.0, 0.0], 0.0, 0.0, 0.49, 50.0),
        ([0.0, 0.0, 0.0], 0.0, 0.0, 12.0, 151.0),
    ] {
        let invalid = OrbitCamera {
            target,
            yaw,
            pitch,
            distance,
            fov_y_degrees: fov,
        };
        assert!(Document::from_orbit(invalid).is_err());
    }
    let saved = Document::from_orbit(orbit()).unwrap();
    assert!(saved
        .clone()
        .with_projection(Projection::Perspective, 100.0, 100.0)
        .is_err());
    assert!(saved
        .clone()
        .with_projection(Projection::Perspective, 0.0009, 10.0)
        .is_err());
    assert!(saved
        .clone()
        .with_projection(
            Projection::Orthographic {
                half_height: 10_001.0
            },
            0.1,
            100.0
        )
        .is_err());
    assert!(saved.camera(&orbit(), (0, 600)).is_err());
    assert!(saved.camera(&orbit(), (16_385, 600)).is_err());
}

#[test]
fn authored_orthographic_pan_uses_the_effective_camera_scale() {
    let saved = Document::from_orbit(orbit())
        .unwrap()
        .with_projection(Projection::Orthographic { half_height: 10.0 }, 0.1, 500.0)
        .unwrap();
    let original_document = saved.clone();
    let mut session = OrbitCamera::new([0.0, 0.0, 0.0], 0.0, 0.0, 12.0);

    saved.pan(&mut session, [25.0, -10.0], (1000, 500));

    assert!((session.target[0] + 1.0).abs() < 1.0e-5);
    assert!((session.target[1] + 0.4).abs() < 1.0e-5);
    assert_eq!(session.target[2], 0.0);
    assert_eq!(saved, original_document);
    assert!(saved.camera(&session, (1000, 500)).is_ok());
}

#[test]
fn transient_orbit_controls_wrap_clamp_and_leave_document_unchanged() {
    let saved = Document::from_orbit(orbit()).unwrap();
    let original_document = saved.clone();
    let mut session = orbit();

    saved.orbit_delta(&mut session, [10_000.0, 10_000.0]);
    assert!((-std::f32::consts::PI..std::f32::consts::PI).contains(&session.yaw));
    assert_eq!(session.pitch, 1.5);
    saved.zoom(&mut session, 1_000_000.0);
    assert_eq!(session.distance, OrbitCamera::MIN_DISTANCE);
    saved.zoom(&mut session, -1_000_000.0);
    assert_eq!(session.distance, OrbitCamera::MAX_DISTANCE);
    assert_eq!(saved, original_document);
}

#[test]
fn invalid_transient_inputs_are_failure_atomic() {
    let saved = Document::from_orbit(orbit()).unwrap();
    let mut session = orbit();
    let original_session = session;

    saved.orbit_delta(&mut session, [f32::NAN, 1.0]);
    assert_eq!(session, original_session);
    saved.orbit_delta(&mut session, [f32::INFINITY, 1.0]);
    assert_eq!(session, original_session);
    saved.orbit_delta(&mut session, [1_000_001.0, 1.0]);
    assert_eq!(session, original_session);
    saved.pan(&mut session, [1.0, f32::NAN], (800, 600));
    assert_eq!(session, original_session);
    saved.pan(&mut session, [1.0, 1.0], (0, 600));
    assert_eq!(session, original_session);
    saved.zoom(&mut session, f32::INFINITY);
    assert_eq!(session, original_session);
    saved.zoom(&mut session, f32::NAN);
    assert_eq!(session, original_session);
}
