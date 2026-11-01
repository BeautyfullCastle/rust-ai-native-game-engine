use super::EditorApp;
#[test]
fn saved_policy_drives_mesh_frame_selection_without_mutating_document() {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("effects.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(&scene, &Default::default()).unwrap();
    let snapshot = editor.snapshot().unwrap();
    let view = orr_remote::mesh_navigation::mesh_from_view(snapshot.predicted()).unwrap();
    let requested = orr_render::PostProcessSettings {
        enabled: true,
        threshold: 2.0,
        ..Default::default()
    };
    let doc =
        crate::mesh_lighting::Document::from_lighting(orr_sample::yard3d_view::yard_lighting())
            .unwrap()
            .with_post_process(requested)
            .unwrap();
    crate::mesh_lighting_store::save(&scene, Some(&doc), None, view.graph.revision()).unwrap();
    let path = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let checksum = editor.checksum();
    let mut app = EditorApp::new(editor, None);
    app.mesh_lighting.sync(&app.editor);
    assert_eq!(app.post_process_for_frame(), requested);
    app.mesh_post_process_runtime
        .reject_render_error(&orr_render::ImportedSceneError::UnsupportedHdr.to_string());
    assert!(!app.post_process_for_frame().enabled);
    // A successful fallback selection must retain the shared viewport warning.
    assert!(app.mesh_post_process_runtime.warning().is_some());
    assert!(!app.post_process_for_frame().enabled);
    assert!(app.mesh_post_process_runtime.warning().is_some());
    assert_eq!(
        app.mesh_lighting.applied_post_process(&app.editor),
        Some(requested)
    );
    assert_eq!(app.editor.checksum(), checksum);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
