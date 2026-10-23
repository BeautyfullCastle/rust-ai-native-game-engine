use super::*;
use orr_sample::authored_ui::Document;

fn fixture() -> (tempfile::TempDir, EditorApp) {
    let directory =
        tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let scene = directory.path().join("scene.yaml");
    std::fs::write(
        &scene,
        include_bytes!("../../../../scenes/collect_dodge_v1.scene.yaml"),
    )
    .unwrap();
    let mut editor =
        crate::Editor::open_game(&scene, crate::game::EditorGame::CollectDodge).unwrap();
    editor.sync();
    let mut app = EditorApp::new(editor, None);
    let prepared = PreparedCollectUi {
        path: directory.path().join("ui.json"),
        document: Document::default_collect(),
        font: include_bytes!("../../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec(),
    };
    std::fs::write(&prepared.path, prepared.document.to_bytes().unwrap()).unwrap();
    app.install_collect_ui_preview(&prepared);
    app.collect_ui = Some(crate::collect_ui_panel::Panel::new(prepared));
    (directory, app)
}

#[test]
fn clean_ui_switch_guard_is_read_only() {
    let (directory, app) = fixture();
    let path = directory.path().join("ui.json");
    let bytes = std::fs::read(&path).unwrap();
    let document = app.collect_ui.as_ref().unwrap().document().clone();
    assert!(app.collect_ui_project_switch_guard().is_ok());
    assert!(app.collect_ui_project_switch_guard().is_ok());
    assert_eq!(app.collect_ui.as_ref().unwrap().document(), &document);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn dirty_ui_rejection_preserves_document_and_undo() {
    let (_directory, mut app) = fixture();
    let original = app.collect_ui.as_ref().unwrap().document().clone();
    let mut changed = original.clone();
    changed.nodes[0].offset[0] += 1;
    app.collect_ui
        .as_mut()
        .unwrap()
        .apply_document(changed.clone())
        .unwrap();
    assert!(app.collect_ui_project_switch_guard().is_err());
    assert_eq!(app.collect_ui.as_ref().unwrap().document(), &changed);
    assert!(app.collect_ui.as_mut().unwrap().undo());
    assert_eq!(app.collect_ui.as_ref().unwrap().document(), &original);
    assert!(app.collect_ui_project_switch_guard().is_ok());
}

#[test]
fn missing_admission_and_changed_host_identity_fail_closed() {
    let (_directory, mut app) = fixture();
    let source = app.collect_ui_preview.source.take();
    assert!(app.collect_ui_project_switch_guard().is_err());
    app.collect_ui_preview.source = source;
    assert!(app.editor.restart());
    app.editor.sync();
    assert!(app.collect_ui_project_switch_guard().is_err());
    app.collect_ui_preview.source = app
        .editor
        .path()
        .map(|path| (path, app.editor.scene_load_epoch()));
    app.collect_ui_preview.state.clear();
    assert!(app.collect_ui_project_switch_guard().is_err());
}

#[test]
fn missing_or_changed_saved_ui_blocks_without_losing_document() {
    let (directory, app) = fixture();
    let document = app.collect_ui.as_ref().unwrap().document().clone();
    let mut external = document.clone();
    external.nodes[0].offset[0] += 1;
    let path = directory.path().join("ui.json");
    std::fs::write(&path, external.to_bytes().unwrap()).unwrap();
    assert!(app.collect_ui_project_switch_guard().is_err());
    assert_eq!(app.collect_ui.as_ref().unwrap().document(), &document);
    std::fs::remove_file(path).unwrap();
    assert!(app.collect_ui_project_switch_guard().is_err());
    assert_eq!(app.collect_ui.as_ref().unwrap().document(), &document);
}

#[test]
fn pending_move_resize_alignment_and_pointer_are_preserved() {
    let (_directory, mut app) = fixture();
    let document = app.collect_ui.as_ref().unwrap().document().clone();
    let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 360.0));
    let scene = app.editor.path().unwrap();
    let epoch = app.editor.scene_load_epoch();
    let position = app
        .collect_ui_preview
        .position_context(&scene, epoch, canvas);
    app.collect_ui_preview
        .position
        .state
        .begin(position.clone(), &document, "title", [0.0, 0.0])
        .unwrap();
    assert!(app.collect_ui_project_switch_guard().is_err());
    assert!(app.collect_ui_preview.position.state.is_dragging());
    app.collect_ui_preview
        .position
        .state
        .finish([8.0, 8.0])
        .unwrap();
    let candidate = app
        .collect_ui_preview
        .position
        .state
        .candidate(&position, &document)
        .unwrap();
    assert!(app.collect_ui_project_switch_guard().is_err());
    assert_eq!(
        app.collect_ui_preview
            .position
            .state
            .candidate(&position, &document)
            .unwrap(),
        candidate
    );
    app.collect_ui_preview.position.state.cancel();
    let resize = app.collect_ui_preview.resize_context(&scene, epoch, canvas);
    app.collect_ui_preview
        .resize
        .state
        .begin(resize.clone(), &document, "title", [0.0, 0.0])
        .unwrap();
    assert!(app.collect_ui_project_switch_guard().is_err());
    assert!(app.collect_ui_preview.resize.state.is_dragging());
    app.collect_ui_preview
        .resize
        .state
        .finish([8.0, 8.0])
        .unwrap();
    let candidate = app
        .collect_ui_preview
        .resize
        .state
        .candidate(&resize, &document)
        .unwrap();
    assert!(app.collect_ui_project_switch_guard().is_err());
    assert_eq!(
        app.collect_ui_preview
            .resize
            .state
            .candidate(&resize, &document)
            .unwrap(),
        candidate
    );
    app.collect_ui_preview.resize.state.cancel();
    let alignment = app
        .collect_ui_preview
        .alignment_context(&scene, epoch, canvas);
    app.collect_ui_preview
        .alignment
        .state
        .stage(
            alignment,
            &document,
            &["title".into(), "title_best".into()],
            "title",
            crate::collect_ui_alignment::Mode::Top,
        )
        .unwrap();
    assert!(app.collect_ui_preview.alignment.state.is_ready());
    assert!(app.collect_ui_project_switch_guard().is_err());
    assert!(app.collect_ui_preview.alignment.state.is_ready());
    app.collect_ui_preview.alignment.state.cancel();
    app.collect_ui_preview.pointer_down = true;
    assert!(app.collect_ui_project_switch_guard().is_err());
    assert!(app.collect_ui_preview.pointer_down);
    app.collect_ui_preview.pointer_down = false;
    assert_eq!(app.collect_ui.as_ref().unwrap().document(), &document);
    assert!(app.collect_ui_project_switch_guard().is_ok());
}
