#![cfg(all(feature = "mesh-navigation", feature = "mesh-animated-models"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    game::EditorGame,
    Editor, EditorApp, Mode,
};

use orr_rhi::Rhi;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
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
    assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    let image =
        egui::ColorImage::from_rgba_unmultiplied([target.0 as usize, target.1 as usize], &pixels);
    orr_editor::app::write_png(
        &PathBuf::from(std::env::var_os("ORR_ANIMATION_CANDIDATE_CAPTURE_DIR").unwrap())
            .join(label),
        &image,
    )
    .unwrap();
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
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|a| a.get_info().backend == wgpu::Backend::Vulkan)
            .cloned()
            .ok_or_else(|| "Vulkan adapter unavailable".to_owned())
    }));
    Harness::builder()
        .with_size([1500.0, 1000.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()))
}

#[test]
#[ignore = "requires software Vulkan and ORR_ANIMATION_CANDIDATE_CAPTURE_DIR"]
fn loaded_candidate_named_clip_previews_without_applying_and_preserves_authoring() {
    let root = PathBuf::from(
        std::env::var_os("ORR_ANIMATION_CANDIDATE_CAPTURE_DIR")
            .expect("fresh capture directory required"),
    );
    assert!(root.is_absolute());
    fs::create_dir(&root).expect("fresh directory");
    let rhi = orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default()).unwrap();
    let software = rhi.is_software();
    let adapter = rhi.adapter_name().to_owned();
    drop(rhi);
    let package = root.join("animation package");
    fs::create_dir(&package).unwrap();
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo");
    let mut originals = Vec::new();
    for name in ["orr.package.json", "animated.glb", "LICENSE.txt"] {
        let bytes = fs::read(assets.join(name)).unwrap();
        fs::write(package.join(name), &bytes).unwrap();
        originals.push((name, bytes));
    }
    let scene = root.join("candidate.scene.yaml");
    // Start from the shipped mesh scene, then use the real authoring controls.
    fs::copy(EditorGame::MeshNavigation.default_scene_path(), &scene).unwrap();
    let mut h = harness(&scene);
    settle(&mut h);
    click(&mut h, "Static-mesh navigation");
    click_button(&mut h, "Create ramp and bridge");
    click_button(&mut h, "Place mesh agent");
    menu_click(&mut h, "File", "Save Ctrl+S");
    let scene_bytes = fs::read(&scene).unwrap();
    let checksum = h.state().editor.checksum();
    let frame_tick = h.state().editor.snapshot().unwrap().predicted().tick();
    let history = h.state().editor.history().clone();
    let sidecar = sidecar_path(&scene, ".mesh-animation.json");
    let baseline = viewport_pixels(&mut h, "01-baseline.png");
    click(&mut h, "Mesh agent animation");
    type_field(
        &mut h,
        "Animation package source directory",
        &package.display().to_string(),
    );
    type_field(&mut h, "Animation model asset", "animated.glb");
    type_field(&mut h, "Animation license asset", "LICENSE.txt");
    assert!(h
        .get_by_role_and_label(Role::Button, "Load mesh animation")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "I have reviewed rights to use this animation");
    click_button(&mut h, "Load mesh animation");
    let loaded = viewport_pixels(&mut h, "02-loaded.png");
    assert_eq!(changed_pixel_count(&baseline, &loaded), 0);
    // Select the nonzero named clip through the actual ComboBox, not the numeric fallback.
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
    let pose0 = viewport_pixels(&mut h, "03-candidate-pulse-0.png");
    assert!(changed_pixel_count(&baseline, &pose0) > 20);
    type_field(&mut h, "Animation preview tick", "16");
    click_button(&mut h, "Preview animation pose");
    let pose16 = viewport_pixels(&mut h, "04-candidate-pulse-16.png");
    let pose_diff = changed_pixel_count(&pose0, &pose16);
    assert!(
        pose_diff > 20,
        "candidate clip must visibly animate: {pose_diff}"
    );
    for (field, value) in [
        ("Animation preview tick", "-1"),
        ("Animation preview tick", "3601"),
        ("Animation preview clip", "999"),
    ] {
        if field.ends_with("clip") {
            type_field(&mut h, "Animation preview tick", "16");
        }
        type_field(&mut h, field, value);
        click_button(&mut h, "Preview animation pose");
        assert_eq!(
            changed_pixel_count(
                &pose16,
                &viewport_pixels(&mut h, "05-invalid-retains-pose.png")
            ),
            0
        );
    }
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(
        h.state().editor.snapshot().unwrap().predicted().tick(),
        frame_tick
    );
    assert_eq!(h.state().editor.history(), &history);
    assert!(!sidecar.exists());
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert!(h
        .get_by_role_and_label(Role::Button, "Undo mesh animation")
        .accesskit_node()
        .is_disabled());
    click_button(&mut h, "Clear animation preview");
    assert_eq!(
        changed_pixel_count(&baseline, &viewport_pixels(&mut h, "06-clear.png")),
        0
    );
    type_field(&mut h, "Animation preview clip", "1");
    click_button(&mut h, "Preview animation pose");
    // Replacing the admitted candidate requires a fresh rights acknowledgement.
    click(&mut h, "I have reviewed rights to use this animation");
    assert!(!h
        .get_by_role_and_label(Role::Button, "Load mesh animation")
        .accesskit_node()
        .is_disabled());
    click_button(&mut h, "Load mesh animation");
    assert_eq!(
        changed_pixel_count(
            &baseline,
            &viewport_pixels(&mut h, "07-replacement-cleared.png")
        ),
        0
    );
    type_field(&mut h, "Animation clip index", "1");
    click_button(&mut h, "Apply mesh animation");
    let applied = viewport_pixels(&mut h, "08-applied.png");
    click_button(&mut h, "Undo mesh animation");
    assert_eq!(
        changed_pixel_count(&baseline, &viewport_pixels(&mut h, "09-undo.png")),
        0
    );
    click_button(&mut h, "Redo mesh animation");
    assert_eq!(
        changed_pixel_count(&applied, &viewport_pixels(&mut h, "10-redo.png")),
        0
    );
    click_button(&mut h, "Save mesh animation");
    let saved = fs::read(&sidecar).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(document["settings"]["clip"], 1);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    drop(h);
    let mut h = harness(&scene);
    settle(&mut h);
    assert_eq!(
        changed_pixel_count(&applied, &viewport_pixels(&mut h, "11-reopened.png")),
        0
    );
    click(&mut h, "Mesh agent animation");
    type_field(&mut h, "Animation preview clip", "1");
    type_field(&mut h, "Animation preview tick", "16");
    click_button(&mut h, "Preview animation pose");
    assert!(changed_pixel_count(&applied, &viewport_pixels(&mut h, "12-applied-preview.png")) > 20);
    click_button(&mut h, LBL_PLAY);
    click_button(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(
        changed_pixel_count(
            &applied,
            &viewport_pixels(&mut h, "13-play-stop-cleared.png")
        ),
        0
    );
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    for (name, bytes) in originals {
        assert_eq!(fs::read(package.join(name)).unwrap(), bytes);
    }
    fs::write(root.join("acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema": 1, "production_editor_app": true, "software_gpu": software,
        "adapter": adapter, "physical_os_input": false, "named_nonzero_candidate_clip": "pulse",
        "candidate_pose_changed_pixels": pose_diff, "frame_history_files_unchanged_before_apply": true,
        "invalid_tick_clip_retains_pose": true, "clear_restores": true, "replacement_clears": true,
        "apply_undo_redo_save_reopen": true, "play_stop_clears": true,
        "rights_acknowledgement_required": true, "scene_sha256": sha256_hex(&scene_bytes)
    })).unwrap()).unwrap();
}
