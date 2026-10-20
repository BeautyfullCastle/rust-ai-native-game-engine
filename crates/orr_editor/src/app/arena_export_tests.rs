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
    let source = app.editor.arena_export_source(root, &app.sprites).unwrap();
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
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.with_close_review(ui, |_, _| {}));
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
fn panic_and_disconnect_are_terminal_but_require_destination_inspection() {
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
fn screenshot_mode_and_path_dialog_refuse_without_source_mutation() {
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
