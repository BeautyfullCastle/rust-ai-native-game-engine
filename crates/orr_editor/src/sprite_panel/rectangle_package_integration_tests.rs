//! Actual rectangle creator, explicit preview/install, Load/Assign/history and source-hidden reopen.
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
    kittest::{NodeT, Queryable},
    Harness,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const PACKAGE: &str = "created-rectangle-atlas";
const EXPECTED_JSON: &str = "{\n  \"format\": \"orr_sprite\",\n  \"version\": 1,\n  \"atlas\": {\n    \"image\": \"atlas.png\",\n    \"width\": 7,\n    \"height\": 5\n  },\n  \"regions\": [\n    {\n      \"id\": 10,\n      \"x\": 0,\n      \"y\": 0,\n      \"width\": 3,\n      \"height\": 2\n    },\n    {\n      \"id\": 42,\n      \"x\": 4,\n      \"y\": 1,\n      \"width\": 2,\n      \"height\": 3\n    },\n    {\n      \"id\": 900,\n      \"x\": 1,\n      \"y\": 3,\n      \"width\": 4,\n      \"height\": 2\n    }\n  ],\n  \"clips\": []\n}\n";

const PIXELS: [[u8; 4]; 35] = [
    [11, 17, 200, 255],
    [34, 17, 193, 255],
    [57, 17, 186, 255],
    [80, 17, 179, 255],
    [103, 17, 172, 255],
    [126, 17, 165, 255],
    [149, 17, 158, 255],
    [11, 56, 197, 255],
    [34, 56, 190, 255],
    [57, 56, 183, 255],
    [80, 56, 176, 255],
    [241, 7, 233, 0],
    [126, 56, 162, 255],
    [149, 56, 155, 255],
    [11, 95, 194, 255],
    [34, 95, 187, 255],
    [57, 95, 180, 255],
    [80, 95, 173, 255],
    [103, 95, 166, 255],
    [126, 95, 159, 255],
    [149, 95, 152, 255],
    [11, 134, 191, 255],
    [34, 134, 184, 255],
    [57, 134, 177, 255],
    [80, 134, 170, 255],
    [103, 134, 163, 255],
    [126, 134, 156, 255],
    [149, 134, 149, 255],
    [11, 173, 188, 255],
    [34, 173, 181, 255],
    [57, 173, 174, 255],
    [80, 173, 167, 255],
    [103, 173, 160, 255],
    [126, 173, 153, 255],
    [149, 173, 146, 255],
];

struct Fixture {
    temp: tempfile::TempDir,
    root: PathBuf,
    scene: PathBuf,
    guid: orr_reflect::Guid,
}

struct State {
    panel: SpritePanel,
    editor: Editor,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn png_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 7, 5);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&PIXELS.concat())
            .unwrap();
    }
    bytes
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

fn fixture() -> (Fixture, State) {
    let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/sprite_demo")
        .canonicalize()
        .unwrap();
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
        },
        State { panel, editor },
    )
}

fn harness(state: State) -> Harness<'static, State> {
    Harness::builder()
        .with_size([1500.0, 2800.0])
        .build_ui_state(
            |ui, state: &mut State| {
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

fn click(h: &mut Harness<'_, State>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}

fn text(h: &mut Harness<'_, State>, label: &str, value: &str) {
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

fn wait_rectangle(h: &mut Harness<'_, State>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().panel.rectangle_package.busy() {
        h.run_steps(3);
        assert!(
            Instant::now() < deadline,
            "{}",
            h.state().panel.rectangle_package.status
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(3);
}

fn wait_install(h: &mut Harness<'_, State>) {
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

fn configure_and_create(h: &mut Harness<'_, State>, fixture: &Fixture) -> PathBuf {
    let png = fixture.temp.path().join("input-asymmetric.png");
    let license = fixture.temp.path().join("rectangle-LICENSE.txt");
    let destination = fixture.temp.path().join("created rectangle source");
    fs::write(&png, png_bytes()).unwrap();
    fs::write(
        &license,
        b"Authored 7 by 5 test atlas; redistribution acknowledged",
    )
    .unwrap();

    click(h, "Sprite bindings (view only)");
    click(h, "Create sprite package from rectangles");
    for (label, value) in [
        ("Rectangle PNG file", png.to_str().unwrap()),
        ("Rectangle license file", license.to_str().unwrap()),
        ("Rectangle package name", PACKAGE),
        ("Rectangle package version", "1.0.0"),
        (
            "Rectangle package destination",
            destination.to_str().unwrap(),
        ),
        ("Rectangle ID row 1", "10"),
        ("Rectangle X row 1", "0"),
        ("Rectangle Y row 1", "0"),
        ("Rectangle Width row 1", "3"),
        ("Rectangle Height row 1", "2"),
    ] {
        text(h, label, value);
    }
    click(h, "Add rectangle");
    for (label, value) in [
        ("Rectangle ID row 2", "42"),
        ("Rectangle X row 2", "4"),
        ("Rectangle Y row 2", "1"),
        ("Rectangle Width row 2", "2"),
        ("Rectangle Height row 2", "3"),
    ] {
        text(h, label, value);
    }
    click(h, "Add rectangle");
    for (label, value) in [
        ("Rectangle ID row 3", "900"),
        ("Rectangle X row 3", "1"),
        ("Rectangle Y row 3", "3"),
        ("Rectangle Width row 3", "4"),
        ("Rectangle Height row 3", "2"),
    ] {
        text(h, label, value);
    }
    assert!(h
        .get_by_label("Create rectangle sprite package")
        .accesskit_node()
        .is_disabled());
    let project_before_inspection = snapshot(&fixture.root);
    click(h, "Inspect rectangle PNG");
    wait_rectangle(h);
    assert_eq!(snapshot(&fixture.root), project_before_inspection);
    assert!(h
        .get_by_label("Create rectangle sprite package")
        .accesskit_node()
        .is_disabled());
    assert!(h.state().panel.rectangle_package.status.contains("7 x 5"));
    assert!(h
        .state()
        .panel
        .rectangle_package
        .status
        .contains(&digest(&png_bytes())));
    click(h, "Rectangle rights acknowledgement");
    assert!(!h
        .get_by_label("Create rectangle sprite package")
        .accesskit_node()
        .is_disabled());
    click(h, "Create rectangle sprite package");
    wait_rectangle(h);
    let report = h
        .state()
        .panel
        .rectangle_package
        .created
        .as_ref()
        .expect("current create completion");
    assert_eq!(
        (report.width, report.height, report.region_count),
        (7, 5, 3)
    );
    assert_eq!(report.source_sha256, digest(&png_bytes()));
    assert!(h.state().panel.package_install.prepared.is_none());
    assert_eq!(
        fs::read(destination.join("atlas.png")).unwrap(),
        png_bytes()
    );
    assert_eq!(
        fs::read(destination.join("sprites.json")).unwrap(),
        EXPECTED_JSON.as_bytes()
    );
    assert_eq!(
        snapshot(&fixture.root),
        project_before_inspection,
        "creation writes only the external package source"
    );
    destination
}

fn preview_and_install(h: &mut Harness<'_, State>, source: &Path) {
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
        .expect("preview button produced a prepared install");
    let summary = prepared.plan.summary();
    assert_eq!((&*summary.name, &*summary.version), (PACKAGE, "1.0.0"));
    assert_eq!(
        (
            summary.width,
            summary.height,
            summary.regions,
            summary.clips
        ),
        (7, 5, 3, 0)
    );
    assert_eq!(summary.files, ["LICENSE.txt", "atlas.png", "sprites.json"]);
    assert_eq!(summary.files.iter().count(), 3);
    click(h, "Install previewed sprite package");
    assert!(h.state().panel.package_install.prepared.is_none());
    assert!(h
        .state()
        .panel
        .package_install
        .status
        .starts_with("Installed created-rectangle-atlas 1.0.0"));
}

fn assert_asset_oracle(asset: &Asset) {
    let document = &asset.document;
    assert_eq!((document.atlas().width, document.atlas().height), (7, 5));
    assert_eq!(asset.rgba, PIXELS.concat(), "complete atlas RGBA oracle");
    assert_eq!(
        document
            .regions()
            .iter()
            .map(|region| (region.id, region.x, region.y, region.width, region.height))
            .collect::<Vec<_>>(),
        vec![(10, 0, 0, 3, 2), (42, 4, 1, 2, 3), (900, 1, 3, 4, 2)]
    );
    assert_eq!(document.clips().len(), 0);
    for (id, expected_uv) in [
        (10, [0.0 / 7.0, 0.0 / 5.0, 3.0 / 7.0, 2.0 / 5.0]),
        (42, [4.0 / 7.0, 1.0 / 5.0, 6.0 / 7.0, 4.0 / 5.0]),
        (900, [1.0 / 7.0, 3.0 / 5.0, 5.0 / 7.0, 5.0 / 5.0]),
    ] {
        assert_eq!(document.uv_rect(id), Some(expected_uv), "region {id} UV");
        let region = document.region(id).unwrap();
        for local_y in 0..region.height {
            for local_x in 0..region.width {
                let x = region.x + local_x;
                let y = region.y + local_y;
                let offset = ((y * 7 + x) * 4) as usize;
                let expected = PIXELS[(y * 7 + x) as usize];
                assert_eq!(
                    &asset.rgba[offset..offset + 4],
                    &expected,
                    "region {id} pixel {x},{y}"
                );
            }
        }
    }
    let transparent = ((1 * 7 + 4) * 4) as usize;
    assert_eq!(&asset.rgba[transparent..transparent + 4], &[241, 7, 233, 0]);
    let region42 = &asset.rgba[((1 * 7 + 4) * 4) as usize..((1 * 7 + 4) * 4 + 4) as usize];
    let alternate10 = &asset.rgba[0..4];
    assert_ne!(
        region42, alternate10,
        "alternate region is a distinguishing negative control"
    );
}

#[test]
fn actual_rectangle_creator_preview_install_region_consumer_and_source_hidden_reopen() {
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
    let old_lock_bytes = fs::read(fixture.root.join("orr.packages.lock.json")).unwrap();
    let old_lock: orr_package::Lock = serde_json::from_slice(&old_lock_bytes).unwrap();
    let old_objects = snapshot(&fixture.root.join(".orr/packages/objects"));
    let old_cached_asset = h.state().panel.loaded
        [&("sample-sprites".into(), "sprites.json".into())]
        .asset
        .clone();
    let before = snapshot(&fixture.root);

    let source = configure_and_create(&mut h, &fixture);
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
    let installed_lock_bytes = fs::read(fixture.root.join("orr.packages.lock.json")).unwrap();
    let installed_lock: orr_package::Lock = serde_json::from_slice(&installed_lock_bytes).unwrap();
    assert_eq!(installed_lock.packages.len(), old_lock.packages.len() + 1);
    assert_eq!(installed_lock.direct.len(), old_lock.direct.len() + 1);
    for (name, locked) in &old_lock.packages {
        assert_eq!(&installed_lock.packages[name], locked);
    }
    for (name, version) in &old_lock.direct {
        assert_eq!(&installed_lock.direct[name], version);
    }
    assert_eq!(
        installed_lock.packages[PACKAGE].files["atlas.png"],
        digest(&png_bytes())
    );
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
        "install does not auto-load"
    );
    assert_eq!(h.state().panel.package, "sample-sprites");
    assert_eq!(h.state().panel.source, Source::Region(10));
    assert_eq!(
        h.state().panel.loaded[&("sample-sprites".into(), "sprites.json".into())]
            .asset
            .rgba,
        old_cached_asset.rgba
    );
    assert_eq!(
        h.state().panel.loaded[&("sample-sprites".into(), "sprites.json".into())]
            .asset
            .document
            .to_json()
            .unwrap(),
        old_cached_asset.document.to_json().unwrap()
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    for object in old_objects {
        assert!(snapshot(&fixture.root.join(".orr/packages/objects")).contains(&object));
    }
    assert!(snapshot(&fixture.root).len() > before.len());

    text(&mut h, "Package name", PACKAGE);
    text(&mut h, "Sprite document path in package", "sprites.json");
    click(&mut h, "Load sprite document");
    assert!(
        h.state().panel.error().is_none(),
        "{:?}",
        h.state().panel.error()
    );
    assert_asset_oracle(&h.state().panel.loaded[&(PACKAGE.into(), "sprites.json".into())].asset);
    click(&mut h, "Sprite source");
    click(&mut h, "Region 42");
    assert_eq!(h.state().panel.source, Source::Region(42));
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
    assert_eq!(binding.source, Source::Region(42));
    assert_eq!(
        binding
            .region(
                &h.state().panel.loaded[&(PACKAGE.into(), "sprites.json".into())]
                    .asset
                    .document,
                0
            )
            .unwrap(),
        42
    );
    assert_eq!(
        h.state()
            .panel
            .sampled_region(&h.state().editor, fixture.guid.as_str()),
        Some(42)
    );
    assert!(h.state().panel.bindings.as_ref().unwrap().dirty());

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

    let hidden_png = fixture.temp.path().join("hidden-input-asymmetric.png");
    let hidden_license = fixture.temp.path().join("hidden-rectangle-LICENSE.txt");
    let hidden_source = fixture.temp.path().join("hidden-created-rectangle-source");
    fs::rename(
        fixture.temp.path().join("input-asymmetric.png"),
        &hidden_png,
    )
    .unwrap();
    fs::rename(
        fixture.temp.path().join("rectangle-LICENSE.txt"),
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
    assert_asset_oracle(installed);
    let binding = &fresh.bindings.as_ref().unwrap().document().bindings[fixture.guid.as_str()];
    assert_eq!(binding.region(&installed.document, 0).unwrap(), 42);
    assert_eq!(
        fresh.sampled_region(&h.state().editor, fixture.guid.as_str()),
        Some(42)
    );
    assert_eq!(fs::read(hidden_png).unwrap(), png_bytes());
    assert_eq!(
        fs::read(hidden_license).unwrap().as_slice(),
        b"Authored 7 by 5 test atlas; redistribution acknowledged"
    );
    assert!(hidden_source.join("sprites.json").is_file());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
}
