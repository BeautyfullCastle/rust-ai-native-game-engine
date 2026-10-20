//! Actual EditorApp root-viewport close events, without a GPU or native window.
//! These tests keep scene, sprite-sidecar, and Collect UI dirty state independent.
use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use std::{fs, path::Path};

pub(super) fn harness(editor: Editor) -> Harness<'static, EditorApp> {
    let mut harness = Harness::builder()
        .with_size([1500.0, 1000.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| EditorApp::new(editor, None));
    settle(&mut harness);
    harness
}

pub(super) fn settle(harness: &mut Harness<'_, EditorApp>) {
    harness.state_mut().editor.sync();
    harness.run_steps(3);
}

pub(super) fn request_root_close(harness: &mut Harness<'_, EditorApp>) -> bool {
    harness
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    harness.run_steps(1);
    harness
        .output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .expect("actual app root viewport output")
        .commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::CancelClose))
}

pub(super) fn host_stamp(harness: &Harness<'_, EditorApp>) -> (u64, crate::model::History, Mode) {
    let editor = &harness.state().editor;
    (editor.checksum(), editor.history().clone(), editor.mode())
}

fn physics_editor(root: &Path) -> (PathBuf, Editor) {
    let path = root.join("scene.yaml");
    fs::write(
        &path,
        include_bytes!("../../../../scenes/physics_demo.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open(&path).unwrap();
    editor.sync();
    (path, editor)
}

#[test]
fn actual_app_clean_scene_close_is_not_cancelled() {
    let root = tempfile::tempdir().unwrap();
    let (path, editor) = physics_editor(root.path());
    let mut harness = harness(editor);
    let original = fs::read(&path).unwrap();
    let stamp = host_stamp(&harness);
    assert!(!harness.state().editor.is_dirty());
    assert!(
        !request_root_close(&mut harness),
        "clean close remains available"
    );
    assert_eq!(host_stamp(&harness), stamp);
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn actual_app_dirty_scene_close_is_cancelled_without_losing_file_or_history() {
    let root = tempfile::tempdir().unwrap();
    let (path, editor) = physics_editor(root.path());
    let mut harness = harness(editor);
    let original = fs::read(&path).unwrap();
    let original_checksum = harness.state().editor.checksum();
    assert!(!harness.state().editor.is_dirty());
    assert!(harness.state_mut().editor.select_named("body_05"));
    assert!(harness.state_mut().editor.set_field(
        &Owner::Component("orr_physics::Body".into()),
        "pos.x",
        Value::Fixed(FP::from_int(2)),
    ));
    settle(&mut harness);
    assert!(harness.state().editor.is_dirty());
    assert_ne!(harness.state().editor.checksum(), original_checksum);
    let stamp = host_stamp(&harness);

    let cancelled = request_root_close(&mut harness);
    assert_eq!(
        host_stamp(&harness),
        stamp,
        "close must not edit the scene/history"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        original,
        "close must not silently save"
    );
    assert!(
        harness.state().editor.is_dirty(),
        "unsaved state must be retained"
    );
    assert!(
        harness.state_mut().editor.undo(),
        "scene Undo must remain available"
    );
    settle(&mut harness);
    assert_eq!(harness.state().editor.checksum(), original_checksum);
    assert_eq!(fs::read(path).unwrap(), original);
    assert!(cancelled, "dirty scene root close must emit CancelClose");
}

#[test]
fn actual_app_dirty_sprite_bindings_close_is_cancelled_with_clean_scene() {
    let root = tempfile::tempdir().unwrap();
    let scene = root.path().join("arena.yaml");
    fs::write(
        &scene,
        include_bytes!("../../../../scenes/arena_blank.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, crate::game::EditorGame::Arena).unwrap();
    editor.sync();
    assert!(editor.spawn_arena_player(0, [0.0, 0.0]));
    editor.sync();
    let guid = editor
        .rows()
        .iter()
        .find_map(|row| row.guid.clone())
        .unwrap();
    assert!(editor.save());
    editor.sync();
    let mut harness = harness(editor);
    let path = root.path().join("arena.sprites.json");
    harness
        .state_mut()
        .sprites
        .create(path.clone(), "arena.yaml".into(), ".".into())
        .unwrap();
    let bindings = harness.state_mut().sprites.bindings.as_mut().unwrap();
    bindings.save().unwrap();
    let saved_document = bindings.document().clone();
    bindings.set_camera_follow(Some(guid)).unwrap();
    let dirty_document = bindings.document().clone();
    assert!(bindings.dirty());
    settle(&mut harness);
    assert!(
        !harness.state().editor.is_dirty(),
        "only the sidecar is dirty"
    );
    let original_scene = fs::read(&scene).unwrap();
    let original_bindings = fs::read(&path).unwrap();
    let stamp = host_stamp(&harness);

    let cancelled = request_root_close(&mut harness);
    assert_eq!(host_stamp(&harness), stamp);
    assert_eq!(fs::read(&scene).unwrap(), original_scene);
    assert_eq!(fs::read(&path).unwrap(), original_bindings);
    let bindings = harness.state_mut().sprites.bindings.as_mut().unwrap();
    assert_eq!(bindings.document(), &dirty_document);
    assert!(bindings.dirty());
    bindings.undo();
    assert_eq!(
        bindings.document(),
        &saved_document,
        "binding Undo is retained"
    );
    assert!(!bindings.dirty());
    bindings.redo();
    assert_eq!(
        bindings.document(),
        &dirty_document,
        "binding Redo is retained"
    );
    assert!(bindings.dirty());
    assert_eq!(fs::read(path).unwrap(), original_bindings);
    assert!(
        cancelled,
        "dirty sprite bindings root close must emit CancelClose"
    );
}

#[cfg(feature = "collect-ui")]
#[test]
fn actual_app_dirty_collect_ui_close_is_cancelled_with_clean_scene() {
    use orr_sample::{
        authored_ui::Document,
        collect_project::{PreparedProject, ProgressSupport, SpriteSupport},
    };

    let root = tempfile::tempdir().unwrap();
    let scene = root.path().join("scene.yaml");
    let ui_path = root.path().join("ui.json");
    fs::write(
        &scene,
        include_bytes!("../../../../scenes/collect_dodge_v1.scene.yaml"),
    )
    .unwrap();
    fs::write(&ui_path, Document::default_collect().to_bytes().unwrap()).unwrap();
    fs::write(
        root.path().join("orr.project.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": 2, "engine": "*",
            "entry": {
                "game": "collect-dodge-v1", "scene": "scene.yaml",
                "ui": {
                    "profile": "collect-authored-v1", "document": "ui.json",
                    "font": {"package": "korean-game-ui", "asset": "OrreryKoreanUI.otf"}
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    orr_package::Project::open(root.path(), orr_package::Runtime::content_only())
        .unwrap()
        .install(&[PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/game_ui_font")
            .canonicalize()
            .unwrap()])
        .unwrap();
    let project = PreparedProject::open_with_ui(
        root.path(),
        ProgressSupport::Unsupported,
        SpriteSupport::Unsupported,
        true,
    )
    .unwrap();
    let mut editor = Editor::start(&crate::HostSpec::PreparedCollect {
        scene: project.path().to_owned(),
        text: project.scene().text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    let mut harness = harness(editor);
    let context = harness.ctx.clone();
    crate::project::install_collect_presentation(harness.state_mut(), project, &context);
    let panel = harness.state_mut().collect_ui.as_mut().unwrap();
    let saved_document = panel.document().clone();
    let mut dirty_document = saved_document.clone();
    dirty_document.nodes[0].offset[0] += 1;
    panel.apply_document(dirty_document.clone()).unwrap();
    settle(&mut harness);
    assert!(
        !harness.state().editor.is_dirty(),
        "only the UI document is dirty"
    );
    assert!(harness.state().sprites.bindings.is_none());
    let original_scene = fs::read(&scene).unwrap();
    let original_ui = fs::read(&ui_path).unwrap();
    let stamp = host_stamp(&harness);

    let cancelled = request_root_close(&mut harness);
    assert_eq!(host_stamp(&harness), stamp);
    assert_eq!(fs::read(scene).unwrap(), original_scene);
    assert_eq!(fs::read(&ui_path).unwrap(), original_ui);
    let panel = harness.state_mut().collect_ui.as_mut().unwrap();
    assert_eq!(panel.document(), &dirty_document);
    assert!(panel.undo(), "UI document Undo must be retained");
    assert_eq!(panel.document(), &saved_document);
    assert!(!panel.undo(), "close must not add hidden UI history");
    assert_eq!(fs::read(ui_path).unwrap(), original_ui);
    assert!(
        cancelled,
        "dirty Collect UI root close must emit CancelClose"
    );
}

fn edit_physics_scene(harness: &mut Harness<'_, EditorApp>) {
    assert!(harness.state_mut().editor.select_named("body_05"));
    assert!(harness.state_mut().editor.set_field(
        &Owner::Component("orr_physics::Body".into()),
        "pos.x",
        Value::Fixed(FP::from_int(2)),
    ));
    settle(harness);
    assert!(harness.state().editor.is_dirty());
}

fn assert_no_explicit_close(harness: &Harness<'_, EditorApp>) {
    assert!(!harness
        .output()
        .viewport_output
        .values()
        .flat_map(|output| &output.commands)
        .any(|command| matches!(command, egui::ViewportCommand::Close)));
}

fn item_names(harness: &Harness<'_, EditorApp>) -> Vec<&'static str> {
    harness
        .state()
        .close_review_items()
        .into_iter()
        .map(|item| item.name)
        .collect()
}

fn add_dirty_bindings(harness: &mut Harness<'_, EditorApp>, root: &Path) -> PathBuf {
    let guid = harness.state().editor.rows()[0].guid.clone().unwrap();
    let scene = harness.state().editor.path().unwrap();
    let path = root.join("view.sprites.json");
    harness
        .state_mut()
        .sprites
        .create(
            path.clone(),
            scene.file_name().unwrap().to_str().unwrap().into(),
            ".".into(),
        )
        .unwrap();
    let bindings = harness.state_mut().sprites.bindings.as_mut().unwrap();
    bindings.save().unwrap();
    bindings.set_camera_follow(Some(guid)).unwrap();
    path
}

#[test]
fn actual_app_passive_close_preserves_focus_geometry_and_ordinary_input() {
    let root = tempfile::tempdir().unwrap();
    let (path, editor) = physics_editor(root.path());
    let mut harness = harness(editor);
    edit_physics_scene(&mut harness);
    harness.get_by_label("Timeline").focus();
    harness.run_steps(2);
    let focus = harness.ctx.memory(|memory| memory.focused());
    assert!(focus.is_some(), "the original widget has keyboard focus");
    let viewport = harness.state().ui.viewport_rect;
    let stamp = host_stamp(&harness);
    let selection = harness.state().editor.selection().cloned();
    let original = fs::read(&path).unwrap();
    for _ in 0..2 {
        assert!(request_root_close(&mut harness));
        harness.run_steps(2);
        assert!(harness.state().ui.close_review.blocked);
        assert_eq!(harness.state().ui.bottom_tab, BottomTab::Timeline);
        assert_eq!(harness.ctx.memory(|memory| memory.focused()), focus);
        assert_eq!(harness.state().ui.viewport_rect, viewport);
        assert!(harness.state().ui.dialog.is_none());
        assert!(harness.query_by_label("Clear close status").is_none());
        assert_eq!(item_names(&harness), ["Scene · unsaved"]);
        assert_eq!(host_stamp(&harness), stamp);
        assert_eq!(harness.state().editor.selection(), selection.as_ref());
    }
    // Nothing needs dismissal. Ordinary Escape still clears the selection.
    harness.key_press(Key::Escape);
    harness.run_steps(1);
    assert!(harness.state().editor.selection().is_none());
    assert_eq!(host_stamp(&harness), stamp);
    assert_eq!(fs::read(&path).unwrap(), original);
    // An independently targeted AccessKit edit remains operational too.
    harness.get_by_label("+ Body").click_accesskit();
    settle(&mut harness);
    assert_ne!(host_stamp(&harness), stamp);
    assert!(harness.state().ui.close_review.blocked);
    harness.get_by_label("Unsaved work details").click();
    harness.run_steps(2);
    assert_eq!(harness.state().ui.bottom_tab, BottomTab::UnsavedWork);
    harness.get_by_label("Clear close status").click();
    harness.run_steps(1);
    assert!(!harness.state().ui.close_review.blocked);
    assert_no_explicit_close(&harness);
    assert!(
        request_root_close(&mut harness),
        "clearing status is not exit consent"
    );
}

#[test]
fn actual_app_same_frame_widget_edit_is_seen_by_final_close_check() {
    let root = tempfile::tempdir().unwrap();
    let (path, editor) = physics_editor(root.path());
    let mut harness = harness(editor);
    assert!(harness.state_mut().editor.select_named("body_05"));
    settle(&mut harness);
    let original = fs::read(&path).unwrap();
    assert!(!harness.state().editor.is_dirty());
    // AccessKit activation and native close are in the SAME actual App callback.
    harness.get_by_label("Delete").click_accesskit();
    assert!(request_root_close(&mut harness));
    assert!(harness.state().editor.is_dirty());
    assert!(harness.state().ui.close_review.blocked);
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn actual_app_scene_save_does_not_authorize_closing_dirty_sidecar_or_auto_close() {
    let root = tempfile::tempdir().unwrap();
    let (scene, editor) = physics_editor(root.path());
    let mut harness = harness(editor);
    let sidecar = add_dirty_bindings(&mut harness, root.path());
    let original_bindings = fs::read(&sidecar).unwrap();
    edit_physics_scene(&mut harness);
    assert!(request_root_close(&mut harness));
    assert_eq!(
        item_names(&harness),
        ["Scene · unsaved", "Sprite bindings · unsaved"]
    );

    // Passive status needs no dismissal: an explicit ordinary Save still works.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::S);
    settle(&mut harness);
    assert!(!harness.state().editor.is_dirty());
    assert_eq!(item_names(&harness), ["Sprite bindings · unsaved"]);
    assert_eq!(fs::read(&sidecar).unwrap(), original_bindings);
    assert!(harness.state().ui.close_review.blocked);
    assert_no_explicit_close(&harness);
    assert!(request_root_close(&mut harness));

    harness
        .state_mut()
        .sprites
        .bindings
        .as_mut()
        .unwrap()
        .save()
        .unwrap();
    harness.run_steps(2);
    assert!(item_names(&harness).is_empty());
    assert!(
        harness.state().ui.close_review.blocked,
        "cleaning does not grant exit consent"
    );
    assert_no_explicit_close(&harness);
    assert!(
        !request_root_close(&mut harness),
        "a fresh clean close is allowed"
    );
    assert!(scene.is_file());
}

#[test]
fn actual_app_save_as_cancel_and_failure_do_not_clear_close_protection() {
    let root = tempfile::tempdir().unwrap();
    let (scene, editor) = physics_editor(root.path());
    let mut harness = harness(editor);
    edit_physics_scene(&mut harness);
    let stamp = host_stamp(&harness);
    let original = fs::read(&scene).unwrap();
    assert!(request_root_close(&mut harness));
    let unused = root.path().join("unused.yaml");
    harness.state_mut().ui.dialog = Some(Dialog {
        kind: DialogKind::SaveAs,
        text: unused.display().to_string(),
    });
    harness.run_steps(2);
    harness.get_by_label("Cancel").click();
    harness.run_steps(2);
    assert!(harness.state().ui.dialog.is_none());
    assert!(!unused.exists());
    assert_eq!(host_stamp(&harness), stamp);

    let impossible = root.path().join("missing-directory").join("scene.yaml");
    harness.state_mut().ui.dialog = Some(Dialog {
        kind: DialogKind::SaveAs,
        text: impossible.display().to_string(),
    });
    harness.run_steps(2);
    harness.get_by_label("OK").click();
    harness.run_steps(2);
    assert!(harness.state().ui.dialog.is_some());
    assert!(!impossible.exists());
    assert_eq!(host_stamp(&harness), stamp);
    assert_eq!(fs::read(&scene).unwrap(), original);
    assert_eq!(harness.state().editor.path().as_ref(), Some(&scene));
    assert_no_explicit_close(&harness);
    assert!(request_root_close(&mut harness));
}

#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    feature = "project-create",
    feature = "collect-ui"
))]
#[test]
fn actual_app_project_modal_early_return_cannot_bypass_close_protection() {
    let root = tempfile::tempdir().unwrap();
    let (scene, editor) = physics_editor(root.path());
    let mut harness = harness(editor);
    harness.get_by_label("File").click();
    harness.run_steps(2);
    harness.get_by_label("New Collect project…").click();
    harness.run_steps(2);
    assert!(harness.state().collect_project_dialog_open());
    edit_physics_scene(&mut harness);
    let stamp = host_stamp(&harness);
    let original = fs::read(&scene).unwrap();
    assert!(request_root_close(&mut harness));
    assert!(harness.state().collect_project_dialog_open());
    assert!(harness.state().ui.close_review.blocked);
    assert_eq!(host_stamp(&harness), stamp);
    assert_eq!(fs::read(scene).unwrap(), original);
}

fn listening_editor(path: PathBuf) -> Editor {
    let mut editor = Editor::start(&crate::HostSpec::Local {
        scene: path,
        listen: Some(orr_remote::ServerConfig::new(orr_remote::Auth::DevNoAuth)),
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    editor
}

#[test]
fn actual_app_dirty_update_ingested_during_close_frame_is_protected() {
    let root = tempfile::tempdir().unwrap();
    let (scene, original_editor) = physics_editor(root.path());
    drop(original_editor);
    let editor = listening_editor(scene.clone());
    let url = editor.erp_status().unwrap().0;
    let mut harness = harness(editor);
    let guid = harness
        .state()
        .editor
        .rows()
        .iter()
        .find(|row| row.name.as_deref() == Some("body_05"))
        .unwrap()
        .guid
        .as_ref()
        .unwrap()
        .to_string();
    let original = fs::read(&scene).unwrap();
    let mut agent = orr_remote::ErpClient::connect(&url, None).unwrap();
    agent
        .call(
            "watch.subscribe",
            serde_json::json!({"topics": ["history"]}),
        )
        .unwrap();
    agent
        .call(
            "world.patch",
            serde_json::json!({
                "entity": guid, "component": "orr_physics::Body", "path": "pos.x", "value": "2"
            }),
        )
        .unwrap();
    wait_for_published_history(&mut agent);
    assert!(
        !harness.state().editor.is_dirty(),
        "real history notification is queued, but the editor has not ingested it"
    );
    assert!(
        request_root_close(&mut harness),
        "close frame must ingest then classify"
    );
    assert!(harness.state().editor.is_dirty());
    assert_eq!(fs::read(scene).unwrap(), original);
}

#[test]
fn actual_app_remote_scene_is_not_owned_but_retained_local_sidecar_is_protected() {
    let root = tempfile::tempdir().unwrap();
    let (scene, original_editor) = physics_editor(root.path());
    drop(original_editor);
    let mut owner = listening_editor(scene.clone());
    assert!(owner.select_named("body_05"));
    assert!(owner.set_field(
        &Owner::Component("orr_physics::Body".into()),
        "pos.x",
        Value::Fixed(FP::from_int(2)),
    ));
    owner.sync();
    let url = owner.erp_status().unwrap().0;
    let remote = Editor::attach(&url, None).unwrap();
    let mut harness = harness(remote);
    assert!(harness.state().editor.is_dirty());
    let stamp = host_stamp(&harness);
    let original = fs::read(&scene).unwrap();
    assert!(
        !request_root_close(&mut harness),
        "remote host outlives the client"
    );
    assert_eq!(host_stamp(&harness), stamp);
    assert!(!harness.state().ui.close_review.blocked);

    let sidecar = add_dirty_bindings(&mut harness, root.path());
    let saved_sidecar = fs::read(&sidecar).unwrap();
    assert!(
        request_root_close(&mut harness),
        "local retained sidecar still needs protection"
    );
    assert_eq!(item_names(&harness), ["Sprite bindings · unsaved"]);
    assert_eq!(host_stamp(&harness), stamp);
    assert_eq!(fs::read(scene).unwrap(), original);
    assert_eq!(fs::read(sidecar).unwrap(), saved_sidecar);
    assert!(
        owner.is_dirty(),
        "closing never saves or undoes the shared host"
    );
}

#[cfg(feature = "collect-ui")]
#[test]
fn actual_app_ready_collect_ui_draft_survives_passive_close_and_explicit_status_clear() {
    use orr_sample::{authored_ui::Document, collect_project::PreparedCollectUi};
    let root = tempfile::tempdir().unwrap();
    let scene = root.path().join("scene.yaml");
    let path = root.path().join("ui.json");
    fs::write(
        &scene,
        include_bytes!("../../../../scenes/collect_dodge_v1.scene.yaml"),
    )
    .unwrap();
    let document = Document::default_collect();
    fs::write(&path, document.to_bytes().unwrap()).unwrap();
    let editor = Editor::open_game(&scene, crate::game::EditorGame::CollectDodge).unwrap();
    let mut harness = harness(editor);
    let prepared = PreparedCollectUi {
        path: path.clone(),
        document: document.clone(),
        font: include_bytes!("../../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec(),
    };
    harness.state_mut().install_collect_ui_preview(&prepared);
    harness.state_mut().collect_ui = Some(crate::collect_ui_panel::Panel::new(prepared));
    harness.state_mut().stage_close_test_position();
    let candidate = harness.state().close_test_position_document().unwrap();
    assert_ne!(candidate, document);
    let stamp = host_stamp(&harness);
    let original_scene = fs::read(&scene).unwrap();
    let original_ui = fs::read(&path).unwrap();
    assert!(!harness.state().editor.is_dirty());
    assert!(!harness.state().collect_ui.as_ref().unwrap().dirty());
    assert!(request_root_close(&mut harness));
    assert_eq!(
        item_names(&harness),
        ["Collect UI preview · unapplied or unfinished"]
    );
    let viewport = harness.state().ui.viewport_rect;
    harness.run_steps(2);
    assert_eq!(harness.state().ui.viewport_rect, viewport);
    assert_eq!(harness.state().ui.bottom_tab, BottomTab::Timeline);
    assert!(harness.query_by_label("Clear close status").is_none());
    assert!(harness.state().collect_ui_staged_authoring());
    assert_eq!(
        harness.state().close_test_position_document(),
        Some(candidate.clone())
    );
    // Only an explicit user/agent choice opens embedded details. Clearing the
    // passive indicator is not a draft cancellation or permission to exit.
    harness.get_by_label("Unsaved work details").click();
    harness.run_steps(2);
    harness.get_by_label("Clear close status").click();
    harness.run_steps(1);
    assert!(!harness.state().ui.close_review.blocked);
    assert!(harness.state().collect_ui_staged_authoring());
    assert_eq!(
        harness.state().close_test_position_document(),
        Some(candidate)
    );
    assert!(request_root_close(&mut harness));
    assert_eq!(
        harness.state().collect_ui.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(host_stamp(&harness), stamp);
    assert_eq!(fs::read(scene).unwrap(), original_scene);
    assert_eq!(fs::read(path).unwrap(), original_ui);
    assert!(!harness.state_mut().collect_ui.as_mut().unwrap().undo());
    assert_no_explicit_close(&harness);
}

#[cfg(all(feature = "tilemap", feature = "project-create", target_os = "linux"))]
#[test]
fn actual_app_existing_tilemap_close_protection_remains_without_gpu() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tiles");
    orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: "arena-tilemap-2d-v1".into(),
        seed: "close-review".into(),
    })
    .unwrap();
    let project = crate::project::PreparedProject::open(&root).unwrap();
    let mut harness = Harness::builder()
        .with_size([1500.0, 1300.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |cc| {
            let mut editor = Editor::start(&project.host_spec()).unwrap();
            editor.sync();
            project.into_app(editor, None, &cc.egui_ctx).unwrap()
        });
    settle(&mut harness);
    let original_scene = fs::read(root.join("arena.scene.yaml")).unwrap();
    let original_map = fs::read(root.join("arena.tilemap.json")).unwrap();
    let stamp = host_stamp(&harness);
    harness
        .get_by_label("Tile tool (owns viewport clicks)")
        .click();
    harness.run_steps(2);
    harness.get_by_label("Static region 10").click();
    harness.run_steps(2);
    let rect = harness.state().ui.viewport_rect.unwrap();
    let px = harness.state().ui.viewport_px;
    let screen = harness
        .state()
        .editor
        .camera
        .world_to_screen([-56.0, -8.0], px);
    let point = rect.min
        + egui::vec2(
            screen[0] * rect.width() / px.0 as f32,
            screen[1] * rect.height() / px.1 as f32,
        );
    assert!(rect.contains(point));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerMoved(point));
        harness.event(egui::Event::PointerButton {
            pos: point,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.run_steps(2);
    }
    assert!(harness.state().tilemap.as_ref().unwrap().dirty());
    let history = harness.state().tilemap.as_ref().unwrap().history_snapshot();
    harness.state_mut().save();
    settle(&mut harness);
    assert!(request_root_close(&mut harness));
    assert_eq!(item_names(&harness), ["Tilemap · unsaved"]);
    assert_eq!(
        harness.state().tilemap.as_ref().unwrap().history_snapshot(),
        history
    );
    assert_eq!(host_stamp(&harness), stamp);
    assert_eq!(
        fs::read(root.join("arena.scene.yaml")).unwrap(),
        original_scene
    );
    assert_eq!(
        fs::read(root.join("arena.tilemap.json")).unwrap(),
        original_map
    );
    assert!(!harness.state().has_gpu());
}

#[test]
fn actual_app_blocked_close_does_not_stall_erp_edits_or_screenshot_completion() {
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    let (scene, original_editor) = physics_editor(root.path());
    drop(original_editor);
    let editor = listening_editor(scene.clone());
    let url = editor.erp_status().unwrap().0;
    let mut harness = Harness::builder()
        .with_size([960.0, 720.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| EditorApp::new(editor, None));
    harness
        .input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .inner_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(960.0, 720.0),
    ));
    settle(&mut harness);
    edit_physics_scene(&mut harness);
    assert!(request_root_close(&mut harness));
    let before = harness.state().editor.checksum();
    let guid = harness
        .state()
        .editor
        .rows()
        .iter()
        .find(|row| row.name.as_deref() == Some("body_05"))
        .unwrap()
        .guid
        .as_ref()
        .unwrap()
        .to_string();
    let mut agent = orr_remote::ErpClient::connect(&url, None).unwrap();
    agent
        .call(
            "watch.subscribe",
            serde_json::json!({"topics": ["history"]}),
        )
        .unwrap();
    agent
        .call(
            "world.patch",
            serde_json::json!({
                "entity": guid, "component": "orr_physics::Body", "path": "pos.x", "value": "3"
            }),
        )
        .unwrap();
    wait_for_published_history(&mut agent);
    settle(&mut harness);
    assert_ne!(harness.state().editor.checksum(), before);
    assert!(harness.state().ui.close_review.blocked);
    let state = agent.call("sim.state", serde_json::Value::Null).unwrap();
    let id = agent
        .post(
            "view.screenshot",
            serde_json::json!({
                "target": "app_framebuffer", "timeout_ms": 5000
            }),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let ticket = loop {
        harness.run_steps(1);
        let ticket = harness
            .output()
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .unwrap()
            .commands
            .iter()
            .find_map(|command| {
                let egui::ViewportCommand::Screenshot(data) = command else {
                    return None;
                };
                data.data.as_ref()?.downcast_ref::<u64>().copied()
            });
        if let Some(ticket) = ticket {
            break ticket;
        }
        assert!(
            Instant::now() < deadline,
            "blocked close must not stall capture polling"
        );
        std::thread::yield_now();
    };
    let pass = harness
        .state()
        .remote_capture
        .as_ref()
        .unwrap()
        .pass
        .unwrap();
    assert_capture_receipt_current(&harness, pass, "ticket");
    // Real App/ERP lifecycle, synthetic framebuffer delivery. No GPU claim.
    harness.event(egui::Event::Screenshot {
        viewport_id: egui::ViewportId::ROOT,
        user_data: egui::UserData::new(ticket),
        image: std::sync::Arc::new(egui::ColorImage::filled([960, 720], egui::Color32::BLACK)),
    });
    harness.run_steps(1);
    assert_capture_receipt_current(&harness, pass, "synthetic delivery");
    let result = loop {
        agent.poll().unwrap();
        if let Some(result) = agent.take_response(id) {
            break result.unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "blocked close must not stall screenshot completion"
        );
        harness.run_steps(1);
        assert_capture_receipt_current(&harness, pass, "encoder completion");
        std::thread::yield_now();
    };
    assert_eq!(result["status"], "captured");
    assert_eq!(result["checksum"], state["checksum"]);
    assert!(harness.state().ui.close_review.blocked);
    assert!(harness.state().ui.dialog.is_none());
    assert_no_explicit_close(&harness);
}

#[test]
fn actual_app_controlled_screenshot_exit_is_not_a_native_close_request() {
    let root = tempfile::tempdir().unwrap();
    let (_scene, editor) = physics_editor(root.path());
    let mut harness = harness(editor);
    edit_physics_scene(&mut harness);
    assert!(request_root_close(&mut harness));
    let png = root.path().join("controlled.png");
    harness.state_mut().shot = Some(ScreenshotJob::new(png.clone(), 0));
    harness.run_steps(1);
    harness.event(egui::Event::Screenshot {
        viewport_id: egui::ViewportId::ROOT,
        user_data: egui::UserData::default(),
        image: std::sync::Arc::new(egui::ColorImage::filled([16, 16], egui::Color32::BLACK)),
    });
    harness.run_steps(1);
    let commands = &harness.output().viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(commands
        .iter()
        .any(|cmd| matches!(cmd, egui::ViewportCommand::Close)));
    assert!(!commands
        .iter()
        .any(|cmd| matches!(cmd, egui::ViewportCommand::CancelClose)));
    assert!(png.is_file());
    assert!(harness.state().shot.is_none());
    assert!(harness.state().editor.is_dirty());
}

// A real host history publication, not a delay or an editor cache mutation.
// The observer receives the same broadcast as the editor. The following read
// barrier ensures that publication has finished before the close-frame pump.
fn wait_for_published_history(agent: &mut orr_remote::ErpClient) {
    use std::time::{Duration, Instant};
    let history = agent.call("history.list", serde_json::Value::Null).unwrap();
    let entries = history["entries"].as_array().unwrap();
    let last_id = entries.last().map(|entry| entry["id"].clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        agent.poll().unwrap();
        if agent.notifications.iter().any(|note| {
            note["method"] == "watch.history"
                && note["params"]["len"].as_u64() == Some(entries.len() as u64)
                && note["params"]["dirty"] == history["dirty"]
                && note["params"]["last"].get("id").cloned() == last_id
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "host did not publish the committed history"
        );
        std::thread::yield_now();
    }
    agent.call("sim.state", serde_json::Value::Null).unwrap();
}

fn assert_capture_receipt_current(
    harness: &Harness<'_, EditorApp>,
    pass: CapturePass,
    stage: &str,
) {
    let app = harness.state();
    let current = framebuffer_info(&harness.ctx).unwrap();
    assert!(app.capture_pass_still_current(&harness.ctx, pass),
        "{stage}: state={:?} expected={:?}; frame_seq={:?} expected={}; framebuffer_matches={}; snapshot_matches={}; pulses={}; waiting={:?}",
        app.editor.capture_view_state(), pass.state, app.editor.capture_snapshot_seq(), pass.frame_seq,
        same_framebuffer(current, pass.framebuffer), app.editor.capture_snapshot_matches(pass.state),
        app.ui.pulses.len(), app.editor.screenshot_waiting_for());
}

#[test]
fn actual_app_same_frame_spawn_and_undo_to_saved_use_acknowledged_history() {
    let root = tempfile::tempdir().unwrap();
    let (scene, editor) = physics_editor(root.path());
    let mut harness = harness(editor);
    let original = fs::read(&scene).unwrap();
    let checksum = harness.state().editor.checksum();
    harness.get_by_label("+ Body").click_accesskit();
    assert!(
        request_root_close(&mut harness),
        "acknowledged spawn must block same-frame close"
    );
    assert!(harness.state().editor.is_dirty());
    // Undo returns authoritative history synchronously. No idle pump is used
    // to turn a stale dirty marker into a clean result before the next close.
    assert!(harness.state_mut().editor.undo());
    assert!(!harness.state().editor.is_dirty());
    assert!(
        !request_root_close(&mut harness),
        "Undo to saved must permit a fresh close"
    );
    settle(&mut harness);
    assert_eq!(harness.state().editor.checksum(), checksum);
    assert_eq!(fs::read(scene).unwrap(), original);
}

#[test]
fn actual_app_noop_failure_and_read_refresh_do_not_invent_unsaved_scene() {
    let root = tempfile::tempdir().unwrap();
    let (scene, editor) = physics_editor(root.path());
    let mut harness = harness(editor);
    edit_physics_scene(&mut harness);
    assert!(harness.state_mut().editor.save());
    settle(&mut harness);
    let stamp = host_stamp(&harness);
    let original = fs::read(&scene).unwrap();
    assert!(!harness.state_mut().editor.set_field(
        &Owner::Component("orr_physics::Body".into()),
        "pos.x",
        Value::Fixed(FP::from_int(2))
    ));
    assert!(
        !harness.state().editor.is_dirty(),
        "no-op must not become semantic dirty"
    );
    assert!(!harness
        .state_mut()
        .editor
        .add_component("invalid::MissingComponent"));
    assert!(
        !harness.state().editor.is_dirty(),
        "rejected mutation must not become semantic dirty"
    );
    assert!(!request_root_close(&mut harness));
    harness.state_mut().editor.sync();
    assert!(!harness.state().editor.is_dirty());
    assert!(!request_root_close(&mut harness));
    assert_eq!(host_stamp(&harness), stamp);
    assert_eq!(fs::read(scene).unwrap(), original);
}

#[test]
fn actual_app_play_spawn_does_not_mark_saved_authoring_scene_dirty() {
    let root = tempfile::tempdir().unwrap();
    let (scene, editor) = physics_editor(root.path());
    let mut harness = harness(editor);
    let original = fs::read(&scene).unwrap();
    assert!(harness.state_mut().editor.start_play());
    settle(&mut harness);
    assert_eq!(harness.state().editor.mode(), Mode::Play);
    assert!(!harness.state().editor.is_dirty());
    assert!(harness.state_mut().editor.spawn_body([0.0, 5.0]));
    assert!(
        !harness.state().editor.is_dirty(),
        "runtime spawn is not an authored scene edit"
    );
    assert!(!request_root_close(&mut harness));
    assert_eq!(fs::read(scene).unwrap(), original);
}

#[test]
fn actual_app_transaction_commit_history_is_visible_before_close() {
    let root = tempfile::tempdir().unwrap();
    let yard = root.path().join("yard.yaml");
    fs::write(
        &yard,
        include_bytes!("../../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    let editor = Editor::open_game(&yard, crate::game::EditorGame::Yard3D).unwrap();
    let mut app = harness(editor);
    assert!(app.state_mut().editor.select_named("box_left"));
    settle(&mut app);
    let original = fs::read(&yard).unwrap();
    let position = orr_fp::FPVec3::new(FP::from_int(-3), FP::from_int(2), FP::ZERO);
    let rotation = [FP::ZERO, FP::ZERO, FP::ZERO, FP::ONE];
    assert!(app
        .state_mut()
        .editor
        .set_yard_transform(position, rotation));
    assert!(
        app.state().editor.is_dirty(),
        "final Yard commit is known without a pump"
    );
    assert!(request_root_close(&mut app));
    assert_eq!(fs::read(&yard).unwrap(), original);
    assert!(app.state_mut().editor.undo());
    assert!(!app.state().editor.is_dirty());
    assert!(!request_root_close(&mut app));
    settle(&mut app);
    // This negative control starts from a separate saved document. Rollback
    // of a pre-existing redo branch is a distinct existing EditDoc behavior,
    // not a contract introduced or changed by the close guard.
    drop(app);
    let editor = Editor::open_game(&yard, crate::game::EditorGame::Yard3D).unwrap();
    let mut app = harness(editor);
    assert!(app.state_mut().editor.select_named("box_left"));
    settle(&mut app);
    let stamp = host_stamp(&app);
    assert!(!app
        .state_mut()
        .editor
        .set_yard_transform(position, [FP::ZERO; 4]));
    // Rollback does not return history, so let existing authoritative refresh
    // confirm the result rather than manufacture clean state from missing data.
    settle(&mut app);
    assert!(!app.state().editor.is_dirty());
    assert!(!request_root_close(&mut app));
    assert_eq!(host_stamp(&app), stamp);
    assert_eq!(fs::read(yard).unwrap(), original);
    assert!(app.state_mut().editor.set_yard_transform(
        orr_fp::FPVec3::new(FP::from_int(-2), FP::ONE, FP::ZERO),
        rotation
    ));
    assert!(
        !app.state().editor.is_dirty(),
        "no-op commit returns verified clean history"
    );
    assert!(!request_root_close(&mut app));

    let arena = root.path().join("arena.yaml");
    fs::write(
        &arena,
        include_bytes!("../../../../scenes/arena_blank.scene.yaml"),
    )
    .unwrap();
    let editor = Editor::open_game(&arena, crate::game::EditorGame::Arena).unwrap();
    let mut app = harness(editor);
    let original = fs::read(&arena).unwrap();
    assert!(app.state_mut().editor.spawn_arena_player(0, [0.0, 0.0]));
    assert!(
        app.state().editor.is_dirty(),
        "final Arena commit is known without a pump"
    );
    assert!(request_root_close(&mut app));
    assert_eq!(fs::read(arena).unwrap(), original);
}
