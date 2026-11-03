//! Actual surface-height grid authoring through the production EditorApp.
//! Headless egui accessibility actions and WGPU pixels, not a native window,
//! physical operating-system input, or a claim about a hardware GPU.
#![cfg(feature = "mesh-navigation")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{game::EditorGame, model::History, Editor, EditorApp};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::{authoring::create_mesh_scene, AgentSpec};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const EVIDENCE_ENV: &str = "ORR_MESH_PROP_SURFACE_GRID_EVIDENCE";
const VIEWPORT_SIZE: (u32, u32) = (900, 850);
const SURFACE_BUTTON: &str = "Stamp surface-height prop grid";
// Independent literal oracle for y = x/2 + z/4, then the explicit +0.5 offset.
// Cell [0,0] is the existing source and deliberately does not lie on that plane.
const SURFACE_CELLS: [[f64; 3]; 6] = [
    [-3.0, 0.25, -2.0],
    [0.0, 0.0, -2.0],
    [3.0, 1.5, -2.0],
    [-3.0, -0.5, 2.0],
    [0.0, 1.0, 2.0],
    [3.0, 2.5, 2.0],
];
const CONTROL_POSITION: [f64; 3] = [5.0, 5.0, -3.0];

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(30);
    h.get_by_label(label).click_accesskit();
    settle(h);
}

fn click_button(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    h.run_steps(30);
    assert!(
        !button_disabled(h, label),
        "required button is disabled: {label}"
    );
    h.get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(h);
}

fn button_disabled(h: &Harness<'_, EditorApp>, label: &str) -> bool {
    h.get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

fn type_field(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(30);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    settle(h);
    assert_eq!(field(h, label), value);
}

fn field(h: &Harness<'_, EditorApp>, label: &str) -> String {
    h.get_by_role_and_label(Role::TextInput, label)
        .value()
        .unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn frame_evidence(h: &Harness<'_, EditorApp>) -> (u64, u64) {
    let frame = h.state().editor.snapshot().unwrap().predicted();
    (frame.checksum(), frame.tick())
}

// Public observers establish checksum/tick/history and persisted scene equality;
// this test does not claim to compare complete serialized Frame bytes.
struct HostInvariant {
    scene: PathBuf,
    scene_bytes: Vec<u8>,
    checksum: u64,
    frame: (u64, u64),
    history: History,
}

impl HostInvariant {
    fn new(h: &Harness<'_, EditorApp>, scene: &Path) -> Self {
        Self {
            scene: scene.to_owned(),
            scene_bytes: fs::read(scene).unwrap(),
            checksum: h.state().editor.checksum(),
            frame: frame_evidence(h),
            history: h.state().editor.history().clone(),
        }
    }

    fn check(&self, h: &Harness<'_, EditorApp>) {
        assert_eq!(h.state().editor.checksum(), self.checksum);
        assert_eq!(frame_evidence(h), self.frame);
        assert_eq!(h.state().editor.history(), &self.history);
        assert_eq!(fs::read(&self.scene).unwrap(), self.scene_bytes);
    }
}

struct Evidence {
    root: PathBuf,
    captures: Vec<Value>,
}

impl Evidence {
    fn capture(
        &mut self,
        h: &mut Harness<'_, EditorApp>,
        host: &HostInvariant,
        name: &str,
    ) -> Vec<u8> {
        for _ in 0..6 {
            settle(h);
            let actual = h
                .state()
                .viewport3d_gpu()
                .expect("required production MeshNavigation GPU viewport")
                .gpu()
                .target()
                .size();
            if actual == VIEWPORT_SIZE {
                break;
            }
            let window = h.ctx.viewport_rect().size();
            let ratio = h.ctx.pixels_per_point();
            h.set_size(egui::vec2(
                window.x + (VIEWPORT_SIZE.0 as f32 - actual.0 as f32) / ratio,
                window.y + (VIEWPORT_SIZE.1 as f32 - actual.1 as f32) / ratio,
            ));
        }
        settle(h);
        host.check(h);
        let gpu = h.state().viewport3d_gpu().unwrap().gpu();
        assert_eq!(gpu.target().size(), VIEWPORT_SIZE);
        let pixels = gpu.read_rgba8();
        assert_eq!(
            pixels.len(),
            (VIEWPORT_SIZE.0 * VIEWPORT_SIZE.1 * 4) as usize
        );
        assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [VIEWPORT_SIZE.0 as usize, VIEWPORT_SIZE.1 as usize],
            &pixels,
        );
        let destination = self.root.join(name);
        orr_editor::app::write_png(&destination, &image).unwrap();
        let adapter = gpu.adapter_name();
        eprintln!("{name}: production viewport adapter={adapter}");
        self.captures.push(json!({
            "path": name, "png_sha256": sha256(&fs::read(destination).unwrap()),
            "rgba_sha256": sha256(&pixels), "production_viewport_adapter": adapter,
            "model_cache_counts": gpu.model_cache_counts(),
            "host_checksum": format!("0x{:016x}", host.checksum),
            "frame_checksum": format!("0x{:016x}", host.frame.0), "tick": host.frame.1,
            "host_history_unchanged": true, "scene_bytes_unchanged": true,
        }));
        pixels
    }
}

fn harness(scene: &Path, adapter_receipt: &Path) -> Harness<'static, EditorApp> {
    let mut editor = Editor::open_game(scene, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let adapter_receipt = adapter_receipt.to_owned();
    Harness::builder()
        .with_size([1500.0, 1000.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| {
            let info = cc
                .wgpu_render_state
                .as_ref()
                .expect("required actual egui WGPU renderer")
                .adapter
                .get_info();
            eprintln!(
                "actual egui adapter={} backend={:?} device={:?}",
                info.name, info.backend, info.device_type
            );
            fs::write(
                &adapter_receipt,
                serde_json::to_vec_pretty(&json!({
                    "name": info.name, "backend": format!("{:?}", info.backend),
                    "device_type": format!("{:?}", info.device_type),
                    "driver": info.driver, "driver_info": info.driver_info,
                    "headless_egui_wgpu": true, "native_window": false,
                    "physical_os_input": false, "physical_gpu_claim": false,
                }))
                .unwrap(),
            )
            .unwrap();
            EditorApp::new(editor, cc.wgpu_render_state.clone())
        })
}

fn author_sloped_scene(scene: &Path) {
    // This is an actual admitted host scene, not a substitute projection helper.
    let graph = StaticMeshGraph::bake(
        &[
            [fp!(-6), fp!(-4), fp!(-4)],
            [fp!(6), fp!(2), fp!(-4)],
            [fp!(-6), fp!(-2), fp!(4)],
            [fp!(6), fp!(4), fp!(4)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let mut document = create_mesh_scene(
        &graph,
        AgentSpec {
            start: [fp!(-5), fp!(-3), fp!(-2)],
            goal: [fp!(5), fp!(3), fp!(2)],
            max_vertical: FP::ZERO,
            distance_per_tick: fp!(0.125),
        },
    )
    .unwrap();
    fs::write(scene, document.save_yaml()).unwrap();
}

fn copy_package(package: &Path) -> Vec<(&'static str, Vec<u8>)> {
    fs::create_dir(package).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
    [
        "orr.package.json",
        "foreground.glb",
        "background.glb",
        "LICENSE.txt",
        "generate.py",
    ]
    .into_iter()
    .map(|name| {
        let bytes = fs::read(source.join(name)).unwrap();
        fs::write(package.join(name), &bytes).unwrap();
        (name, bytes)
    })
    .collect()
}

fn sidecar_path(scene: &Path) -> PathBuf {
    let mut name = scene.as_os_str().to_os_string();
    name.push(".mesh-props.json");
    PathBuf::from(name)
}

fn select(h: &mut Harness<'_, EditorApp>, record: &Value) {
    let id = record["id"].as_u64().unwrap();
    let name = record["name"].as_str().unwrap();
    click(h, &format!("{name} · prop-{id:016x}"));
}

fn prop_labels(h: &Harness<'_, EditorApp>) -> Vec<String> {
    h.query_all_by_label_contains(" · prop-")
        .map(|node| {
            // StaticText content can live in value rather than label in AccessKit.
            node.accesskit_node()
                .value()
                .or_else(|| node.accesskit_node().label())
                .unwrap()
        })
        .collect()
}

fn assert_ui_positions(h: &mut Harness<'_, EditorApp>, expected: &[[f64; 3]]) {
    let labels = prop_labels(h);
    assert_eq!(labels.len(), expected.len());
    assert_eq!(labels.iter().collect::<BTreeSet<_>>().len(), labels.len());
    let mut positions = Vec::new();
    for label in labels {
        click(h, &label);
        positions.push([
            field(h, "Prop position X").parse::<f64>().unwrap(),
            field(h, "Prop position Y").parse::<f64>().unwrap(),
            field(h, "Prop position Z").parse::<f64>().unwrap(),
        ]);
    }
    assert_eq!(positions.len(), expected.len());
    for position in expected {
        assert_eq!(
            positions
                .iter()
                .filter(|actual| *actual == position)
                .count(),
            1,
            "literal expected cell {position:?} missing or duplicated in {positions:?}"
        );
    }
}

fn changed_mask(left: &[u8], right: &[u8]) -> Vec<usize> {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>()
        .0
        .iter()
        .zip(right.as_chunks::<4>().0.iter())
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

fn same_pixels(left: &[u8], right: &[u8], context: &str) {
    assert_eq!(left.len(), right.len(), "{context}");
    assert_eq!(
        left.iter().zip(right).filter(|(a, b)| a != b).count(),
        0,
        "{context}: byte-exact production viewport RGBA mismatch"
    );
}

fn unchanged_control(before: &[u8], after: &[u8], mask: &[usize]) {
    assert!(mask.len() > 20, "same-asset control must be visible");
    for &i in mask {
        assert_eq!(
            &before[i * 4..i * 4 + 4],
            &after[i * 4..i * 4 + 4],
            "untouched same-asset distinct-appearance control changed at pixel {i}"
        );
    }
}

#[cfg(feature = "mesh-prop-shading")]
fn shade(h: &mut Harness<'_, EditorApp>, metallic: &str, roughness: &str) {
    if h.query_by_label("Use scalar shading for prop slot")
        .is_none()
    {
        click(h, "Scalar material shading");
    }
    if h.get_by_label("Use scalar shading for prop slot")
        .accesskit_node()
        .toggled()
        != Some(egui::accesskit::Toggled::True)
    {
        click(h, "Use scalar shading for prop slot");
    }
    type_field(h, "Prop slot metallic", metallic);
    type_field(h, "Prop slot roughness", roughness);
}

fn expected_grid(surface: bool) -> Vec<[f64; 3]> {
    let mut cells = SURFACE_CELLS.to_vec();
    if !surface {
        for cell in &mut cells {
            cell[1] = 0.25;
        }
    }
    cells.push(CONTROL_POSITION);
    cells
}

fn assert_saved_grid(saved: &Value, baseline: &Value) {
    let records = saved["props"].as_array().unwrap();
    let originals = baseline["props"].as_array().unwrap();
    let source = originals
        .iter()
        .find(|prop| prop["name"] == "Source")
        .unwrap();
    let control = originals
        .iter()
        .find(|prop| prop["name"] == "Control")
        .unwrap();
    assert_eq!(records.len(), 7);
    assert_eq!(
        records
            .iter()
            .map(|p| p["id"].as_u64().unwrap())
            .collect::<BTreeSet<_>>()
            .len(),
        7
    );
    assert!(records.iter().all(|p| p["id"].as_u64().unwrap() != 0));
    assert_eq!(
        records
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect::<BTreeSet<_>>()
            .len(),
        7
    );
    assert_eq!(
        records.iter().find(|p| p["id"] == source["id"]).unwrap(),
        source,
        "complete source cell [0,0] must be unchanged"
    );
    assert_eq!(
        records.iter().find(|p| p["id"] == control["id"]).unwrap(),
        control,
        "unselected control must be unchanged"
    );
    for position in SURFACE_CELLS {
        let matching = records
            .iter()
            .filter(|p| p["art"]["offset"] == json!(position))
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1, "literal saved surface cell {position:?}");
        let mut art = matching[0]["art"].clone();
        art["offset"] = source["art"]["offset"].clone();
        assert_eq!(art, source["art"], "only position changes; embedded art, scale, rotation, UV, tint and license remain exact");
        #[cfg(feature = "mesh-prop-shading")]
        assert_eq!(matching[0]["shading"], source["shading"]);
    }
    assert_ne!(source["art"]["tint"], control["art"]["tint"]);
    assert_eq!(
        source["art"]["asset_sha256"],
        control["art"]["asset_sha256"]
    );
    #[cfg(feature = "mesh-prop-shading")]
    {
        assert!(!source["shading"].is_null());
        assert_ne!(source["shading"], control["shading"]);
    }
}

#[test]
#[ignore = "requires actual WGPU and a fresh ORR_MESH_PROP_SURFACE_GRID_EVIDENCE directory"]
fn surface_height_grid_preserves_props_history_pixels_and_source_free_reopen() {
    let root =
        PathBuf::from(std::env::var_os(EVIDENCE_ENV).expect("fresh evidence directory required"));
    assert!(root.is_absolute());
    fs::create_dir(&root).expect("evidence directory must not already exist");
    let mut evidence = Evidence {
        root: root.clone(),
        captures: Vec::new(),
    };
    let package = root.join("package");
    let package_bytes = copy_package(&package);
    let scene = root.join("surface-grid.scene.yaml");
    author_sloped_scene(&scene);
    let sidecar = sidecar_path(&scene);
    let mut h = harness(&scene, &root.join("initial-adapter.json"));
    settle(&mut h);
    let host = HostInvariant::new(&h, &scene);
    let empty = evidence.capture(&mut h, &host, "00-empty.png");
    click(&mut h, "Static-mesh navigation");
    click(&mut h, "Mesh props");
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
        ("Prop name", "Control"),
        ("Prop position X", "5"),
        ("Prop position Y", "5"),
        ("Prop position Z", "-3"),
        ("Prop scale", "0.5"),
        ("Prop rotation Y (degrees)", "27"),
        ("Prop tint R", "0.9"),
        ("Prop tint G", "0.4"),
        ("Prop tint B", "0.7"),
    ] {
        type_field(&mut h, label, value);
    }
    #[cfg(feature = "mesh-prop-shading")]
    shade(&mut h, "0", "0.85");
    click_button(&mut h, "Add prop");
    let control_pixels = evidence.capture(&mut h, &host, "01-control.png");
    let control_mask = changed_mask(&empty, &control_pixels);
    assert!(control_mask.len() > 20);
    for (label, value) in [
        ("Prop name", "Source"),
        ("Prop position X", "-3"),
        ("Prop position Y", "0.25"),
        ("Prop position Z", "-2"),
        ("Prop tint R", "0.4"),
        ("Prop tint G", "0.85"),
        ("Prop tint B", "1"),
    ] {
        type_field(&mut h, label, value);
    }
    #[cfg(feature = "mesh-prop-shading")]
    shade(&mut h, "1", "0.2");
    click_button(&mut h, "Add prop");
    let baseline_pixels = evidence.capture(&mut h, &host, "02-source-and-control.png");
    assert!(changed_mask(&control_pixels, &baseline_pixels).len() > 20);
    unchanged_control(&control_pixels, &baseline_pixels, &control_mask);
    let cache = h
        .state()
        .viewport3d_gpu()
        .unwrap()
        .gpu()
        .model_cache_counts();
    assert!(cache.0 >= 3 && cache.0 <= 8 && cache.1 == 0);
    click_button(&mut h, "Save props");
    let baseline_bytes = fs::read(&sidecar).unwrap();
    let baseline: Value = serde_json::from_slice(&baseline_bytes).unwrap();
    let source = baseline["props"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "Source")
        .unwrap()
        .clone();
    if h.query_by_label("Prop grid columns").is_none() {
        click(&mut h, "Prop grid");
    }
    assert_eq!(field(&h, "Surface grid maximum vertical distance"), "4");
    assert_eq!(field(&h, "Surface grid height offset"), "0");
    for (label, value) in [
        ("Prop grid columns", "3"),
        ("Prop grid rows", "2"),
        ("Prop grid spacing X", "3"),
        ("Prop grid spacing Z", "4"),
        ("Surface grid height offset", "0.5"),
    ] {
        type_field(&mut h, label, value);
    }
    // Existing flat grid is the negative control: identical XZ and appearance,
    // but all pivots remain at 0.25 instead of the independent plane heights.
    click_button(&mut h, "Stamp selected prop grid");
    assert_ui_positions(&mut h, &expected_grid(false));
    select(&mut h, &source);
    let flat_pixels = evidence.capture(&mut h, &host, "03-flat-grid-control.png");
    assert!(changed_mask(&baseline_pixels, &flat_pixels).len() > 20);
    assert_eq!(fs::read(&sidecar).unwrap(), baseline_bytes);
    click_button(&mut h, "Undo prop edit");
    same_pixels(
        &baseline_pixels,
        &evidence.capture(&mut h, &host, "04-flat-grid-undone.png"),
        "one flat grid undo",
    );
    assert_ui_positions(&mut h, &[CONTROL_POSITION, SURFACE_CELLS[0]]);
    select(&mut h, &source);
    // An unapplied transform/name draft must retain the real UI guard.
    type_field(&mut h, "Prop name", "Unapplied source draft");
    assert!(button_disabled(&h, SURFACE_BUTTON));
    select(&mut h, &source);
    click_button(&mut h, SURFACE_BUTTON);
    assert_ui_positions(&mut h, &expected_grid(true));
    select(&mut h, &source);
    let surface_pixels = evidence.capture(&mut h, &host, "05-surface-grid.png");
    let flat_difference = changed_mask(&flat_pixels, &surface_pixels).len();
    assert!(
        flat_difference > 20,
        "surface projection must visibly differ from old flat grid"
    );
    unchanged_control(&baseline_pixels, &surface_pixels, &control_mask);
    assert_eq!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .model_cache_counts(),
        cache,
        "same-appearance clones must not grow the admitted asset cache"
    );
    assert_eq!(
        fs::read(&sidecar).unwrap(),
        baseline_bytes,
        "Stamp is not Save"
    );
    let mut rejections = Vec::new();
    for (case, columns, spacing, diagnostic, capture) in [
        (
            "missing-cell",
            "3",
            "8",
            "cell [2,0]",
            "06-missing-cell-rejected.png",
        ),
        (
            "scene-budget",
            "16",
            "3",
            "128-prop",
            "07-budget-rejected.png",
        ),
    ] {
        type_field(&mut h, "Prop grid columns", columns);
        type_field(
            &mut h,
            "Prop grid rows",
            if case == "scene-budget" { "16" } else { "2" },
        );
        type_field(&mut h, "Prop grid spacing X", spacing);
        click_button(&mut h, SURFACE_BUTTON);
        let status = h
            .query_by_label_contains("Surface prop grid was not applied:")
            .map(|node| {
                node.accesskit_node()
                    .value()
                    .or_else(|| node.accesskit_node().label())
                    .unwrap()
            })
            .expect("visible rejection diagnostic");
        assert!(
            status.contains(diagnostic),
            "unexpected rejection: {status}"
        );
        assert_ui_positions(&mut h, &expected_grid(true));
        select(&mut h, &source);
        same_pixels(
            &surface_pixels,
            &evidence.capture(&mut h, &host, capture),
            "failed grid is atomic",
        );
        assert_eq!(fs::read(&sidecar).unwrap(), baseline_bytes);
        rejections
            .push(json!({"case": case, "status": status, "pixels_and_document_unchanged": true}));
    }
    // A single undo after both failures must remove the entire surface stamp.
    // A hidden failed/no-op history entry would leave the grid present here.
    click_button(&mut h, "Undo prop edit");
    assert_ui_positions(&mut h, &[CONTROL_POSITION, SURFACE_CELLS[0]]);
    select(&mut h, &source);
    same_pixels(
        &baseline_pixels,
        &evidence.capture(&mut h, &host, "08-surface-grid-undone.png"),
        "one surface grid undo",
    );
    click_button(&mut h, "Redo prop edit");
    assert_ui_positions(&mut h, &expected_grid(true));
    select(&mut h, &source);
    same_pixels(
        &surface_pixels,
        &evidence.capture(&mut h, &host, "09-surface-grid-redone.png"),
        "one surface grid redo",
    );
    click_button(&mut h, "Save props");
    let saved_bytes = fs::read(&sidecar).unwrap();
    let saved: Value = serde_json::from_slice(&saved_bytes).unwrap();
    assert_saved_grid(&saved, &baseline);
    host.check(&h);
    for (name, bytes) in &package_bytes {
        assert_eq!(fs::read(package.join(name)).unwrap(), *bytes);
    }
    drop(h);
    fs::rename(&package, root.join("package.hidden")).unwrap();
    assert!(!package.exists());
    let mut h = harness(&scene, &root.join("reopened-adapter.json"));
    settle(&mut h);
    let reopened_host = HostInvariant::new(&h, &scene);
    assert_eq!(reopened_host.checksum, host.checksum);
    assert_eq!(reopened_host.frame, host.frame);
    same_pixels(
        &surface_pixels,
        &evidence.capture(&mut h, &reopened_host, "10-source-free-reopen.png"),
        "fresh host and app reopen without original package",
    );
    click(&mut h, "Static-mesh navigation");
    click(&mut h, "Mesh props");
    assert_ui_positions(&mut h, &expected_grid(true));
    assert!(
        button_disabled(&h, "Undo prop edit"),
        "fresh app must load persisted props, not retain history"
    );
    assert_eq!(fs::read(&sidecar).unwrap(), saved_bytes);
    assert_saved_grid(
        &serde_json::from_slice::<Value>(&fs::read(&sidecar).unwrap()).unwrap(),
        &baseline,
    );
    assert_eq!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .model_cache_counts(),
        cache
    );
    reopened_host.check(&h);
    assert_eq!(evidence.captures.len(), 11);
    fs::write(root.join("acceptance.json"), serde_json::to_vec_pretty(&json!({
        "test": "surface_height_grid_preserves_props_history_pixels_and_source_free_reopen",
        "feature": if cfg!(feature = "mesh-prop-shading") { "mesh-prop-shading" } else { "mesh-navigation" },
        "input": "headless egui accessibility actions and text input",
        "normal_harness_wgpu": true, "gpu_required": true,
        "native_window": false, "physical_os_input": false, "physical_gpu_claim": false,
        "plane": "y=x/2+z/4", "expected_surface_cells": SURFACE_CELLS,
        "height_offset": 0.5, "max_vertical_distance": 4,
        "flat_control_changed_pixels_gt12": flat_difference,
        "source_and_control_preserved": true, "one_undo_one_redo": true,
        "same_asset_cache_counts": cache, "source_package_hidden_for_fresh_reopen": true,
        "saved_sidecar_sha256": sha256(&saved_bytes), "scene_sha256": sha256(&host.scene_bytes),
        "frame_evidence_scope": "public checksum and tick; no serialized Frame-byte claim",
        "rejections": rejections, "captures": evidence.captures,
    })).unwrap()).unwrap();
}
