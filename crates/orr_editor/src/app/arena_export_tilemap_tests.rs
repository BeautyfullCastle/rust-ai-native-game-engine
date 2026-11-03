//! Real egui dispatch and captured saved tilemap authority; no native-input claim.
use super::*;
use crate::editor::Editor;
use egui_kittest::{kittest::Queryable, Harness};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

fn fixture() -> (tempfile::TempDir, EditorApp) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: "arena-tilemap-2d-v1".into(),
        seed: "editor-tilemap-export".into(),
    })
    .unwrap();
    let map_path = root.join("arena.tilemap.json");
    let mut map: orr_tilemap::Document =
        serde_json::from_slice(&fs::read(&map_path).unwrap()).unwrap();
    map.cells[0] = Some(10);
    fs::write(&map_path, serde_json::to_vec(&map).unwrap()).unwrap();
    let prepared = crate::project::PreparedProject::open(&root).unwrap();
    let mut editor = Editor::start(&prepared.host_spec()).unwrap();
    editor.sync();
    let mut app = prepared
        .into_app(editor, None, &egui::Context::default())
        .unwrap();
    app.arena_export.project = root.display().to_string();
    app.arena_export.runtime = temp.path().join("unused-runtime").display().to_string();
    app.arena_export.hash = "a".repeat(64);
    app.arena_export.output = temp.path().join("bundle").display().to_string();
    app.arena_export.trusted = true;
    assert!(app
        .tilemap
        .as_ref()
        .unwrap()
        .document()
        .document()
        .cells
        .iter()
        .any(Option::is_some));
    (temp, app)
}
fn spawn_probe(app: &mut EditorApp, expected: bool) -> String {
    let mut called = false;
    let error = app
        .arena_export_dispatch_with(|_, _task| {
            called = true;
            Err(std::io::Error::other("spawn sentinel; no worker executed"))
        })
        .unwrap_err();
    assert_eq!(called, expected, "{error}");
    assert!(!app.arena_export.busy());
    assert_eq!(app.arena_export.next_id, 0);
    error
}
fn harness(app: EditorApp, tiles: bool) -> Harness<'static, EditorApp> {
    Harness::builder().with_size([1600., 1400.]).build_ui_state(
        move |ui, app: &mut EditorApp| {
            if tiles {
                app.tilemap
                    .as_mut()
                    .unwrap()
                    .show(ui, &app.editor, &mut app.sprites);
            } else {
                app.show_arena_export(ui);
            }
        },
        app,
    )
}
#[test]
fn saved_matching_tilemap_reaches_spawn_without_mutation() {
    let (temp, mut app) = fixture();
    let map = temp.path().join("project/arena.tilemap.json");
    let before = fs::read(&map).unwrap();
    let history = app.editor.history().clone();
    assert!(spawn_probe(&mut app, true).contains("spawn sentinel"));
    assert_eq!(fs::read(map).unwrap(), before);
    assert_eq!(app.editor.history(), &history);
    assert!(!temp.path().join("bundle").exists());
}
#[test]
fn missing_panel_play_and_external_tilemap_change_refuse_before_spawn() {
    for case in ["missing", "play", "external", "reload-blocked"] {
        let (temp, mut app) = fixture();
        let output = temp.path().join("bundle");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("keep"), b"existing output").unwrap();
        match case {
            "missing" => {
                app.tilemap = None;
            }
            "play" => {
                app.editor.play();
                app.editor.sync();
            }
            "external" => {
                let path = temp.path().join("project/arena.tilemap.json");
                let mut bytes = fs::read(&path).unwrap();
                bytes.push(b'\n');
                fs::write(path, bytes).unwrap();
            }
            "reload-blocked" => {
                let epoch = app.editor.scene_load_epoch();
                // A tilemap project deliberately blocks a scene-only reload;
                // this does not replace the panel or advance its lifecycle.
                assert!(!app
                    .editor
                    .open_path(&temp.path().join("project/arena.scene.yaml")));
                app.editor.sync();
                assert_eq!(epoch, app.editor.scene_load_epoch());
            }
            _ => unreachable!(),
        }
        assert!(!spawn_probe(&mut app, case == "reload-blocked").is_empty());
        assert_eq!(fs::read(output.join("keep")).unwrap(), b"existing output");
    }
}
#[test]
fn actual_resize_requires_explicit_tilemap_save_before_export() {
    let (temp, app) = fixture();
    let map = temp.path().join("project/arena.tilemap.json");
    let original = fs::read(&map).unwrap();
    let width = app.tilemap.as_ref().unwrap().document().document().width;
    let mut h = harness(app, true);
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Width")
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Width")
        .type_text(&(width + 1).to_string());
    h.run_steps(2);
    h.get_by_label("Resize").click();
    h.run_steps(2);
    assert!(h.state().tilemap.as_ref().unwrap().dirty());
    assert_eq!(fs::read(&map).unwrap(), original);
    spawn_probe(h.state_mut(), false);
    h.get_by_label("Save tilemap").click();
    h.run_steps(2);
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    assert_ne!(fs::read(&map).unwrap(), original);
    spawn_probe(h.state_mut(), true);
}
fn use_runtime(app: &mut EditorApp, runtime: &Path) {
    app.arena_export.runtime = runtime.display().to_string();
    app.arena_export.hash = Sha256::digest(fs::read(runtime).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    app.arena_export.trusted = true;
}
fn reap(app: &mut EditorApp) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while app.arena_export.busy() {
        assert!(Instant::now() < deadline, "worker completion deadline");
        app.arena_export.poll();
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
#[ignore = "requires explicit trusted ORR_EDITOR_TILEMAP_RUNTIME; executes real exported player"]
fn actual_export_button_worker_relocation_and_captured_source_guards() {
    let supplied = PathBuf::from(
        std::env::var_os("ORR_EDITOR_TILEMAP_RUNTIME")
            .expect("actual trusted tilemap Arena binary required"),
    );
    assert!(supplied.is_absolute() && supplied.is_file());
    let (temp, mut app) = fixture();
    let runtime = temp.path().join("supplied-arena");
    fs::copy(&supplied, &runtime).unwrap();
    use_runtime(&mut app, &runtime);
    let saved = fs::read(temp.path().join("project/arena.tilemap.json")).unwrap();
    let history = app.editor.history().clone();
    let mut h = harness(app, false);
    h.run_steps(2);
    h.get_by_label("Export saved Arena").click();
    h.run_steps(2);
    assert_eq!(
        h.state().arena_export.next_id,
        1,
        "actual widget must dispatch"
    );
    reap(h.state_mut());
    assert!(
        h.state().arena_export.completed_successfully,
        "{}",
        h.state().arena_export.status
    );
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(
        fs::read(temp.path().join("project/arena.tilemap.json")).unwrap(),
        saved
    );
    drop(h);
    let relocated = temp.path().join("relocated");
    fs::rename(temp.path().join("bundle"), &relocated).unwrap();
    fs::rename(
        temp.path().join("project"),
        temp.path().join("hidden-project"),
    )
    .unwrap();
    fs::rename(&runtime, temp.path().join("hidden-runtime")).unwrap();
    assert!(!runtime.exists() && !temp.path().join("project").exists());
    let cwd = temp.path().join("empty-cwd");
    fs::create_dir(&cwd).unwrap();
    let run = std::process::Command::new(relocated.join("run-arena"))
        .args(["--headless", "--ticks", "60", "--hold", "right"])
        .current_dir(cwd)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("project tick: 60 checksum:"));
    // Ordinary owned-source relocation, NOT sandbox isolation or GPU pixel coverage.
    let (temp, mut app) = fixture();
    use_runtime(&mut app, &supplied);
    let (release, wait) = mpsc::sync_channel(1);
    app.arena_export_dispatch_with(|_, task| {
        thread::Builder::new().spawn(move || {
            wait.recv_timeout(Duration::from_secs(10)).unwrap();
            task();
        })
    })
    .unwrap();
    let map = temp.path().join("project/arena.tilemap.json");
    let mut changed = fs::read(&map).unwrap();
    changed.push(b'\n');
    fs::write(map, changed).unwrap();
    release.send(()).unwrap();
    reap(&mut app);
    assert!(!app.arena_export.completed_successfully);
    assert!(!temp.path().join("bundle").exists());
    assert!(
        app.arena_export.status.contains("source") || app.arena_export.status.contains("changed"),
        "{}",
        app.arena_export.status
    );
    let (temp, mut app) = fixture();
    use_runtime(&mut app, &supplied);
    fs::create_dir(temp.path().join("bundle")).unwrap();
    fs::write(temp.path().join("bundle/keep"), b"untouched").unwrap();
    app.arena_export_dispatch().unwrap();
    reap(&mut app);
    assert!(!app.arena_export.completed_successfully);
    assert_eq!(
        fs::read(temp.path().join("bundle/keep")).unwrap(),
        b"untouched"
    );
}
