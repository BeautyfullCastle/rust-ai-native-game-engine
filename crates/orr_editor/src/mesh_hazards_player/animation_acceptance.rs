#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::{acceptance::*, animation_pose_digest, start_playback, verify_loaded_animation_bundle};
use crate::{Editor, EditorApp, game::EditorGame};
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn harness(editor: Editor) -> Harness<'static, EditorApp> {
    Harness::builder()
        .with_size([960.0, 600.0])
        .with_step_dt(1.0 / 60.0)
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| EditorApp::new(editor, None))
}

fn package(root: &Path) -> PathBuf {
    let result = root.join("original-animation-package");
    fs::create_dir(&result).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo");
    for name in ["animated.glb", "LICENSE.txt", "orr.package.json"] {
        fs::copy(source.join(name), result.join(name)).unwrap();
    }
    result
}

fn native_report(runtime: &Path, ticks: u32, evidence: &Path) -> serde_json::Value {
    let output = evidence.join(format!("tick-{ticks}.json"));
    let error = evidence.join(format!("tick-{ticks}.stderr"));
    let mut child = Command::new(runtime)
        .args(["--verify-headless", "--ticks", &ticks.to_string()])
        .current_dir(evidence)
        .stdin(Stdio::null())
        .stdout(
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)
                .unwrap(),
        )
        .stderr(
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&error)
                .unwrap(),
        )
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{}", fs::read_to_string(error).unwrap());
            return serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native animation verification timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn hazards_animation_widget_save_reopen_export_pose_restart() {
    scenario(false);
}

#[test]
#[ignore = "requires built animation-capable player, independently pinned SHA-256 and new evidence directory"]
fn hazards_animation_relocated_native_headless_evidence() {
    scenario(true);
}

fn scenario(native: bool) {
    let temp = tempfile::tempdir().unwrap();
    let evidence = if native {
        env_path("ORR_HAZARDS_ANIMATION_EVIDENCE")
    } else {
        temp.path().join("evidence")
    };
    fs::create_dir(&evidence).unwrap();
    let scene = temp.path().join("authored.scene.yaml");
    let source_package = package(temp.path());
    let editor = crate::mesh_hazards_creator::prepare(&scene, &Default::default(), true).unwrap();
    let checksum = editor.checksum();
    let mut h = harness(editor);
    settle_editor(&mut h);
    if h.query_by_label("Animation package source directory")
        .is_none()
    {
        h.get_by_label("Mesh agent animation").click_accesskit();
        h.step();
        settle_editor(&mut h);
    }
    type_editor_field(
        &mut h,
        "Animation package source directory",
        source_package.to_str().unwrap(),
    );
    click_editor(
        &mut h,
        Role::CheckBox,
        "I have reviewed rights to use this animation",
    );
    click_editor(&mut h, Role::Button, "Load mesh animation");
    assert!(!h.state().hazards_animation_matches(Some("unapplied")));
    type_editor_field(&mut h, "Animation clip index", "1");
    click_editor(&mut h, Role::Button, "Apply mesh animation");
    assert_eq!(h.state().editor.checksum(), checksum);
    click_editor(&mut h, Role::Button, "Undo mesh animation");
    click_editor(&mut h, Role::Button, "Redo mesh animation");
    click_editor(&mut h, Role::Button, "Save mesh animation");
    assert_eq!(h.state().editor.checksum(), checksum);
    let sidecar = crate::mesh_animation_store::sidecar_path(&scene).unwrap();
    let saved = fs::read(&sidecar).unwrap();
    drop(h);
    // Only disposable fixtures are hidden; the reopened creator must use its saved embedded package.
    fs::rename(&source_package, temp.path().join("hidden-package")).unwrap();
    let mut fresh = harness(Editor::open_game(&scene, EditorGame::MeshHazardsV1).unwrap());
    settle_editor(&mut fresh);
    let saved_digest = crate::mesh_animation_store::sidecar_stamp(&scene)
        .unwrap()
        .unwrap();
    assert!(fresh.state().hazards_animation_matches(Some(&saved_digest)));
    assert_eq!(fresh.state().editor.checksum(), checksum);
    drop(fresh);
    let platform = if cfg!(windows) {
        crate::mesh_hazards_export::RuntimePlatform::WindowsX86_64
    } else {
        crate::mesh_hazards_export::RuntimePlatform::LinuxX86_64
    };
    let source_runtime = temp.path().join(platform.runtime_filename());
    if native {
        let runtime = env_path("ORR_HAZARDS_ANIMATION_RUNTIME");
        let expected = std::env::var("ORR_HAZARDS_ANIMATION_EXPECTED_RUNTIME_SHA256").unwrap();
        assert_eq!(super::sha256_file_bounded(&runtime).unwrap(), expected);
        fs::copy(runtime, &source_runtime).unwrap();
    } else {
        fs::write(
            &source_runtime,
            crate::mesh_hazards_export::test_animation_runtime(platform),
        )
        .unwrap();
    }
    let output = temp.path().join("exported");
    let sources = crate::mesh_hazards_export::capture_sources(&scene).unwrap();
    let receipt = crate::mesh_hazards_export::export_interactive_sources(
        crate::mesh_hazards_export::ExportRequest {
            scene: &scene,
            expected_checksum: checksum,
            destination: &output,
            runtime: &source_runtime,
            trusted_runtime: true,
        },
        &sources,
    )
    .unwrap();
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    let relocated = evidence.join("relocated");
    fs::rename(&output, &relocated).unwrap();
    fs::rename(&scene, temp.path().join("hidden-scene")).unwrap();
    fs::rename(&sidecar, temp.path().join("hidden-sidecar")).unwrap();
    fs::rename(&source_runtime, temp.path().join("hidden-runtime")).unwrap();
    let bundle = crate::mesh_hazards_export::open_bundle(&relocated).unwrap();
    assert!(bundle.animation_sha256.is_some());
    assert_eq!(bundle.animation_sha256, receipt.animation_sha256);
    let before = inventory(&relocated);
    assert_eq!(before.len(), 4);
    let mut editor = Editor::open_game(&bundle.scene, EditorGame::MeshHazardsV1).unwrap();
    editor.sync();
    let mut animation = crate::mesh_animation_panel::Panel::default();
    animation.sync(&editor);
    verify_loaded_animation_bundle(&relocated, &bundle, &animation).unwrap();
    start_playback(&mut editor, Some(0), false).unwrap();
    let pose0 = animation_pose_digest(&animation.placement(&editor).unwrap().unwrap());
    editor.step(30);
    editor.sync();
    let before_sampling = editor.snapshot().unwrap().predicted().checksum();
    let pose30 = animation_pose_digest(&animation.placement(&editor).unwrap().unwrap());
    assert_ne!(
        pose0, pose30,
        "skeletal pose must change independently of world instance movement"
    );
    assert_eq!(
        editor.snapshot().unwrap().predicted().checksum(),
        before_sampling
    );
    let mut app = EditorApp::new(editor, None);
    app.install_validated_hazards_animation(animation);
    app.verify_hazards_restart().unwrap();
    assert!(app.hazards_animation_matches(bundle.animation_sha256.as_deref()));
    assert_eq!(
        app.hazards_animation_pose_digest().unwrap().as_deref(),
        Some(pose0.as_str())
    );
    assert_eq!(app.editor.checksum(), checksum);
    if native {
        let zero = native_report(&bundle.runtime, 0, &evidence);
        let thirty = native_report(&bundle.runtime, 30, &evidence);
        for report in [&zero, &thirty] {
            assert_eq!(
                report["animation_sha256"],
                bundle.animation_sha256.as_deref().unwrap()
            );
            assert_eq!(report["animation_restart_parity"], true);
            assert_eq!(report["animation_restarted_pose_sha256"], pose0);
            assert_eq!(report["animation_initial_pose_sha256"], pose0);
            assert_eq!(report["restarted_tick"], 0);
        }
        assert_eq!(zero["animation_pose_sha256"], pose0);
        assert_eq!(thirty["animation_pose_sha256"], pose30);
        assert_ne!(
            zero["animation_pose_sha256"],
            thirty["animation_pose_sha256"]
        );
    }
    assert_eq!(inventory(&relocated), before);
    // Loading cannot substitute changed authored bytes for the admitted digest.
    let mut wrong = bundle.clone();
    wrong.animation_sha256 = Some("0".repeat(64));
    let mut panel = crate::mesh_animation_panel::Panel::default();
    panel.sync(&app.editor);
    assert!(verify_loaded_animation_bundle(&relocated, &wrong, &panel).is_err());
}
