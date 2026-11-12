#![cfg(all(feature = "mesh-navigation", feature = "mesh-animated-models"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{game::EditorGame, Editor, EditorApp};

use orr_rhi::Rhi;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
}

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(30);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn click_button(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness
        .get_by_role_and_label(Role::Button, label)
        .scroll_to_me();
    harness.run_steps(30);
    harness
        .get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(harness);
}

fn type_field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    harness.run_steps(30);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    settle(harness);
}

fn menu_click(harness: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    if harness.query_by_label(item).is_none() {
        click(harness, menu);
    }
    harness.get_by_label(item).scroll_to_me();
    harness.run_steps(30);
    harness.get_by_label(item).click_accesskit();
    settle(harness);
}

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>, label: &str) -> Vec<u8> {
    let target = (900, 850);
    for _ in 0..6 {
        settle(harness);
        let actual = harness
            .state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .target()
            .size();
        if actual == target {
            break;
        }
        let window = harness.ctx.viewport_rect().size();
        let ratio = harness.ctx.pixels_per_point();
        harness.set_size(egui::vec2(
            window.x + (target.0 as f32 - actual.0 as f32) / ratio,
            window.y + (target.1 as f32 - actual.1 as f32) / ratio,
        ));
    }
    settle(harness);
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("MeshNavigation production viewport rendered");
    assert_eq!(viewport.gpu().target().size(), target);
    eprintln!("Mesh animation App GPU: {}", viewport.gpu().adapter_name());
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (target.0 * target.1 * 4) as usize);
    let image =
        egui::ColorImage::from_rgba_unmultiplied([target.0 as usize, target.1 as usize], &pixels);
    orr_editor::app::write_png(
        &PathBuf::from(std::env::var_os("ORR_MESH_ANIMATION_IMPORT_EVIDENCE").unwrap()).join(label),
        &image,
    )
    .unwrap();
    assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    pixels
}

fn changed_pixel_count(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>()
        .0
        .iter()
        .zip(right.as_chunks::<4>().0.iter())
        .filter(|(first_pixel, second_pixel)| {
            first_pixel[..3]
                .iter()
                .zip(second_pixel[..3].iter())
                .any(|(first, second)| first.abs_diff(*second) > 12)
        })
        .count()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sidecar_path(scene: &Path, suffix: &str) -> PathBuf {
    let mut name = scene.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn harness(scene: &Path) -> Harness<'static, EditorApp> {
    let mut editor = Editor::open_game(scene, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    Harness::builder()
        .with_size([1500.0, 1000.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| {
            let info = cc
                .wgpu_render_state
                .as_ref()
                .expect("required actual renderer")
                .adapter
                .get_info();
            eprintln!(
                "actual egui adapter={} backend={:?} device={:?}",
                info.name, info.backend, info.device_type
            );
            let root =
                PathBuf::from(std::env::var_os("ORR_MESH_ANIMATION_IMPORT_EVIDENCE").unwrap());
            fs::write(
                root.join("actual-adapter.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "name": info.name, "backend": format!("{:?}", info.backend),
                    "device_type": format!("{:?}", info.device_type),
                    "driver": info.driver, "driver_info": info.driver_info,
                    "headless_egui_wgpu": true, "native_window": false
                }))
                .unwrap(),
            )
            .unwrap();
            EditorApp::new(editor, cc.wgpu_render_state.clone())
        })
}

// Public FrameView exposes the checksum of the complete Frame, not serialized bytes.
fn frame_evidence(h: &Harness<'_, EditorApp>) -> (u64, u64) {
    let frame = h.state().editor.snapshot().unwrap().predicted();
    (frame.checksum(), frame.tick())
}

fn pulse_oracle(model: &orr_model::animation::AnimatedModel, tick: u32) {
    assert_eq!(model.source().clips[1].name, "pulse");
    let pose = model.sample_clip(1, tick as f32 / 60.0).unwrap();
    // Fixture's STEP ancestor translation stays fixed before 0.5 seconds;
    // lower-joint scale linearly interpolates [1,1,1] -> [1.15,0.8,1].
    let expected_scale = [
        1.0 + 0.15 * tick as f32 / 30.0,
        1.0 - 0.2 * tick as f32 / 30.0,
        1.0,
    ];
    for (actual, expected) in pose.local()[0].translation.iter().zip([0.3, 0.2, 0.0]) {
        assert!((*actual - expected).abs() < 0.00001);
    }
    for (actual, expected) in pose.local()[1].scale.iter().zip(expected_scale) {
        assert!((*actual - expected).abs() < 0.00001);
    }
}

#[test]
#[ignore = "requires a GPU adapter and a fresh absolute ORR_MESH_ANIMATION_IMPORT_EVIDENCE directory"]
fn raw_animation_import_preview_apply_save_and_source_free_reopen() {
    match orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default()) {
        Ok(gpu) => eprintln!(
            "preflight adapter: {} (software: {})",
            gpu.adapter_name(),
            gpu.is_software()
        ),
        Err(error) => {
            assert!(
                std::env::var("ORR_REQUIRE_GPU").as_deref() != Ok("1"),
                "required GPU unavailable: {error}"
            );
            eprintln!("SKIP: no GPU adapter: {error}");
            return;
        }
    }
    let root = PathBuf::from(
        std::env::var_os("ORR_MESH_ANIMATION_IMPORT_EVIDENCE")
            .expect("fresh evidence directory required"),
    );
    assert!(root.is_absolute());
    fs::create_dir(&root).expect("evidence directory must be fresh");
    let raw = root.join("raw source");
    fs::create_dir(&raw).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo");
    let model_bytes = fs::read(fixture.join("animated.glb")).unwrap();
    let license_bytes = fs::read(fixture.join("LICENSE.txt")).unwrap();
    // Deliberately use different raw filenames. The cooked source identity must
    // be the imported package asset name, not the original filename/path.
    let raw_model = raw.join("original character.glb");
    let raw_license = raw.join("original rights.txt");
    fs::write(&raw_model, &model_bytes).unwrap();
    fs::write(&raw_license, &license_bytes).unwrap();
    let package = root.join("new animation package");
    let expected_model =
        orr_model::animation_import::import_with_resolver("animated.glb", &model_bytes, |_| {
            Err(orr_model::Error::message("fixture must be embedded"))
        })
        .unwrap();
    pulse_oracle(&expected_model, 0);
    pulse_oracle(&expected_model, 16);
    assert_ne!(
        expected_model.sample_clip(1, 0.0).unwrap().global(),
        expected_model.sample_clip(1, 16.0 / 60.0).unwrap().global()
    );

    let scene = root.join("raw-import.scene.yaml");
    fs::copy(EditorGame::MeshNavigation.default_scene_path(), &scene).unwrap();
    let sidecar = sidecar_path(&scene, ".mesh-animation.json");
    let mut h = harness(&scene);
    settle(&mut h);
    click(&mut h, "Static-mesh navigation");
    click_button(&mut h, "Create ramp and bridge");
    click_button(&mut h, "Place mesh agent");
    menu_click(&mut h, "File", "Save Ctrl+S");
    let scene_bytes = fs::read(&scene).unwrap();
    let frame_before = frame_evidence(&h);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let baseline = viewport_pixels(&mut h, "01-baseline.png");
    click(&mut h, "Mesh agent animation");
    click(&mut h, "Import animation source");
    for (label, value) in [
        ("Animation source (.glb or cooked JSON)", raw_model.display().to_string()),
        ("Animation source license", raw_license.display().to_string()),
        (
            "New animation package directory",
            package.display().to_string(),
        ),
        ("Animation package name", "imported-pulse".to_owned()),
        ("Animation package version", "1.2.3".to_owned()),
    ] {
        type_field(&mut h, label, &value);
    }
    assert!(h
        .get_by_role_and_label(Role::Button, "Import animation package")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "I have reviewed rights to import this animation");
    click_button(&mut h, "Import animation package");
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        settle(&mut h);
        if h.query_by_label_contains("; staged for review, not applied.")
            .is_some()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "raw import worker did not stage candidate"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let loaded = viewport_pixels(&mut h, "02-imported-not-applied.png");
    assert_eq!(changed_pixel_count(&baseline, &loaded), 0);
    assert_eq!(fs::read(package.join("animated.glb")).unwrap(), model_bytes);
    assert_eq!(
        fs::read(package.join("LICENSE.txt")).unwrap(),
        license_bytes
    );
    let mut package_files = fs::read_dir(&package)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    package_files.sort();
    assert_eq!(
        package_files,
        ["LICENSE.txt", "animated.glb", "orr.package.json"]
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("orr.package.json")).unwrap()).unwrap();
    assert_eq!(manifest["name"], "imported-pulse");
    assert_eq!(manifest["version"], "1.2.3");
    assert_eq!(manifest["capabilities"], serde_json::json!(["animation"]));
    h.get_by_role_and_label(Role::ComboBox, "Animation preview clip choice")
        .scroll_to_me();
    h.run_steps(30);
    h.get_by_role_and_label(Role::ComboBox, "Animation preview clip choice")
        .click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Clip 1: pulse (1.000s)")
        .click_accesskit();
    settle(&mut h);
    type_field(&mut h, "Animation preview tick", "0");
    click_button(&mut h, "Preview animation pose");
    let candidate0 = viewport_pixels(&mut h, "03-candidate-pulse-0.png");
    type_field(&mut h, "Animation preview tick", "16");
    click_button(&mut h, "Preview animation pose");
    let candidate16 = viewport_pixels(&mut h, "04-candidate-pulse-16.png");
    let pose_changed = changed_pixel_count(&candidate0, &candidate16);
    assert!(changed_pixel_count(&baseline, &candidate0) > 20);
    assert!(
        pose_changed > 20,
        "pulse must visibly change: {pose_changed}"
    );
    assert_eq!(frame_evidence(&h), frame_before);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert!(!sidecar.exists());
    assert!(h
        .get_by_role_and_label(Role::Button, "Undo mesh animation")
        .accesskit_node()
        .is_disabled());
    click_button(&mut h, "Clear animation preview");
    let cleared = viewport_pixels(&mut h, "05-cleared.png");
    assert_eq!(changed_pixel_count(&baseline, &cleared), 0);
    type_field(&mut h, "Animation clip index", "1");
    click_button(&mut h, "Apply mesh animation");
    let applied = viewport_pixels(&mut h, "06-applied.png");
    assert_eq!(changed_pixel_count(&candidate0, &applied), 0);
    assert!(!sidecar.exists(), "Apply must not save");
    click_button(&mut h, "Undo mesh animation");
    let undone = viewport_pixels(&mut h, "07-undone.png");
    assert_eq!(changed_pixel_count(&baseline, &undone), 0);
    click_button(&mut h, "Redo mesh animation");
    let redone = viewport_pixels(&mut h, "08-redone.png");
    assert_eq!(changed_pixel_count(&applied, &redone), 0);
    click_button(&mut h, "Save mesh animation");
    let saved = fs::read(&sidecar).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(document["settings"]["clip"], 1);
    assert_eq!(document["asset"], "animated.glb");
    assert_eq!(document["asset_sha256"], sha256_hex(&model_bytes));
    assert_eq!(document["license_sha256"], sha256_hex(&license_bytes));
    assert_eq!(
        document["license_text"],
        std::str::from_utf8(&license_bytes).unwrap()
    );
    assert_eq!(
        document["model"],
        serde_json::to_value(expected_model.source()).unwrap()
    );
    let embedded = orr_model::animation::AnimatedModel::new(
        serde_json::from_value(document["model"].clone()).unwrap(),
    )
    .unwrap();
    pulse_oracle(&embedded, 0);
    pulse_oracle(&embedded, 16);
    assert_eq!(frame_evidence(&h), frame_before);
    assert_eq!(h.state().editor.history(), &history);
    drop(h);
    fs::rename(&raw, root.join("raw source.hidden")).unwrap();
    fs::rename(&package, root.join("new animation package.hidden")).unwrap();
    assert!(!raw_model.exists() && !raw_license.exists() && !package.exists());
    let mut h = harness(&scene);
    settle(&mut h);
    let reopened0 = viewport_pixels(&mut h, "09-source-free-reopen-0.png");
    assert_eq!(changed_pixel_count(&applied, &reopened0), 0);
    click(&mut h, "Mesh agent animation");
    type_field(&mut h, "Animation preview clip", "1");
    type_field(&mut h, "Animation preview tick", "16");
    click_button(&mut h, "Preview animation pose");
    let reopened16 = viewport_pixels(&mut h, "10-source-free-preview-16.png");
    assert_eq!(changed_pixel_count(&candidate16, &reopened16), 0);
    assert!(changed_pixel_count(&reopened0, &reopened16) > 20);
    assert_eq!(frame_evidence(&h), frame_before);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    assert!(!raw_model.exists() && !raw_license.exists() && !package.exists());
    assert_eq!(
        fs::read(root.join("raw source.hidden/original character.glb")).unwrap(),
        model_bytes
    );
    assert_eq!(
        fs::read(root.join("raw source.hidden/original rights.txt")).unwrap(),
        license_bytes
    );
    assert_eq!(
        fs::read(root.join("new animation package.hidden/animated.glb")).unwrap(),
        model_bytes
    );
    assert_eq!(
        fs::read(root.join("new animation package.hidden/LICENSE.txt")).unwrap(),
        license_bytes
    );
    let captures = ["01-baseline.png", "02-imported-not-applied.png", "03-candidate-pulse-0.png",
        "04-candidate-pulse-16.png", "05-cleared.png", "06-applied.png", "07-undone.png",
        "08-redone.png", "09-source-free-reopen-0.png", "10-source-free-preview-16.png"]
        .map(|name| serde_json::json!({"path":name, "sha256":sha256_hex(&fs::read(root.join(name)).unwrap())}));
    fs::write(root.join("acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1, "actual_adapter":serde_json::from_slice::<serde_json::Value>(&fs::read(root.join("actual-adapter.json")).unwrap()).unwrap(),
        "production_editor_app":true, "headless_egui_wgpu":true, "physical_os_input":false,
        "native_window":false, "physical_gpu_claim":false,
        "raw_import_worker_to_candidate":true, "rights_acknowledgement_required":true,
        "explicit_apply_undo_redo_save":true, "source_hidden_during_fresh_reopen":true,
        "clip":1, "clip_name":"pulse", "preview_ticks":[0,16],
        "independent_fixture_local_pose_oracle":true, "candidate_pose_changed_pixels":pose_changed,
        "frame_evidence_scope":"complete Frame checksum and tick; no serialized Frame byte equality claim",
        "frame_checksum":frame_before.0, "frame_tick":frame_before.1,
        "scene_sha256":sha256_hex(&scene_bytes), "sidecar_sha256":sha256_hex(&saved),
        "scene_history_unchanged_before_apply":true, "capture_count":captures.len(), "captures":captures
    })).unwrap()).unwrap();
}
