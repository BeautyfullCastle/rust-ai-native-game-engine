#![cfg(all(feature = "mesh-navigation", feature = "mesh-animated-models"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{game::EditorGame, Editor, EditorApp};

#[path = "../../orr_model/tests/common/animated_matrix_fixture.rs"]
mod animated_matrix_fixture;

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

// The existing widgets are exercised with accessibility actions and text input.
// This is headless egui/WGPU evidence, not physical mouse or native-window input.
const EVIDENCE_ENV: &str = "ORR_ANIMATED_MATRIX_IMPORT_EVIDENCE";
const VIEWPORT_SIZE: (u32, u32) = (900, 850);

struct Evidence {
    root: PathBuf,
    captures: Vec<serde_json::Value>,
}

impl Evidence {
    fn capture(&mut self, h: &mut Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
        for _ in 0..6 {
            settle(h);
            let actual = h
                .state()
                .viewport3d_gpu()
                .expect("required production MeshNavigation viewport")
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
        let viewport = h.state().viewport3d_gpu().unwrap();
        assert_eq!(viewport.gpu().target().size(), VIEWPORT_SIZE);
        let adapter = viewport.gpu().adapter_name();
        eprintln!("{name}: production MeshNavigation adapter={adapter}");
        let pixels = viewport.gpu().read_rgba8();
        assert_eq!(
            pixels.len(),
            (VIEWPORT_SIZE.0 * VIEWPORT_SIZE.1 * 4) as usize
        );
        assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [VIEWPORT_SIZE.0 as usize, VIEWPORT_SIZE.1 as usize],
            &pixels,
        );
        orr_editor::app::write_png(&self.root.join(name), &image).unwrap();
        self.captures.push(serde_json::json!({
            "path": name,
            "sha256": sha256_hex(&fs::read(self.root.join(name)).unwrap()),
            "rgba_sha256": sha256_hex(&pixels),
            "production_viewport_adapter": adapter,
        }));
        pixels
    }
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

// Equality is byte-exact. The >12 RGB threshold above is used only to prove a
// substantial visible pose change, never to hide a matrix/TRS raster mismatch.
fn assert_same_pixels(left: &[u8], right: &[u8], context: &str) {
    assert_eq!(left.len(), right.len(), "{context}");
    let differing_bytes = left.iter().zip(right).filter(|(a, b)| a != b).count();
    assert_eq!(differing_bytes, 0, "{context}: exact RGBA mismatch");
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sidecar_path(scene: &Path) -> PathBuf {
    let mut name = scene.as_os_str().to_os_string();
    name.push(".mesh-animation.json");
    PathBuf::from(name)
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
                serde_json::to_vec_pretty(&serde_json::json!({
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

// Public FrameView observes the complete Frame checksum and tick, not bytes.
fn frame_evidence(h: &Harness<'_, EditorApp>) -> (u64, u64) {
    let frame = h.state().editor.snapshot().unwrap().predicted();
    (frame.checksum(), frame.tick())
}

fn button_disabled(h: &Harness<'_, EditorApp>, label: &str) -> bool {
    h.get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

fn start_import(
    h: &mut Harness<'_, EditorApp>,
    raw_model: &Path,
    license: &Path,
    package: &Path,
    name: &str,
) {
    for (label, value) in [
        ("Animation source (.glb or cooked JSON)", raw_model.display().to_string()),
        ("Animation source license", license.display().to_string()),
        (
            "New animation package directory",
            package.display().to_string(),
        ),
        ("Animation package name", name.to_owned()),
        ("Animation package version", "1.2.3".to_owned()),
    ] {
        type_field(h, label, &value);
    }
    assert!(button_disabled(h, "Import animation package"));
    click(h, "I have reviewed rights to import this animation");
    assert!(!button_disabled(h, "Import animation package"));
    click_button(h, "Import animation package");
}

fn await_import(h: &mut Harness<'_, EditorApp>, success: bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        settle(h);
        let failed = h
            .query_by_label_contains("Animation import failed:")
            .and_then(|node| {
                node.accesskit_node()
                    .value()
                    .or_else(|| node.accesskit_node().label())
            });
        let staged = h
            .query_by_label_contains("; staged for review, not applied.")
            .and_then(|node| {
                node.accesskit_node()
                    .value()
                    .or_else(|| node.accesskit_node().label())
            });
        if failed.is_some() || staged.is_some() {
            let status = failed.as_ref().or(staged.as_ref()).unwrap();
            assert_eq!(
                staged.is_some(),
                success,
                "unexpected raw import result: {status}"
            );
            assert_eq!(
                failed.is_some(),
                !success,
                "unexpected raw import result: {status}"
            );
            return status.clone();
        }
        assert!(
            Instant::now() < deadline,
            "raw import worker did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn preview(h: &mut Harness<'_, EditorApp>, tick: u32) {
    type_field(h, "Animation preview clip", "1");
    type_field(h, "Animation preview tick", &tick.to_string());
    assert!(!button_disabled(h, "Preview animation pose"));
    click_button(h, "Preview animation pose");
    assert!(h
        .query_by_label(&format!(
            "Previewing clip 1 at tick {tick} (transient; not saved)"
        ))
        .is_some());
}

fn assert_pulse_oracle(model: &orr_model::animation::AnimatedModel, tick: u32) {
    assert_eq!(model.source().clips[1].name, "pulse");
    let pose = model.sample_clip(1, tick as f32 / 60.0).unwrap();
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
    let parent = pose.local()[animated_matrix_fixture::PARENT_NODE];
    assert_eq!(parent.translation, [0.4, 3.0, 0.2]);
    assert_eq!(parent.scale, [1.25, 0.75, 1.5]);
    assert_eq!(parent.rotation, [0.0, 0.0, 1.0, 0.0]);
}

struct TwinResult {
    baseline: Vec<u8>,
    candidate: [Vec<u8>; 2],
    reopened: [Vec<u8>; 2],
    receipt: serde_json::Value,
}

fn run_twin(
    evidence: &mut Evidence,
    variant: &str,
    model_bytes: &[u8],
    rejected: &[(&str, Vec<u8>)],
) -> TwinResult {
    let root = evidence.root.join(variant);
    fs::create_dir(&root).unwrap();
    let raw = root.join("raw source");
    fs::create_dir(&raw).unwrap();
    let license_bytes = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo/LICENSE.txt"),
    )
    .unwrap();
    let raw_model = raw.join(format!("{variant} original character.glb"));
    let raw_license = raw.join("original rights.txt");
    fs::write(&raw_model, model_bytes).unwrap();
    fs::write(&raw_license, &license_bytes).unwrap();
    let package = root.join("new animation package");
    let name = format!("imported-{variant}-pulse");
    let expected =
        orr_model::animation_import::import_with_resolver("animated.glb", model_bytes, |_| {
            Err(orr_model::Error::message("fixture must be embedded"))
        })
        .unwrap();
    assert_pulse_oracle(&expected, 0);
    assert_pulse_oracle(&expected, 16);
    assert_ne!(
        expected.sample_clip(1, 0.0).unwrap().global(),
        expected.sample_clip(1, 16.0 / 60.0).unwrap().global()
    );

    let scene = root.join("raw-import.scene.yaml");
    fs::copy(EditorGame::MeshNavigation.default_scene_path(), &scene).unwrap();
    let sidecar = sidecar_path(&scene);
    let mut h = harness(&scene, &root.join("authoring-adapter.json"));
    settle(&mut h);
    click(&mut h, "Static-mesh navigation");
    click_button(&mut h, "Create ramp and bridge");
    click_button(&mut h, "Place mesh agent");
    menu_click(&mut h, "File", "Save Ctrl+S");
    let scene_bytes = fs::read(&scene).unwrap();
    let frame_before = frame_evidence(&h);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let baseline = evidence.capture(&mut h, &format!("{variant}/01-baseline.png"));
    click(&mut h, "Mesh agent animation");
    click(&mut h, "Import animation source");
    start_import(&mut h, &raw_model, &raw_license, &package, &name);
    await_import(&mut h, true);
    let loaded = evidence.capture(&mut h, &format!("{variant}/02-imported-not-applied.png"));
    assert_same_pixels(&baseline, &loaded, "staging must not apply animation");
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
    assert_eq!(manifest["name"], name);
    assert_eq!(manifest["version"], "1.2.3");
    assert_eq!(manifest["capabilities"], serde_json::json!(["animation"]));
    assert_eq!(frame_evidence(&h), frame_before);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert!(!sidecar.exists(), "Import must not save");
    assert!(button_disabled(&h, "Undo mesh animation"));
    assert!(button_disabled(&h, "Save mesh animation"));

    preview(&mut h, 0);
    let candidate0 = evidence.capture(&mut h, &format!("{variant}/03-candidate-pulse-0.png"));
    preview(&mut h, 16);
    let candidate16 = evidence.capture(&mut h, &format!("{variant}/04-candidate-pulse-16.png"));
    let pose_changed = changed_pixel_count(&candidate0, &candidate16);
    assert!(changed_pixel_count(&baseline, &candidate0) > 20);
    assert!(
        pose_changed > 20,
        "{variant} pulse must visibly change: {pose_changed}"
    );
    assert_eq!(frame_evidence(&h), frame_before);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert!(!sidecar.exists(), "Preview must not save");
    assert!(button_disabled(&h, "Undo mesh animation"));
    assert!(button_disabled(&h, "Save mesh animation"));
    click_button(&mut h, "Clear animation preview");
    let cleared = evidence.capture(&mut h, &format!("{variant}/05-cleared.png"));
    assert_same_pixels(&baseline, &cleared, "cleared candidate preview");

    type_field(&mut h, "Animation clip index", "1");
    click_button(&mut h, "Apply mesh animation");
    let applied = evidence.capture(&mut h, &format!("{variant}/06-applied.png"));
    assert_same_pixels(
        &candidate0,
        &applied,
        "explicit Apply uses candidate pose zero",
    );
    assert!(!sidecar.exists(), "Apply must not save");
    click_button(&mut h, "Undo mesh animation");
    let undone = evidence.capture(&mut h, &format!("{variant}/07-undone.png"));
    assert_same_pixels(&baseline, &undone, "explicit Undo restores baseline");
    click_button(&mut h, "Redo mesh animation");
    let redone = evidence.capture(&mut h, &format!("{variant}/08-redone.png"));
    assert_same_pixels(&applied, &redone, "explicit Redo restores animation");
    assert!(!sidecar.exists(), "Undo/Redo must not save");
    click_button(&mut h, "Save mesh animation");
    let saved = fs::read(&sidecar).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(document["settings"]["clip"], 1);
    assert_eq!(document["asset"], "animated.glb");
    assert_eq!(document["asset_sha256"], sha256_hex(model_bytes));
    assert_eq!(document["license_sha256"], sha256_hex(&license_bytes));
    assert_eq!(
        document["license_text"],
        std::str::from_utf8(&license_bytes).unwrap()
    );
    assert_eq!(
        document["model"],
        serde_json::to_value(expected.source()).unwrap()
    );
    let embedded = orr_model::animation::AnimatedModel::new(
        serde_json::from_value(document["model"].clone()).unwrap(),
    )
    .unwrap();
    assert_pulse_oracle(&embedded, 0);
    assert_pulse_oracle(&embedded, 16);
    assert_eq!(frame_evidence(&h), frame_before);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert!(!button_disabled(&h, "Undo mesh animation"));
    assert!(button_disabled(&h, "Save mesh animation"));

    let mut rejection_receipts = Vec::new();
    for (label, bytes) in rejected {
        let invalid = raw.join(format!("invalid {label}.glb"));
        let destination = root.join(format!("rejected {label} package"));
        fs::write(&invalid, bytes).unwrap();
        start_import(
            &mut h,
            &invalid,
            &raw_license,
            &destination,
            &format!("invalid-{label}"),
        );
        let status = await_import(&mut h, false);
        let diagnostic = match *label {
            "targeted" => "animation-targeted matrix",
            "mirrored" => "mirrored matrix",
            "shear" => "unsupported shear",
            "singular" => "singular node transform",
            "projective" => "non-affine",
            _ => panic!("unexpected invalid fixture: {label}"),
        };
        assert!(
            status.contains(diagnostic),
            "{label} rejected for wrong reason: {status}"
        );
        let rejected_pixels = evidence.capture(&mut h, &format!("{variant}/rejected-{label}.png"));
        assert_same_pixels(
            &applied,
            &rejected_pixels,
            "invalid import preserves applied animation",
        );
        assert!(
            !destination.exists(),
            "rejected GLB must not publish a package"
        );
        assert_eq!(fs::read(&invalid).unwrap(), *bytes);
        assert_eq!(fs::read(&raw_model).unwrap(), model_bytes);
        assert_eq!(fs::read(&raw_license).unwrap(), license_bytes);
        assert_eq!(fs::read(&sidecar).unwrap(), saved);
        assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
        assert_eq!(frame_evidence(&h), frame_before);
        assert_eq!(h.state().editor.checksum(), checksum);
        assert_eq!(h.state().editor.history(), &history);
        assert!(!button_disabled(&h, "Undo mesh animation"));
        assert!(button_disabled(&h, "Redo mesh animation"));
        assert!(button_disabled(&h, "Save mesh animation"));
        rejection_receipts.push(serde_json::json!({
            "case": label, "status": status, "source_sha256": sha256_hex(bytes),
            "source_preserved": true, "destination_absent": true,
            "scene_sidecar_frame_history_preserved": true,
            "applied_pose_rgba_unchanged": true,
        }));
    }
    if !rejected.is_empty() {
        // Prove invalid imports did not insert hidden animation undo entries.
        click_button(&mut h, "Undo mesh animation");
        let undone = evidence.capture(&mut h, &format!("{variant}/rejection-history-undone.png"));
        assert_same_pixels(
            &baseline,
            &undone,
            "rejections retain original animation history",
        );
        click_button(&mut h, "Redo mesh animation");
        let redone = evidence.capture(&mut h, &format!("{variant}/rejection-history-redone.png"));
        assert_same_pixels(
            &applied,
            &redone,
            "rejections retain original animation Redo",
        );
        assert_eq!(fs::read(&sidecar).unwrap(), saved);
        assert!(button_disabled(&h, "Save mesh animation"));
    }
    drop(h);
    fs::rename(&raw, root.join("raw source.hidden")).unwrap();
    fs::rename(&package, root.join("new animation package.hidden")).unwrap();
    assert!(!raw_model.exists() && !raw_license.exists() && !package.exists());
    let mut h = harness(&scene, &root.join("reopened-adapter.json"));
    settle(&mut h);
    let reopened_history = h.state().editor.history().clone();
    let reopened0 = evidence.capture(&mut h, &format!("{variant}/09-source-free-reopen-0.png"));
    assert_same_pixels(&applied, &reopened0, "fresh source-free reopen pose zero");
    click(&mut h, "Mesh agent animation");
    preview(&mut h, 16);
    let reopened16 = evidence.capture(&mut h, &format!("{variant}/10-source-free-preview-16.png"));
    assert_same_pixels(
        &candidate16,
        &reopened16,
        "fresh source-free reopen pose sixteen",
    );
    assert!(changed_pixel_count(&reopened0, &reopened16) > 20);
    assert_eq!(frame_evidence(&h), frame_before);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &reopened_history);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    assert!(!raw_model.exists() && !raw_license.exists() && !package.exists());
    assert_eq!(
        fs::read(
            root.join("raw source.hidden")
                .join(raw_model.file_name().unwrap())
        )
        .unwrap(),
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
    for (label, bytes) in rejected {
        assert_eq!(
            fs::read(
                root.join("raw source.hidden")
                    .join(format!("invalid {label}.glb"))
            )
            .unwrap(),
            *bytes
        );
    }
    let adapters = ["authoring-adapter.json", "reopened-adapter.json"].map(|name| {
        serde_json::from_slice::<serde_json::Value>(&fs::read(root.join(name)).unwrap()).unwrap()
    });
    TwinResult {
        baseline,
        candidate: [candidate0, candidate16],
        reopened: [reopened0, reopened16],
        receipt: serde_json::json!({
            "variant": variant, "raw_source_sha256": sha256_hex(model_bytes),
            "scene_sha256": sha256_hex(&scene_bytes), "sidecar_sha256": sha256_hex(&saved),
            "frame_checksum": frame_before.0, "frame_tick": frame_before.1,
            "candidate_pose_changed_pixels": pose_changed,
            "actual_adapters": adapters, "rejected_imports": rejection_receipts,
        }),
    }
}

#[test]
#[ignore = "requires ORR_REQUIRE_GPU=1 and a fresh absolute ORR_ANIMATED_MATRIX_IMPORT_EVIDENCE directory"]
fn static_matrix_raw_import_matches_trs_through_source_free_editor_reopen() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let root =
        PathBuf::from(std::env::var_os(EVIDENCE_ENV).expect("fresh evidence directory required"));
    assert!(root.is_absolute(), "evidence directory must be absolute");
    fs::create_dir(&root).expect("evidence directory must be fresh");
    let gpu = orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default())
        .expect("matrix import acceptance requires an actual WGPU adapter; no skip");
    let preflight = serde_json::json!({"name": gpu.adapter_name(), "software": gpu.is_software()});
    eprintln!("matrix import GPU preflight: {preflight}");
    drop(gpu);
    let (matrix_bytes, trs_bytes) = animated_matrix_fixture::twins();
    assert_ne!(
        matrix_bytes, trs_bytes,
        "independent matrix and explicit-TRS source GLBs"
    );
    let rejected = animated_matrix_fixture::rejected_cases();
    assert_eq!(rejected.len(), 5);
    let mut evidence = Evidence {
        root,
        captures: Vec::new(),
    };
    let matrix = run_twin(&mut evidence, "matrix", &matrix_bytes, &rejected);
    let trs = run_twin(&mut evidence, "trs", &trs_bytes, &[]);
    assert_same_pixels(
        &matrix.baseline,
        &trs.baseline,
        "independent Editor scene baselines",
    );
    for (index, tick) in [0, 16].into_iter().enumerate() {
        assert_same_pixels(
            &matrix.candidate[index],
            &trs.candidate[index],
            &format!("matrix/TRS candidate pose {tick}"),
        );
        assert_same_pixels(
            &matrix.reopened[index],
            &trs.reopened[index],
            &format!("matrix/TRS source-free reopened pose {tick}"),
        );
    }
    assert_eq!(evidence.captures.len(), 27);
    fs::write(evidence.root.join("acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema": 1, "production_editor_app": true, "headless_egui_wgpu": true,
        "native_window": false, "physical_os_input": false, "physical_gpu_claim": false,
        "widget_actions": "egui accessibility button actions and text input",
        "gpu_required_no_skip": true, "preflight_adapter": preflight,
        "raw_import_worker_to_candidate": true, "animation_only_packages": true,
        "rights_acknowledgement_required": true, "explicit_apply_undo_redo_save": true,
        "source_hidden_during_fresh_reopen": true,
        "clip": 1, "clip_name": "pulse", "preview_ticks": [0,16],
        "source_fixture_parent_translation": [0.4,3.0,0.2],
        "source_fixture_parent_rotation": [0.0,0.0,1.0,0.0],
        "source_fixture_parent_scale": [1.25,0.75,1.5],
        "matrix_trs_candidate_rgba_equal_at_both_ticks": true,
        "matrix_trs_reopened_rgba_equal_at_both_ticks": true,
        "pixel_equality": "byte-exact RGBA; visible pose delta uses RGB difference >12 in >20 pixels",
        "independent_fixture_local_pose_oracle": true,
        "frame_evidence_scope": "complete Frame checksum and tick; no serialized Frame byte equality claim",
        "scene_history_unchanged_during_staging_preview_and_invalid_import": true,
        "twins": [matrix.receipt, trs.receipt],
        "capture_count": evidence.captures.len(), "captures": evidence.captures,
    })).unwrap()).unwrap();
}
