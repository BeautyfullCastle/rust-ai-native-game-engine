//! Actual CPU widgets through saved camera lifecycle; no window, GPU, or OS input.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn click(h: &mut Harness<'_, (Panel, Editor)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
fn text(h: &mut Harness<'_, (Panel, Editor)>, label: &str, value: &str) {
    h.get_by_label(label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label(label).type_text(value);
    h.run_steps(3);
    assert_eq!(
        h.get_by_label(label).accesskit_node().value().as_deref(),
        Some(value)
    );
}
#[test]
fn perspective_and_orthographic_preview_stage_apply_undo_redo_save_and_reopen_off() {
    for orthographic in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let scene = dir.path().join("camera.scene.yaml");
        let mut editor =
            crate::mesh_workspace::mission::prepare(&scene, &Default::default()).unwrap();
        let frame = editor.checksum();
        let history = editor.history().clone();
        let scene_bytes = std::fs::read(&scene).unwrap();
        let orbit = editor.camera3d;
        let mut panel = Panel::default();
        panel.sync(&mut editor);
        let mut h = Harness::builder()
            .with_size([1500.0, 3000.0])
            .build_ui_state(
                |ui, s: &mut (Panel, Editor)| s.0.show(ui, &mut s.1),
                (panel, editor),
            );
        h.run_steps(3);
        click(&mut h, "Mesh game camera");
        text(&mut h, "Game camera target X", "3");
        text(&mut h, "Game camera target Y", "4");
        text(&mut h, "Game camera distance", "20");
        if orthographic {
            click(&mut h, "Orthographic game camera");
            text(&mut h, "Game camera orthographic half height", "7");
        }
        let expected = {
            let s = h.state();
            s.0.parse_form(&scene, s.0.graph_revision.unwrap()).unwrap()
        };
        let dirty = h.state().0.dirty();
        let count = h.state().0.history.len();
        click(&mut h, "Preview static camera draft");
        assert!(h.state().0.static_preview_enabled());
        {
            let s = h.state_mut();
            let camera = s.0.static_preview_camera(&s.1, (800, 600)).unwrap();
            assert_eq!(camera.target, expected.target);
            match (camera.projection, orthographic) {
                (orr_render::Projection::Orthographic { .. }, true)
                | (orr_render::Projection::Perspective { .. }, false) => {}
                _ => panic!("wrong preview projection"),
            }
        }
        assert_eq!(h.state().0.dirty(), dirty);
        assert_eq!(h.state().0.history.len(), count);
        assert!(h.state().0.current_document().is_none());
        assert_eq!(format!("{:?}", h.state().1.camera3d), format!("{orbit:?}"));
        assert_eq!(h.state().1.checksum(), frame);
        assert_eq!(h.state().1.history(), &history);
        assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
        assert!(!crate::mesh_camera_store::sidecar_path(&scene)
            .unwrap()
            .exists());
        click(&mut h, "Apply game camera");
        assert!(!h.state().0.static_preview_enabled());
        assert_eq!(h.state().0.current_document(), Some(&expected));
        click(&mut h, "Preview static camera draft");
        assert!(h.state().0.static_preview_enabled());
        click(&mut h, "Undo game camera");
        assert!(!h.state().0.static_preview_enabled());
        assert!(h.state().0.current_document().is_none());
        click(&mut h, "Preview static camera draft");
        assert!(h.state().0.static_preview_enabled());
        click(&mut h, "Redo game camera");
        assert!(!h.state().0.static_preview_enabled());
        assert_eq!(h.state().0.current_document(), Some(&expected));
        click(&mut h, "Preview static camera draft");
        assert!(h.state().0.static_preview_enabled());
        click(&mut h, "Save game camera");
        assert!(!h.state().0.static_preview_enabled());
        assert!(!h.state().0.dirty());
        let mut fresh =
            crate::mesh_workspace::prepare(&scene, crate::mesh_workspace::Intent::Open).unwrap();
        let mut reopened = Panel::default();
        reopened.sync(&mut fresh);
        assert_eq!(reopened.current_document(), Some(&expected));
        assert!(!reopened.static_preview_enabled());
        assert!(reopened.static_preview_camera(&fresh, (800, 600)).is_none());
        assert_eq!(fresh.checksum(), frame);
        assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    }
}
