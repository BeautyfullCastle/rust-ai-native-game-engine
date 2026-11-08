//! Exercise JSON package creation through egui, then use the existing explicit
//! preview/install and sprite Load/Assign/Save/reopen consumer flow.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use crate::{
    editor::Editor,
    game::EditorGame,
    model::Mode,
    sprite_bindings::{self, Asset, Binding, Bindings, Source},
    sprite_panel::SpritePanel,
};
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const PACKAGE: &str = "json-authored-atlas";
const SEQUENCE: &str = "json-sequence";

struct Fixture {
    temp: tempfile::TempDir,
    root: PathBuf,
    scene: PathBuf,
    guid: orr_reflect::Guid,
    sample_atlas: PathBuf,
}

struct HarnessState {
    panel: SpritePanel,
    editor: Editor,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.push((path.strip_prefix(root).unwrap().into(), Vec::new()));
                visit(root, &path, out);
            } else {
                out.push((
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort();
    out
}

fn settle(editor: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.sync();
        if editor.mode() == Mode::Edit
            && editor.can_mutate()
            && SpritePanel::display_coherent(editor)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "coherent editable Arena fixture: {:?}",
            editor.status()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn fixture() -> (Fixture, HarnessState) {
    let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/sprite_demo")
        .canonicalize()
        .unwrap();
    let sample_atlas = sample.join("lantern_keeper.png");
    orr_package::Project::open_for_install(
        &root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[sample])
    .unwrap();
    let scene = root.join("arena.yaml");
    fs::write(
        &scene,
        include_str!("../../../../scenes/arena_blank.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Arena).unwrap();
    settle(&mut editor);
    assert!(editor.spawn_arena_player(0, [0.0, 0.0]));
    settle(&mut editor);
    let guid = editor.selected_guid().unwrap();
    assert!(editor.save());
    settle(&mut editor);
    let path = root.join("arena.sprites.json");
    let mut bindings = Bindings::create(path.clone(), "arena.yaml".into(), ".".into()).unwrap();
    let binding = Binding {
        package: "sample-sprites".into(),
        document: "sprites.json".into(),
        source: Source::Region(10),
        units_per_pixel: 0.2,
        orientation: None,
    };
    let admitted = sprite_bindings::load_asset(&root, &binding.package, &binding.document).unwrap();
    binding.region(&admitted.document, 0).unwrap();
    bindings
        .assign(std::slice::from_ref(&guid), Some(binding))
        .unwrap();
    bindings.save().unwrap();
    let panel = SpritePanel {
        bindings: Some(Bindings::open(path).unwrap()),
        package: "sample-sprites".into(),
        document: "sprites.json".into(),
        source: Source::Region(10),
        scale: 0.2,
        ..Default::default()
    };
    (
        Fixture {
            temp,
            root,
            scene,
            guid,
            sample_atlas,
        },
        HarnessState { panel, editor },
    )
}

fn harness(state: HarnessState) -> Harness<'static, HarnessState> {
    Harness::builder()
        .with_size([1500.0, 2600.0])
        .build_ui_state(
            |ui, state: &mut HarnessState| {
                state.editor.sync();
                if state.panel.loaded.is_empty() {
                    state
                        .panel
                        .load(ui.ctx(), "sample-sprites", "sprites.json")
                        .unwrap();
                }
                state.panel.show(ui, &state.editor);
            },
            state,
        )
}

fn click(h: &mut Harness<'_, HarnessState>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}

fn text(h: &mut Harness<'_, HarnessState>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.key_press(egui::Key::Backspace);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}

fn wait_json(h: &mut Harness<'_, HarnessState>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().panel.json_package.busy() {
        h.run_steps(3);
        assert!(
            Instant::now() < deadline,
            "{}",
            h.state().panel.json_package.status
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(3);
}

fn wait_install(h: &mut Harness<'_, HarnessState>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().panel.package_install.busy() {
        h.run_steps(3);
        assert!(
            Instant::now() < deadline,
            "{}",
            h.state().panel.package_install.status
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(3);
}

fn export_sequence(h: &mut Harness<'_, HarnessState>, fixture: &Fixture) -> PathBuf {
    let output = fixture.temp.path().join("clip-sequence-export.json");
    click(h, "Sprite bindings (view only)");
    click(h, "Begin clip sequence");
    text(h, "Sequence clip ID", SEQUENCE);
    click(h, "Apply sequence ID");
    text(h, "Selected sequence frame duration ms", "70");
    click(h, "Apply sequence frame duration");

    click(h, "Sequence append region");
    click(h, "Sequence region 20");
    click(h, "Append sequence frame");
    text(h, "Selected sequence frame duration ms", "230");
    click(h, "Apply sequence frame duration");

    click(h, "Sequence append region");
    click(h, "Sequence region 10");
    click(h, "Append sequence frame");
    text(h, "Selected sequence frame duration ms", "90");
    click(h, "Apply sequence frame duration");

    click(h, "Sequence append region");
    click(h, "Sequence region 21");
    click(h, "Append sequence frame");
    text(h, "Selected sequence frame duration ms", "310");
    click(h, "Apply sequence frame duration");
    click(h, "Sequence Once");
    text(h, "Sequence output path", output.to_str().unwrap());
    click(h, "Export clip sequence JSON");

    let json = fs::read_to_string(&output).expect("clip sequence exported from the UI");
    let document = orr_sprite::SpriteDocument::from_json(&json).unwrap();
    let clip = document.clip(SEQUENCE).expect("exported clip sequence");
    assert_eq!(clip.mode(), orr_sprite::PlaybackMode::Once);
    assert_eq!(
        clip.frames()
            .iter()
            .map(|frame| (frame.region, frame.duration_ms))
            .collect::<Vec<_>>(),
        vec![(10, 70), (20, 230), (10, 90), (21, 310)]
    );
    output
}

fn configure_and_create(
    h: &mut Harness<'_, HarnessState>,
    fixture: &Fixture,
    json: &Path,
) -> PathBuf {
    let png = fixture.temp.path().join("input-lantern-keeper.png");
    let license = fixture.temp.path().join("json-LICENSE.txt");
    let destination = fixture.temp.path().join("created json source");
    fs::copy(&fixture.sample_atlas, &png).unwrap();
    fs::write(
        &license,
        b"Authored clip timing variant; redistribution acknowledged",
    )
    .unwrap();

    click(h, "Create sprite package from JSON");
    for (label, value) in [
        ("Sprite JSON file", json.to_str().unwrap()),
        ("Sprite JSON PNG file", png.to_str().unwrap()),
        ("Sprite JSON license file", license.to_str().unwrap()),
        ("Sprite JSON package name", PACKAGE),
        ("Sprite JSON package version", "1.0.0"),
        (
            "Sprite JSON package destination",
            destination.to_str().unwrap(),
        ),
    ] {
        text(h, label, value);
    }
    assert!(
        h.get_by_label("Create sprite JSON package")
            .accesskit_node()
            .is_disabled()
    );
    let project_before_inspection = snapshot(&fixture.root);
    click(h, "Inspect sprite JSON and PNG");
    wait_json(h);
    assert_eq!(snapshot(&fixture.root), project_before_inspection);
    assert!(
        h.get_by_label("Create sprite JSON package")
            .accesskit_node()
            .is_disabled()
    );
    assert!(h.state().panel.json_package.status.contains("64 x 16"));
    assert!(
        h.state()
            .panel
            .json_package
            .status
            .contains(&digest(&fs::read(json).unwrap()))
    );
    assert!(
        h.state()
            .panel
            .json_package
            .status
            .contains(&digest(&fs::read(&png).unwrap()))
    );
    click(h, "Sprite JSON rights acknowledgement");
    assert!(
        !h.get_by_label("Create sprite JSON package")
            .accesskit_node()
            .is_disabled()
    );
    click(h, "Create sprite JSON package");
    wait_json(h);
    let report = h
        .state()
        .panel
        .json_package
        .created
        .as_ref()
        .expect("current JSON package creation completion");
    assert_eq!(
        (
            report.width,
            report.height,
            report.region_count,
            report.clip_count
        ),
        (64, 16, 4, 4)
    );
    assert_eq!(report.image_filename, "lantern_keeper.png");
    let json_bytes = fs::read(json).unwrap();
    let png_bytes = fs::read(&png).unwrap();
    assert_eq!(report.json_sha256, digest(&json_bytes));
    assert_eq!(report.png_sha256, digest(&png_bytes));
    assert!(h.state().panel.package_install.prepared.is_none());
    assert_eq!(
        fs::read(destination.join("lantern_keeper.png")).unwrap(),
        png_bytes
    );
    assert_eq!(
        fs::read(destination.join("sprites.json")).unwrap(),
        json_bytes
    );
    assert_eq!(
        snapshot(&fixture.root),
        project_before_inspection,
        "source creation leaves the project unchanged"
    );
    destination
}

fn preview_and_install(h: &mut Harness<'_, HarnessState>, source: &Path) {
    click(h, "Install local sprite package");
    text(
        h,
        "Local sprite package directory",
        source.to_str().unwrap(),
    );
    let before_preview = snapshot(h.state().panel.bindings.as_ref().unwrap().base());
    click(h, "Preview sprite package install");
    wait_install(h);
    assert_eq!(
        snapshot(h.state().panel.bindings.as_ref().unwrap().base()),
        before_preview,
        "explicit package preview is read-only"
    );
    let prepared = h
        .state()
        .panel
        .package_install
        .prepared
        .as_ref()
        .expect("preview produced a prepared local install");
    let summary = prepared.plan.summary();
    assert_eq!((&*summary.name, &*summary.version), (PACKAGE, "1.0.0"));
    assert_eq!(
        (
            summary.width,
            summary.height,
            summary.regions,
            summary.clips
        ),
        (64, 16, 4, 4)
    );
    assert_eq!(
        summary.files,
        ["LICENSE.txt", "lantern_keeper.png", "sprites.json"]
    );
    click(h, "Install previewed sprite package");
    assert!(h.state().panel.package_install.prepared.is_none());
    assert!(
        h.state()
            .panel
            .package_install
            .status
            .starts_with("Installed json-authored-atlas 1.0.0")
    );
}

fn assert_asset_oracle(
    asset: &Asset,
    expected_document: &orr_sprite::SpriteDocument,
    expected_rgba: &[u8],
) {
    let document = &asset.document;
    assert_eq!((document.atlas().width, document.atlas().height), (64, 16));
    assert_eq!(asset.rgba, expected_rgba, "complete admitted atlas RGBA");
    assert_eq!(
        document.to_json().unwrap(),
        expected_document.to_json().unwrap(),
        "all admitted regions and clips survive package creation"
    );
    assert_eq!(
        document
            .regions()
            .iter()
            .map(|region| (region.id, region.x, region.y, region.width, region.height))
            .collect::<Vec<_>>(),
        vec![
            (10, 0, 0, 16, 16),
            (11, 16, 0, 16, 16),
            (20, 32, 0, 16, 16),
            (21, 48, 0, 16, 16)
        ]
    );
    assert_eq!(document.clips().len(), 4);
    let loop_clip = document.clip("idle").unwrap();
    assert_eq!(loop_clip.mode(), orr_sprite::PlaybackMode::Loop);
    assert_eq!(loop_clip.duration_ms(), 1000);
    assert_eq!(
        (loop_clip.sample(999).region, loop_clip.sample(999).finished),
        (11, false)
    );
    assert_eq!(
        (
            loop_clip.sample(1000).region,
            loop_clip.sample(1000).finished
        ),
        (10, false)
    );
    assert_eq!(
        (
            loop_clip.sample(u64::MAX).region,
            loop_clip.sample(u64::MAX).finished
        ),
        (11, false),
        "large elapsed loop sampling wraps without overflow"
    );

    let existing_once = document.clip("greet").unwrap();
    assert_eq!(existing_once.mode(), orr_sprite::PlaybackMode::Once);
    assert_eq!(
        (
            existing_once.sample(500).region,
            existing_once.sample(500).finished
        ),
        (11, true)
    );
    assert_eq!(
        (
            existing_once.sample(u64::MAX).region,
            existing_once.sample(u64::MAX).finished
        ),
        (11, true),
        "large elapsed once sampling holds the final frame"
    );

    let authored = document.clip(SEQUENCE).unwrap();
    assert_eq!(authored.mode(), orr_sprite::PlaybackMode::Once);
    assert_eq!(authored.duration_ms(), 700);
    for (elapsed, expected_region, finished) in [
        (0, 10, false),
        (69, 10, false),
        (70, 20, false),
        (299, 20, false),
        (300, 10, false),
        (390, 21, false),
        (699, 21, false),
        (700, 21, true),
        (u64::MAX, 21, true),
    ] {
        let sample = authored.sample(elapsed);
        assert_eq!(
            (sample.region, sample.finished),
            (expected_region, finished),
            "at {elapsed} ms"
        );
    }
}

#[test]
fn actual_sequence_export_json_create_install_clip_consumer_and_source_hidden_reopen() {
    let (fixture, state) = fixture();
    let mut h = harness(state);
    h.run_steps(3);
    let bindings_path = fixture.root.join("arena.sprites.json");
    let original_sidecar_bytes = fs::read(&bindings_path).unwrap();
    let original_bindings = h
        .state()
        .panel
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let scene_bytes = fs::read(&fixture.scene).unwrap();
    let before = snapshot(&fixture.root);
    let original_asset = h.state().panel.loaded[&("sample-sprites".into(), "sprites.json".into())]
        .asset
        .clone();

    let exported_json = export_sequence(&mut h, &fixture);
    let exported_bytes = fs::read(&exported_json).unwrap();
    let exported_document =
        orr_sprite::SpriteDocument::from_json(std::str::from_utf8(&exported_bytes).unwrap())
            .unwrap();
    assert_eq!(
        snapshot(&fixture.root),
        before,
        "sequence export is external to the project"
    );
    assert_eq!(fs::read(&bindings_path).unwrap(), original_sidecar_bytes);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);

    let source = configure_and_create(&mut h, &fixture, &exported_json);
    assert_eq!(fs::read(&bindings_path).unwrap(), original_sidecar_bytes);
    assert_eq!(
        h.state().panel.bindings.as_ref().unwrap().document(),
        &original_bindings
    );
    assert!(!h.state().panel.bindings.as_ref().unwrap().dirty());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);

    let source_before_install = snapshot(&source);
    preview_and_install(&mut h, &source);
    assert_eq!(snapshot(&source), source_before_install);
    assert_eq!(fs::read(&bindings_path).unwrap(), original_sidecar_bytes);
    assert_eq!(
        h.state().panel.bindings.as_ref().unwrap().document(),
        &original_bindings
    );
    assert!(!h.state().panel.bindings.as_ref().unwrap().dirty());
    assert_eq!(
        h.state().panel.loaded.len(),
        1,
        "install does not auto-load the new sprite"
    );
    assert_eq!(h.state().panel.package, "sample-sprites");
    assert_eq!(h.state().panel.source, Source::Region(10));
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert!(snapshot(&fixture.root).len() > before.len());

    text(&mut h, "Package name", PACKAGE);
    text(&mut h, "Sprite document path in package", "sprites.json");
    click(&mut h, "Load sprite document");
    assert!(
        h.state().panel.error().is_none(),
        "{:?}",
        h.state().panel.error()
    );
    assert_asset_oracle(
        &h.state().panel.loaded[&(PACKAGE.into(), "sprites.json".into())].asset,
        &exported_document,
        &original_asset.rgba,
    );
    click(&mut h, "Sprite source");
    click(&mut h, &format!("Clip {SEQUENCE}"));
    assert_eq!(h.state().panel.source, Source::Clip(SEQUENCE.into()));
    click(&mut h, "Assign sprite to selection");
    assert!(
        h.state().panel.error().is_none(),
        "{:?}",
        h.state().panel.error()
    );
    let assigned = h
        .state()
        .panel
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let binding = &assigned.bindings[fixture.guid.as_str()];
    assert_eq!(
        (&*binding.package, &*binding.document),
        (PACKAGE, "sprites.json")
    );
    assert_eq!(binding.source, Source::Clip(SEQUENCE.into()));
    assert_eq!(
        h.state()
            .panel
            .sampled_region(&h.state().editor, fixture.guid.as_str()),
        Some(10)
    );
    assert!(h.state().panel.bindings.as_ref().unwrap().dirty());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);

    click(&mut h, "Undo binding");
    assert_eq!(
        h.state().panel.bindings.as_ref().unwrap().document(),
        &original_bindings
    );
    click(&mut h, "Redo binding");
    assert_eq!(
        h.state().panel.bindings.as_ref().unwrap().document(),
        &assigned
    );

    click(&mut h, "Save bindings");
    assert!(!h.state().panel.bindings.as_ref().unwrap().dirty());
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    let assigned_sidecar_bytes = fs::read(&bindings_path).unwrap();

    let hidden_json = fixture.temp.path().join("hidden-clip-sequence-export.json");
    let hidden_png = fixture.temp.path().join("hidden-input-lantern-keeper.png");
    let hidden_license = fixture.temp.path().join("hidden-json-LICENSE.txt");
    let hidden_source = fixture.temp.path().join("hidden-created-json-source");
    fs::rename(&exported_json, &hidden_json).unwrap();
    fs::rename(
        fixture.temp.path().join("input-lantern-keeper.png"),
        &hidden_png,
    )
    .unwrap();
    fs::rename(
        fixture.temp.path().join("json-LICENSE.txt"),
        &hidden_license,
    )
    .unwrap();
    fs::rename(&source, &hidden_source).unwrap();
    assert!(!source.exists());

    let mut fresh = SpritePanel::default();
    fresh
        .open(&egui::Context::default(), bindings_path.clone())
        .unwrap();
    assert_eq!(fresh.bindings.as_ref().unwrap().document(), &assigned);
    assert!(!fresh.bindings.as_ref().unwrap().dirty());
    assert_eq!(fs::read(&bindings_path).unwrap(), assigned_sidecar_bytes);
    assert_eq!(fresh.loaded.len(), 1);
    let installed = &fresh.loaded[&(PACKAGE.into(), "sprites.json".into())].asset;
    assert_asset_oracle(installed, &exported_document, &original_asset.rgba);
    let binding = &fresh.bindings.as_ref().unwrap().document().bindings[fixture.guid.as_str()];
    assert_eq!(binding.source, Source::Clip(SEQUENCE.into()));
    assert_eq!(
        fresh.sampled_region(&h.state().editor, fixture.guid.as_str()),
        Some(10)
    );
    assert_eq!(fs::read(&hidden_json).unwrap(), exported_bytes);
    assert_eq!(
        fs::read(&hidden_png).unwrap(),
        fs::read(&fixture.sample_atlas).unwrap()
    );
    assert_eq!(
        fs::read(hidden_license).unwrap().as_slice(),
        b"Authored clip timing variant; redistribution acknowledged"
    );
    assert!(hidden_source.join("sprites.json").is_file());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
}
