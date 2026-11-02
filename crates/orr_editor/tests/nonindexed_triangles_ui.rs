//! Non-indexed TRIANGLES through installed packages and production Editor viewports.
//! Ignored actual GPU tests; software backend explicitly recorded, never silently skipped.
#![cfg(all(feature = "mesh-navigation", feature = "mesh-animated-models"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "../../orr_model/tests/common/nonindexed_fixture.rs"]
mod fixtures;
use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{game::EditorGame, Editor, EditorApp};
use orr_package::{Project, Runtime};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
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
        &PathBuf::from(std::env::var_os("ORR_NONINDEXED_CAPTURE_DIR").unwrap()).join(label),
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
    let make = if animated {
        fixtures::animated_glb
    } else {
        fixtures::static_glb
    };
    for (name, omitted) in [("indexed.glb", false), ("omitted.glb", true)] {
        fs::write(source.join(name), make(omitted)).unwrap();
    }
    let license: &[u8] = if animated {
        include_bytes!("../../orr_model/tests/fixtures/animated_strip_LICENSE.txt")
    } else {
        b"Original Orrery static fixture asset copyright metadata: MIT OR Apache-2.0. Test-only provenance record, not independent rights verification.\n"
    };
    fs::write(source.join("LICENSE.txt"), license).unwrap();
    fs::write(source.join("orr.package.json"),serde_json::to_vec(&json!({"schema":1,"name":"nonindexed-oracle","version":"1.0.0","engine":"^0.0.1","capabilities":[if animated {"animation"} else {"models"}],"dependencies":{},"files":["indexed.glb","omitted.glb","LICENSE.txt"]})).unwrap()).unwrap();
    let project = Project::open_for_install(root, Runtime::content_only().engine_version).unwrap();
    let lock = project.install(std::slice::from_ref(&source)).unwrap();
    project.verify().unwrap();
    let mut runtime = Runtime::content_only();
    runtime
        .capabilities
        .extend(["models".into(), "animation".into()]);
    let project = Project::open(root, runtime).unwrap();
    for name in ["indexed.glb", "omitted.glb"] {
        let bytes = project.read_asset("nonindexed-oracle", name).unwrap();
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
        .join(&lock.packages["nonindexed-oracle"].digest);
    fs::rename(&source, root.join("hidden-source")).unwrap();
    assert!(!source.exists());
    (source, installed)
}
fn root(kind: &str) -> PathBuf {
    let base = PathBuf::from(
        std::env::var_os("ORR_NONINDEXED_CAPTURE_DIR").expect("absolute capture root required"),
    );
    assert!(base.is_absolute());
    fs::create_dir_all(&base).unwrap();
    let root = base.join(kind);
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
#[ignore = "requires actual GPU and ORR_NONINDEXED_CAPTURE_DIR; no window claim"]
fn animated_nonindexed_package_two_poses_save_and_hidden_source_reopen() {
    let root = root("animated");
    let (_, installed) = package(&root, true);
    let scene = scene(&root);
    let mut h = harness(&scene);
    prepare(&mut h);
    let bytes = fs::read(&scene).unwrap();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    click(&mut h, "Mesh agent animation");
    load(&mut h, &installed, "indexed.glb");
    let expected = poses(&mut h, "animated-indexed");
    load(&mut h, &installed, "omitted.glb");
    let actual = poses(&mut h, "animated-omitted");
    assert_eq!(actual,expected,"omitted indices must render exactly like independently expanded sequential-index twin at both poses");
    assert!(
        changed_pixel_count(&actual[0], &actual[1]) > 20,
        "actual skin animation must move pixels"
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    click_button(&mut h, "Clear animation preview");
    click_button(&mut h, "Apply mesh animation");
    click_button(&mut h, "Save mesh animation");
    let sidecar = sidecar_path(&scene, ".mesh-animation.json");
    let saved = fs::read(&sidecar).unwrap();
    drop(h);
    fs::rename(root.join(".orr"), root.join("hidden-installed-cache")).unwrap();
    assert!(!installed.exists());
    let mut h = harness(&scene);
    settle(&mut h);
    click(&mut h, "Mesh agent animation");
    assert_eq!(poses(&mut h, "animated-reopened"), actual);
    assert_eq!(fs::read(sidecar).unwrap(), saved);
    assert_eq!(fs::read(scene).unwrap(), bytes);
    assert_eq!(h.state().editor.checksum(), checksum);
}
#[test]
#[ignore = "requires actual GPU and ORR_NONINDEXED_CAPTURE_DIR; static consumer proof separate from animation"]
fn static_nonindexed_package_prop_save_and_hidden_source_reopen() {
    let root = root("static");
    let (_, installed) = package(&root, false);
    let scene = scene(&root);
    let mut h = harness(&scene);
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
    type_field(&mut h, "Prop model asset", "indexed.glb");
    type_field(&mut h, "Prop license asset", "LICENSE.txt");
    click(&mut h, "I have reviewed rights to use these props");
    click_button(&mut h, "Load prop candidate");
    type_field(&mut h, "Prop name", "Triangle prop");
    type_field(&mut h, "Prop position Y", "0.5");
    click_button(&mut h, "Add prop");
    let expected = viewport_pixels(&mut h, "static-indexed.png");
    assert!(
        changed_pixel_count(&empty, &expected) > 20,
        "static prop must be visible"
    );
    type_field(&mut h, "Prop model asset", "omitted.glb");
    click(&mut h, "I have reviewed rights to use these props");
    click_button(&mut h, "Load prop candidate");
    // Loading a new source resets draft placement; restore the identical control transform.
    type_field(&mut h, "Prop name", "Triangle prop");
    type_field(&mut h, "Prop position Y", "0.5");
    click_button(&mut h, "Update selected prop");
    let actual = viewport_pixels(&mut h, "static-omitted.png");
    assert_eq!(
        actual, expected,
        "static nonindexed actual viewport must match explicit twin"
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    click_button(&mut h, "Save props");
    let sidecar = sidecar_path(&scene, ".mesh-props.json");
    let saved = fs::read(&sidecar).unwrap();
    drop(h);
    fs::rename(root.join(".orr"), root.join("hidden-installed-cache")).unwrap();
    assert!(!installed.exists());
    let mut h = harness(&scene);
    settle(&mut h);
    assert_eq!(viewport_pixels(&mut h, "static-reopened.png"), actual);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(fs::read(&scene).unwrap(), bytes);
    assert_eq!(fs::read(sidecar).unwrap(), saved);
}
