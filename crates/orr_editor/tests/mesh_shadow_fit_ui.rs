//! Explicit offscreen static-shadow fit acceptance; synthetic UI, not a native window.
#![cfg(feature = "mesh-navigation")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{
    accesskit::{Role, Toggled},
    Key, Modifiers,
};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{game::EditorGame, Editor, EditorApp};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::{authoring::create_mesh_scene, AgentSpec};
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
    eprintln!("Mesh shadow fit App GPU: {}", viewport.gpu().adapter_name());
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (target.0 * target.1 * 4) as usize);
    assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    let image =
        egui::ColorImage::from_rgba_unmultiplied([target.0 as usize, target.1 as usize], &pixels);
    orr_editor::app::write_png(
        &PathBuf::from(std::env::var_os("ORR_MESH_SHADOW_FIT_CAPTURE_DIR").unwrap()).join(label),
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

fn set_check(h: &mut Harness<'_, EditorApp>, label: &str, value: bool) {
    let checked = h.get_by_label(label).accesskit_node().toggled() == Some(Toggled::True);
    if checked != value {
        click(h, label);
    }
}
fn text(h: &Harness<'_, EditorApp>, label: &str) -> String {
    h.get_by_role_and_label(Role::TextInput, label)
        .value()
        .unwrap()
}
fn ground(scene: &Path) {
    let graph = StaticMeshGraph::bake(
        &[
            [fp!(-8), FP::ZERO, fp!(-6)],
            [fp!(8), FP::ZERO, fp!(-6)],
            [fp!(-8), FP::ZERO, fp!(6)],
            [fp!(8), FP::ZERO, fp!(6)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let mut doc = create_mesh_scene(
        &graph,
        AgentSpec {
            start: [fp!(-6), FP::ZERO, fp!(-4)],
            goal: [fp!(-5), FP::ZERO, fp!(-4)],
            max_vertical: FP::ZERO,
            distance_per_tick: fp!(0.125),
        },
    )
    .unwrap();
    fs::write(scene, doc.save_yaml()).unwrap();
}
fn reflected_package(root: &Path) -> Vec<(String, Vec<u8>)> {
    fs::create_dir(root).unwrap();
    let original = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
    for name in [
        "orr.package.json",
        "foreground.glb",
        "background.glb",
        "LICENSE.txt",
        "generate.py",
    ] {
        fs::copy(original.join(name), root.join(name)).unwrap();
    }
    // UI instance scales must be positive. Exercise a real negative node scale
    // in the admitted GLB, then a positive scaled/transformed prop instance.
    let input = fs::read(root.join("foreground.glb")).unwrap();
    let n = u32::from_le_bytes(input[12..16].try_into().unwrap()) as usize;
    let mut json: serde_json::Value = serde_json::from_slice(&input[20..20 + n]).unwrap();
    json["nodes"][0]["scale"] = serde_json::json!([-1, 1, 1]);
    let mut payload = serde_json::to_vec(&json).unwrap();
    while !payload.len().is_multiple_of(4) {
        payload.push(b' ');
    }
    let mut output = Vec::new();
    output.extend_from_slice(b"glTF");
    output.extend_from_slice(&2_u32.to_le_bytes());
    output.extend_from_slice(&((20 + payload.len() + input.len() - 20 - n) as u32).to_le_bytes());
    output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    output.extend_from_slice(&0x4e4f534a_u32.to_le_bytes());
    output.extend_from_slice(&payload);
    output.extend_from_slice(&input[20 + n..]);
    fs::write(root.join("foreground.glb"), output).unwrap();
    fs::read_dir(root)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (
                p.file_name().unwrap().to_str().unwrap().into(),
                fs::read(p).unwrap(),
            )
        })
        .collect()
}
#[test]
#[ignore = "actual GPU; requires fresh absolute ORR_MESH_SHADOW_FIT_CAPTURE_DIR"]
fn actual_static_shadow_fit_stages_then_applies_receiver_shadow_and_reopens() {
    let root = PathBuf::from(
        std::env::var_os("ORR_MESH_SHADOW_FIT_CAPTURE_DIR").expect("capture directory required"),
    );
    assert!(root.is_absolute());
    fs::create_dir(&root).unwrap();
    let package = root.join("source-package");
    let source = reflected_package(&package);
    let scene = root.join("level.scene.yaml");
    ground(&scene);
    let mut h = harness(&scene);
    settle(&mut h);
    click(&mut h, "Mesh game camera");
    for (label, value) in [
        ("Game camera target X", "1"),
        ("Game camera target Y", "0"),
        ("Game camera target Z", "0"),
        ("Game camera yaw (degrees)", "30"),
        ("Game camera pitch (degrees)", "50"),
        ("Game camera distance", "20"),
        ("Game camera orthographic half height", "7"),
    ] {
        type_field(&mut h, label, value);
    }
    set_check(&mut h, "Follow mesh agent", false);
    click(&mut h, "Orthographic game camera");
    click_button(&mut h, "Apply game camera");
    click_button(&mut h, "Save game camera");
    click(&mut h, "Static-mesh navigation");
    click(&mut h, "Mesh props");
    type_field(
        &mut h,
        "Prop package source directory",
        package.to_str().unwrap(),
    );
    type_field(&mut h, "Prop model asset", "foreground.glb");
    type_field(&mut h, "Prop license asset", "LICENSE.txt");
    click(&mut h, "I have reviewed rights to use these props");
    click_button(&mut h, "Load prop candidate");
    for (label, value) in [
        ("Prop name", "Shadow caster"),
        ("Prop position X", "2"),
        ("Prop position Y", "0.5"),
        ("Prop position Z", "0"),
        ("Prop scale", "1.5"),
    ] {
        type_field(&mut h, label, value);
    }
    click_button(&mut h, "Add prop");
    click_button(&mut h, "Save props");
    click(&mut h, "Mesh game lighting");
    for (label, value) in [
        ("Lighting direction X", "-0.7"),
        ("Lighting direction Y", "-1"),
        ("Lighting direction Z", "-0.3"),
        ("Light color R", "1"),
        ("Light color G", "1"),
        ("Light color B", "1"),
        ("Light intensity", "1.5"),
        ("Ambient", "0.15"),
        ("Exposure", "1"),
        ("Shadow center X", "100"),
        ("Shadow center Y", "0"),
        ("Shadow center Z", "100"),
        ("Shadow radius", "0.5"),
    ] {
        type_field(&mut h, label, value);
    }
    set_check(&mut h, "Enable shadows", true);
    set_check(&mut h, "Enable tonemapping", true);
    click_button(&mut h, "Apply game lighting");
    click_button(&mut h, "Save game lighting");
    let lighting = sidecar_path(&scene, ".mesh-lighting.json");
    let camera = sidecar_path(&scene, ".mesh-camera.json");
    let props = sidecar_path(&scene, ".mesh-props.json");
    let old_lighting = fs::read(&lighting).unwrap();
    let camera_bytes = fs::read(&camera).unwrap();
    let props_bytes = fs::read(&props).unwrap();
    let scene_bytes = fs::read(&scene).unwrap();
    let history = h.state().editor.history().clone();
    let checksum = h.state().editor.checksum();
    let tiny = viewport_pixels(&mut h, "01-tiny-outside-volume.png");
    let other_fields = [
        "Lighting direction X",
        "Lighting direction Y",
        "Lighting direction Z",
        "Light color R",
        "Light color G",
        "Light color B",
        "Light intensity",
        "Ambient",
        "Exposure",
    ];
    let fields_before: Vec<_> = other_fields.iter().map(|label| text(&h, label)).collect();
    click_button(&mut h, "Fit static shadow bounds");
    assert!(text(&h, "Shadow radius").parse::<f32>().unwrap() > 8.0);
    assert!(text(&h, "Shadow center X").parse::<f32>().unwrap().abs() < 10.0);
    assert_eq!(
        other_fields
            .iter()
            .map(|label| text(&h, label))
            .collect::<Vec<_>>(),
        fields_before
    );
    assert_eq!(
        h.get_by_label("Enable shadows").accesskit_node().toggled(),
        Some(Toggled::True)
    );
    assert_eq!(viewport_pixels(&mut h, "02-fit-draft.png"), tiny);
    assert_eq!(fs::read(&lighting).unwrap(), old_lighting);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(h.state().editor.checksum(), checksum);
    click_button(&mut h, "Apply game lighting");
    let fitted = viewport_pixels(&mut h, "03-fitted-applied.png");
    assert!(
        changed_pixel_count(&tiny, &fitted) > 20,
        "fitting must visibly change shadow coverage"
    );
    assert_eq!(fs::read(&lighting).unwrap(), old_lighting);
    click_button(&mut h, "Undo game lighting");
    assert_eq!(viewport_pixels(&mut h, "04-undo.png"), tiny);
    click_button(&mut h, "Redo game lighting");
    assert_eq!(viewport_pixels(&mut h, "05-redo.png"), fitted);
    click_button(&mut h, "Save game lighting");
    let saved = fs::read(&lighting).unwrap();
    assert_ne!(saved, old_lighting);
    set_check(&mut h, "Enable shadows", false);
    click_button(&mut h, "Apply game lighting");
    let prop_off = viewport_pixels(&mut h, "06-prop-shadow-off.png");
    click_button(&mut h, "Undo game lighting");
    assert_eq!(viewport_pixels(&mut h, "07-restored-saved-fit.png"), fitted);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&camera).unwrap(), camera_bytes);
    assert_eq!(fs::read(&props).unwrap(), props_bytes);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    drop(h);
    let hidden = root.join("hidden-source-package");
    fs::rename(&package, &hidden).unwrap();
    let mut reopened = harness(&scene);
    settle(&mut reopened);
    assert_eq!(
        viewport_pixels(&mut reopened, "08-source-hidden-reopen.png"),
        fitted
    );
    assert_eq!(reopened.state().editor.checksum(), checksum);
    drop(reopened);
    // Matching no-prop controls remove both object silhouette and unrelated
    // agent/graph shadows from the receiver-shadow oracle below.
    let control = root.join("no-prop-control");
    fs::create_dir(&control).unwrap();
    let control_scene = control.join("level.scene.yaml");
    fs::write(&control_scene, &scene_bytes).unwrap();
    fs::write(
        sidecar_path(&control_scene, ".mesh-camera.json"),
        &camera_bytes,
    )
    .unwrap();
    fs::write(sidecar_path(&control_scene, ".mesh-lighting.json"), &saved).unwrap();
    let mut control_h = harness(&control_scene);
    settle(&mut control_h);
    let empty_on = viewport_pixels(&mut control_h, "09-no-prop-shadow-on.png");
    click(&mut control_h, "Mesh game lighting");
    set_check(&mut control_h, "Enable shadows", false);
    click_button(&mut control_h, "Apply game lighting");
    let empty_off = viewport_pixels(&mut control_h, "10-no-prop-shadow-off.png");
    let mut receiver_pixels = 0;
    for (((on, off), ground_on), ground_off) in fitted
        .as_chunks::<4>().0.iter()
        .zip(prop_off.as_chunks::<4>().0.iter())
        .zip(empty_on.as_chunks::<4>().0.iter())
        .zip(empty_off.as_chunks::<4>().0.iter())
    {
        let same_ground = (0..3).all(|i| off[i].abs_diff(ground_off[i]) <= 2);
        let no_other_shadow = (0..3).all(|i| ground_on[i].abs_diff(ground_off[i]) <= 2);
        let darkening: i32 = (0..3).map(|i| i32::from(off[i]) - i32::from(on[i])).sum();
        if same_ground && no_other_shadow && darkening > 30 {
            receiver_pixels += 1;
        }
    }
    assert!(receiver_pixels>20,"expected prop-cast shadow on unchanged ground outside object silhouette, got {receiver_pixels}");
    eprintln!("actual static-fit receiver pixels={receiver_pixels}");
    assert_eq!(fs::read(&lighting).unwrap(), saved);
    assert_eq!(fs::read(&props).unwrap(), props_bytes);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    for (name, bytes) in source {
        assert_eq!(fs::read(hidden.join(name)).unwrap(), bytes);
    }
}
