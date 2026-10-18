use super::*;
use orr_render::Camera3D;
#[test]
fn perspective_sphere_fits_landscape_portrait_preserving_angles() {
    let orbit = OrbitCamera::new([0.0, 0.0, 0.0], 0.7, 0.4, 10.0);
    for size in [(800, 600), (600, 800)] {
        for radius in [0.0001, 1.0, 12.0] {
            let result = math::fit(
                orbit,
                orbit.camera(),
                [20.0, 5.0, -30.0],
                radius,
                size,
                0.5,
                400.0,
            )
            .unwrap();
            assert_eq!(
                (result.yaw, result.pitch, result.fov_y_degrees),
                (orbit.yaw, orbit.pitch, orbit.fov_y_degrees)
            );
            assert_eq!(result.target, [20.0, 5.0, -30.0]);
            math::validate_fit(&result.camera(), result.target, radius, size).unwrap();
        }
    }
}
#[test]
fn orthographic_uses_actual_projection_scale_in_both_aspects() {
    let orbit = OrbitCamera::new([0.0, 0.0, 0.0], 0.7, 0.4, 24.0);
    for size in [(800, 600), (600, 800)] {
        let aspect = size.0 as f32 / size.1 as f32;
        let camera = Camera3D::orthographic(orbit.eye(), orbit.target, 8.0 / aspect.min(1.0));
        let result = math::fit(orbit, camera, [10.0, 3.0, 4.0], 5.0, size, 1.0, 128.0).unwrap();
        let effective = Camera3D::orthographic(
            result.eye(),
            result.target,
            8.0 * (result.distance / 24.0) / aspect.min(1.0),
        );
        math::validate_fit(&effective, result.target, 5.0, size).unwrap();
        assert_eq!((result.yaw, result.pitch), (orbit.yaw, orbit.pitch));
    }
}
#[test]
fn enclosing_radius_covers_rotated_box_and_capsule() {
    let half = [1.0, 3.0, 2.0];
    let bound = radius(Shape3::Box { half }).unwrap();
    for yaw in [0.0_f32, 0.5, 1.5] {
        let q = [0.0, (yaw / 2.0).sin(), 0.0, (yaw / 2.0).cos()];
        for x in [-1.0, 1.0] {
            for y in [-1.0, 1.0] {
                for z in [-1.0, 1.0] {
                    let point =
                        orr_render::math3::quat_rotate(q, [half[0] * x, half[1] * y, half[2] * z]);
                    let distance = point.into_iter().map(|v| v * v).sum::<f32>().sqrt();
                    assert!(distance <= bound + 0.00001);
                }
            }
        }
    }
    assert_eq!(
        radius(Shape3::Capsule {
            half_length: 3.0,
            radius: 0.5
        })
        .unwrap(),
        3.5
    );
}
#[test]
fn oversized_nonfinite_invalid_viewports_and_clipping_are_refused() {
    let orbit = OrbitCamera::new([0.0, 0.0, 0.0], 0.5, 0.5, 10.0);
    for radius in [f32::NAN, f32::INFINITY, -1.0, 0.0, 1000.0] {
        assert!(
            math::fit(
                orbit,
                orbit.camera(),
                [0.0; 3],
                radius,
                (800, 600),
                0.5,
                400.0
            )
            .is_err()
        );
    }
    for size in [(0, 600), (800, 0), (16385, 600)] {
        assert!(math::fit(orbit, orbit.camera(), [0.0; 3], 1.0, size, 0.5, 400.0).is_err());
    }
    assert!(
        math::fit(
            orbit,
            orbit.camera(),
            [f32::NAN, 0.0, 0.0],
            1.0,
            (800, 600),
            0.5,
            400.0
        )
        .is_err()
    );
    let mut camera = orbit.camera();
    camera.projection = orr_render::camera3d::Projection::Perspective {
        fov_y: 1.0,
        near: 0.1,
        far: 5.0,
    };
    assert!(math::validate_fit(&camera, [0.0; 3], 1.0, (800, 600)).is_err());
    assert!(
        radius(Shape3::Box {
            half: [1.0, f32::INFINITY, 1.0]
        })
        .is_err()
    );
}
#[cfg(feature = "room-project")]
fn room() -> (tempfile::TempDir, Editor) {
    use orr_sample::room_game::{RoomConfig, RoomEscapeV1};
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("room.scene.yaml");
    let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, 42);
    let text = orr_reflect::Scene::unbake(&orr_sample::room_project::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    std::fs::write(&scene, &text).unwrap();
    let mut editor = Editor::start(&crate::HostSpec::PreparedRoom {
        scene,
        text,
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    let target = editor
        .rows
        .iter()
        .find(|row| row.guid.is_some())
        .unwrap()
        .guid
        .clone()
        .unwrap();
    editor.select(Some(Target::Guid(target)));
    editor.sync();
    (temp, editor)
}
#[cfg(feature = "room-project")]
#[test]
fn room_frame_is_session_only_and_stale_mapping_rejects_unchanged() {
    let (_temp, mut editor) = room();
    let document = orr_sample::room_camera::Document::readable_default();
    editor.install_room_camera(document.clone()).unwrap();
    let checksum = editor.sim().checksum;
    let history = editor.history().entries.len();
    let selected = editor.selection().cloned();
    let bytes = std::fs::read(editor.path().unwrap()).unwrap();
    editor.camera3d.target = [40.0, 20.0, 40.0];
    editor.frame_selected_collider((800, 600)).unwrap();
    assert_ne!(editor.camera3d.target, [40.0, 20.0, 40.0]);
    assert_eq!(editor.sim().checksum, checksum);
    assert_eq!(editor.history().entries.len(), history);
    assert_eq!(editor.selection(), selected.as_ref());
    assert_eq!(editor.room_camera.as_ref(), Some(&document));
    assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), bytes);
    let camera = editor.camera3d;
    editor.dirty.rows = true;
    assert!(editor.frame_selected_collider((800, 600)).is_err());
    assert_eq!(editor.camera3d, camera);
}
#[cfg(feature = "room-project")]
#[test]
fn missing_selection_and_play_refuse_without_camera_mutation() {
    let (_temp, mut editor) = room();
    let before = editor.camera3d;
    editor.select(None);
    editor.sync();
    assert!(editor.frame_selected_collider((800, 600)).is_err());
    assert_eq!(editor.camera3d, before);
    assert!(editor.start_play());
    editor.sync();
    assert!(editor.frame_selected_collider((800, 600)).is_err());
    assert_eq!(editor.camera3d, before);
}
