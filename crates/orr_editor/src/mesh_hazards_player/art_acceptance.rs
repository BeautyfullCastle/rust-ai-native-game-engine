#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
//! Production UI + relocated executable acceptance. GPU evidence is explicitly
//! offscreen software GL, distinct from the standalone process's CPU verification.
use super::{PlayerApp, acceptance::*, start_playback};
use crate::{Editor, EditorApp, game::EditorGame};
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use egui_wgpu::wgpu;
use orr_remote::mesh_navigation::mesh_from_view;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn static_package(root: &Path) -> PathBuf {
    let package = root.join("original-package");
    fs::create_dir(&package).unwrap();
    fs::write(
        package.join("model.glb"),
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
    )
    .unwrap();
    fs::write(package.join("LICENSE.txt"), b"Acceptance fixture license\n").unwrap();
    let manifest = orr_package::Manifest {
        schema: 1,
        name: "hazards-acceptance-art".into(),
        version: "1.0.0".into(),
        engine: format!("={}", orr_package::Runtime::content_only().engine_version),
        capabilities: BTreeSet::from(["models".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["model.glb".into(), "LICENSE.txt".into()]),
    };
    fs::write(
        package.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    package
}

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
    navigation: crate::mesh_navigation::Panel,
    gpu: bool,
) -> Harness<'static, PlayerApp> {
    start_playback(&mut editor, Some(0), false).unwrap();
    let builder = Harness::builder()
        .with_size([960.0, 600.0])
        .with_step_dt(1.0 / 60.0);
    let create = move |cc: &mut eframe::CreationContext<'_>| {
        let mut inner = EditorApp::new(editor, cc.wgpu_render_state.clone());
        inner.install_validated_hazards_navigation(navigation);
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

fn art_present(app: &EditorApp) -> bool {
    app.mesh_navigation.art_model(&app.editor).is_some()
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
    // No --bundle: the shipped executable must find its own relocated siblings.
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
fn hazards_art_widget_apply_undo_redo_save_reopen_export_restart() {
    scenario(false, false);
}

#[test]
#[ignore = "requires independently built runtime, SHA-256 and fresh evidence directory"]
fn hazards_art_relocated_native_headless_evidence() {
    scenario(false, true);
}

#[test]
#[ignore = "requires software GL CPU adapter, independently built runtime and fresh evidence directory"]
fn hazards_art_relocated_software_gl_evidence() {
    scenario(true, true);
}

fn scenario(gpu: bool, native: bool) {
    let native_evidence = native.then(|| env_path("ORR_HAZARDS_ART_EVIDENCE"));
    // Real delivery retains an atomic rename, even when /tmp is a separate mount.
    let temp = if let Some(root) = &native_evidence {
        tempfile::tempdir_in(root.parent().expect("evidence directory has a parent")).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    };
    let evidence = if let Some(root) = native_evidence {
        fs::create_dir(&root).expect("evidence directory must be fresh");
        root
    } else {
        temp.path().join("evidence")
    };
    if !native {
        fs::create_dir(&evidence).unwrap();
    }
    let scene = temp.path().join("authored.scene.yaml");
    let package = static_package(temp.path());
    let editor = crate::mesh_hazards_creator::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
        true,
    )
    .unwrap();
    let mut h = editor_harness(editor, gpu);
    settle_editor(&mut h);
    let source_checksum = h.state().editor.checksum();
    let source_position = position(&h.state().editor);
    assert!(!art_present(h.state()));
    if h.query_by_label("Art package source directory").is_none() {
        h.get_by_label("Mesh agent art").click_accesskit();
        h.step();
        settle_editor(&mut h);
    }
    type_editor_field(
        &mut h,
        "Art package source directory",
        package.to_str().unwrap(),
    );
    click_editor(
        &mut h,
        Role::CheckBox,
        "I have reviewed rights to use this agent art",
    );
    click_editor(&mut h, Role::Button, "Load agent art");
    assert!(
        !art_present(h.state()),
        "loading a candidate must not apply it"
    );
    type_editor_field(&mut h, "Agent art offset Y", "0.5");
    type_editor_field(&mut h, "Agent art scale", "2");
    click_editor(&mut h, Role::Button, "Apply agent art");
    assert!(art_present(h.state()));
    assert!(h.state().mesh_navigation.art_dirty());
    click_editor(&mut h, Role::Button, "Undo agent art");
    assert!(!art_present(h.state()));
    click_editor(&mut h, Role::Button, "Redo agent art");
    assert!(art_present(h.state()));
    click_editor(&mut h, Role::Button, "Save agent art");
    assert!(!h.state().mesh_navigation.art_dirty());
    assert_eq!(h.state().editor.checksum(), source_checksum);
    assert_eq!(position(&h.state().editor), source_position);
    let sidecar = crate::mesh_art::sidecar_path(&scene).unwrap();
    assert!(sidecar.is_file());
    if gpu {
        capture_frame(&mut h, &evidence, "01-editor-saved-art.png");
    }
    drop(h);
    // Remove only disposable fixture inputs. Reopen cannot use the original package.
    fs::remove_dir_all(&package).unwrap();
    let mut fresh = editor_harness(
        Editor::open_game(&scene, EditorGame::MeshHazardsV1).unwrap(),
        gpu,
    );
    settle_editor(&mut fresh);
    assert!(art_present(fresh.state()));
    assert_eq!(fresh.state().editor.checksum(), source_checksum);
    if gpu {
        capture_frame(&mut fresh, &evidence, "02-editor-source-hidden-reopen.png");
    }
    drop(fresh);

    let platform = if cfg!(windows) {
        crate::mesh_hazards_export::RuntimePlatform::WindowsX86_64
    } else {
        crate::mesh_hazards_export::RuntimePlatform::LinuxX86_64
    };
    let source_runtime = temp.path().join(platform.runtime_filename());
    let runtime_sha = if native {
        let source = env_path("ORR_HAZARDS_ART_RUNTIME");
        let expected = std::env::var("ORR_HAZARDS_ART_EXPECTED_RUNTIME_SHA256")
            .expect("set independently built runtime SHA-256");
        assert_eq!(super::sha256_file_bounded(&source).unwrap(), expected);
        fs::copy(source, &source_runtime).unwrap();
        expected
    } else {
        fs::write(
            &source_runtime,
            crate::mesh_hazards_export::test_art_runtime(platform),
        )
        .unwrap();
        super::sha256_file_bounded(&source_runtime).unwrap()
    };
    let staging = temp.path().join("export-staging");
    let sources = crate::mesh_hazards_export::capture_sources(&scene).unwrap();
    let receipt = crate::mesh_hazards_export::export_interactive_sources(
        crate::mesh_hazards_export::ExportRequest {
            scene: &scene,
            expected_checksum: source_checksum,
            destination: &staging,
            runtime: &source_runtime,
            trusted_runtime: true,
        },
        &sources,
    )
    .unwrap();
    let relocated = evidence.join("relocated-bundle");
    fs::rename(staging, &relocated).unwrap();
    fs::remove_file(&scene).unwrap();
    fs::remove_file(&sidecar).unwrap();
    fs::remove_file(&source_runtime).unwrap();
    assert!(!scene.exists() && !sidecar.exists() && !package.exists() && !source_runtime.exists());
    let bundle = crate::mesh_hazards_export::open_bundle(&relocated).unwrap();
    assert_eq!(bundle.runtime_sha256, runtime_sha);
    assert_eq!(bundle.art_sha256, receipt.art_sha256);
    assert!(bundle.art_sha256.is_some());
    let before = inventory(&relocated);
    assert_eq!(before.len(), 4);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(relocated.join(crate::mesh_hazards_export::MANIFEST_FILENAME)).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["schema"], 3);
    assert!(manifest["art"].is_object());

    let mut reopened = Editor::open_game(&bundle.scene, EditorGame::MeshHazardsV1).unwrap();
    reopened.sync();
    let mut navigation = crate::mesh_navigation::Panel::default();
    navigation.sync(&reopened);
    super::verify_loaded_art_bundle(&relocated, &bundle, &navigation).unwrap();
    let initial_translation = navigation
        .art_model(&reopened)
        .unwrap()
        .instance
        .translation;
    start_playback(&mut reopened, Some(50), false).unwrap();
    let played_checksum = reopened.snapshot().unwrap().predicted().checksum();
    let moved_position = position(&reopened);
    assert_ne!(
        moved_position, source_position,
        "art follows actual advancing agent"
    );
    let moved_translation = navigation
        .art_model(&reopened)
        .unwrap()
        .instance
        .translation;
    assert_ne!(moved_translation, initial_translation);
    let expected_translation = mesh_from_view(reopened.snapshot().unwrap().predicted())
        .unwrap()
        .agent
        .position
        .map(|p| p.to_f32());
    assert_eq!(
        moved_translation,
        [
            expected_translation[0],
            expected_translation[1] + 0.5,
            expected_translation[2]
        ]
    );
    assert!(reopened.stop().is_some());
    reopened.sync();
    assert_eq!(reopened.checksum(), source_checksum);
    let mut player = player_harness(reopened, navigation, gpu);
    settle(&mut player);
    assert!(art_present(&player.state().inner));
    if gpu {
        capture_frame(&mut player, &evidence, "03-relocated-player-art.png");
    }
    click(&mut player, "Resume");
    wait_for(&mut player, "art Player simulation advances", |app| {
        app.inner.editor.sim().head_tick > 0
    });
    click(&mut player, "Pause");
    assert!(art_present(&player.state().inner));
    click(&mut player, "Restart game…");
    click(&mut player, "Restart game");
    crate::app::mesh_hazards_player::validate_fresh_session(&player.state().inner.editor).unwrap();
    assert!(art_present(&player.state().inner));
    assert_eq!(position(&player.state().inner.editor), source_position);
    if gpu {
        capture_frame(&mut player, &evidence, "04-player-restarted-art.png");
    }

    // Literal no-art control uses an independent scene copy with no sidecar.
    let control_scene = temp.path().join("literal-no-art.scene.yaml");
    fs::copy(&bundle.scene, &control_scene).unwrap();
    let mut control = Editor::open_game(&control_scene, EditorGame::MeshHazardsV1).unwrap();
    control.sync();
    let mut no_art = crate::mesh_navigation::Panel::default();
    no_art.sync(&control);
    assert!(no_art.art_model(&control).is_none());
    start_playback(&mut control, Some(50), false).unwrap();
    assert_eq!(
        control.snapshot().unwrap().predicted().checksum(),
        played_checksum
    );
    assert_eq!(position(&control), moved_position);
    assert!(control.stop().is_some());
    control.sync();
    let mut control = player_harness(control, no_art, gpu);
    settle(&mut control);
    assert!(!art_present(&control.state().inner));
    let changed_pixels = if gpu {
        capture_frame(&mut control, &evidence, "05-player-literal-no-art.png");
        settle(&mut player);
        let a = player.state().inner.viewport3d_gpu().unwrap().gpu();
        let b = control.state().inner.viewport3d_gpu().unwrap().gpu();
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
            "actual Player viewport must differ from literal no-art control"
        );
        changed
    } else {
        0
    };
    let native_reports = if native {
        let first = native_report(&bundle.runtime, 50, &evidence, "native-first");
        let second = native_report(&bundle.runtime, 50, &evidence, "native-repeat");
        assert_eq!(
            first, second,
            "repeated standalone processes must report deterministic state"
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
        assert_eq!(first["art_sha256"], serde_json::json!(bundle.art_sha256));
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
        "editor_widget_load_apply_undo_redo_save": true, "fresh_editor_source_package_removed": true,
        "original_scene_sidecar_runtime_removed": true, "bundle_hashes_unchanged": before,
        "runtime_sha256": runtime_sha, "art_sha256": bundle.art_sha256,
        "source_checksum": source_checksum, "art_and_literal_no_art_50_tick_checksum": played_checksum,
        "source_position_raw": source_position, "moved_position_raw": moved_position,
        "literal_no_art_changed_viewport_pixels": changed_pixels, "native_report": native_reports,
        "player_restart_keeps_art_and_resets_agent": true
    });
    fs::write(
        evidence.join("receipt.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
