//! Focused egui dispatch and owned-worker lifecycle; no GPU/window-input claim.
use super::*;
use crate::{editor::Editor, game::EditorGame};
use egui_kittest::{kittest::Queryable, Harness};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

fn fixture() -> (tempfile::TempDir, EditorApp) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("source");
    fs::create_dir_all(root.join("navigation")).unwrap();
    let scene = root.join("point.scene.yaml");
    fs::write(
        &scene,
        include_bytes!("../../../../scenes/navigation_point.scene.yaml"),
    )
    .unwrap();
    fs::write(
        root.join("navigation/point_demo.orrt"),
        include_bytes!("../../../../scenes/navigation/point_demo.orrt"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        editor.sync();
        if editor.snapshot().is_some() && editor.can_mutate() && editor.yard_rows_coherent() {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let mut app = EditorApp::new(editor, None);
    app.terrain.local_path = "navigation/point_demo.orrt".into();
    app.terrain.open_for_editor(&app.editor).unwrap();
    app.navigation.sync_from_editor(&app.editor);
    app.navigation.observe_terrain(&app.editor, &app.terrain);
    app.navigation_export.runtime = dir.path().join("unused-runtime").display().to_string();
    app.navigation_export.hash = "a".repeat(64);
    app.navigation_export.output = dir.path().join("bundle").display().to_string();
    app.navigation_export.trusted = true;
    app.editor
        .navigation_export_source(&app.navigation, &app.terrain)
        .unwrap();
    (dir, app)
}
fn harness(app: EditorApp) -> Harness<'static, EditorApp> {
    Harness::builder().with_size([1500., 1000.]).build_ui_state(
        |ui, app: &mut EditorApp| app.show_navigation_export(ui),
        app,
    )
}
// Local helper: the sprite-only test module is not part of this feature closure.
fn request_root_close(harness: &mut Harness<'_, EditorApp>) -> bool {
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

fn synthetic_job(app: &mut EditorApp, root: &Path) -> mpsc::SyncSender<()> {
    let source = Arc::new(
        app.editor
            .navigation_export_source(&app.navigation, &app.terrain)
            .unwrap(),
    );
    let options = ExportRequest {
        runtime: root.join("unused"),
        runtime_sha256: "a".repeat(64),
        output: root.join("new"),
        trusted_runtime: true,
        source_revision: None,
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        sender.send(Err("test terminal result".into())).unwrap();
        wait.recv_timeout(Duration::from_secs(10)).unwrap();
    });
    app.navigation_export.job = Some(Job {
        id: 7,
        source,
        options,
        receiver,
        handle: Some(handle),
        result: None,
        disconnected: false,
    });
    release
}
fn reap(app: &mut EditorApp) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while app.navigation_export.busy() {
        assert!(Instant::now() < deadline, "worker deadline");
        app.navigation_export.poll();
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn spawn_failure_preserves_source_history_and_allows_explicit_retry() {
    let (dir, mut app) = fixture();
    let scene = dir.path().join("source/point.scene.yaml");
    let before = fs::read(&scene).unwrap();
    let history = app.editor.history().clone();
    for _ in 0..2 {
        let mut called = false;
        let error = app
            .navigation_export_dispatch_with(|_, _| {
                called = true;
                Err(std::io::Error::other("spawn sentinel"))
            })
            .unwrap_err();
        assert!(called && error.contains("spawn sentinel"));
        assert!(!app.navigation_export.busy());
        assert_eq!(app.navigation_export.next_id, 0);
        assert!(app.navigation_export.uncertain_output.is_none());
    }
    assert_eq!(fs::read(scene).unwrap(), before);
    assert_eq!(app.editor.history(), &history);
    assert!(!dir.path().join("bundle").exists());
}
#[test]
fn result_does_not_release_ownership_until_terminal_and_joined() {
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    let until = Instant::now() + Duration::from_secs(10);
    while app.navigation_export.job.as_ref().unwrap().result.is_none() {
        assert!(Instant::now() < until);
        app.navigation_export.poll();
        thread::yield_now();
    }
    assert!(app.navigation_export.busy());
    assert!(app
        .navigation_export_dispatch()
        .unwrap_err()
        .contains("still owned"));
    app.navigation_export.output = "different form output".into();
    release.send(()).unwrap();
    reap(&mut app);
    assert!(app.navigation_export.status.contains("Job 7"));
    assert_eq!(
        app.navigation_export.uncertain_output,
        Some(dir.path().join("new"))
    );
    assert!(app
        .navigation_export_dispatch()
        .unwrap_err()
        .contains("Inspect"));
}
#[test]
fn hidden_tab_close_and_clear_status_do_not_drop_worker() {
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    let mut h = egui_kittest::Harness::builder()
        .with_size([1500.0, 1000.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| app);
    h.run_steps(3);
    assert_eq!(h.state().ui.bottom_tab, crate::app::BottomTab::Timeline);
    assert!(request_root_close(&mut h));
    h.state_mut().ui.close_review.blocked = false;
    assert!(request_root_close(&mut h));
    assert_eq!(h.state().ui.bottom_tab, crate::app::BottomTab::Timeline);
    release.send(()).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while h.state().navigation_export.busy() {
        assert!(Instant::now() < until);
        h.run_steps(1);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(!request_root_close(&mut h));
}
#[test]
fn outer_early_return_poll_and_normal_teardown_join() {
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    release.send(()).unwrap();
    let ctx = egui::Context::default();
    let until = Instant::now() + Duration::from_secs(10);
    while app.navigation_export.busy() {
        assert!(Instant::now() < until);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            app.with_close_review(ui, |_, _| {});
        });
        // This CPU lifecycle test has no renderer to consume font textures.
        // Explicitly discard the delta before FullOutput is dropped.
        output.textures_delta.clear();
        thread::sleep(Duration::from_millis(1));
    }
    let release = synthetic_job(&mut app, dir.path());
    release.send(()).unwrap();
    let handle = app
        .navigation_export
        .job
        .as_ref()
        .unwrap()
        .handle
        .as_ref()
        .unwrap()
        .thread()
        .id();
    drop(app); // joins; never detaches.
    let _ = handle;
}
#[test]
fn panic_after_result_requires_destination_inspection() {
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    release.send(()).unwrap();
    reap(&mut app);
    app.navigation_export.uncertain_output = None;
    let release = synthetic_job(&mut app, dir.path());
    drop(release); // test worker panics after delivering a result.
    reap(&mut app);
    assert!(app.navigation_export.status.contains("uncertain"));
    assert!(app.navigation_export.uncertain_output.is_some());
}

#[test]
fn disconnected_worker_requires_inspection_without_automatic_retry() {
    let (dir, mut app) = fixture();
    let source = Arc::new(
        app.editor
            .navigation_export_source(&app.navigation, &app.terrain)
            .unwrap(),
    );
    let options = ExportRequest {
        runtime: dir.path().join("unused"),
        runtime_sha256: "a".repeat(64),
        output: dir.path().join("new"),
        trusted_runtime: true,
        source_revision: None,
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        drop(sender);
    }); // Normal return with no terminal message, not panic.
    app.navigation_export.job = Some(Job {
        id: 7,
        source,
        options,
        receiver,
        handle: Some(handle),
        result: None,
        disconnected: false,
    });
    reap(&mut app);
    assert!(app.navigation_export.status.contains("uncertain"));
    assert!(app
        .navigation_export_dispatch()
        .unwrap_err()
        .contains("Inspect"));
    let mut h = harness(app);
    h.run_steps(2);
    h.get_by_label("Inspect previous destination").click();
    h.run_steps(2);
    assert!(h.state().navigation_export.uncertain_output.is_none());
    assert!(!h.state().navigation_export.busy());
    assert_eq!(h.state().navigation_export.next_id, 0);
}
#[test]
fn actual_widget_runtime_hash_changes_revoke_trust_and_preserve_destination() {
    let (dir, mut app) = fixture();
    let destination = dir.path().join("existing");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("keep"), b"preserved").unwrap();
    app.navigation_export.uncertain_output = Some(destination.clone());
    let mut h = harness(app);
    h.run_steps(2);
    for label in ["Runtime path", "Expected lowercase SHA256"] {
        h.state_mut().navigation_export.trusted = true;
        h.get_by_role_and_label(egui::accesskit::Role::TextInput, label)
            .focus();
        h.run_steps(2);
        h.get_by_role_and_label(egui::accesskit::Role::TextInput, label)
            .type_text("x");
        h.run_steps(2);
        assert!(!h.state().navigation_export.trusted);
    }
    h.get_by_label("Inspect previous destination").click();
    h.run_steps(2);
    assert!(h.state().navigation_export.uncertain_output.is_none());
    assert!(!h.state().navigation_export.busy());
    assert_eq!(fs::read(destination.join("keep")).unwrap(), b"preserved");
}
#[test]
#[ignore = "requires ORR_EDITOR_NAVIGATION_RUNTIME; executes explicitly supplied trusted runtime"]
fn actual_button_worker_relocates_and_rejects_queued_source_change() {
    use orr_navigation_view::authored::AuthoredPlayer;
    use sha2::{Digest, Sha256};
    let supplied = PathBuf::from(
        std::env::var_os("ORR_EDITOR_NAVIGATION_RUNTIME")
            .expect("explicit trusted bundle-capable runtime required"),
    );
    assert!(supplied.is_absolute());
    let (dir, mut app) = fixture();
    let scene = dir.path().join("source/point.scene.yaml");
    assert!(app.editor.save());
    app.editor.sync();
    let history = app.editor.history().clone();
    let saved = fs::read(&scene).unwrap();
    let mut expected = AuthoredPlayer::open(&scene).unwrap();
    for _ in 0..60 {
        expected.step().unwrap();
    }
    let runtime = dir.path().join("supplied-player");
    fs::copy(&supplied, &runtime).unwrap();
    app.navigation_export.runtime = runtime.display().to_string();
    app.navigation_export.hash = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut h = harness(app);
    h.run_steps(2);
    h.get_by_label("Export saved Navigation").click();
    h.run_steps(2);
    assert_eq!(h.state().navigation_export.next_id, 1);
    reap(h.state_mut());
    assert!(
        h.state().navigation_export.completed_successfully,
        "{}",
        h.state().navigation_export.status
    );
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&scene).unwrap(), saved);
    drop(h);
    let relocated = dir.path().join("relocated bundle with spaces");
    fs::rename(dir.path().join("bundle"), &relocated).unwrap();
    fs::rename(dir.path().join("source"), dir.path().join("hidden-source")).unwrap();
    fs::rename(&runtime, dir.path().join("hidden-runtime")).unwrap();
    let cwd = dir.path().join("empty-cwd");
    fs::create_dir(&cwd).unwrap();
    let output = std::process::Command::new(relocated.join("run-navigation"))
        .args(["--ticks", "60", "--restart-check"])
        .current_dir(&cwd)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["checksum"].as_u64(), Some(expected.checksum()));
    assert_eq!(receipt["tick"], 60);
    assert_eq!(receipt["restart_checked"], true);
    assert!(receipt["capture"].is_null());
    assert_eq!(fs::read_dir(&cwd).unwrap().count(), 0);
    let (dir, mut app) = fixture();
    app.navigation_export.runtime = supplied.display().to_string();
    app.navigation_export.hash = Sha256::digest(fs::read(&supplied).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let (release, wait) = mpsc::sync_channel(1);
    app.navigation_export_dispatch_with(|_, task| {
        thread::Builder::new().spawn(move || {
            wait.recv_timeout(Duration::from_secs(10)).unwrap();
            task();
        })
    })
    .unwrap();
    let path = dir.path().join("source/point.scene.yaml");
    let mut altered = fs::read(&path).unwrap();
    altered.push(b'\n');
    fs::write(&path, altered).unwrap();
    assert!(app
        .navigation_export_dispatch()
        .unwrap_err()
        .contains("still owned"));
    release.send(()).unwrap();
    reap(&mut app);
    assert!(!app.navigation_export.completed_successfully);
    assert!(app.navigation_export.uncertain_output.is_some());
    assert!(!dir.path().join("bundle").exists());
}
