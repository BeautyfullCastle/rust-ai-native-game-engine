//! Actual installed GLB -> Mesh agent animation authoring -> production skinned viewport.
//! Explicit ignored GPU acceptance. No OS-window, physical-GPU or sandbox claim.
#![cfg(all(feature = "mesh-navigation", feature = "mesh-animated-models"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{game::EditorGame, Editor, EditorApp};
use orr_package::{Project, Runtime};
use orr_rhi::Rhi;
use serde_json::{json, Value};
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
        &PathBuf::from(std::env::var_os("ORR_ANIMATED_TEXTURE_TRANSFORM_CAPTURE_DIR").unwrap())
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

fn glb(document: &Value, binary: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(document).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = binary.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut result = Vec::new();
    result.extend_from_slice(b"glTF");
    result.extend_from_slice(&2u32.to_le_bytes());
    result.extend_from_slice(&((28 + json.len() + bin.len()) as u32).to_le_bytes());
    result.extend_from_slice(&(json.len() as u32).to_le_bytes());
    result.extend_from_slice(b"JSON");
    result.extend_from_slice(&json);
    result.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    result.extend_from_slice(b"BIN\0");
    result.extend_from_slice(&bin);
    result
}
fn write_package(root: &Path) {
    fs::create_dir(root).unwrap();
    let original = include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb");
    let json_len = u32::from_le_bytes(original[12..16].try_into().unwrap()) as usize;
    let mut document: Value = serde_json::from_slice(&original[20..20 + json_len]).unwrap();
    let binary_start = 28 + json_len;
    let binary_len = document["buffers"][0]["byteLength"].as_u64().unwrap() as usize;
    let binary = &original[binary_start..binary_start + binary_len];
    // Keep the original asymmetric embedded checker, two joints and both clips.
    fs::write(root.join("identity.glb"), glb(&document, binary)).unwrap();
    let primitive = &document["meshes"][0]["primitives"][0];
    let index = primitive["attributes"]["TEXCOORD_0"].as_u64().unwrap() as usize;
    let accessor = &document["accessors"][index];
    let view = &document["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
    let offset = (view["byteOffset"].as_u64().unwrap()
        + accessor["byteOffset"].as_u64().unwrap_or(0)) as usize;
    let count = accessor["count"].as_u64().unwrap() as usize;
    let mut baked = binary.to_vec();
    for i in 0..count {
        let at = offset + i * 8;
        let u = f32::from_le_bytes(binary[at..at + 4].try_into().unwrap());
        // Literal independent corner oracle: mirror into the middle half of the checker.
        // V bytes untouched; no production UV/transform function participates.
        let expected: f32 = match u {
            0.0 => 0.75,
            1.0 => 0.25,
            _ => panic!("unexpected fixture U"),
        };
        baked[at..at + 4].copy_from_slice(&expected.to_le_bytes());
    }
    fs::write(root.join("baked.glb"), glb(&document, &baked)).unwrap();
    document["extensionsUsed"] = json!(["KHR_texture_transform"]);
    document["extensionsRequired"] = json!(["KHR_texture_transform"]);
    document["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] =
        json!({"KHR_texture_transform":{"offset":[0.75,0.0],"scale":[-0.5,1.0]}});
    fs::write(root.join("transformed.glb"), glb(&document, binary)).unwrap();
    fs::write(
        root.join("LICENSE.txt"),
        include_bytes!("../../orr_model/tests/fixtures/animated_strip_LICENSE.txt"),
    )
    .unwrap();
    fs::write(root.join("orr.package.json"),serde_json::to_vec_pretty(&json!({"schema":1,"name":"animated-transform-oracle","version":"1.0.0","engine":"^0.0.1","capabilities":["animation"],"dependencies":{},"files":["identity.glb","baked.glb","transformed.glb","LICENSE.txt"]})).unwrap()).unwrap();
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
#[test]
#[ignore = "requires actual headless GPU and fresh ORR_ANIMATED_TEXTURE_TRANSFORM_CAPTURE_DIR; actual editor GPU"]
fn installed_animated_transform_matches_literal_uv_oracle_and_reopens_without_sources() {
    let root = PathBuf::from(
        std::env::var_os("ORR_ANIMATED_TEXTURE_TRANSFORM_CAPTURE_DIR")
            .expect("fresh GPU evidence directory required"),
    );
    assert!(root.is_absolute());
    fs::create_dir(&root).unwrap();
    let gpu =
        orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default()).expect("required actual GPU");
    eprintln!(
        "animated transform adapter={} software={}",
        gpu.adapter_name(),
        gpu.is_software()
    );
    drop(gpu);
    let source = root.join("source-package");
    write_package(&source);
    let project = Project::open_for_install(&root, Runtime::content_only().engine_version).unwrap();
    let lock = project.install(std::slice::from_ref(&source)).unwrap();
    project.verify().unwrap();
    // Installation deliberately does not grant asset-load capabilities. This
    // test is compiled only with mesh-animated-models (and its models feature),
    // so reopen with that real host inventory, never package-declared claims.
    let mut runtime = Runtime::content_only();
    runtime
        .capabilities
        .extend(["models".to_owned(), "animation".to_owned()]);
    let project = Project::open(&root, runtime).unwrap();
    let installed = root
        .join(".orr/packages/objects")
        .join(&lock.packages["animated-transform-oracle"].digest);
    for name in ["identity.glb", "baked.glb", "transformed.glb"] {
        assert_eq!(
            project
                .read_asset("animated-transform-oracle", name)
                .unwrap(),
            fs::read(source.join(name)).unwrap()
        );
    }
    fs::rename(&source, root.join("hidden-source-package")).unwrap();
    assert!(!source.exists());
    let imported = |name: &str| {
        orr_model::animation_import::import_with_resolver(
            name,
            &project
                .read_asset("animated-transform-oracle", name)
                .unwrap(),
            |_| panic!("embedded fixture must not resolve externally"),
        )
        .unwrap()
    };
    let identity = imported("identity.glb");
    let transformed = imported("transformed.glb");
    let baked = imported("baked.glb");
    assert_eq!(identity.source().nodes, transformed.source().nodes);
    assert_eq!(identity.source().skins, transformed.source().skins);
    assert_eq!(identity.source().clips, transformed.source().clips);
    assert_eq!(
        baked.source().primitives.len(),
        transformed.source().primitives.len()
    );
    for (oracle, actual) in baked
        .source()
        .primitives
        .iter()
        .zip(&transformed.source().primitives)
    {
        // Asset-specific primitive IDs intentionally differ between the two source files.
        assert_eq!(oracle.node, actual.node);
        assert_eq!(oracle.skin, actual.skin);
        assert_eq!(oracle.vertices, actual.vertices);
        assert_eq!(oracle.indices, actual.indices);
        assert_eq!(oracle.material, actual.material);
    }
    let scene = root.join("mesh.scene.yaml");
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
    click(&mut h, "Mesh agent animation");
    load(&mut h, &installed, "identity.glb");
    let identity_pixels = poses(&mut h, "identity");
    load(&mut h, &installed, "baked.glb");
    let baked_pixels = poses(&mut h, "baked");
    load(&mut h, &installed, "transformed.glb");
    let transformed_pixels = poses(&mut h, "transformed");
    for pose in 0..2 {
        assert_eq!(
            transformed_pixels[pose], baked_pixels[pose],
            "production skinned viewport must match independently prebaked UV at pose {pose}"
        );
        assert!(
            changed_pixel_count(&identity_pixels[pose], &transformed_pixels[pose]) > 20,
            "extension must visibly change checker at pose {pose}"
        );
    }
    assert!(
        changed_pixel_count(&transformed_pixels[0], &transformed_pixels[1]) > 20,
        "clip must visibly deform the posed model"
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    click_button(&mut h, "Clear animation preview");
    click_button(&mut h, "Apply mesh animation");
    click_button(&mut h, "Save mesh animation");
    let sidecar = sidecar_path(&scene, ".mesh-animation.json");
    let saved = fs::read(&sidecar).unwrap();
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(h.state().editor.checksum(), checksum);
    drop(h);
    fs::rename(root.join(".orr"), root.join("hidden-installed-cache")).unwrap();
    assert!(!installed.exists());
    let mut h = harness(&scene);
    settle(&mut h);
    click(&mut h, "Mesh agent animation");
    let reopened = poses(&mut h, "reopened");
    assert_eq!(
        reopened, transformed_pixels,
        "embedded saved animation survives both source and installed cache removal"
    );
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(h.state().editor.checksum(), checksum);
}
