use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
};

const GAME_ID: &str = "bf3b65a7-752c-4c56-91bb-d76c5de01ce3";

fn current() -> (tempfile::TempDir, EditorApp) {
    let directory = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let scene = directory.path().join("current.scene.yaml");
    fs::write(
        &scene,
        include_bytes!("../../../../../scenes/physics_demo.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open(&scene).unwrap();
    editor.sync();
    let mut app = EditorApp::new(editor, None);
    app.ui.collect_project.open(Kind::New);
    (directory, app)
}

fn create(path: &Path) -> Request {
    Request {
        path: path.into(),
        create: Some(("editor-tests".into(), GAME_ID.into())),
    }
}

fn start(_: &mut EditorApp, project: &PreparedProject) -> Result<Editor, String> {
    Editor::start(&HostSpec::PreparedCollect {
        scene: project.path().into(),
        text: project.scene().text().into(),
        listen: None,
        debug_hooks: false,
    })
}

fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn collect(root: &Path, path: &Path, result: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                collect(root, &entry.path(), result);
            } else {
                result.push((
                    entry.path().strip_prefix(root).unwrap().into(),
                    fs::read(entry.path()).unwrap(),
                ));
            }
        }
    }
    let mut result = Vec::new();
    collect(root, root, &mut result);
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

#[test]
fn host_failure_retains_created_project_and_explicit_open_recovery() {
    let (directory, mut app) = current();
    let ctx = egui::Context::default();
    let old = app.editor.collect_project_source_stamp().unwrap();
    let target = directory.path().join("created");
    assert!(
        !app.switch_collect_project_with(&ctx, create(&target), |_, _| Err(
            "injected host failure".into()
        ))
    );
    assert_eq!(app.editor.collect_project_source_stamp().unwrap(), old);
    assert!(app.collect_ui.is_none());
    let before = files(&target);
    let recovery = app.ui.collect_project.request().unwrap();
    assert_eq!(recovery.path, target);
    assert!(
        recovery.create.is_none(),
        "recovery must never repeat creation"
    );
    assert!(app.switch_collect_project_with(&ctx, recovery, start));
    assert!(app.editor.game().is_collect());
    assert!(app.collect_ui.is_some());
    assert!(app.sprites.bindings.is_some());
    assert!(!app.collect_project_dialog_open());
    assert_eq!(files(&target), before);
}

#[test]
fn same_path_reopen_during_preparation_rejects_adoption_and_retains_created_files() {
    let (directory, mut app) = current();
    let ctx = egui::Context::default();
    let old_path = app.editor.path().unwrap();
    let old_epoch = app.editor.scene_load_epoch();
    let target = directory.path().join("created");
    assert!(
        !app.switch_collect_project_with(&ctx, create(&target), |app, project| {
            assert!(app.editor.open_path(&old_path));
            app.editor.sync();
            start(app, project)
        })
    );
    assert_eq!(app.editor.path().unwrap(), old_path);
    assert_ne!(app.editor.scene_load_epoch(), old_epoch);
    assert!(target.join("orr.project.json").is_file());
    assert!(app.collect_ui.is_none());
    assert!(app.ui.collect_project.request().unwrap().create.is_none());
}

#[test]
fn presentation_change_during_preparation_prevents_replacement() {
    let (directory, mut app) = current();
    let old_path = app.editor.path().unwrap();
    let target = directory.path().join("created");
    assert!(!app.switch_collect_project_with(
        &egui::Context::default(),
        create(&target),
        |app, project| {
            app.ui.take_control = true;
            start(app, project)
        }
    ));
    assert_eq!(app.editor.path().unwrap(), old_path);
    assert!(app.ui.take_control);
    assert!(target.is_dir());
    assert!(app.collect_ui.is_none());
}

#[test]
fn repeated_creation_never_replaces_preexisting_output() {
    let (directory, mut app) = current();
    let target = directory.path().join("created");
    let ctx = egui::Context::default();
    assert!(
        !app.switch_collect_project_with(&ctx, create(&target), |_, _| Err(
            "injected host failure".into()
        ))
    );
    let before = files(&target);
    let old = app.editor.collect_project_source_stamp().unwrap();
    assert!(
        !app.switch_collect_project_with(&ctx, create(&target), |_, _| panic!(
            "existing destination must reject before host startup"
        ))
    );
    assert_eq!(files(&target), before);
    assert_eq!(app.editor.collect_project_source_stamp().unwrap(), old);
}

#[test]
fn pending_control_rejects_before_creating_any_files() {
    let (directory, mut app) = current();
    let target = directory.path().join("never-created");
    app.ui.take_control = true;
    assert!(!app.switch_collect_project_with(
        &egui::Context::default(),
        create(&target),
        |_, _| panic!("blocked before creation")
    ));
    assert!(!target.exists());
}

#[test]
fn open_rejects_incomplete_project_without_replacement() {
    let (directory, mut app) = current();
    let old = app.editor.collect_project_source_stamp().unwrap();
    let target = directory.path().join("headless");
    orr_sample::project_create::create_collect(
        &orr_sample::project_create::CreateOptions {
            output: target.clone(),
            template: orr_sample::project_create::COLLECT_TEMPLATE.into(),
            seed: "headless".into(),
        },
        GAME_ID,
    )
    .unwrap();
    let before = files(&target);
    assert!(!app.switch_collect_project_with(
        &egui::Context::default(),
        Request {
            path: target.clone(),
            create: None
        },
        |_, _| panic!("incomplete project must reject before startup")
    ));
    assert_eq!(app.editor.collect_project_source_stamp().unwrap(), old);
    assert_eq!(files(&target), before);
}

#[test]
fn wrong_candidate_host_cannot_replace_current_project() {
    let (directory, mut app) = current();
    let old_path = app.editor.path().unwrap();
    let target = directory.path().join("created");
    let old = app.editor.collect_project_source_stamp().unwrap();
    assert!(!app.switch_collect_project_with(
        &egui::Context::default(),
        create(&target),
        |_, _| Editor::open(&old_path)
    ));
    assert_eq!(app.editor.collect_project_source_stamp().unwrap(), old);
    assert!(target.is_dir());
    assert!(app.collect_ui.is_none());
    assert!(app.ui.collect_project.request().unwrap().create.is_none());
}

#[test]
fn collect_candidate_with_wrong_scene_checksum_cannot_replace_current_project() {
    let (directory, mut app) = current();
    let target = directory.path().join("created");
    let old = app.editor.collect_project_source_stamp().unwrap();
    assert!(!app.switch_collect_project_with(
        &egui::Context::default(),
        create(&target),
        |_, project| {
            let fixture = include_str!("../../../../../scenes/collect_dodge_v1.scene.yaml");
            assert!(fixture.contains("time_limit_ticks: 600"));
            let text = fixture.replace("time_limit_ticks: 600", "time_limit_ticks: 601");
            assert_ne!(
                orr_sample::collect_project::PreparedScene::parse(&text)
                    .unwrap()
                    .frame()
                    .checksum(),
                project.scene().frame().checksum()
            );
            Editor::start(&HostSpec::PreparedCollect {
                scene: project.path().into(),
                text,
                listen: None,
                debug_hooks: false,
            })
        }
    ));
    assert_eq!(app.editor.collect_project_source_stamp().unwrap(), old);
    assert!(target.is_dir());
    assert!(app.collect_ui.is_none());
    assert!(app.ui.collect_project.request().unwrap().create.is_none());
}
