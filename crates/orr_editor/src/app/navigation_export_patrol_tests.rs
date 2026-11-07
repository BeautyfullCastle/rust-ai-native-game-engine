//! CPU egui export acceptance for an already-authored saved patrol. Authoring
//! widgets are covered separately; no GPU, native window or OS input is used.
use super::*;
use crate::{editor::Editor, game::EditorGame};
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_remote::navigation_yard3d::{self as remote, patrol};
use std::{
    fs,
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
    let mut document = remote::navigation_yard3d_doc_from_path(&scene).unwrap();
    let view = remote::navigation_from_view(orr_bridge::FrameView::of(document.frame())).unwrap();
    let mut points = [[orr_fp::FP::ZERO; 2]; 8];
    points[0] = view.pin.agent.goal;
    points[1] = view.pin.agent.start;
    let config = patrol::TerrainPatrol {
        enabled: 1,
        count: 2,
        looped: 1,
        wait_ticks: 2,
        points,
        graph_revision: view.pin.graph_revision,
    };
    patrol::set_patrol(&mut document, &config).unwrap();
    fs::write(&scene, document.to_yaml()).unwrap();
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
    assert_eq!(
        patrol::from_view(editor.snapshot().unwrap().predicted())
            .unwrap()
            .unwrap()
            .config,
        config
    );
    let mut app = EditorApp::new(editor, None);
    app.terrain.local_path = "navigation/point_demo.orrt".into();
    app.terrain.open_for_editor(&app.editor).unwrap();
    app.navigation.sync_from_editor(&app.editor);
    app.navigation.observe_terrain(&app.editor, &app.terrain);
    let captured = app
        .editor
        .navigation_export_source(&app.navigation, &app.terrain)
        .unwrap();
    assert_eq!(captured.initial_checksum(), app.editor.checksum());
    (dir, app)
}
fn harness(app: EditorApp) -> Harness<'static, EditorApp> {
    Harness::builder().with_size([1500., 1200.]).build_ui_state(
        |ui, app: &mut EditorApp| app.show_navigation_export(ui),
        app,
    )
}
fn text(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(2);
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, label)
            .accesskit_node()
            .value()
            .as_deref(),
        Some(value)
    );
}
fn reap(h: &mut Harness<'_, EditorApp>) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while h.state().navigation_export.busy() {
        assert!(Instant::now() < deadline, "worker deadline");
        h.state_mut().navigation_export.poll();
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn actual_patrol_export_widgets_require_trust_and_report_failed_worker_without_mutation() {
    let (dir, mut app) = fixture();
    let scene = dir.path().join("source/point.scene.yaml");
    let saved = fs::read(&scene).unwrap();
    let frame = app
        .editor
        .navigation_export_source(&app.navigation, &app.terrain)
        .unwrap()
        .initial_frame_bytes()
        .to_vec();
    let history = app.editor.history().clone();
    let mut h = harness(app);
    h.run_steps(2);
    text(
        &mut h,
        "Runtime path",
        &dir.path().join("missing-runtime").display().to_string(),
    );
    text(&mut h, "Expected lowercase SHA256", &"a".repeat(64));
    text(
        &mut h,
        "New output path",
        &dir.path().join("bundle").display().to_string(),
    );
    h.get_by_label("Export saved Navigation").click_accesskit();
    h.run_steps(2);
    assert!(
        h.state()
            .navigation_export
            .status
            .contains("Explicitly trust")
    );
    assert!(!h.state().navigation_export.busy());
    assert_eq!(h.state().navigation_export.next_id, 0);
    h.get_by_label("I trust this supplied runtime and allow smoke verification to execute it")
        .click_accesskit();
    h.run_steps(2);
    h.get_by_label("Export saved Navigation").click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().navigation_export.next_id, 1);
    reap(&mut h);
    assert!(!h.state().navigation_export.completed_successfully);
    assert!(h.state().navigation_export.uncertain_output.is_some());
    assert!(!dir.path().join("bundle").exists());
    let after = {
        let app = h.state_mut();
        app.editor
            .navigation_export_source(&app.navigation, &app.terrain)
            .unwrap()
            .initial_frame_bytes()
            .to_vec()
    };
    assert_eq!(after, frame);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(scene).unwrap(), saved);
}
#[test]
#[ignore = "requires ORR_TERRAIN_PATROL_EXPORT_RUNTIME; executes explicitly supplied trusted runtime"]
fn actual_patrol_export_button_delivers_saved_profile_and_native_checksum() {
    use orr_navigation_view::authored::AuthoredPlayer;
    use sha2::{Digest, Sha256};
    let runtime = PathBuf::from(
        std::env::var_os("ORR_TERRAIN_PATROL_EXPORT_RUNTIME")
            .expect("explicit trusted patrol runtime required"),
    );
    assert!(runtime.is_absolute());
    let (dir, app) = fixture();
    let scene = dir.path().join("source/point.scene.yaml");
    let saved = fs::read(&scene).unwrap();
    let history = app.editor.history().clone();
    let mut expected = AuthoredPlayer::open(&scene).unwrap();
    for _ in 0..8 {
        expected.step().unwrap();
    }
    let hash = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mut h = harness(app);
    h.run_steps(2);
    text(&mut h, "Runtime path", &runtime.display().to_string());
    text(&mut h, "Expected lowercase SHA256", &hash);
    text(
        &mut h,
        "New output path",
        &dir.path().join("bundle").display().to_string(),
    );
    h.get_by_label("I trust this supplied runtime and allow smoke verification to execute it")
        .click_accesskit();
    h.run_steps(2);
    h.get_by_label("Export saved Navigation").click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().navigation_export.next_id, 1);
    reap(&mut h);
    assert!(
        h.state().navigation_export.completed_successfully,
        "{}",
        h.state().navigation_export.status
    );
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&scene).unwrap(), saved);
    drop(h);
    let output = std::process::Command::new(dir.path().join("bundle/run-navigation"))
        .args(["--ticks", "8", "--restart-check"])
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
    assert_eq!(receipt["profile"], "NavigationYard3DPatrolV1");
    assert_eq!(receipt["patrol"]["count"], 2);
    assert_eq!(receipt["patrol"]["looped"], 1);
    assert_eq!(receipt["patrol"]["wait_ticks"], 2);
    assert_eq!(receipt["checksum"].as_u64(), Some(expected.checksum()));
    assert_eq!(receipt["restart_checked"], true);
    assert!(receipt["capture"].is_null());
}
