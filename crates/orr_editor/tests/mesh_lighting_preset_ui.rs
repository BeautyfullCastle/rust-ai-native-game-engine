#![cfg(feature = "mesh-navigation")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

//! Opt-in production EditorApp acceptance using synthetic egui input and software Vulkan.
//! This is not hardware-GPU, physical operating-system-input, or runtime/export evidence.
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use egui_wgpu::{WgpuSetupCreateNew, wgpu};
use orr_editor::{Editor, EditorApp, app::LBL_PLAY, game::EditorGame};
use orr_fp::{FP, fp};
use orr_navigation::{AgentProfile, mesh::StaticMeshGraph};
use orr_remote::mesh_navigation::{AgentSpec, authoring::create_mesh_scene, mesh_from_view};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

fn fixture(path: &Path, width: FP) {
    let graph = StaticMeshGraph::bake(
        &[
            [FP::ZERO; 3],
            [width, FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(4)],
            [width, FP::ZERO, fp!(4)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let mut document = create_mesh_scene(
        &graph,
        AgentSpec {
            start: [fp!(1), FP::ZERO, fp!(1)],
            goal: [fp!(3), FP::ZERO, fp!(3)],
            max_vertical: FP::ZERO,
            distance_per_tick: fp!(0.125),
        },
    )
    .unwrap();
    fs::write(path, document.save_yaml()).unwrap();
}

fn harness(path: &Path, game: EditorGame) -> Harness<'static, EditorApp> {
    let mut editor = Editor::open_game(path, game).unwrap();
    editor.sync();
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan && info.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "software Vulkan adapter required".to_owned())
    }));
    Harness::builder()
        .with_size([1400.0, 1050.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu_setup(setup.into())
        .build_eframe(move |context| EditorApp::new(editor, context.wgpu_render_state.clone()))
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(10);
    h.get_by_label(label).click_accesskit();
    settle(h);
}

fn field(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(10);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    settle(h);
}

fn text(h: &Harness<'_, EditorApp>, label: &str) -> String {
    h.get_by_role_and_label(Role::TextInput, label)
        .value()
        .unwrap()
        .to_owned()
}

fn menu(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label("File").click_accesskit();
    h.run_steps(2);
    h.get_by_label(label).click_accesskit();
    settle(h);
}

fn open_mesh(h: &mut Harness<'_, EditorApp>, scene: &Path) {
    menu(h, "Open mesh scene…");
    field(h, "Scene path", &scene.display().to_string());
    click(h, "OK");
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        settle(h);
        if h.state().editor.game() == EditorGame::MeshNavigation
            && h.state().editor.path().as_deref() == Some(scene)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "opening the target mesh scene did not complete: {:?}",
        h.state().editor.path()
    );
}

fn open_lighting(h: &mut Harness<'_, EditorApp>) {
    if h.query_by_label("Lighting direction X").is_none() {
        click(h, "Mesh game lighting");
    }
}

fn sidecar(scene: &Path) -> PathBuf {
    PathBuf::from(format!("{}.mesh-lighting.json", scene.display()))
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn pixels(h: &mut Harness<'_, EditorApp>, root: &Path, name: &str) -> Vec<u8> {
    let target = (700, 520);
    for _ in 0..6 {
        settle(h);
        let actual = h.state().viewport3d_gpu().unwrap().gpu().target().size();
        if actual == target {
            break;
        }
        let window = h.ctx.viewport_rect().size();
        let scale = h.ctx.pixels_per_point();
        h.set_size(egui::vec2(
            window.x + (target.0 as f32 - actual.0 as f32) / scale,
            window.y + (target.1 as f32 - actual.1 as f32) / scale,
        ));
    }
    settle(h);
    let viewport = h
        .state()
        .viewport3d_gpu()
        .expect("production Mesh viewport");
    assert_eq!(viewport.gpu().target().size(), target);
    let bytes = viewport.gpu().read_rgba8();
    assert_eq!(bytes.len(), (target.0 * target.1 * 4) as usize);
    let image =
        egui::ColorImage::from_rgba_unmultiplied([target.0 as usize, target.1 as usize], &bytes);
    orr_editor::app::write_png(&root.join(format!("{name}.png")), &image).unwrap();
    bytes
}

fn capture_preset_controls(h: &mut Harness<'_, EditorApp>, root: &Path) -> serde_json::Value {
    h.set_size(egui::vec2(1400.0, 1050.0));
    settle(h);
    h.get_by_label_contains("Lighting preset staged;")
        .scroll_to_me();
    h.run_steps(20);
    let inspector = egui::containers::panel::PanelState::load(&h.ctx, egui::Id::new("inspector"))
        .expect("production inspector bounds")
        .outer_rect;
    let screen = h.ctx.viewport_rect();
    assert!(
        (inspector.width() - 340.0).abs() < 1.0,
        "preset controls must not widen the inspector"
    );
    let mut controls = serde_json::Map::new();
    for label in [
        "Lighting preset file",
        "Save applied lighting preset",
        "Load lighting preset",
    ] {
        let rect = if label == "Lighting preset file" {
            h.get_by_role_and_label(Role::TextInput, label).rect()
        } else {
            h.get_by_role_and_label(Role::Button, label).rect()
        };
        assert!(
            rect.min.x >= inspector.min.x - 0.5 && rect.max.x <= inspector.max.x + 0.5,
            "{label} must fit horizontally inside the inspector: {rect:?} vs {inspector:?}"
        );
        assert!(
            rect.min.y >= inspector.min.y - 0.5 && rect.max.y <= inspector.max.y + 0.5,
            "{label} must be visible vertically inside the inspector: {rect:?} vs {inspector:?}"
        );
        assert!(
            screen.contains_rect(rect),
            "{label} must be visible in the full editor screenshot"
        );
        controls.insert(
            label.into(),
            serde_json::json!([rect.min.x, rect.min.y, rect.max.x, rect.max.y]),
        );
    }
    let status = h.get_by_label_contains("Lighting preset staged;").rect();
    assert!(
        inspector.contains_rect(status) && screen.contains_rect(status),
        "staged status must wrap inside the visible inspector"
    );
    let image = h.render().expect("full production EditorApp screenshot");
    let rgba = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    );
    orr_editor::app::write_png(&root.join("09-preset-controls-layout.png"), &rgba).unwrap();
    serde_json::json!({
        "screenshot": "09-preset-controls-layout.png", "controls_visible_and_inside_inspector": true,
        "inspector_width": inspector.width(), "controls": controls,
        "staged_status_bounds": [status.min.x, status.min.y, status.max.x, status.max.y]
    })
}

fn changed_pixels(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>()
        .0
        .iter()
        .zip(right.as_chunks::<4>().0.iter())
        .filter(|(a, b)| {
            a[..3]
                .iter()
                .zip(b[..3].iter())
                .any(|(a, b)| a.abs_diff(*b) > 12)
        })
        .count()
}

fn bounds(h: &Harness<'_, EditorApp>) -> [String; 4] {
    [
        "Shadow center X",
        "Shadow center Y",
        "Shadow center Z",
        "Shadow radius",
    ]
    .map(|label| text(h, label))
}

fn all_lighting_fields(h: &Harness<'_, EditorApp>) -> Vec<String> {
    [
        "Lighting direction X",
        "Lighting direction Y",
        "Lighting direction Z",
        "Light color R",
        "Light color G",
        "Light color B",
        "Light intensity",
        "Sky color R",
        "Sky color G",
        "Sky color B",
        "Ground color R",
        "Ground color G",
        "Ground color B",
        "Ambient",
        "Shadow center X",
        "Shadow center Y",
        "Shadow center Z",
        "Shadow radius",
        "Exposure",
    ]
    .map(|label| text(h, label))
    .to_vec()
}

fn history_buttons(h: &Harness<'_, EditorApp>) -> [bool; 2] {
    ["Undo game lighting", "Redo game lighting"]
        .map(|label| h.get_by_label(label).accesskit_node().is_disabled())
}

#[test]
#[ignore = "opt-in software Vulkan EditorApp acceptance; set ORR_MESH_LIGHTING_PRESET_CAPTURE_DIR"]
fn portable_lighting_preset_cross_scene_stage_apply_undo_save_and_fresh_reopen() {
    assert_eq!(
        std::env::var("ORR_MESH_LIGHTING_PRESET_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    assert_eq!(std::env::var("WGPU_BACKEND").as_deref(), Ok("vulkan"));
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("software Vulkan preflight");
    assert!(
        gpu.is_software(),
        "this acceptance explicitly requires software GPU"
    );
    let adapter = gpu.adapter_name().to_owned();
    eprintln!("Lighting preset acceptance software GPU: {adapter}");
    drop(gpu);
    let root = PathBuf::from(std::env::var_os("ORR_MESH_LIGHTING_PRESET_CAPTURE_DIR").unwrap());
    assert!(root.is_absolute());
    fs::create_dir(&root).expect("fresh evidence directory");
    let root = root.canonicalize().unwrap();
    let scene_a = root.join("source-a.scene.yaml");
    let scene_b = root.join("target-b.scene.yaml");
    let physics = root.join("physics.scene.yaml");
    fs::copy(EditorGame::PhysGame.default_scene_path(), &physics).unwrap();
    fixture(&scene_a, fp!(4));
    fixture(&scene_b, fp!(8));
    let preset = root.join("authored.mesh-lighting-preset.json");
    let mut h = harness(&scene_a, EditorGame::MeshNavigation);
    settle(&mut h);
    assert!(h.state().has_gpu());
    open_lighting(&mut h);
    let graph_a = mesh_from_view(h.state().editor.snapshot().unwrap().predicted())
        .unwrap()
        .graph
        .revision();
    for (label, value) in [
        ("Lighting direction X", "-0.7"),
        ("Lighting direction Y", "-0.45"),
        ("Light color R", "0.15"),
        ("Light color G", "0.35"),
        ("Light color B", "0.95"),
        ("Light intensity", "3.5"),
        ("Ambient", "0.07"),
        ("Exposure", "1.7"),
        ("Shadow center X", "90"),
        ("Shadow radius", "120"),
    ] {
        field(&mut h, label, value);
    }
    click(&mut h, "Apply game lighting");
    let source_pixels = pixels(&mut h, &root, "01-source-authored");
    let source_frame = h.state().editor.checksum();
    field(
        &mut h,
        "Lighting preset file",
        &preset.display().to_string(),
    );
    click(&mut h, "Save applied lighting preset");
    assert!(
        h.query_by_label_contains("Saved applied lighting to a new portable preset;")
            .is_some()
    );
    let preset_bytes = fs::read(&preset).unwrap();
    assert!(
        !sidecar(&scene_a).exists(),
        "saving preset must not save workspace lighting"
    );
    assert_eq!(h.state().editor.checksum(), source_frame);
    let portable: serde_json::Value = serde_json::from_slice(&preset_bytes).unwrap();
    assert_eq!(portable["identity"], "ORR_MESH_LIGHTING_PRESET_V1");
    assert!(portable["lighting"].get("shadow_center").is_none());
    assert!(portable["lighting"].get("shadow_radius").is_none());
    assert!(portable.get("scene_filename").is_none());
    assert!(portable.get("graph_revision").is_none());
    click(&mut h, "Save applied lighting preset");
    assert!(
        h.query_by_label_contains("Save lighting preset failed:")
            .is_some()
    );
    assert_eq!(fs::read(&preset).unwrap(), preset_bytes);
    menu(&mut h, "Save mesh workspace");
    let a_scene_bytes = fs::read(&scene_a).unwrap();
    let a_sidecar_bytes = fs::read(sidecar(&scene_a)).unwrap();

    open_mesh(&mut h, &scene_b);
    open_lighting(&mut h);
    let graph_b = mesh_from_view(h.state().editor.snapshot().unwrap().predicted())
        .unwrap()
        .graph
        .revision();
    assert_ne!(
        graph_a, graph_b,
        "preset must cross distinct graph identities"
    );
    for (label, value) in [
        ("Shadow center X", "3"),
        ("Shadow center Y", "1.25"),
        ("Shadow center Z", "2"),
        ("Shadow radius", "18"),
    ] {
        field(&mut h, label, value);
    }
    click(&mut h, "Apply game lighting");
    menu(&mut h, "Save mesh workspace");
    let target_bounds = bounds(&h);
    let baseline = pixels(&mut h, &root, "02-target-before-load");
    let frame_before = h.state().editor.checksum();
    let history_before = h.state().editor.history().entries.len();
    let lighting_history_before = history_buttons(&h);
    let b_scene_bytes = fs::read(&scene_b).unwrap();
    let b_sidecar_before = fs::read(sidecar(&scene_b)).unwrap();
    field(
        &mut h,
        "Lighting preset file",
        &preset.display().to_string(),
    );
    click(&mut h, "Load lighting preset");
    assert!(
        h.query_by_label_contains("Lighting preset staged;")
            .is_some()
    );
    assert_eq!(text(&h, "Light intensity"), "3.5");
    assert_eq!(bounds(&h), target_bounds);
    assert_eq!(history_buttons(&h), lighting_history_before);
    assert_eq!(h.state().editor.checksum(), frame_before);
    assert_eq!(h.state().editor.history().entries.len(), history_before);
    assert_eq!(fs::read(&scene_b).unwrap(), b_scene_bytes);
    assert_eq!(fs::read(sidecar(&scene_b)).unwrap(), b_sidecar_before);
    assert_eq!(fs::read(&scene_a).unwrap(), a_scene_bytes);
    assert_eq!(fs::read(sidecar(&scene_a)).unwrap(), a_sidecar_bytes);
    assert_eq!(fs::read(&preset).unwrap(), preset_bytes);
    assert!(h.get_by_label(LBL_PLAY).accesskit_node().is_disabled());
    assert!(
        h.get_by_label("Load lighting preset")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        h.get_by_label("Save applied lighting preset")
            .accesskit_node()
            .is_disabled()
    );
    let staged = pixels(&mut h, &root, "03-target-staged-only");
    assert_eq!(
        baseline, staged,
        "staging must leave rendered pixels exactly unchanged"
    );

    let preset_controls_layout = capture_preset_controls(&mut h, &root);

    click(&mut h, "Apply game lighting");
    let applied = pixels(&mut h, &root, "04-target-applied");
    let apply_changed_pixels = changed_pixels(&baseline, &applied);
    assert!(
        apply_changed_pixels > 20,
        "Apply must visibly change the production viewport"
    );
    assert_eq!(bounds(&h), target_bounds);
    assert_eq!(h.state().editor.checksum(), frame_before);
    assert_eq!(h.state().editor.history().entries.len(), history_before);
    assert_eq!(fs::read(sidecar(&scene_b)).unwrap(), b_sidecar_before);
    click(&mut h, "Undo game lighting");
    let undone = pixels(&mut h, &root, "05-target-undone");
    assert_eq!(undone, baseline);
    click(&mut h, "Redo game lighting");
    let redone = pixels(&mut h, &root, "06-target-redone");
    assert_eq!(redone, applied);
    menu(&mut h, "Save mesh workspace");
    let saved = fs::read(sidecar(&scene_b)).unwrap();
    let saved_json: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(
        saved_json["lighting"]["shadow_center"],
        serde_json::json!([3.0, 1.25, 2.0])
    );
    assert_eq!(saved_json["lighting"]["shadow_radius"], 18.0);
    for (key, value) in portable["lighting"].as_object().unwrap() {
        assert_eq!(&saved_json["lighting"][key], value, "reusable field {key}");
    }
    assert_eq!(fs::read(&scene_b).unwrap(), b_scene_bytes);
    let corrupt = root.join("corrupt.mesh-lighting-preset.json");
    fs::write(&corrupt, b"{\"schema\":1,\"lighting\":{}}").unwrap();
    field(
        &mut h,
        "Lighting preset file",
        &corrupt.display().to_string(),
    );
    let fields_before_failure = all_lighting_fields(&h);
    let history_before_failure = history_buttons(&h);
    click(&mut h, "Load lighting preset");
    assert!(
        h.query_by_label_contains("Load lighting preset failed:")
            .is_some()
    );
    assert_eq!(all_lighting_fields(&h), fields_before_failure);
    assert_eq!(history_buttons(&h), history_before_failure);
    assert_eq!(bounds(&h), target_bounds);
    assert_eq!(h.state().editor.checksum(), frame_before);
    assert_eq!(h.state().editor.history().entries.len(), history_before);
    assert_eq!(fs::read(sidecar(&scene_b)).unwrap(), saved);
    let failed = pixels(&mut h, &root, "07-corrupt-load-preserved");
    assert_eq!(failed, applied);
    drop(h);

    let mut fresh = harness(&physics, EditorGame::PhysGame);
    settle(&mut fresh);
    open_mesh(&mut fresh, &scene_b);
    open_lighting(&mut fresh);
    assert_eq!(bounds(&fresh), target_bounds);
    assert_eq!(all_lighting_fields(&fresh), fields_before_failure);
    assert_eq!(fresh.state().editor.checksum(), frame_before);
    let reopened = pixels(&mut fresh, &root, "08-fresh-target-reopen");
    assert_eq!(
        reopened, applied,
        "saved scene-local lighting survives a fresh editor open"
    );
    assert_eq!(fs::read(sidecar(&scene_b)).unwrap(), saved);
    assert_eq!(fs::read(&scene_a).unwrap(), a_scene_bytes);
    assert_eq!(fs::read(sidecar(&scene_a)).unwrap(), a_sidecar_bytes);
    assert_eq!(fs::read(&preset).unwrap(), preset_bytes);
    let receipt = serde_json::json!({
        "test": "portable_lighting_preset_cross_scene_stage_apply_undo_save_and_fresh_reopen",
        "adapter": adapter, "software_vulkan": true, "synthetic_egui_input": true,
        "hardware_gpu": false, "physical_os_input": false, "runtime_or_export": false,
        "source_scene": scene_a, "target_scene": scene_b, "distinct_graphs": graph_a != graph_b,
        "preset_sha256": sha256(&preset_bytes), "preset_bytes": preset_bytes.len(),
        "source_viewport_sha256": sha256(&source_pixels),
        "target_before_sha256": sha256(&baseline), "target_staged_sha256": sha256(&staged),
        "target_applied_sha256": sha256(&applied), "target_undone_sha256": sha256(&undone),
        "target_redone_sha256": sha256(&redone), "target_reopened_sha256": sha256(&reopened),
        "apply_changed_pixels": apply_changed_pixels, "target_lighting_sha256": sha256(&saved),
        "target_shadow_bounds": target_bounds, "frame_checksum": frame_before,
        "staging_preserved_frame_history_files_and_pixels": true,
        "corrupt_load_preserved_draft_and_history": true,
        "save_did_not_overwrite_preset": true,
        "preset_controls_layout": preset_controls_layout,
        "captures": ["01-source-authored.png", "02-target-before-load.png", "03-target-staged-only.png",
                     "04-target-applied.png", "05-target-undone.png", "06-target-redone.png",
                     "07-corrupt-load-preserved.png", "08-fresh-target-reopen.png", "09-preset-controls-layout.png"]
    });
    fs::write(
        root.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    eprintln!(
        "Lighting preset actual-UI receipt: {}",
        root.join("receipt.json").display()
    );
}
