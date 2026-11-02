//! Actual shaded prefab save/load worker and explicit candidate application.
//! Offscreen GPU only; observed backend is logged. No standalone or native-window claim.
#![cfg(feature = "mesh-prop-shading")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{game::EditorGame, Editor, EditorApp};
use serde_json::Value;
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
    eprintln!(
        "Mesh prop shading App GPU: {}",
        viewport.gpu().adapter_name()
    );
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (target.0 * target.1 * 4) as usize);
    assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    let image =
        egui::ColorImage::from_rgba_unmultiplied([target.0 as usize, target.1 as usize], &pixels);
    orr_editor::app::write_png(
        &PathBuf::from(std::env::var_os("ORR_SHADED_PROP_PREFAB_CAPTURE_DIR").unwrap()).join(label),
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
            let info = cc.wgpu_render_state.as_ref().unwrap().adapter.get_info();
            eprintln!(
                "actual egui adapter={} backend={:?} device={:?}",
                info.name, info.backend, info.device_type
            );
            EditorApp::new(editor, cc.wgpu_render_state.clone())
        })
}

fn wait_label(h: &mut Harness<'_, EditorApp>, text: &str) {
    let until = Instant::now() + Duration::from_secs(90);
    loop {
        settle(h);
        if h.query_by_label_contains(text).is_some() {
            return;
        }
        assert!(Instant::now() < until, "worker did not report {text}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
#[ignore = "requires actual GPU and fresh ORR_SHADED_PROP_PREFAB_CAPTURE_DIR"]
fn shaded_prefab_candidate_preserves_scalar_pixels_and_reopens_without_inputs() {
    let root = PathBuf::from(
        std::env::var_os("ORR_SHADED_PROP_PREFAB_CAPTURE_DIR")
            .expect("fresh evidence directory required"),
    );
    assert!(root.is_absolute());
    fs::create_dir(&root).unwrap();
    let package = root.join("source-package");
    fs::create_dir(&package).unwrap();
    let original = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
    let mut source_bytes = Vec::new();
    for name in [
        "orr.package.json",
        "foreground.glb",
        "background.glb",
        "LICENSE.txt",
        "generate.py",
    ] {
        let bytes = fs::read(original.join(name)).unwrap();
        fs::write(package.join(name), &bytes).unwrap();
        source_bytes.push((name, bytes));
    }
    let scene = root.join("prefab.scene.yaml");
    fs::copy(EditorGame::MeshNavigation.default_scene_path(), &scene).unwrap();
    let mut h = harness(&scene);
    settle(&mut h);
    click(&mut h, "Static-mesh navigation");
    click_button(&mut h, "Create ramp and bridge");
    click_button(&mut h, "Place mesh agent");
    menu_click(&mut h, "File", "Save Ctrl+S");
    let scene_bytes = fs::read(&scene).unwrap();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    click(&mut h, "Mesh props");
    let empty = viewport_pixels(&mut h, "00-empty.png");
    type_field(
        &mut h,
        "Prop package source directory",
        &package.display().to_string(),
    );
    type_field(&mut h, "Prop model asset", "foreground.glb");
    type_field(&mut h, "Prop license asset", "LICENSE.txt");
    click(&mut h, "I have reviewed rights to use these props");
    click_button(&mut h, "Load prop candidate");
    for (label, value) in [
        ("Prop name", "Shaded source"),
        ("Prop position X", "-3"),
        ("Prop position Y", "0.25"),
        ("Prop position Z", "1"),
        ("Prop scale", "1"),
    ] {
        type_field(&mut h, label, value);
    }
    click_button(&mut h, "Add prop");
    let legacy = viewport_pixels(&mut h, "01-legacy.png");
    assert!(
        changed_pixel_count(&empty, &legacy) > 20,
        "model must actually render"
    );
    click(&mut h, "Scalar material shading");
    click(&mut h, "Use scalar shading for prop slot");
    type_field(&mut h, "Prop slot metallic", "1");
    type_field(&mut h, "Prop slot roughness", "0.15");
    assert_eq!(viewport_pixels(&mut h, "02-draft.png"), legacy);
    click_button(&mut h, "Update selected prop");
    let shaded = viewport_pixels(&mut h, "03-shaded.png");
    assert!(
        changed_pixel_count(&legacy, &shaded) > 20,
        "scalar shading must be visible, not a black/control coincidence"
    );
    click_button(&mut h, "Save props");
    let sidecar = sidecar_path(&scene, ".mesh-props.json");
    let first_saved = fs::read(&sidecar).unwrap();
    let first: Value = serde_json::from_slice(&first_saved).unwrap();
    let shading = first["props"][0]["shading"].clone();
    assert!(!shading.is_null());
    let prefab = root.join("shaded.mesh-prop-prefab.json");
    click(&mut h, "Prop prefab");
    type_field(&mut h, "Prop prefab file", &prefab.display().to_string());
    click_button(&mut h, "Save selected prop prefab");
    wait_label(&mut h, "Saved prop prefab asset. SHA-256:");
    let prefab_bytes = fs::read(&prefab).unwrap();
    let prefab_json: Value = serde_json::from_slice(&prefab_bytes).unwrap();
    assert_eq!(prefab_json["shading"], shading);
    assert_eq!(fs::read(&sidecar).unwrap(), first_saved);
    assert_eq!(viewport_pixels(&mut h, "04-after-prefab-save.png"), shaded);
    // Deliberately change the applied prop. Loading must remain a candidate, not restore it implicitly.
    click(&mut h, "Use scalar shading for prop slot");
    click_button(&mut h, "Update selected prop");
    assert_eq!(viewport_pixels(&mut h, "05-legacy-again.png"), legacy);
    click_button(&mut h, "Save props");
    let legacy_saved = fs::read(&sidecar).unwrap();
    click(&mut h, "I have reviewed rights to use this prop prefab");
    click_button(&mut h, "Load prop prefab");
    wait_label(&mut h, "Loaded prop prefab candidate; no prop was added.");
    assert_eq!(viewport_pixels(&mut h, "06-loaded-candidate.png"), legacy);
    assert_eq!(fs::read(&sidecar).unwrap(), legacy_saved);
    assert!(h.query_by_label("Props: 1/128").is_some());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    click_button(&mut h, "Update selected prop");
    assert_eq!(
        viewport_pixels(&mut h, "07-applied-prefab.png"),
        shaded,
        "candidate must restore original scalar appearance"
    );
    assert_eq!(
        fs::read(&sidecar).unwrap(),
        legacy_saved,
        "Apply is not Save"
    );
    click_button(&mut h, "Duplicate selected prop");
    assert!(h.query_by_label("Props: 2/128").is_some());
    type_field(&mut h, "Prop position X", "3");
    click_button(&mut h, "Update selected prop");
    let duplicated = viewport_pixels(&mut h, "08-duplicate.png");
    assert!(changed_pixel_count(&shaded, &duplicated) > 20);
    click_button(&mut h, "Undo prop edit");
    click_button(&mut h, "Undo prop edit");
    assert!(h.query_by_label("Props: 1/128").is_some());
    assert_eq!(viewport_pixels(&mut h, "09-undo-duplicate.png"), shaded);
    click_button(&mut h, "Redo prop edit");
    click_button(&mut h, "Redo prop edit");
    assert_eq!(viewport_pixels(&mut h, "10-redo-duplicate.png"), duplicated);
    click_button(&mut h, "Save props");
    let saved = fs::read(&sidecar).unwrap();
    let document: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(document["props"].as_array().unwrap().len(), 2);
    for prop in document["props"].as_array().unwrap() {
        assert_eq!(prop["shading"], shading);
        for field in [
            "asset_sha256",
            "license_sha256",
            "cooked_sha256",
            "license_text",
        ] {
            assert_eq!(prop["art"][field], first["props"][0]["art"][field]);
        }
    }
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&prefab).unwrap(), prefab_bytes);
    for (name, bytes) in &source_bytes {
        assert_eq!(fs::read(package.join(name)).unwrap(), *bytes);
    }
    drop(h);
    let hidden_source = root.join("hidden-source");
    let hidden_prefab = root.join("hidden-prefab.json");
    fs::rename(&package, &hidden_source).unwrap();
    fs::rename(&prefab, &hidden_prefab).unwrap();
    assert!(!package.exists() && !prefab.exists());
    let mut h = harness(&scene);
    settle(&mut h);
    assert_eq!(viewport_pixels(&mut h, "11-reopened.png"), duplicated);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(hidden_prefab).unwrap(), prefab_bytes);
    for (name, bytes) in source_bytes {
        assert_eq!(fs::read(hidden_source.join(name)).unwrap(), bytes);
    }
}
