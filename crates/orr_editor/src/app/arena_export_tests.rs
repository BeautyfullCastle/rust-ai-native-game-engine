use super::*;
use crate::{editor::Editor, game::EditorGame};
use egui_kittest::kittest::Queryable;
use std::{
    fs,
    sync::mpsc,
    time::{Duration, Instant},
};

fn fixture() -> (tempfile::TempDir, EditorApp) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("arena.scene.yaml"),
        include_bytes!("../../../../scenes/arena_blank.scene.yaml"),
    )
    .unwrap();
    fs::write(
        dir.path().join("orr.project.json"),
        br#"{"schema":2,"engine":"*","entry":{"game":"arena","scene":"arena.scene.yaml"}}"#,
    )
    .unwrap();
    let mut editor =
        Editor::open_game(&dir.path().join("arena.scene.yaml"), EditorGame::Arena).unwrap();
    editor.sync();
    let app = EditorApp::new(editor, None);
    (dir, app)
}
fn synthetic_job(app: &mut EditorApp, root: &std::path::Path) -> mpsc::SyncSender<()> {
    let source = app
        .editor
        .arena_export_source(root, &app.sprites, None)
        .unwrap();
    let options = ExportOptions {
        project: root.into(),
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
    app.arena_export.job = Some(Job {
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
    let until = Instant::now() + Duration::from_secs(10);
    while app.arena_export.busy() {
        assert!(Instant::now() < until);
        app.arena_export.poll();
        thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn result_does_not_release_ownership_until_terminal_and_joined() {
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    let until = Instant::now() + Duration::from_secs(10);
    while app.arena_export.job.as_ref().unwrap().result.is_none() {
        assert!(Instant::now() < until);
        app.arena_export.poll();
        thread::yield_now();
    }
    assert!(app.arena_export.busy());
    assert!(app
        .arena_export_dispatch()
        .unwrap_err()
        .contains("still owned"));
    app.arena_export.output = "different form output".into();
    release.send(()).unwrap();
    reap(&mut app);
    assert!(app.arena_export.status.contains("Job 7"));
    assert_eq!(
        app.arena_export.uncertain_output,
        Some(dir.path().join("new"))
    );
    assert!(app.arena_export_dispatch().unwrap_err().contains("Inspect"));
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
    assert!(crate::app::unsaved_close_tests::request_root_close(&mut h));
    h.state_mut().ui.close_review.blocked = false;
    assert!(crate::app::unsaved_close_tests::request_root_close(&mut h));
    assert_eq!(h.state().ui.bottom_tab, crate::app::BottomTab::Timeline);
    release.send(()).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while h.state().arena_export.busy() {
        assert!(Instant::now() < until);
        h.run_steps(1);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(!crate::app::unsaved_close_tests::request_root_close(&mut h));
}
#[test]
fn outer_early_return_poll_and_normal_teardown_join() {
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    release.send(()).unwrap();
    let ctx = egui::Context::default();
    let until = Instant::now() + Duration::from_secs(10);
    while app.arena_export.busy() {
        assert!(Instant::now() < until);
        let _ = ctx.run_ui(Default::default(), |ui| {
            app.with_close_review(ui, |_, _| {});
        });
        thread::sleep(Duration::from_millis(1));
    }
    let release = synthetic_job(&mut app, dir.path());
    release.send(()).unwrap();
    let handle = app
        .arena_export
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
    app.arena_export.uncertain_output = None;
    let release = synthetic_job(&mut app, dir.path());
    drop(release); // test worker panics after delivering a result.
    reap(&mut app);
    assert!(app.arena_export.status.contains("uncertain"));
    assert!(app.arena_export.uncertain_output.is_some());
}
#[test]
fn screenshot_mode_refuses_without_source_mutation() {
    let (dir, mut app) = fixture();
    let bytes = fs::read(dir.path().join("arena.scene.yaml")).unwrap();
    let history = app.editor.history().clone();
    app.shot = Some(crate::app::ScreenshotJob {
        path: dir.path().join("shot.png"),
        frames: 0,
        requested_at: None,
        settle: false,
        started_at: None,
    });
    assert!(app
        .arena_export_dispatch()
        .unwrap_err()
        .contains("screenshot"));
    assert_eq!(app.editor.history(), &history);
    assert_eq!(
        fs::read(dir.path().join("arena.scene.yaml")).unwrap(),
        bytes
    );
}
#[test]
fn actual_tab_is_explicit_and_guidance_does_not_dispatch() {
    let (_dir, app) = fixture();
    let mut h = egui_kittest::Harness::builder()
        .with_size([1500.0, 1000.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| app);
    h.run_steps(3);
    h.get_by_label("Arena export").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.bottom_tab, crate::app::BottomTab::ArenaExport);
    assert!(!h.state().arena_export.busy());
    assert!(!h.state().arena_export.trusted);
    assert!(h.state().ui.dialog.is_none());
}
#[test]
fn terminal_disconnected_without_result_is_uncertain_and_joined() {
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    release.send(()).unwrap();
    reap(&mut app);
    app.arena_export.uncertain_output = None;
    let source = app
        .editor
        .arena_export_source(dir.path(), &app.sprites, None)
        .unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || drop(sender));
    while !handle.is_finished() {
        thread::yield_now();
    }
    app.arena_export.job = Some(Job {
        id: 9,
        source,
        options: ExportOptions {
            project: dir.path().into(),
            runtime: dir.path().join("runtime"),
            runtime_sha256: "a".repeat(64),
            output: dir.path().join("out"),
            trusted_runtime: true,
            source_revision: None,
        },
        receiver,
        handle: Some(handle),
        result: None,
        disconnected: false,
    });
    app.arena_export.poll();
    assert!(!app.arena_export.busy());
    assert!(app.arena_export.status.contains("uncertain"));
    assert!(!app.arena_export.completed_successfully);
}
#[test]
fn owned_job_destructor_waits_for_delayed_worker_release() {
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    let job = app.arena_export.job.take().unwrap();
    let (done, finished) = mpsc::sync_channel(1);
    let dropper = thread::spawn(move || {
        drop(job);
        done.send(()).unwrap();
    });
    assert!(
        finished.recv_timeout(Duration::from_millis(30)).is_err(),
        "Drop cannot complete while worker waits"
    );
    release.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(10)).unwrap();
    dropper.join().unwrap();
}
#[test]
fn screenshot_builder_refuses_active_job_and_preserves_inactive_behavior() {
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    app = app.with_screenshot(crate::app::ScreenshotJob::new(
        dir.path().join("shot.png"),
        1,
    ));
    assert!(app.shot.is_none());
    assert!(app.arena_export.busy());
    assert!(app.arena_export.status.contains("refused"));
    release.send(()).unwrap();
    reap(&mut app);
    app = app.with_screenshot(crate::app::ScreenshotJob::new(
        dir.path().join("shot.png"),
        1,
    ));
    assert!(app.shot.is_some());
}
fn options(app: &mut EditorApp, dir: &std::path::Path) {
    app.arena_export.project = dir.display().to_string();
    app.arena_export.runtime = dir.join("unused").display().to_string();
    app.arena_export.hash = "a".repeat(64);
    app.arena_export.output = dir.join("out").display().to_string();
    app.arena_export.trusted = true;
}
#[test]
fn spawn_failure_is_terminal_without_running_worker_or_publishing() {
    let (dir, mut app) = fixture();
    options(&mut app, dir.path());
    let error = app
        .arena_export_dispatch_with(|_, _task| Err(std::io::Error::other("injected spawn failure")))
        .unwrap_err();
    assert!(error.contains("no export was run"));
    assert!(!app.arena_export.busy());
    assert!(!dir.path().join("out").exists());
    assert_eq!(app.arena_export.next_id, 0);
}
#[test]
fn path_dialog_refuses_and_completion_retains_pre_reload_identity() {
    let (dir, mut app) = fixture();
    app.ui.dialog = Some(crate::app::Dialog {
        kind: crate::app::DialogKind::Open,
        text: "unchanged".into(),
    });
    assert!(app
        .arena_export_dispatch()
        .unwrap_err()
        .contains("path dialogs"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().text, "unchanged");
    app.ui.dialog = None;
    let release = synthetic_job(&mut app, dir.path());
    let captured = app
        .arena_export
        .job
        .as_ref()
        .unwrap()
        .source
        .canonical_scene()
        .to_owned();
    let other = dir.path().join("other.scene.yaml");
    fs::copy(&captured, &other).unwrap();
    assert!(app.editor.open_path(&other));
    app.editor.sync();
    release.send(()).unwrap();
    reap(&mut app);
    assert!(app.arena_export.status.contains(captured.to_str().unwrap()));
    assert!(!app.arena_export.status.contains("other.scene.yaml"));
}
#[test]
fn same_frame_owned_worker_root_close_vetoes() {
    let (dir, mut app) = fixture();
    let ctx = egui::Context::default();
    let mut release = None;
    let mut input = egui::RawInput::default();
    input
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    let output = ctx.run_ui(input, |ui| {
        app.with_close_review(ui, |app, _| {
            release = Some(synthetic_job(app, dir.path()));
        });
    });
    assert!(output.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .iter()
        .any(|c| matches!(c, egui::ViewportCommand::CancelClose)));
    assert!(app.arena_export.busy());
    release.take().unwrap().send(()).unwrap();
    reap(&mut app);
}
#[test]
fn actual_widget_runtime_hash_changes_clear_trust_and_inspection_does_not_retry() {
    let (dir, mut app) = fixture();
    options(&mut app, dir.path());
    app.ui.bottom_tab = crate::app::BottomTab::ArenaExport;
    app.arena_export.uncertain_output = Some(dir.path().join("out"));
    fs::create_dir(dir.path().join("out")).unwrap();
    fs::write(dir.path().join("out/keep"), "preserve").unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_size([1500.0, 1000.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| app);
    h.run_steps(3);
    for label in ["Runtime path", "Expected lowercase SHA256"] {
        h.state_mut().arena_export.trusted = true;
        h.get_by_role_and_label(egui::accesskit::Role::TextInput, label)
            .focus();
        h.run_steps(2);
        let before = if label == "Runtime path" {
            h.state().arena_export.runtime.clone()
        } else {
            h.state().arena_export.hash.clone()
        };
        h.get_by_role_and_label(egui::accesskit::Role::TextInput, label)
            .type_text("x");
        h.run_steps(2);
        assert!(!h.state().arena_export.trusted);
        let after = if label == "Runtime path" {
            &h.state().arena_export.runtime
        } else {
            &h.state().arena_export.hash
        };
        assert_ne!(after, &before);
    }
    h.get_by_label("Inspect previous destination").click();
    h.run_steps(2);
    assert!(h.state().arena_export.uncertain_output.is_none());
    assert!(!h.state().arena_export.busy());
    assert_eq!(
        fs::read_to_string(dir.path().join("out/keep")).unwrap(),
        "preserve"
    );
}
#[cfg(feature = "mesh-navigation")]
#[test]
fn active_arena_job_refuses_mesh_whole_app_replacement() {
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    assert!(app.mesh_workspace_blocker().is_some());
    assert!(!app.switch_mesh_workspace(
        &dir.path().join("missing.scene.yaml"),
        crate::mesh_workspace::Intent::Open
    ));
    assert!(!app.switch_mesh_workspace(
        &dir.path().join("new.scene.yaml"),
        crate::mesh_workspace::Intent::Create
    ));
    assert!(!dir.path().join("new.scene.yaml").exists());
    assert!(app.editor.mesh_workspace_source_stamp().is_err());
    assert!(app.arena_export.busy());
    release.send(()).unwrap();
    reap(&mut app);
}
#[test]
fn remote_capture_and_encoder_refuse_without_cancellation() {
    let (_dir, mut app) = fixture();
    let state = app.editor.capture_view_state();
    app.remote_capture = Some(crate::app::RemoteCapture {
        request: orr_remote::CaptureRequest {
            endpoint_id: 1,
            serial: 1,
            options: orr_remote::ScreenshotOptions {
                max_width: 32,
                max_height: 32,
            },
            requested: state,
            deadline: Instant::now() + Duration::from_secs(5),
        },
        pass: None,
        screenshot_requested: false,
        image_received: false,
        orphaned: false,
    });
    assert!(app.arena_export_dispatch().unwrap_err().contains("capture"));
    assert!(app.remote_capture.is_some());
    app.remote_capture = None;
    app.encoder = Some(crate::app::EncoderWorker {
        serial: 1,
        pass: crate::app::CapturePass {
            state,
            frame_seq: 1,
            ui_frame: 1,
            framebuffer: crate::app::FramebufferInfo {
                viewport: egui::ViewportId::ROOT,
                logical_width: 1.0,
                logical_height: 1.0,
                pixels_per_point: 1.0,
                width: 1,
                height: 1,
            },
        },
        handle: thread::spawn(|| Ok(Vec::new())),
    });
    assert!(app.arena_export_dispatch().unwrap_err().contains("capture"));
    app.encoder.take().unwrap().handle.join().unwrap().unwrap();
}
#[cfg(feature = "project-ui")]
#[test]
fn loaded_ui_is_refused_even_without_manifest_declaration() {
    let (_dir, mut app) = fixture();
    app.project_ui = Some(crate::project_ui::Preview::install(
        orr_sample::project::PreparedUi {
            descriptor: orr_package::ProjectUi {
                profile: orr_package::ProjectUiProfile::ArenaKoreanV1,
                font: orr_package::ProjectFont {
                    package: "unused".into(),
                    asset: "font.ttf".into(),
                },
                document: None,
            },
            font: Vec::new(),
        },
        &egui::Context::default(),
    ));
    assert!(app
        .arena_export_dispatch()
        .unwrap_err()
        .contains("Loaded UI"));
    assert!(app.project_ui.is_some());
}
#[cfg(all(feature = "tilemap", feature = "project-create"))]
#[test]
fn loaded_tilemap_is_refused_independently_of_saved_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tiles");
    orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: "arena-tilemap-2d-v1".into(),
        seed: "arena-export-refusal".into(),
    })
    .unwrap();
    let prepared = crate::project::PreparedProject::open(&root).unwrap();
    let mut editor = Editor::start(&prepared.host_spec()).unwrap();
    editor.sync();
    let mut app = prepared
        .into_app(editor, None, &egui::Context::default())
        .unwrap();
    let path = root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["entry"].as_object_mut().unwrap().remove("tilemap");
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(app
        .arena_export_dispatch()
        .unwrap_err()
        .contains("Loaded tilemap"));
    assert!(app.tilemap.is_some());
}
#[test]
fn actual_escape_does_not_cancel_worker_or_bypass_next_close() {
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    let mut h = egui_kittest::Harness::builder()
        .with_size([1500.0, 1000.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| app);
    h.run_steps(2);
    let tab = h.state().ui.bottom_tab;
    h.key_press(egui::Key::Escape);
    h.run_steps(1);
    assert!(h.state().arena_export.busy());
    assert_eq!(h.state().ui.bottom_tab, tab);
    assert!(crate::app::unsaved_close_tests::request_root_close(&mut h));
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().arena_export.busy() {
        assert!(Instant::now() < deadline);
        h.run_steps(1);
        thread::sleep(Duration::from_millis(1));
    }
}
#[cfg(feature = "mesh-mission")]
#[test]
fn active_arena_job_refuses_mesh_creation_replacement() {
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    let path = dir.path().join("new-mesh.scene.yaml");
    assert!(!app.create_mesh_game(&path, &crate::mesh_workspace::mission::Settings::default()));
    assert!(!path.exists());
    assert_eq!(app.editor.game(), EditorGame::Arena);
    assert!(app.arena_export.busy());
    release.send(()).unwrap();
    reap(&mut app);
}
#[cfg(feature = "package-manager")]
#[test]
fn actual_package_verification_worker_blocks_arena_export_without_cancellation() {
    let (dir, mut app) = fixture();
    app.package_manager.open = true;
    let root = dir.path().to_owned();
    let mut h = egui_kittest::Harness::builder()
        .with_size([1200.0, 1000.0])
        .build_ui_state(
            move |ui, state: &mut (EditorApp, bool)| {
                state.0.package_manager.show(ui.ctx(), Some(&root), None);
                if state.0.package_manager.busy() {
                    assert!(state
                        .0
                        .arena_export_dispatch()
                        .unwrap_err()
                        .contains("pending content"));
                    assert!(
                        state.0.package_manager.busy(),
                        "export does not cancel content worker"
                    );
                    state.1 = true;
                }
            },
            (app, false),
        );
    h.run_steps(2);
    h.get_by_label("Verify installed packages").click();
    h.run_steps(1);
    assert!(h.state().1, "real package worker dispatch must be observed");
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().0.package_manager.busy() {
        assert!(Instant::now() < deadline);
        h.run_steps(1);
        thread::sleep(Duration::from_millis(1));
    }
}
#[cfg(all(feature = "collect-ui", feature = "project-create"))]
#[test]
fn active_arena_job_retains_app_through_disabled_collect_replacement_menu() {
    use egui_kittest::kittest::NodeT;
    let (dir, mut app) = fixture();
    let release = synthetic_job(&mut app, dir.path());
    let mut h = egui_kittest::Harness::builder()
        .build_ui_state(|ui, app: &mut EditorApp| app.collect_project_menu(ui), app);
    h.run_steps(2);
    for label in ["New Collect project…", "Open Collect project…"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
        h.get_by_label(label).click();
        h.run_steps(1);
        assert!(!h.state().collect_project_dialog_open());
        assert_eq!(h.state().editor.game(), EditorGame::Arena);
        assert!(h.state().arena_export.busy());
    }
    release.send(()).unwrap();
    reap(h.state_mut());
}

#[cfg(feature = "project-ui")]
#[test]
fn installed_saved_ui_dispatches_and_mismatches_stop_before_worker_start() {
    let (dir, mut app) = fixture();
    options(&mut app, dir.path());
    let installer = orr_package::Project::open_for_install(
        dir.path(),
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap();
    installer
        .install(&[std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/game_ui_font")
            .canonicalize()
            .unwrap()])
        .unwrap();
    let manifest = dir.path().join("orr.project.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["entry"]["ui"] = serde_json::json!({"profile":"arena-korean-v1","font":{"package":"korean-game-ui","asset":"OrreryKoreanUI.otf"}});
    fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let prepared = crate::project::PreparedProject::open(dir.path()).unwrap();
    let ui = prepared.ui().unwrap().clone();
    let ctx = egui::Context::default();
    let no_spawn = |_: String, _: Box<dyn FnOnce() + Send>| -> std::io::Result<JoinHandle<()>> {
        panic!("mismatch reached worker spawn")
    };
    assert!(app.arena_export_dispatch_with(no_spawn).is_err());
    app.project_ui = Some(crate::project_ui::Preview::install(ui.clone(), &ctx));
    app.project_ui.as_mut().unwrap().descriptor.font.package = "wrong-package".into();
    assert!(app.arena_export_dispatch_with(no_spawn).is_err());
    app.project_ui = Some(crate::project_ui::Preview::install(ui.clone(), &ctx));
    app.project_ui.as_mut().unwrap().descriptor.font.asset = "wrong.otf".into();
    assert!(app.arena_export_dispatch_with(no_spawn).is_err());
    app.project_ui = Some(crate::project_ui::Preview::install(ui.clone(), &ctx));
    let error = app
        .arena_export_dispatch_with(|_, _| Err(std::io::Error::other("valid UI reached spawn")))
        .unwrap_err();
    assert!(error.contains("valid UI reached spawn"));
    assert!(!app.arena_export.busy());
    assert_eq!(app.arena_export.next_id, 0);
    assert!(app.ui.dialog.is_none());
    value["entry"].as_object_mut().unwrap().remove("ui");
    fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(app.arena_export_dispatch_with(no_spawn).is_err());
}
