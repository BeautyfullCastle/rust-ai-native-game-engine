use super::EditorApp;
use crate::{editor::Editor, game::EditorGame, mesh_lighting::Document};

#[test]
fn saved_point_light_is_selected_each_frame_and_scene_switch_clears_it() {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("light.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(&scene, &Default::default()).unwrap();
    let snapshot = editor.snapshot().unwrap();
    let view = orr_remote::mesh_navigation::mesh_from_view(snapshot.predicted()).unwrap();
    let light = orr_render::PointLight {
        position: [1.0, 3.0, -2.0],
        color: [0.2, 0.8, 1.0],
        intensity: 4.0,
        range: 12.0,
    };
    let mut document = Document::from_lighting(orr_render::Lighting::default()).unwrap();
    document.point_light = Some(light);
    crate::mesh_lighting_store::save(&scene, Some(&document), None, view.graph.revision()).unwrap();
    let sidecar = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
    let bytes = std::fs::read(&sidecar).unwrap();
    let checksum = editor.checksum();
    let mut app = EditorApp::new(editor, None);
    app.mesh_lighting.sync(&app.editor);
    for _ in 0..3 {
        assert_eq!(
            app.point_light_settings_for_frame().unwrap().point_light,
            Some(light)
        );
    }
    assert_eq!(app.editor.checksum(), checksum);
    assert_eq!(std::fs::read(&sidecar).unwrap(), bytes);
    // Native players share EditorApp. A fresh app must select identical settings.
    let mut restarted = EditorApp::new(
        Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap(),
        None,
    );
    restarted.mesh_lighting.sync(&restarted.editor);
    assert_eq!(
        restarted.point_light_settings_for_frame().unwrap(),
        app.point_light_settings_for_frame().unwrap()
    );
    let other = temp.path().join("unlit.scene.yaml");
    app.editor = crate::mesh_workspace::mission::prepare(&other, &Default::default()).unwrap();
    // Even before panel synchronization, an old scene must never supply its light.
    assert_eq!(
        app.point_light_settings_for_frame().unwrap().point_light,
        None
    );
    app.mesh_lighting.sync(&app.editor);
    assert_eq!(
        app.point_light_settings_for_frame().unwrap().point_light,
        None
    );
    assert_eq!(std::fs::read(sidecar).unwrap(), bytes);
}

#[test]
fn invalid_saved_light_cannot_supply_a_previous_scene_light() {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("bad.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(&scene, &Default::default()).unwrap();
    let sidecar = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
    let invalid = br#"{"schema":3,"lighting":{"point_light":null}}"#;
    std::fs::write(&sidecar, invalid).unwrap();
    let mut app = EditorApp::new(editor, None);
    app.mesh_lighting.sync(&app.editor);
    assert!(app.mesh_lighting.play_blocker(&app.editor).is_some());
    assert_eq!(
        app.point_light_settings_for_frame().unwrap().point_light,
        None
    );
    assert_eq!(std::fs::read(sidecar).unwrap(), invalid);
}
