#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
//! Production Editor/Player widget acceptance and relocated executable evidence.
//! Software-GL pixels and headless native processes are separate observations;
//! neither establishes native-window, physical-input, or hardware-GPU acceptance.
use super::{PlayerApp, acceptance::*, start_playback};
use crate::{Editor, EditorApp, game::EditorGame};
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
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
    lighting: crate::mesh_lighting_panel::Panel,
    gpu: bool,
    without_hdr: bool,
) -> Harness<'static, PlayerApp> {
    let mut navigation = crate::mesh_navigation::Panel::default();
    navigation.sync(&editor);
    start_playback(&mut editor, Some(0), false).unwrap();
    let builder = Harness::builder()
        .with_size([960.0, 600.0])
        .with_step_dt(1.0 / 60.0);
    let create = move |cc: &mut eframe::CreationContext<'_>| {
        let mut inner = EditorApp::new(editor, cc.wgpu_render_state.clone());
        if without_hdr {
            inner = inner.without_hdr_support();
        }
        inner.install_validated_hazards_navigation(navigation);
        inner.install_validated_hazards_lighting(lighting);
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

fn applied(app: &EditorApp) -> Option<orr_render::Lighting> {
    app.hazard_lighting_panel().applied_lighting(&app.editor)
}

fn effects(app: &EditorApp) -> Option<orr_render::PostProcessSettings> {
    app.hazard_lighting_panel()
        .applied_post_process(&app.editor)
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
                "production export worker rejected the lighting bundle"
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
fn hazards_lighting_widget_apply_undo_redo_save_reopen_export_restart() {
    scenario(false, false);
}

#[test]
#[ignore = "requires independently built runtime, SHA-256 and fresh evidence directory"]
fn hazards_lighting_relocated_native_headless_evidence() {
    scenario(false, true);
}

#[test]
#[ignore = "requires software GL CPU adapter, independently built runtime and fresh evidence directory"]
fn hazards_lighting_relocated_software_gl_evidence() {
    scenario(true, true);
}

fn scenario(gpu: bool, native: bool) {
    let native_evidence = native.then(|| env_path("ORR_HAZARDS_LIGHTING_EVIDENCE"));
    // Keep relocation an atomic rename even when /tmp is on another filesystem.
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
    assert_eq!(effects(h.state()), None);
    if h.query_by_label("Light intensity").is_none() {
        h.get_by_label("Mesh game lighting").click_accesskit();
        h.step();
        settle_editor(&mut h);
    }
    for (label, value) in [
        ("Light intensity", "4"),
        ("Light color R", "1"),
        ("Light color G", "0.2"),
        ("Light color B", "0.1"),
        ("Exposure", "0.7"),
    ] {
        type_editor_field(&mut h, label, value);
    }
    click_control(&mut h, Role::CheckBox, "Saved HDR post-processing");
    assert_eq!(applied(h.state()), None, "draft must not apply lighting");
    assert!(h.state().hazard_lighting_panel().dirty());
    assert!(
        h.state()
            .hazard_lighting_panel()
            .play_blocker(&h.state().editor)
            .is_some()
    );
    click_control(&mut h, Role::Button, "Apply game lighting");
    let authored = applied(h.state()).unwrap();
    let authored_effects = effects(h.state()).unwrap();
    assert_eq!(authored.intensity, 4.0);
    assert_eq!(authored.color, [1.0, 0.2, 0.1]);
    assert_eq!(authored.exposure, 0.7);
    assert!(authored_effects.enabled);
    assert!(h.state().hazard_lighting_panel().dirty());
    click_control(&mut h, Role::Button, "Undo game lighting");
    assert_eq!(applied(h.state()), None);
    assert_eq!(effects(h.state()), None);
    click_control(&mut h, Role::Button, "Redo game lighting");
    assert_eq!(applied(h.state()), Some(authored));
    assert_eq!(effects(h.state()), Some(authored_effects));
    click_control(&mut h, Role::Button, "Save game lighting");
    assert!(!h.state().hazard_lighting_panel().dirty());
    assert!(
        h.state()
            .hazard_lighting_panel()
            .play_blocker(&h.state().editor)
            .is_none()
    );
    assert_eq!(h.state().editor.checksum(), source_checksum);
    assert_eq!(position(&h.state().editor), source_position);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    let sidecar = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
    let saved_sidecar = fs::read(&sidecar).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&saved_sidecar).unwrap();
    assert_eq!(
        saved["schema"], 2,
        "post-processing must be explicitly authored"
    );
    if gpu {
        capture_frame(&mut h, &evidence, "01-editor-saved-lighting.png");
    }
    drop(h);

    let mut fresh = editor_harness(
        Editor::open_game(&scene, EditorGame::MeshHazardsV1).unwrap(),
        gpu,
    );
    settle_editor(&mut fresh);
    assert_eq!(applied(fresh.state()), Some(authored));
    assert_eq!(effects(fresh.state()), Some(authored_effects));
    assert!(!fresh.state().hazard_lighting_panel().dirty());
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
        let source = env_path("ORR_HAZARDS_LIGHTING_RUNTIME");
        let expected = std::env::var("ORR_HAZARDS_LIGHTING_EXPECTED_RUNTIME_SHA256")
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
            crate::mesh_hazards_export::test_lighting_runtime(platform, false, false),
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
    assert!(
        fresh
            .get_by_role_and_label(Role::Button, "Export hazard game")
            .accesskit_node()
            .is_disabled()
    );
    click_control(
        &mut fresh,
        Role::CheckBox,
        "I trust this self-built, QA-tested hazard player",
    );
    click_control(&mut fresh, Role::Button, "Export hazard game");
    wait_export(&mut fresh, "Hazard export complete at");
    assert!(
        fresh
            .query_by_label_contains("Player was not launched.")
            .is_some()
    );
    let staged_before = inventory(&staging);
    assert_eq!(staged_before.len(), 4);
    if gpu {
        capture_frame(&mut fresh, &evidence, "03-editor-export-complete.png");
    }
    // Repeated actual UI dispatch refuses overwrite and preserves the first result.
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
    assert!(bundle.lighting_sha256.is_some());
    assert_eq!(bundle.camera_sha256, None);
    assert_eq!(bundle.art_sha256, None);
    let before = inventory(&relocated);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(relocated.join(crate::mesh_hazards_export::MANIFEST_FILENAME)).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["schema"], 4);
    assert!(manifest["lighting"].is_object());
    let delivered_sidecar = crate::mesh_lighting_store::sidecar_path(&bundle.scene).unwrap();
    let delivered_bytes = fs::read(&delivered_sidecar).unwrap();
    let mut reopened = Editor::open_game(&bundle.scene, EditorGame::MeshHazardsV1).unwrap();
    reopened.sync();
    let mut lighting = crate::mesh_lighting_panel::Panel::default();
    lighting.sync(&reopened);
    super::verify_loaded_lighting_bundle(&relocated, &bundle, &lighting).unwrap();
    assert_eq!(lighting.applied_lighting(&reopened), Some(authored));
    assert_eq!(
        lighting.applied_post_process(&reopened),
        Some(authored_effects)
    );
    start_playback(&mut reopened, Some(50), false).unwrap();
    let played_checksum = reopened.snapshot().unwrap().predicted().checksum();
    let moved_position = position(&reopened);
    assert_ne!(moved_position, source_position);
    assert!(reopened.stop().is_some());
    reopened.sync();
    assert_eq!(reopened.checksum(), source_checksum);
    let mut player = player_harness(reopened, lighting, gpu, false);
    settle(&mut player);
    assert_eq!(applied(&player.state().inner), Some(authored));
    assert_eq!(effects(&player.state().inner), Some(authored_effects));
    if gpu {
        capture_frame(&mut player, &evidence, "04-relocated-player-saved-hdr.png");
    }
    click(&mut player, "Resume");
    wait_for(&mut player, "lighting Player simulation advances", |app| {
        app.inner.editor.sim().head_tick > 0
    });
    click(&mut player, "Pause");
    click(&mut player, "Restart game…");
    click(&mut player, "Restart game");
    crate::app::mesh_hazards_player::validate_fresh_session(&player.state().inner.editor).unwrap();
    assert_eq!(applied(&player.state().inner), Some(authored));
    assert_eq!(effects(&player.state().inner), Some(authored_effects));
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
        capture_frame(&mut player, &evidence, "05-player-restarted-lighting.png");
    }

    // Literal no-lighting control has identical simulation bytes but no sidecar.
    let control_scene = temp.path().join("literal-no-lighting.scene.yaml");
    fs::copy(&bundle.scene, &control_scene).unwrap();
    let mut control = Editor::open_game(&control_scene, EditorGame::MeshHazardsV1).unwrap();
    control.sync();
    let mut no_lighting = crate::mesh_lighting_panel::Panel::default();
    no_lighting.sync(&control);
    assert_eq!(no_lighting.applied_lighting(&control), None);
    assert_eq!(no_lighting.applied_post_process(&control), None);
    start_playback(&mut control, Some(50), false).unwrap();
    assert_eq!(control.sim().head_tick, 50);
    assert_eq!(
        control.snapshot().unwrap().predicted().checksum(),
        played_checksum
    );
    assert_eq!(position(&control), moved_position);
    assert!(control.stop().is_some());
    control.sync();
    let mut control = player_harness(control, no_lighting, gpu, false);
    settle(&mut control);
    click(&mut control, "Restart game…");
    click(&mut control, "Restart game");
    crate::app::mesh_hazards_player::validate_fresh_session(&control.state().inner.editor).unwrap();
    assert_eq!(position(&control.state().inner.editor), source_position);
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
    assert_eq!(applied(&control.state().inner), None);

    let gpu_evidence = if gpu {
        capture_frame(&mut control, &evidence, "06-player-literal-no-lighting.png");
        let actual_gpu = player.state().inner.viewport3d_gpu().unwrap().gpu();
        let actual_hdr = actual_gpu.post_process_settings().enabled;
        let actual_warning = player.state().inner.hazard_lighting_warning();
        if actual_hdr {
            assert_eq!(actual_gpu.post_process_settings(), authored_effects);
            assert!(actual_gpu.post_process_allocated_bytes() > 0);
            assert!(actual_gpu.post_process_size().is_some());
            assert_eq!(actual_warning, None);
        } else {
            assert!(
                actual_warning.is_some(),
                "unsupported saved HDR must be explicitly reported"
            );
        }
        let actual_warning = actual_warning.map(str::to_owned);

        // Force the renderer's documented unsupported-HDR route. Comparing its
        // raw viewport against no-lighting isolates lighting from post-processing
        // and excludes the warning text drawn by egui over the viewport.
        let mut fallback_editor =
            Editor::open_game(&bundle.scene, EditorGame::MeshHazardsV1).unwrap();
        fallback_editor.sync();
        let mut fallback_lighting = crate::mesh_lighting_panel::Panel::default();
        fallback_lighting.sync(&fallback_editor);
        super::verify_loaded_lighting_bundle(&relocated, &bundle, &fallback_lighting).unwrap();
        let mut fallback = player_harness(fallback_editor, fallback_lighting, true, true);
        settle(&mut fallback);
        assert_eq!(applied(&fallback.state().inner), Some(authored));
        assert_eq!(effects(&fallback.state().inner), Some(authored_effects));
        assert_eq!(
            fallback.state().inner.hazard_lighting_warning(),
            Some(
                orr_render::ImportedSceneError::UnsupportedHdr
                    .to_string()
                    .as_str()
            )
        );
        let a = fallback.state().inner.viewport3d_gpu().unwrap().gpu();
        let b = control.state().inner.viewport3d_gpu().unwrap().gpu();
        assert!(!a.post_process_settings().enabled);
        assert!(!b.post_process_settings().enabled);
        assert_eq!(a.post_process_allocated_bytes(), 0);
        assert_eq!(a.target().size(), b.target().size());
        let a = a.read_rgba8();
        let b = b.read_rgba8();
        assert_eq!(a.len(), b.len());
        let changed = a
            .as_chunks::<4>()
            .0
            .iter()
            .zip(b.as_chunks::<4>().0.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed > 20,
            "actual Player lighting must visibly differ with HDR disabled in both controls"
        );
        capture_frame(
            &mut fallback,
            &evidence,
            "07-player-unsupported-hdr-keeps-lighting.png",
        );
        assert_eq!(fs::read(&delivered_sidecar).unwrap(), delivered_bytes);
        serde_json::json!({
            "saved_post_processing_enabled": true, "actual_hdr_enabled": actual_hdr,
            "actual_hdr_fallback_warning": actual_warning, "forced_unsupported_hdr_fallback": true,
            "fallback_keeps_authored_policy_and_sidecar": true,
            "lighting_only_changed_viewport_pixels": changed,
        })
    } else {
        serde_json::Value::Null
    };

    let native_reports = if native {
        let first = native_report(&bundle.runtime, 50, &evidence, "native-first");
        let second = native_report(&bundle.runtime, 50, &evidence, "native-repeat");
        assert_eq!(
            first, second,
            "repeated standalone processes must be deterministic"
        );
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
        assert_eq!(
            first["lighting_sha256"],
            serde_json::json!(bundle.lighting_sha256)
        );
        assert_eq!(first["restart_parity"], true);
        assert_eq!(first["restarted_tick"], 0);
        Some(first)
    } else {
        None
    };
    assert_eq!(inventory(&relocated), before);
    let report = serde_json::json!({
        "schema": 1, "runtime_fixture": !native, "standalone_process_executed": native,
        "standalone_process_gpu": false, "native_window": false, "physical_input": false,
        "offscreen_player_software_gl": gpu, "hardware_gpu": false,
        "editor_widget_apply_undo_redo_save": true, "fresh_editor_reopen": true,
        "production_export_ui_job": true, "repeated_export_refuses_overwrite": true,
        "original_scene_sidecar_copied_runtime_removed": true, "bundle_hashes_unchanged": before,
        "runtime_sha256": runtime_sha, "lighting_sha256": bundle.lighting_sha256,
        "source_checksum": source_checksum, "lighting_and_no_lighting_50_tick_checksum": played_checksum,
        "source_position_raw": source_position, "moved_position_raw": moved_position,
        "gpu": gpu_evidence, "native_report": native_reports,
        "player_restart_keeps_lighting_and_resets_simulation": true,
        "no_lighting_control_restart_matches": true,
    });
    fs::write(
        evidence.join("receipt.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
