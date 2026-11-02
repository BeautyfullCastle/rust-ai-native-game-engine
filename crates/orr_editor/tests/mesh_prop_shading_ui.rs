//! Actual scalar-prop authoring and production viewport differential acceptance.
//! Software GPU is explicit; controls prove isolation/reversibility, not a BRDF formula oracle.
#![cfg(feature = "mesh-prop-shading")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{app::LBL_STOP, game::EditorGame, Editor, EditorApp, Mode};
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
        &PathBuf::from(std::env::var_os("ORR_MESH_PROP_SHADING_CAPTURE_DIR").unwrap()).join(label),
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

fn mask(before: &[u8], after: &[u8]) -> Vec<usize> {
    assert_eq!(before.len(), after.len());
    before
        .as_chunks::<4>()
        .0
        .iter()
        .zip(after.as_chunks::<4>().0.iter())
        .enumerate()
        .filter_map(|(i, (a, b))| {
            a[..3]
                .iter()
                .zip(&b[..3])
                .any(|(a, b)| a.abs_diff(*b) > 12)
                .then_some(i)
        })
        .collect()
}
fn same_mask(before: &[u8], after: &[u8], pixels: &[usize]) {
    assert!(pixels.len() > 20, "control prop must cover visible pixels");
    for &i in pixels {
        assert_eq!(
            &before[i * 4..i * 4 + 4],
            &after[i * 4..i * 4 + 4],
            "unmodified prop pixel {i} changed"
        );
    }
}
fn select(h: &mut Harness<'_, EditorApp>, name: &str, id: u64) {
    click(h, &format!("{name} · prop-{id:016x}"));
}
fn shade(h: &mut Harness<'_, EditorApp>, metallic: &str, roughness: &str) {
    if h.query_by_label("Use scalar shading for prop slot")
        .is_none()
    {
        click(h, "Scalar material shading");
    }
    click(h, "Use scalar shading for prop slot");
    type_field(h, "Prop slot metallic", metallic);
    type_field(h, "Prop slot roughness", roughness);
}
#[test]
#[ignore = "requires actual GPU and fresh ORR_MESH_PROP_SHADING_CAPTURE_DIR"]
fn selected_prop_scalar_shading_is_atomic_isolated_saved_and_reopened() {
    let root = PathBuf::from(
        std::env::var_os("ORR_MESH_PROP_SHADING_CAPTURE_DIR")
            .expect("fresh capture directory required"),
    );
    assert!(root.is_absolute());
    fs::create_dir(&root).unwrap();
    let package = root.join("package");
    fs::create_dir(&package).unwrap();
    let originals = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
    let mut source_bytes = Vec::new();
    for file in [
        "orr.package.json",
        "foreground.glb",
        "background.glb",
        "LICENSE.txt",
        "generate.py",
    ] {
        let bytes = fs::read(originals.join(file)).unwrap();
        fs::write(package.join(file), &bytes).unwrap();
        source_bytes.push((file, bytes));
    }
    let scene = root.join("props.scene.yaml");
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
    let empty = viewport_pixels(&mut h, "00-no-props.png");
    type_field(
        &mut h,
        "Prop package source directory",
        &package.display().to_string(),
    );
    type_field(&mut h, "Prop model asset", "foreground.glb");
    type_field(&mut h, "Prop license asset", "LICENSE.txt");
    click(&mut h, "I have reviewed rights to use these props");
    click_button(&mut h, "Load prop candidate");
    for (field, value) in [
        ("Prop name", "A"),
        ("Prop position X", "-3"),
        ("Prop position Y", "0.25"),
        ("Prop position Z", "1"),
        ("Prop scale", "1"),
    ] {
        type_field(&mut h, field, value);
    }
    click_button(&mut h, "Add prop");
    let a_only = viewport_pixels(&mut h, "01-a-only.png");
    // Same immutable source asset, distinct instances and positions.
    type_field(&mut h, "Prop name", "B");
    type_field(&mut h, "Prop position X", "3");
    click_button(&mut h, "Add prop");
    let legacy = viewport_pixels(&mut h, "02-two-legacy.png");
    let a_mask = mask(&empty, &a_only);
    let b_mask = mask(&a_only, &legacy);
    assert!(a_mask.len() > 20 && b_mask.len() > 20);
    assert!(
        a_mask.iter().all(|p| !b_mask.contains(p)),
        "fixture instances must not overlap in screen space"
    );
    click_button(&mut h, "Save props");
    let sidecar = sidecar_path(&scene, ".mesh-props.json");
    let legacy_saved = fs::read(&sidecar).unwrap();
    select(&mut h, "A", 1);
    shade(&mut h, "1", "0.15");
    assert_eq!(
        viewport_pixels(&mut h, "03-a-draft.png"),
        legacy,
        "draft controls must not alter applied viewport"
    );
    assert_eq!(fs::read(&sidecar).unwrap(), legacy_saved);
    click_button(&mut h, "Update selected prop");
    let a_shaded = viewport_pixels(&mut h, "04-a-shaded.png");
    assert!(
        changed_pixel_count(&legacy, &a_shaded) > 20,
        "opt-in shading must visibly affect A"
    );
    same_mask(&legacy, &a_shaded, &b_mask);
    assert_eq!(
        fs::read(&sidecar).unwrap(),
        legacy_saved,
        "Update is not Save"
    );
    click_button(&mut h, "Undo prop edit");
    assert_eq!(viewport_pixels(&mut h, "05-undo.png"), legacy);
    click_button(&mut h, "Redo prop edit");
    assert_eq!(viewport_pixels(&mut h, "06-redo.png"), a_shaded);
    select(&mut h, "B", 2);
    shade(&mut h, "0", "0.85");
    assert_eq!(viewport_pixels(&mut h, "07-b-draft.png"), a_shaded);
    click_button(&mut h, "Update selected prop");
    let both = viewport_pixels(&mut h, "08-both-shaded.png");
    assert!(
        changed_pixel_count(&a_shaded, &both) > 20,
        "B opt-in must not reuse A or legacy material cache"
    );
    same_mask(&a_shaded, &both, &a_mask);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    click_button(&mut h, "Save props");
    let saved = fs::read(&sidecar).unwrap();
    let value: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(value["props"].as_array().unwrap().len(), 2);
    assert_ne!(value["props"][0]["shading"], value["props"][1]["shading"]);
    assert!(!value["props"][0]["shading"].is_null());
    let legacy_value: Value = serde_json::from_slice(&legacy_saved).unwrap();
    for i in 0..2 {
        assert_eq!(
            value["props"][i]["art"], legacy_value["props"][i]["art"],
            "shading must preserve source art/license snapshot"
        );
    }
    for (name, bytes) in &source_bytes {
        assert_eq!(fs::read(package.join(name)).unwrap(), *bytes);
    }
    drop(h);
    fs::rename(&package, root.join("hidden-package")).unwrap();
    assert!(!package.exists());
    let mut h = harness(&scene);
    settle(&mut h);
    assert_eq!(viewport_pixels(&mut h, "09-reopened.png"), both);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    click(&mut h, "Mesh props");
    select(&mut h, "B", 2);
    if h.query_by_label("Use scalar shading for prop slot")
        .is_none()
    {
        click(&mut h, "Scalar material shading");
    }
    // Paused Play at the identical initial Frame exercises the normal Play viewport without elapsed-time motion.
    h.state_mut().editor.step(0);
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        settle(&mut h);
        if h.state().editor.mode() == Mode::Play
            && h.state()
                .editor
                .snapshot()
                .is_some_and(|s| s.predicted().tick() == 0)
        {
            break;
        }
        assert!(Instant::now() < until);
    }
    assert!(h
        .get_by_role_and_label(Role::TextInput, "Prop slot metallic")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .get_by_role_and_label(Role::Button, "Update selected prop")
        .accesskit_node()
        .is_disabled());
    let play = viewport_pixels(&mut h, "10-play-zero.png");
    same_mask(&both, &play, &a_mask);
    same_mask(&both, &play, &b_mask);
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    click_button(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
}
