use super::*;

fn panel_with_scene(scene: &Path) -> Panel {
    let mut panel = Panel::default();
    panel.scene = Some(scene.to_path_buf());
    panel.scene_epoch = Some(1);
    panel.graph_revision = Some([7; 32]);
    panel.document_pending = false;
    panel
}

#[test]
fn default_lighting_roundtrips_through_strict_document() {
    let panel = panel_with_scene(Path::new("lighting.scene.yaml"));
    let document = panel
        .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
        .unwrap();
    assert_eq!(
        document.lighting().unwrap(),
        orr_sample::yard3d_view::yard_lighting()
    );
    let encoded = document.encode("lighting.scene.yaml", [7; 32]).unwrap();
    assert!(Document::decode(&encoded, "lighting.scene.yaml", [7; 32]).is_ok());
}

#[test]
fn invalid_field_values_do_not_create_history_entries() {
    let mut panel = panel_with_scene(Path::new("lighting.scene.yaml"));
    let initial_history = panel.history.len();
    panel.direction = ["0".into(), "0".into(), "0".into()];
    assert!(panel
        .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
        .is_err());
    assert_eq!(panel.history.len(), initial_history);
    panel.direction = ["-0.45".into(), "-0.8".into(), "-0.35".into()];
    panel.exposure = "0".into();
    assert!(panel
        .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
        .is_err());
    assert_eq!(panel.history.len(), initial_history);
}

#[test]
fn local_history_changes_dirty_state_and_undo_redo_preserve_values() {
    let mut panel = panel_with_scene(Path::new("lighting.scene.yaml"));
    let initial = panel
        .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
        .unwrap();
    panel.push_initial(Some(initial.clone()));
    panel.saved_id = Some(panel.history[0].id);
    panel.intensity = "3.5".into();
    let changed = panel
        .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
        .unwrap();
    panel.push_state(Some(changed));
    assert!(panel.history_is_dirty());
    panel.undo();
    assert_eq!(panel.current_document(), Some(&initial));
    assert!(!panel.history_is_dirty());
    panel.redo();
    assert_eq!(
        panel
            .current_document()
            .unwrap()
            .lighting()
            .unwrap()
            .intensity,
        3.5
    );
    assert!(panel.history_is_dirty());
}

#[test]
fn history_is_bounded_and_truncates_redo_after_new_apply() {
    let mut panel = panel_with_scene(Path::new("lighting.scene.yaml"));
    for index in 0..MAX_HISTORY + 3 {
        panel.intensity = (index as f32 + 1.0).to_string();
        let document = panel
            .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
            .unwrap();
        panel.push_state(Some(document));
    }
    assert_eq!(panel.history.len(), MAX_HISTORY);
    panel.undo();
    panel.intensity = "7".into();
    let document = panel
        .parse_document(Path::new("lighting.scene.yaml"), [7; 32])
        .unwrap();
    panel.push_state(Some(document));
    assert_eq!(panel.cursor + 1, panel.history.len());
}
