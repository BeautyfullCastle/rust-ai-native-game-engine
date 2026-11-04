//! Real AccessKit Save/Load widgets and viewport pointer input. The shared
//! acceptance helpers retain GPU bytes before checking independent atlas texels.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::tilemap_copy_stamp_tests::{
    BEFORE, STAMPED, TestApp, app_play_oracle, capture, click, consume_saved, document, drain,
    files, fingerprint, open, rectangle, stamp, write_json,
};
use super::*;
use egui_kittest::{
    Harness,
    kittest::Queryable,
};
use orr_render::{Camera, orr_rhi::Wgpu};
use serde_json::json;
use std::{fs, path::Path};

fn pattern_file(h: &mut Harness<'_, TestApp>, path: &Path) {
    use egui::accesskit::Role;
    let label = "Pattern file";
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    drain(h, 2);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    drain(h, 2);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    drain(h, 2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(path.to_str().unwrap());
    drain(h, 2);
}

#[test]
#[ignore = "explicit actual EditorApp reusable pattern asset GPU acceptance; ORR_REQUIRE_GPU=1"]
fn actual_pattern_asset_save_load_stamp_reopen_play_literal_cells() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_TILEMAP_PATTERN_ASSET_EVIDENCE")
            .expect("fresh retained pattern asset evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    assert!(fs::read_dir(&evidence).unwrap().next().is_none());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: "arena-tilemap-2d-v1".into(),
        seed: "reusable-pattern-acceptance".into(),
    })
    .unwrap();
    let map_path = root.join("arena.tilemap.json");
    let mut map: orr_tilemap::Document =
        serde_json::from_slice(&fs::read(&map_path).unwrap()).unwrap();
    map.width = 7;
    map.height = 4;
    map.origin = [-56., 144.];
    map.cells = BEFORE.to_vec();
    fs::write(&map_path, serde_json::to_vec(&map).unwrap()).unwrap();
    let source_files = files(&root);
    let pattern_path = evidence.join("motif.pattern.json");
    let malformed_path = evidence.join("malformed.pattern.json");
    fs::write(&malformed_path, b"{invalid JSON").unwrap();
    let mut captures = Vec::new();
    let mut h = open(&root);
    let initial = capture(&mut h, &evidence, "01-before", &mut captures).to_bytes();
    let initial_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    click(&mut h, "Tile tool (owns viewport clicks)");
    click(&mut h, "Copy");
    rectangle(&mut h, (2, 1), (0, 0));
    // Independent asymmetric motif: [10, empty, 11; 20, 21, 10].
    h.get_by_label("Pattern: 3 × 2");
    click(&mut h, "Reusable pattern files");
    pattern_file(&mut h, &pattern_path);
    click(&mut h, "Save pattern");
    assert!(h.state().tilemap.as_ref().unwrap().test_error().is_none());
    let asset_bytes = fs::read(&pattern_path).unwrap();
    let asset: serde_json::Value = serde_json::from_slice(&asset_bytes).unwrap();
    assert_eq!(asset["width"], 3);
    assert_eq!(asset["height"], 2);
    assert_eq!(asset["cells"], json!([10, null, 11, 20, 21, 10]));
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        initial_history
    );
    assert_eq!(
        files(&root),
        source_files,
        "saving a pattern is not a map save"
    );
    assert_eq!(
        capture(&mut h, &evidence, "02-pattern-saved", &mut captures).to_bytes(),
        initial
    );

    // A different clipboard makes byte preservation a meaningful overwrite test.
    rectangle(&mut h, (3, 0), (3, 0));
    h.get_by_label("Pattern: 1 × 1");
    click(&mut h, "Save pattern");
    assert!(h.state().tilemap.as_ref().unwrap().test_error().is_some());
    assert_eq!(fs::read(&pattern_path).unwrap(), asset_bytes);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        initial_history
    );
    assert_eq!(files(&root), source_files);
    drop(h);

    // New App instance cannot inherit the original in-memory copied pattern.
    let mut h = open(&root);
    click(&mut h, "Tile tool (owns viewport clicks)");
    h.get_by_label("No copied pattern");
    let fresh_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    let scene_history = h.state().editor.history().clone();
    click(&mut h, "Reusable pattern files");
    pattern_file(&mut h, &pattern_path);
    click(&mut h, "Load pattern");
    assert!(h.state().tilemap.as_ref().unwrap().test_error().is_none());
    h.get_by_label("Pattern: 3 × 2");
    document(&h, &map, &BEFORE);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        fresh_history
    );
    assert_eq!(
        capture(&mut h, &evidence, "03-fresh-loaded", &mut captures).to_bytes(),
        initial
    );
    click(&mut h, "Stamp");
    stamp(&mut h, (4, 2));
    document(&h, &map, &STAMPED);
    let stamped_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    assert_eq!(stamped_history.2.len(), fresh_history.2.len() + 1);
    assert_eq!(
        capture(&mut h, &evidence, "04-stamped", &mut captures).to_bytes(),
        initial
    );
    click(&mut h, "Undo tile");
    document(&h, &map, &BEFORE);
    let undone = h.state().tilemap.as_ref().unwrap().history_snapshot();
    assert_eq!(undone.3.len(), 1);

    // A failed Load must retain not only current map but also redo and clipboard.
    pattern_file(&mut h, &malformed_path);
    click(&mut h, "Load pattern");
    assert!(h.state().tilemap.as_ref().unwrap().test_error().is_some());
    h.get_by_label("Pattern: 3 × 2");
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        undone
    );
    assert_eq!(
        capture(&mut h, &evidence, "05-malformed-keeps-redo", &mut captures).to_bytes(),
        initial
    );
    // Even structurally valid files must match the exact admitted artwork.
    for (name, field, value) in [
        ("digest", "package_digest", json!("f".repeat(64))),
        (
            "atlas",
            "atlas_json",
            json!(format!("{} ", asset["atlas_json"].as_str().unwrap())),
        ),
        ("tile-size", "tile_size", json!([8, 16])),
    ] {
        let mut incompatible = asset.clone();
        assert_ne!(incompatible[field], value);
        incompatible[field] = value;
        let path = evidence.join(format!("incompatible-{name}.pattern.json"));
        write_json(&path, &incompatible);
        pattern_file(&mut h, &path);
        click(&mut h, "Load pattern");
        assert!(h.state().tilemap.as_ref().unwrap().test_error().is_some());
        assert_eq!(
            h.state().tilemap.as_ref().unwrap().history_snapshot(),
            undone
        );
        h.get_by_label("Pattern: 3 × 2");
    }
    click(&mut h, "Redo tile");
    document(&h, &map, &STAMPED);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        stamped_history
    );
    click(&mut h, "Undo tile");
    // A new stamp, rather than only Redo, independently proves clipboard survival.
    click(&mut h, "Stamp");
    stamp(&mut h, (4, 2));
    document(&h, &map, &STAMPED);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        stamped_history
    );
    assert_eq!(h.state().editor.history(), &scene_history);
    assert_eq!(files(&root), source_files, "Load and Stamp never autosave");
    click(&mut h, "Save tilemap");
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    assert_eq!(
        capture(&mut h, &evidence, "06-map-saved", &mut captures).to_bytes(),
        initial
    );
    let saved = fs::read(&map_path).unwrap();
    let mut expected_map = map.clone();
    expected_map.cells = STAMPED.to_vec();
    assert_eq!(
        serde_json::from_slice::<orr_tilemap::Document>(&saved).unwrap(),
        expected_map
    );
    fs::write(evidence.join("saved.tilemap.json"), &saved).unwrap();
    let mut expected_files = source_files.clone();
    expected_files.insert(PathBuf::from("arena.tilemap.json"), saved);
    assert_eq!(files(&root), expected_files);
    drop(h);

    let mut reopened = open(&root);
    document(&reopened, &map, &STAMPED);
    assert!(!reopened.state().tilemap.as_ref().unwrap().dirty());
    assert!(
        reopened
            .state()
            .tilemap
            .as_ref()
            .unwrap()
            .history_snapshot()
            .2
            .is_empty()
    );
    assert_eq!(
        capture(&mut reopened, &evidence, "07-saved-reopen", &mut captures).to_bytes(),
        initial
    );
    click(&mut reopened, LBL_STEP);
    reopened.state_mut().sprites.playback.suspend_follow();
    reopened.state_mut().editor.camera = Camera::new([0., 80.], 160.);
    drain(&mut reopened, 3);
    let play = capture(&mut reopened, &evidence, "08-app-play", &mut captures);
    assert_eq!(reopened.state().editor.mode(), Mode::Play);
    assert_eq!(play.tick(), 1);
    app_play_oracle(&reopened, &evidence, "08-app-play", &STAMPED);
    let state = reopened.state().render_state.as_ref().unwrap();
    let gpu = Wgpu::from_parts(
        state.instance.clone(),
        state.adapter.clone(),
        state.device.clone(),
        state.queue.clone(),
    );
    let (_, runtime_frame) = consume_saved(
        &root,
        &evidence,
        &gpu,
        "09-runtime-saved",
        &STAMPED,
        &mut captures,
    );
    assert_eq!(
        play.to_bytes(),
        runtime_frame,
        "App Play and runtime full same-tick Frame parity"
    );
    assert_eq!(fs::read(&pattern_path).unwrap(), asset_bytes);
    assert_eq!(files(&root), expected_files);
    assert_eq!(captures.len(), 9);
    write_json(
        &evidence.join("receipt.json"),
        &json!({
            "completed":true,"captures":captures,"capture_count":9,
            "literal_before":BEFORE,"literal_stamped":STAMPED,
            "actual_app_play_oracle_texels":7168,"runtime_oracle_pixels":7168,
            "pattern_file_fnv1a64":fingerprint(&asset_bytes),
            "test_source_fnv1a64":fingerprint(include_bytes!("tilemap_pattern_asset_tests.rs")),
            "flow":"Copy → Save pattern → fresh App → Load pattern → Stamp → Undo → malformed Load → Redo → Undo → Stamp → Save tilemap → reopen → Play/runtime",
            "no_overwrite":true,"malformed_preserves_redo_and_clipboard":true,
            "incompatible_loads_rejected":["package_digest","atlas_json","tile_size"],
            "frame_parity":"authoring unchanged; actual App Play and saved runtime full same-tick Frame bytes equal",
            "native_os_input":false,"native_window":false,"windows":false,"export":false
        }),
    );
}
