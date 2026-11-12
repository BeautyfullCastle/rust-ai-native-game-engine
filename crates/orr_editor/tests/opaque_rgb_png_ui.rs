//! Opaque RGB8 PNG textures through installed packages and production Editor viewports.
//! Edit-preview GPU evidence only: no native-window or authoritative Play claim.
//! Run with ORR_REQUIRE_GPU=1 and an absolute, entirely absent capture directory.
//! Ignored actual GPU tests; software backend explicitly recorded, never silently skipped.
#![cfg(all(feature = "mesh-navigation", feature = "mesh-animated-models"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "../../orr_model/tests/common/opaque_rgb_fixture.rs"]
mod fixtures;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_editor::{Editor, EditorApp, game::EditorGame};
use orr_package::{Project, Runtime};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
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
    assert!(
        !harness
            .get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "disabled actual button {label}"
    );
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
    assert_eq!(
        harness
            .get_by_role_and_label(Role::TextInput, label)
            .value()
            .as_deref(),
        Some(value),
        "field {label}"
    );
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
    eprintln!("viewport adapter name: {}", viewport.gpu().adapter_name());
    fs::write(
        capture_root().join(format!("{label}.json")),
        serde_json::to_vec_pretty(&json!({
            "viewport_adapter_name": viewport.gpu().adapter_name(),
            "scope": "production EditorApp Edit-preview offscreen GPU readback",
            "physical_os_input": false,
            "native_window": false,
            "authoritative_play": false,
        }))
        .unwrap(),
    )
    .unwrap();
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (target.0 * target.1 * 4) as usize);
    assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    let image =
        egui::ColorImage::from_rgba_unmultiplied([target.0 as usize, target.1 as usize], &pixels);
    orr_editor::app::write_png(
        &PathBuf::from(std::env::var_os("ORR_OPAQUE_RGB_CAPTURE_DIR").unwrap()).join(label),
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

fn harness(scene: &Path, phase: &str) -> Harness<'static, EditorApp> {
    let adapter_record = scene
        .parent()
        .unwrap()
        .join(format!("{phase}-egui-adapter.json"));
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
            fs::write(
                &adapter_record,
                serde_json::to_vec_pretty(&json!({
                    "egui_adapter_name": info.name,
                    "egui_adapter_backend": format!("{:?}", info.backend),
                    "egui_adapter_device_type": format!("{:?}", info.device_type),
                    "software_gpu": info.device_type == egui_wgpu::wgpu::DeviceType::Cpu,
                }))
                .unwrap(),
            )
            .unwrap();
            EditorApp::new(editor, cc.wgpu_render_state.clone())
        })
}

fn load(h: &mut Harness<'_, EditorApp>, package: &Path, asset: &str) {
    type_field(
        h,
        "Animation package source directory",
        &package.display().to_string(),
    );
    type_field(h, "Animation model asset", asset);
    click(h, "I have reviewed rights to use this animation");
    click_button(h, "Load mesh animation");
    assert!(
        !h.get_by_role_and_label(Role::Button, "Apply mesh animation")
            .accesskit_node()
            .is_disabled(),
        "actual imported candidate must be applicable"
    );
}
fn poses(h: &mut Harness<'_, EditorApp>, prefix: &str) -> [Vec<u8>; 2] {
    type_field(h, "Animation preview clip", "0");
    [0, 30].map(|tick| {
        type_field(h, "Animation preview tick", &tick.to_string());
        click_button(h, "Preview animation pose");
        viewport_pixels(h, &format!("{prefix}-{tick}.png"))
    })
}

fn package(root: &Path, animated: bool) -> (PathBuf, PathBuf) {
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    for (name, rgb) in [("rgba.glb", false), ("rgb.glb", true)] {
        fs::write(source.join(name), fixtures::glb(animated, rgb)).unwrap();
    }
    let license: &[u8] = if animated {
        include_bytes!("../../orr_model/tests/fixtures/animated_strip_LICENSE.txt")
    } else {
        b"Original Orrery static fixture asset copyright metadata: MIT OR Apache-2.0. Test-only provenance record, not independent rights verification.\n"
    };
    fs::write(source.join("LICENSE.txt"), license).unwrap();
    fs::write(source.join("orr.package.json"),serde_json::to_vec(&json!({"schema":1,"name":"opaque-rgb-oracle","version":"1.0.0","engine":"^0.0.1","capabilities":[if animated {"animation"} else {"models"}],"dependencies":{},"files":["rgba.glb","rgb.glb","LICENSE.txt"]})).unwrap()).unwrap();
    let project = Project::open_for_install(root, Runtime::content_only().engine_version).unwrap();
    let lock = project.install(std::slice::from_ref(&source)).unwrap();
    project.verify().unwrap();
    let mut runtime = Runtime::content_only();
    runtime
        .capabilities
        .extend(["models".into(), "animation".into()]);
    let project = Project::open(root, runtime).unwrap();
    for name in ["rgba.glb", "rgb.glb"] {
        let bytes = project.read_asset("opaque-rgb-oracle", name).unwrap();
        assert_eq!(bytes, fs::read(source.join(name)).unwrap());
        // Exercise native strict import and cooked reload, not fixture-expected shader data.
        if animated {
            let model = orr_model::animation_import::import_with_resolver(name, &bytes, |_| {
                panic!("embedded GLB")
            })
            .unwrap();
            let cooked = model.to_bytes().unwrap();
            let reopened = orr_model::animation::AnimatedModel::from_bytes(&cooked).unwrap();
            assert_eq!(model.source(), reopened.source());
        } else {
            let model =
                orr_model::import::import_with_resolver(name, &bytes, |_| panic!("embedded GLB"))
                    .unwrap();
            let cooked = model.to_bytes().unwrap();
            let reopened = orr_model::StaticModel::from_bytes(&cooked).unwrap();
            assert_eq!(model.source(), reopened.source());
        }
    }
    let installed = root
        .join(".orr/packages/objects")
        .join(&lock.packages["opaque-rgb-oracle"].digest);
    fs::rename(&source, root.join("hidden-source")).unwrap();
    assert!(!source.exists());
    (source, installed)
}
fn capture_root() -> &'static Path {
    static BASE: OnceLock<PathBuf> = OnceLock::new();
    BASE.get_or_init(|| {
        assert_eq!(
            std::env::var("ORR_REQUIRE_GPU").as_deref(),
            Ok("1"),
            "explicit GPU execution required; missing devices fail rather than skip"
        );
        let base = PathBuf::from(
            std::env::var_os("ORR_OPAQUE_RGB_CAPTURE_DIR")
                .expect("absolute, absent capture root required"),
        );
        assert!(base.is_absolute());
        assert!(
            !base.exists(),
            "capture root must be entirely absent before this test process"
        );
        fs::create_dir_all(base.parent().unwrap()).unwrap();
        fs::create_dir(&base).unwrap();
        base
    })
    .as_path()
}
fn root(kind: &str) -> PathBuf {
    let root = capture_root().join(kind);
    fs::create_dir(&root).expect("case evidence directory must be fresh");
    root
}
fn scene(root: &Path) -> PathBuf {
    let path = root.join("mesh.scene.yaml");
    fs::copy(EditorGame::MeshNavigation.default_scene_path(), &path).unwrap();
    path
}
fn prepare(h: &mut Harness<'_, EditorApp>) {
    settle(h);
    click(h, "Static-mesh navigation");
    click_button(h, "Create ramp and bridge");
    click_button(h, "Place mesh agent");
    menu_click(h, "File", "Save Ctrl+S");
}
#[test]
#[ignore = "requires actual GPU and ORR_OPAQUE_RGB_CAPTURE_DIR; no window claim"]
fn animated_opaque_rgb_png_package_two_poses_save_and_hidden_source_reopen() {
    let root = root("animated");
    let (_, installed) = package(&root, true);
    let scene = scene(&root);
    let mut h = harness(&scene, "initial");
    prepare(&mut h);
    let bytes = fs::read(&scene).unwrap();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    click(&mut h, "Mesh agent animation");
    let empty = viewport_pixels(&mut h, "animated-empty.png");
    load(&mut h, &installed, "rgba.glb");
    let expected = poses(&mut h, "animated-rgba");
    load(&mut h, &installed, "rgb.glb");
    let actual = poses(&mut h, "animated-rgb");
    assert_eq!(
        actual, expected,
        "opaque RGB PNG must render exactly like independently encoded RGBA PNG twin at both poses"
    );
    assert!(
        changed_pixel_count(&actual[0], &actual[1]) > 20,
        "actual skin animation must move pixels"
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    click_button(&mut h, "Clear animation preview");
    assert_eq!(
        viewport_pixels(&mut h, "animated-before-apply.png"),
        empty,
        "loading and previewing must not implicitly Apply the candidate"
    );
    assert!(!sidecar_path(&scene, ".mesh-animation.json").exists());
    assert_eq!(fs::read(&scene).unwrap(), bytes);
    click_button(&mut h, "Apply mesh animation");
    click_button(&mut h, "Save mesh animation");
    let sidecar = sidecar_path(&scene, ".mesh-animation.json");
    let saved = fs::read(&sidecar).unwrap();
    drop(h);
    fs::rename(root.join(".orr"), root.join("hidden-installed-cache")).unwrap();
    assert!(!installed.exists());
    let mut h = harness(&scene, "reopened");
    settle(&mut h);
    click(&mut h, "Mesh agent animation");
    assert_eq!(poses(&mut h, "animated-reopened"), actual);
    assert_eq!(fs::read(sidecar).unwrap(), saved);
    assert_eq!(fs::read(scene).unwrap(), bytes);
    assert_eq!(h.state().editor.checksum(), checksum);
}
#[test]
#[ignore = "requires actual GPU and ORR_OPAQUE_RGB_CAPTURE_DIR; static consumer proof separate from animation"]
fn static_opaque_rgb_png_package_prop_save_and_hidden_source_reopen() {
    let root = root("static");
    let (_, installed) = package(&root, false);
    let scene = scene(&root);
    let mut h = harness(&scene, "initial");
    prepare(&mut h);
    let bytes = fs::read(&scene).unwrap();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    click(&mut h, "Mesh props");
    let empty = viewport_pixels(&mut h, "static-empty.png");
    type_field(
        &mut h,
        "Prop package source directory",
        &installed.display().to_string(),
    );
    type_field(&mut h, "Prop model asset", "rgba.glb");
    type_field(&mut h, "Prop license asset", "LICENSE.txt");
    click(&mut h, "I have reviewed rights to use these props");
    click_button(&mut h, "Load prop candidate");
    assert_eq!(fs::read(&scene).unwrap(), bytes);
    type_field(&mut h, "Prop name", "Opaque RGB prop");
    type_field(&mut h, "Prop position Y", "0.5");
    assert_eq!(
        h.state().viewport3d_gpu().unwrap().gpu().read_rgba8(),
        empty,
        "Load must not implicitly add the prop"
    );
    assert!(!sidecar_path(&scene, ".mesh-props.json").exists());
    click_button(&mut h, "Add prop");
    let expected = viewport_pixels(&mut h, "static-rgba.png");
    assert!(
        changed_pixel_count(&empty, &expected) > 20,
        "static prop must be visible"
    );
    type_field(&mut h, "Prop model asset", "rgb.glb");
    click(&mut h, "I have reviewed rights to use these props");
    click_button(&mut h, "Load prop candidate");
    // Loading a new source resets draft placement; restore the identical control transform.
    assert_eq!(fs::read(&scene).unwrap(), bytes);
    type_field(&mut h, "Prop name", "Opaque RGB prop");
    type_field(&mut h, "Prop position Y", "0.5");
    click_button(&mut h, "Update selected prop");
    let actual = viewport_pixels(&mut h, "static-rgb.png");
    assert_eq!(
        actual, expected,
        "static opaque RGB PNG viewport must match independently encoded RGBA PNG twin"
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    click_button(&mut h, "Save props");
    let sidecar = sidecar_path(&scene, ".mesh-props.json");
    let saved = fs::read(&sidecar).unwrap();
    drop(h);
    fs::rename(root.join(".orr"), root.join("hidden-installed-cache")).unwrap();
    assert!(!installed.exists());
    let mut h = harness(&scene, "reopened");
    settle(&mut h);
    assert_eq!(viewport_pixels(&mut h, "static-reopened.png"), actual);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(fs::read(&scene).unwrap(), bytes);
    assert_eq!(fs::read(sidecar).unwrap(), saved);
}
