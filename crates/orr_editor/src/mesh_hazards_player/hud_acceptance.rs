#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
//! Production Editor/Player widget acceptance and relocated executable evidence.
//! Software-GL pixels and headless native processes are separate observations;
//! neither establishes native-window, physical-input, or hardware-GPU acceptance.
use super::{acceptance::*, start_playback, PlayerApp};
use crate::{game::EditorGame, Editor, EditorApp};
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::wgpu;
use orr_remote::mesh_navigation::mesh_from_view;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn editor_harness(editor: Editor, gpu: bool) -> Harness<'static, EditorApp> {
    let builder = Harness::builder()
        .with_size([960.0, 600.0])
        .with_step_dt(1.0 / 60.0);
    if gpu {
        builder
            .wgpu_setup(software_gpu_setup(wgpu::Backend::Gl).into())
            .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()))
    } else {
        builder
            .renderer(egui_kittest::LazyRenderer::Uninitialized {
                textures_delta: Default::default(),
                builder: None,
            })
            .build_eframe(move |_| EditorApp::new(editor, None))
    }
}

fn player_harness(
    mut editor: Editor,
    hud: crate::mesh_hud_panel::Panel,
    gpu: bool,
) -> Harness<'static, PlayerApp> {
    let mut navigation = crate::mesh_navigation::Panel::default();
    navigation.sync(&editor);
    start_playback(&mut editor, Some(0), false).unwrap();
    let builder = Harness::builder()
        .with_size([960.0, 600.0])
        .with_step_dt(1.0 / 60.0);
    let create = move |cc: &mut eframe::CreationContext<'_>| {
        let mut inner = EditorApp::new(editor, cc.wgpu_render_state.clone());
        inner.install_validated_hazards_navigation(navigation);
        inner.install_validated_hazards_hud(hud);
        inner.set_mesh_player_input_allowed(true);
        PlayerApp {
            inner,
            session: Default::default(),
        }
    };
    if gpu {
        builder
            .wgpu_setup(software_gpu_setup(wgpu::Backend::Gl).into())
            .build_eframe(create)
    } else {
        builder
            .renderer(egui_kittest::LazyRenderer::Uninitialized {
                textures_delta: Default::default(),
                builder: None,
            })
            .build_eframe(create)
    }
}

fn position(editor: &Editor) -> [i64; 3] {
    mesh_from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .agent
        .position
        .map(|p| p.raw())
}

fn applied(app: &EditorApp) -> Option<crate::mesh_hud::Document> {
    app.hazard_hud_panel().applied_document(&app.editor)
}

fn click_control(h: &mut Harness<'_, EditorApp>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).scroll_to_me();
    h.run_steps(3);
    click_editor(h, role, label);
}

fn open_export(h: &mut Harness<'_, EditorApp>) {
    for label in ["File", "Export hazard game…"] {
        h.get_by_label(label).click_accesskit();
        h.step();
        settle_editor(h);
    }
}

fn wait_export(h: &mut Harness<'_, EditorApp>, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        settle_editor(h);
        if h.query_by_label_contains(expected).is_some() {
            return;
        }
        if expected.starts_with("Hazard export complete") {
            assert!(
                h.query_by_label_contains("Hazard export failed:").is_none(),
                "production export worker rejected the HUD bundle"
            );
        }
        assert!(Instant::now() < deadline, "export UI timed out: {expected}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn native_report(runtime: &Path, ticks: u32, root: &Path, name: &str) -> serde_json::Value {
    let stdout = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(format!("{name}.json")))
        .unwrap();
    let stderr = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(format!("{name}.stderr")))
        .unwrap();
    // Omit --bundle: delivery must discover siblings beside the relocated binary.
    let mut child = Command::new(runtime)
        .arg("--verify-headless")
        .arg("--ticks")
        .arg(ticks.to_string())
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("relocated native verification exceeded 30 seconds");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        status.success(),
        "native verification failed: {}",
        fs::read_to_string(root.join(format!("{name}.stderr"))).unwrap()
    );
    serde_json::from_slice(&fs::read(root.join(format!("{name}.json"))).unwrap()).unwrap()
}

#[test]
fn hazards_hud_widget_apply_undo_redo_save_reopen_export_restart() {
    scenario(false, false);
}

#[test]
#[ignore = "requires independently built runtime, SHA-256 and fresh evidence directory"]
fn hazards_hud_relocated_native_headless_evidence() {
    scenario(false, true);
}

#[test]
#[ignore = "requires software GL CPU adapter, independently built runtime and fresh evidence directory"]
fn hazards_hud_relocated_software_gl_evidence() {
    scenario(true, true);
}

fn assert_visible_hud(h: &Harness<'_, PlayerApp>, document: &crate::mesh_hud::Document) {
    let snapshot = h.state().inner.editor.snapshot().unwrap();
    let view = mesh_from_view(snapshot.predicted()).unwrap();
    let hazard_status = crate::mesh_navigation::hazard_status(&h.state().inner.editor)
        .unwrap()
        .unwrap();
    assert!(
        h.query_by_label(&hazard_status).is_some(),
        "authoritative hazard status stays visible"
    );
    assert!(h.query_by_label(&document.title).is_some());
    assert!(h.query_by_label(document.status_text(&view)).is_some());
    assert!(h
        .query_by_label(&format!("Tick {}", h.state().inner.editor.sim().head_tick))
        .is_some());
}

fn scenario(gpu: bool, native: bool) {
    let native_evidence = native.then(|| env_path("ORR_HAZARDS_HUD_EVIDENCE"));
    let temp = if let Some(root) = &native_evidence {
        tempfile::tempdir_in(root.parent().expect("evidence directory has a parent")).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    };
    let evidence = native_evidence.unwrap_or_else(|| temp.path().join("evidence"));
    fs::create_dir(&evidence).expect("evidence directory must be fresh");
    let scene = temp.path().join("authored.scene.yaml");
    let editor = crate::mesh_hazards_creator::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
        true,
    )
    .unwrap();
    let scene_bytes = fs::read(&scene).unwrap();
    let mut h = editor_harness(editor, gpu);
    settle_editor(&mut h);
    let source_checksum = h.state().editor.checksum();
    let source_position = position(&h.state().editor);
    assert_eq!(applied(h.state()), None);
    if h.query_by_label("HUD title").is_none() {
        h.get_by_label("Mesh game HUD").click_accesskit();
        h.step();
        settle_editor(&mut h);
    }
    for (label, value) in [
        ("HUD title", "Hazard expedition"),
        ("HUD idle text", "Choose a safe route"),
        ("HUD moving text", "Crossing the hazard field"),
        ("HUD arrived text", "Route endpoint reached"),
        ("HUD font size", "24"),
        ("HUD background color R", "90"),
        ("HUD background color A", "255"),
    ] {
        type_editor_field(&mut h, label, value);
    }
    assert_eq!(applied(h.state()), None, "draft cannot apply HUD");
    assert!(h.state().hazard_hud_panel().dirty());
    assert!(h
        .state()
        .hazard_hud_panel()
        .play_blocker(&h.state().editor)
        .is_some());
    click_control(&mut h, Role::Button, "Apply game HUD");
    let authored = applied(h.state()).unwrap();
    assert_eq!(authored.title, "Hazard expedition");
    assert_eq!(authored.moving_text, "Crossing the hazard field");
    assert_eq!(authored.font_size, 24.0);
    assert!(authored.enabled && authored.show_tick);
    click_control(&mut h, Role::Button, "Undo HUD edit");
    assert_eq!(applied(h.state()), None);
    click_control(&mut h, Role::Button, "Redo HUD edit");
    assert_eq!(applied(h.state()), Some(authored.clone()));
    click_control(&mut h, Role::Button, "Save game HUD");
    assert!(!h.state().hazard_hud_panel().dirty());
    assert!(h
        .state()
        .hazard_hud_panel()
        .play_blocker(&h.state().editor)
        .is_none());
    assert_eq!(h.state().editor.checksum(), source_checksum);
    assert_eq!(position(&h.state().editor), source_position);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    let sidecar = crate::mesh_hud_store::sidecar_path(&scene).unwrap();
    let saved_sidecar = fs::read(&sidecar).unwrap();
    if gpu {
        capture_frame(&mut h, &evidence, "01-editor-saved-hud.png");
    }
    drop(h);

    let mut fresh = editor_harness(
        Editor::open_game(&scene, EditorGame::MeshHazardsV1).unwrap(),
        gpu,
    );
    settle_editor(&mut fresh);
    assert_eq!(applied(fresh.state()), Some(authored.clone()));
    assert!(!fresh.state().hazard_hud_panel().dirty());
    assert_eq!(fresh.state().editor.checksum(), source_checksum);
    if gpu {
        capture_frame(&mut fresh, &evidence, "02-editor-fresh-reopen.png");
    }
    let platform = if cfg!(windows) {
        crate::mesh_hazards_export::RuntimePlatform::WindowsX86_64
    } else {
        crate::mesh_hazards_export::RuntimePlatform::LinuxX86_64
    };
    let source_runtime = temp.path().join(platform.runtime_filename());
    let runtime_sha = if native {
        let source = env_path("ORR_HAZARDS_HUD_RUNTIME");
        let expected = std::env::var("ORR_HAZARDS_HUD_EXPECTED_RUNTIME_SHA256")
            .expect("set independently built runtime SHA-256");
        assert_eq!(super::sha256_file_bounded(&source).unwrap(), expected);
        fs::copy(source, &source_runtime).unwrap();
        assert_eq!(
            super::sha256_file_bounded(&source_runtime).unwrap(),
            expected
        );
        expected
    } else {
        fs::write(
            &source_runtime,
            crate::mesh_hazards_export::test_hud_runtime(platform, false, false, false),
        )
        .unwrap();
        super::sha256_file_bounded(&source_runtime).unwrap()
    };
    let staging = temp.path().join("export-staging");
    open_export(&mut fresh);
    type_editor_field(
        &mut fresh,
        "Hazard player executable",
        source_runtime.to_str().unwrap(),
    );
    type_editor_field(
        &mut fresh,
        "New output directory (must not exist)",
        staging.to_str().unwrap(),
    );
    assert!(fresh
        .get_by_role_and_label(Role::Button, "Export hazard game")
        .accesskit_node()
        .is_disabled());
    click_control(
        &mut fresh,
        Role::CheckBox,
        "I trust this self-built, QA-tested hazard player",
    );
    click_control(&mut fresh, Role::Button, "Export hazard game");
    wait_export(&mut fresh, "Hazard export complete at");
    assert!(fresh
        .query_by_label_contains("Player was not launched.")
        .is_some());
    let staged_before = inventory(&staging);
    assert_eq!(staged_before.len(), 4);
    if gpu {
        capture_frame(&mut fresh, &evidence, "03-editor-export-complete.png");
    }
    click_control(&mut fresh, Role::Button, "Export hazard game");
    wait_export(&mut fresh, "Hazard export failed:");
    assert_eq!(inventory(&staging), staged_before);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&sidecar).unwrap(), saved_sidecar);
    drop(fresh);

    let relocated = evidence.join("relocated-bundle");
    fs::rename(&staging, &relocated).unwrap();
    fs::remove_file(&scene).unwrap();
    fs::remove_file(&sidecar).unwrap();
    fs::remove_file(&source_runtime).unwrap();
    assert!(!scene.exists() && !sidecar.exists() && !source_runtime.exists() && !staging.exists());
    let bundle = crate::mesh_hazards_export::open_bundle(&relocated).unwrap();
    assert_eq!(bundle.runtime_sha256, runtime_sha);
    assert!(bundle.hud_sha256.is_some());
    assert_eq!(bundle.camera_sha256, None);
    assert_eq!(bundle.art_sha256, None);
    assert_eq!(bundle.lighting_sha256, None);
    let before = inventory(&relocated);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(relocated.join(crate::mesh_hazards_export::MANIFEST_FILENAME)).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["schema"], 5);
    assert!(manifest["hud"].is_object());
    let delivered_sidecar = crate::mesh_hud_store::sidecar_path(&bundle.scene).unwrap();
    let delivered_bytes = fs::read(&delivered_sidecar).unwrap();
    let mut reopened = Editor::open_game(&bundle.scene, EditorGame::MeshHazardsV1).unwrap();
    reopened.sync();
    let mut hud = crate::mesh_hud_panel::Panel::default();
    hud.sync(&reopened);
    super::verify_loaded_hud_bundle(&relocated, &bundle, &hud).unwrap();
    assert_eq!(hud.applied_document(&reopened), Some(authored.clone()));
    start_playback(&mut reopened, Some(50), false).unwrap();
    let played_checksum = reopened.snapshot().unwrap().predicted().checksum();
    let moved_position = position(&reopened);
    assert_ne!(moved_position, source_position);
    assert!(reopened.stop().is_some());
    reopened.sync();
    assert_eq!(reopened.checksum(), source_checksum);
    let mut player = player_harness(reopened, hud, gpu);
    settle(&mut player);
    assert_eq!(applied(&player.state().inner), Some(authored.clone()));
    if gpu {
        assert_visible_hud(&player, &authored);
    }
    if gpu {
        capture_frame(&mut player, &evidence, "04-relocated-player-hud.png");
    }
    click(&mut player, "Resume");
    wait_for(&mut player, "HUD Player simulation advances", |app| {
        app.inner.editor.sim().head_tick > 0
    });
    click(&mut player, "Pause");
    if gpu {
        assert_visible_hud(&player, &authored);
    }
    click(&mut player, "Restart game…");
    click(&mut player, "Restart game");
    crate::app::mesh_hazards_player::validate_fresh_session(&player.state().inner.editor).unwrap();
    assert_eq!(applied(&player.state().inner), Some(authored.clone()));
    assert_eq!(position(&player.state().inner.editor), source_position);
    assert_eq!(
        player
            .state()
            .inner
            .editor
            .snapshot()
            .unwrap()
            .predicted()
            .checksum(),
        source_checksum
    );
    if gpu {
        assert_visible_hud(&player, &authored);
    }
    if gpu {
        capture_frame(&mut player, &evidence, "05-player-restarted-hud.png");
    }

    // Disabled is a saved document, not an absent sidecar. Only presentation changes.
    let control_scene = temp.path().join("disabled-hud.scene.yaml");
    fs::copy(&bundle.scene, &control_scene).unwrap();
    let mut control = Editor::open_game(&control_scene, EditorGame::MeshHazardsV1).unwrap();
    control.sync();
    let graph_revision = mesh_from_view(control.snapshot().unwrap().predicted())
        .unwrap()
        .graph
        .revision();
    let disabled = crate::mesh_hud::Document {
        enabled: false,
        ..authored.clone()
    };
    let disabled_digest =
        crate::mesh_hud_store::save(&control_scene, Some(&disabled), None, graph_revision).unwrap();
    let mut no_hud = crate::mesh_hud_panel::Panel::default();
    no_hud.sync(&control);
    assert!(no_hud.matches_saved_sidecar(disabled_digest.as_deref()));
    assert_eq!(no_hud.applied_document(&control), Some(disabled.clone()));
    start_playback(&mut control, Some(50), false).unwrap();
    assert_eq!(
        control.snapshot().unwrap().predicted().checksum(),
        played_checksum
    );
    assert_eq!(position(&control), moved_position);
    assert!(control.stop().is_some());
    control.sync();
    let mut control = player_harness(control, no_hud, gpu);
    settle(&mut control);
    assert_eq!(applied(&control.state().inner), Some(disabled.clone()));
    assert!(control.query_by_label(&authored.title).is_none());
    assert!(control.query_by_label(&authored.moving_text).is_none());
    assert!(control.query_by_label("Tick 0").is_none());
    click(&mut control, "Restart game…");
    click(&mut control, "Restart game");
    assert_eq!(applied(&control.state().inner), Some(disabled));
    assert_eq!(
        control
            .state()
            .inner
            .editor
            .snapshot()
            .unwrap()
            .predicted()
            .checksum(),
        source_checksum
    );
    assert!(control.query_by_label(&authored.title).is_none());
    let changed_pixels = if gpu {
        capture_frame(&mut control, &evidence, "06-player-disabled-hud.png");
        let a = player.render().unwrap();
        let b = control.render().unwrap();
        assert_eq!(a.dimensions(), b.dimensions());
        let changed = a
            .as_raw()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(b.as_raw().as_chunks::<4>().0.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed > 100,
            "enabled HUD must visibly alter actual Player composite pixels"
        );
        Some(changed)
    } else {
        None
    };

    let native_report = if native {
        let first = native_report(&bundle.runtime, 50, &evidence, "native-first");
        let second = native_report(&bundle.runtime, 50, &evidence, "native-repeat");
        assert_eq!(first, second);
        assert_eq!(first["tick"], 50);
        assert_eq!(first["checksum"], format!("{played_checksum:016x}"));
        assert_eq!(
            first["restarted_checksum"],
            format!("{source_checksum:016x}")
        );
        assert_eq!(
            first["agent_position_raw"],
            serde_json::json!(moved_position)
        );
        assert_eq!(first["hud_sha256"], serde_json::json!(bundle.hud_sha256));
        assert_eq!(first["restart_parity"], true);
        assert_eq!(first["hud_restart_parity"], true);
        assert_eq!(first["restarted_tick"], 0);
        Some(first)
    } else {
        None
    };
    assert_eq!(inventory(&relocated), before);
    // A loaded panel must not silently admit a sidecar changed after bundle validation.
    let mut stale_editor = Editor::open_game(&bundle.scene, EditorGame::MeshHazardsV1).unwrap();
    stale_editor.sync();
    let mut stale_panel = crate::mesh_hud_panel::Panel::default();
    stale_panel.sync(&stale_editor);
    let mut tampered: serde_json::Value = serde_json::from_slice(&delivered_bytes).unwrap();
    tampered["hud"]["title"] = serde_json::json!("Unexpected replacement");
    fs::write(&delivered_sidecar, serde_json::to_vec(&tampered).unwrap()).unwrap();
    assert!(crate::mesh_hazards_export::open_bundle(&relocated).is_err());
    assert!(super::verify_loaded_hud_bundle(&relocated, &bundle, &stale_panel).is_err());
    fs::write(&delivered_sidecar, &delivered_bytes).unwrap();
    assert_eq!(inventory(&relocated), before);
    let report = serde_json::json!({
        "schema": 1, "runtime_fixture": !native, "standalone_process_executed": native,
        "standalone_process_gpu": false, "native_window": false, "physical_input": false,
        "offscreen_player_software_gl": gpu, "hardware_gpu": false,
        "editor_widget_apply_undo_redo_save": true, "fresh_editor_reopen": true,
        "production_export_ui_job": true, "repeated_export_refuses_overwrite": true,
        "original_scene_sidecar_copied_runtime_removed": true, "bundle_hashes_unchanged": before,
        "runtime_sha256": runtime_sha, "hud_sha256": bundle.hud_sha256,
        "source_checksum": source_checksum, "enabled_disabled_50_tick_checksum": played_checksum,
        "player_hud_title_status_tick_visible": gpu, "player_restart_keeps_hud": true,
        "saved_disabled_hud_retained_but_hidden": true, "stale_tampered_bundle_refused": true,
        "changed_composite_pixels": changed_pixels, "native_report": native_report,
    });
    fs::write(
        evidence.join("receipt.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
