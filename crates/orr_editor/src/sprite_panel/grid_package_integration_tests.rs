//! Real CPU creator/Load/Assign/history widgets and source-hidden playback.
//! Package installation uses the existing Project API explicitly, not an install UI.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use crate::{
    editor::Editor,
    game::EditorGame,
    model::Mode,
    sprite_bindings::{self, Asset, Binding, Bindings, Orientation, Source},
    sprite_panel::SpritePanel,
};
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_sprite::PlaybackMode;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const PACKAGE: &str = "created-sprite-grid";
const CLIP: &str = "walk-grid";
const ORIENTATION: Orientation = Orientation {
    quarter_turns: 1,
    flip_x: true,
    flip_y: false,
};
// A 2x2 tile grid, with asymmetric 2x2 pixels inside every tile. Alpha is data;
// the CPU oracle reads the decoded straight RGBA, not an egui/GPU screenshot.
const PIXELS: [[u8; 4]; 16] = [
    [240, 1, 2, 255],
    [220, 3, 4, 200],
    [5, 240, 6, 255],
    [7, 220, 8, 190],
    [200, 9, 10, 180],
    [180, 11, 12, 170],
    [13, 200, 14, 160],
    [15, 180, 16, 150],
    [17, 18, 240, 140],
    [19, 20, 220, 130],
    [240, 240, 21, 120],
    [220, 220, 22, 110],
    [23, 24, 200, 100],
    [25, 26, 180, 90],
    [200, 200, 27, 80],
    [180, 180, 28, 70],
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
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn png_bytes(changed: bool) -> Vec<u8> {
    let mut pixels = PIXELS.concat();
    if changed {
        pixels[0] = 99;
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 4, 4);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
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
    settle(&mut editor); // public Save refreshes state/history asynchronously.
    let path = root.join("arena.sprites.json");
    let mut bindings = Bindings::create(path.clone(), "arena.yaml".into(), ".".into()).unwrap();
    let binding = Binding {
        package: "sample-sprites".into(),
        document: "sprites.json".into(),
        source: Source::Clip("idle".into()),
        units_per_pixel: 0.2,
        orientation: Some(ORIENTATION),
    };
    let admitted = sprite_bindings::load_asset(&root, &binding.package, &binding.document).unwrap();
    binding.region(&admitted.document, 0).unwrap(); // validate the fixture reference before authoring it.
    bindings
        .assign(std::slice::from_ref(&guid), Some(binding))
        .unwrap();
    bindings.save().unwrap();
    let panel = SpritePanel {
        bindings: Some(Bindings::open(path).unwrap()),
        package: "sample-sprites".into(),
        document: "sprites.json".into(),
        source: Source::Clip("idle".into()),
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
        .with_size([1400.0, 2600.0])
        .build_ui_state(
            |ui, state: &mut State| {
                state.editor.sync();
                if state.panel.loaded.is_empty() && state.panel.package == "sample-sprites" {
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
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}
fn wait(h: &mut Harness<'_, State>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().panel.grid_package.busy() {
        h.run_steps(3);
        assert!(
            Instant::now() < deadline,
            "{}",
            h.state().panel.grid_package.status
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(3);
}
fn configure(h: &mut Harness<'_, State>, fixture: &Fixture, mode: PlaybackMode) {
    let png = fixture.temp.path().join("input-grid.png");
    fs::write(&png, png_bytes(false)).unwrap();
    let license = fixture.temp.path().join("grid-LICENSE.txt");
    fs::write(
        &license,
        b"Original test image; redistribution acknowledged",
    )
    .unwrap();
    let destination = fixture.temp.path().join("created grid package");
    h.run_steps(3);
    click(h, "Sprite bindings (view only)");
    click(h, "Create sprite package from PNG grid");
    for (label, value) in [
        ("Sprite grid PNG file", png.to_str().unwrap()),
        ("Grid cell width", "2"),
        ("Grid cell height", "2"),
        ("Grid clip name", CLIP),
        ("Grid frame duration (ms)", "125"),
        ("Grid license file", license.to_str().unwrap()),
        ("Grid package name", PACKAGE),
        ("Grid package version", "1.0.0"),
        ("Grid package destination", destination.to_str().unwrap()),
    ] {
        text(h, label, value);
    }
    click(h, "Grid playback mode");
    click(
        h,
        match mode {
            PlaybackMode::Loop => "Loop grid clip",
            PlaybackMode::Once => "Play grid clip once",
        },
    );
    assert!(h
        .get_by_label("Create sprite grid package")
        .accesskit_node()
        .is_disabled());
    let before = snapshot(fixture.temp.path());
    click(h, "Inspect sprite PNG");
    wait(h);
    assert_eq!(
        snapshot(fixture.temp.path()),
        before,
        "inspection cannot publish or edit inputs/project"
    );
    assert!(
        h.get_by_label("Create sprite grid package")
            .accesskit_node()
            .is_disabled(),
        "inspection cannot grant rights"
    );
    assert_eq!(
        h.get_by_label("Grid rights acknowledgement")
            .accesskit_node()
            .toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    click(h, "Grid rights acknowledgement");
    assert!(!h
        .get_by_label("Create sprite grid package")
        .accesskit_node()
        .is_disabled());
}
fn assert_playback(asset: &Asset, binding: &Binding, mode: PlaybackMode) {
    let document = &asset.document;
    assert_eq!((document.atlas().width, document.atlas().height), (4, 4));
    assert_eq!(document.atlas().image, "atlas.png");
    assert_eq!(document.regions().len(), 4);
    assert_eq!(document.clips().len(), 1);
    assert_eq!(asset.rgba, PIXELS.concat());
    let clip = document.clip(CLIP).unwrap();
    assert_eq!(clip.mode(), mode);
    assert_eq!(clip.duration_ms(), 500);
    assert_eq!(
        clip.frames(),
        &[
            orr_sprite::Frame {
                region: 0,
                duration_ms: 125
            },
            orr_sprite::Frame {
                region: 1,
                duration_ms: 125
            },
            orr_sprite::Frame {
                region: 2,
                duration_ms: 125
            },
            orr_sprite::Frame {
                region: 3,
                duration_ms: 125
            }
        ]
    );
    for (id, region) in document.regions().iter().enumerate() {
        let (column, row) = (id % 2, id / 2);
        assert_eq!(
            *region,
            orr_sprite::Region {
                id: id as u32,
                x: column as u32 * 2,
                y: row as u32 * 2,
                width: 2,
                height: 2
            }
        );
        assert_eq!(
            document.uv_rect(id as u32).unwrap(),
            [
                column as f32 * 0.5,
                row as f32 * 0.5,
                (column + 1) as f32 * 0.5,
                (row + 1) as f32 * 0.5
            ]
        );
    }
    assert_eq!(binding.orientation, Some(ORIENTATION));
    assert_eq!(binding.source, Source::Clip(CLIP.into()));
    assert_eq!(binding.units_per_pixel, 0.2);
    for elapsed in [0, 124, 125, 249, 250, 374, 375, 499, 500, 625, u64::MAX] {
        let expected = match mode {
            PlaybackMode::Loop => (elapsed / 125) % 4,
            PlaybackMode::Once => (elapsed / 125).min(3),
        } as u32;
        let region = binding.region(document, elapsed).unwrap();
        assert_eq!(region, expected, "elapsed {elapsed}");
        assert_eq!(
            clip.sample(elapsed).finished,
            mode == PlaybackMode::Once && elapsed >= 500
        );
        let uv = ORIENTATION.uv_rect(document.uv_rect(region).unwrap());
        let (column, row) = (region % 2, region / 2);
        assert_eq!(
            uv,
            [
                (column + 1) as f32 * 0.5,
                row as f32 * 0.5,
                column as f32 * 0.5,
                (row + 1) as f32 * 0.5
            ]
        );
        for local_y in 0..2 {
            for local_x in 0..2 {
                let u = uv[0] + (uv[2] - uv[0]) * (local_x as f32 * 0.5 + 0.25);
                let v = uv[1] + (uv[3] - uv[1]) * (local_y as f32 * 0.5 + 0.25);
                let pixel = ((v * 4.0) as usize * 4 + (u * 4.0) as usize) * 4;
                let expected_pixel =
                    ((row * 2 + local_y) * 4 + column * 2 + (1 - local_x)) as usize;
                assert_eq!(
                    &asset.rgba[pixel..pixel + 4],
                    &PIXELS[expected_pixel],
                    "top-left row-major frame {region}, mirrored texel {local_x},{local_y}"
                );
            }
        }
    }
}

#[test]
fn actual_grid_creator_install_load_clip_assign_history_save_source_hidden_cpu_playback() {
    for mode in [PlaybackMode::Loop, PlaybackMode::Once] {
        let (fixture, state) = fixture();
        let mut h = harness(state);
        h.run_steps(3);
        let original = h
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
        configure(&mut h, &fixture, mode);
        click(&mut h, "Create sprite grid package");
        wait(&mut h);
        let report = h
            .state_mut()
            .panel
            .grid_package
            .created
            .take()
            .expect("current inspected creator result");
        assert_eq!(report.package, PACKAGE);
        assert_eq!(report.document, "sprites.json");
        assert_eq!((report.width, report.height, report.cells), (4, 4, 4));
        assert_eq!(report.source_sha256, digest(&png_bytes(false)));
        assert_eq!(
            fs::read(report.root.join("atlas.png")).unwrap(),
            png_bytes(false)
        );
        assert_eq!(
            fs::read(report.root.join("LICENSE.txt")).unwrap(),
            b"Original test image; redistribution acknowledged"
        );
        assert_eq!(
            snapshot(&fixture.root),
            before,
            "creation does not install, assign or edit the project"
        );
        assert_eq!(
            h.state().panel.bindings.as_ref().unwrap().document(),
            &original
        );
        assert!(!h.state().panel.bindings.as_ref().unwrap().dirty());
        assert!(!h
            .state()
            .panel
            .loaded
            .contains_key(&(PACKAGE.into(), "sprites.json".into())));
        let old_objects = snapshot(&fixture.root.join(".orr/packages/objects"));
        // Explicit existing API install. This is deliberately not a new UI action.
        let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/sprite_demo")
            .canonicalize()
            .unwrap();
        orr_package::Project::open_for_install(
            &fixture.root,
            orr_package::Runtime::content_only().engine_version,
        )
        .unwrap()
        .install(&[sample, report.root.clone()])
        .unwrap();
        let installed = snapshot(&fixture.root.join(".orr/packages/objects"));
        for object in old_objects {
            assert!(installed.contains(&object));
        }
        h.state_mut().panel.package = PACKAGE.into();
        h.state_mut().panel.document = "sprites.json".into();
        click(&mut h, "Load sprite document");
        assert!(
            h.state().panel.error().is_none(),
            "{:?}",
            h.state().panel.error()
        );
        click(&mut h, "Sprite source");
        click(&mut h, &format!("Clip {CLIP}"));
        assert_eq!(h.state().panel.source, Source::Clip(CLIP.into()));
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
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
        assert_playback(
            &h.state().panel.loaded[&(PACKAGE.into(), "sprites.json".into())].asset,
            &assigned.bindings[fixture.guid.as_str()],
            mode,
        );
        assert!(h.state().panel.bindings.as_ref().unwrap().dirty());
        click(&mut h, "Undo binding");
        assert_eq!(
            h.state().panel.bindings.as_ref().unwrap().document(),
            &original
        );
        click(&mut h, "Redo binding");
        assert_eq!(
            h.state().panel.bindings.as_ref().unwrap().document(),
            &assigned
        );
        click(&mut h, "Save bindings");
        assert!(!h.state().panel.bindings.as_ref().unwrap().dirty());
        assert_eq!(h.state().editor.checksum(), checksum);
        assert_eq!(h.state().editor.history(), &history);
        assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
        assert_eq!(
            snapshot(&fixture.root.join(".orr/packages/objects")),
            installed
        );
        fs::rename(
            fixture.temp.path().join("input-grid.png"),
            fixture.temp.path().join("hidden-input.png"),
        )
        .unwrap();
        fs::rename(
            fixture.temp.path().join("grid-LICENSE.txt"),
            fixture.temp.path().join("hidden-license.txt"),
        )
        .unwrap();
        fs::rename(
            &report.root,
            fixture.temp.path().join("hidden-created-source"),
        )
        .unwrap();
        let mut fresh = SpritePanel::default();
        fresh
            .open(
                &egui::Context::default(),
                fixture.root.join("arena.sprites.json"),
            )
            .unwrap();
        assert_eq!(fresh.bindings.as_ref().unwrap().document(), &assigned);
        assert!(!fresh.bindings.as_ref().unwrap().dirty());
        let binding = &fresh.bindings.as_ref().unwrap().document().bindings[fixture.guid.as_str()];
        assert_playback(
            &fresh.loaded[&(PACKAGE.into(), "sprites.json".into())].asset,
            binding,
            mode,
        );
        assert_eq!(
            fresh.sampled_region(&h.state().editor, fixture.guid.as_str()),
            Some(0)
        );
        assert_eq!(
            fs::read(fixture.temp.path().join("hidden-input.png")).unwrap(),
            png_bytes(false)
        );
        assert_eq!(h.state().editor.checksum(), checksum);
    }
}

#[test]
fn inspected_same_size_png_change_precedes_license_read_and_existing_destination_never_overwrites()
{
    let (fixture, state) = fixture();
    let mut h = harness(state);
    h.run_steps(3);
    configure(&mut h, &fixture, PlaybackMode::Loop);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let bindings = h
        .state()
        .panel
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    fs::write(fixture.temp.path().join("input-grid.png"), png_bytes(true)).unwrap();
    fs::remove_file(fixture.temp.path().join("grid-LICENSE.txt")).unwrap();
    let before = snapshot(fixture.temp.path());
    click(&mut h, "Create sprite grid package");
    wait(&mut h);
    assert!(h.state().panel.grid_package.created.is_none());
    let status = h.state().panel.grid_package.status.to_ascii_lowercase();
    assert!(
        status.contains("changed") || status.contains("hash"),
        "same-size PNG hash must reject before missing license: {status}"
    );
    assert_eq!(snapshot(fixture.temp.path()), before);
    let destination = fixture.temp.path().join("created grid package");
    assert!(!destination.exists());
    fs::write(
        fixture.temp.path().join("grid-LICENSE.txt"),
        b"Original test image; redistribution acknowledged",
    )
    .unwrap();
    click(&mut h, "Inspect sprite PNG");
    wait(&mut h);
    let stale_destination = fixture.temp.path().join("prior context output");
    text(
        &mut h,
        "Grid package destination",
        stale_destination.to_str().unwrap(),
    );
    click(&mut h, "Grid rights acknowledgement");
    // Submit through the actual button, then remove its bindings context before
    // the next poll. This is context setup through the public panel field, not
    // a claim that a native Close action was exercised.
    h.get_by_label("Create sprite grid package")
        .click_accesskit();
    h.run_steps(1);
    assert!(h.state().panel.grid_package.busy());
    let detached = h.state_mut().panel.bindings.take().unwrap();
    wait(&mut h);
    assert!(h.state().panel.grid_package.created.is_none());
    assert_eq!(
        h.state().panel.grid_package.last_output.as_ref(),
        Some(&stale_destination)
    );
    assert!(h
        .state()
        .panel
        .grid_package
        .status
        .contains("prior context"));
    assert_eq!(
        fs::read(stale_destination.join("atlas.png")).unwrap(),
        png_bytes(true)
    );
    assert!(
        stale_destination.join("sprites.json").is_file(),
        "completed prior-context output is retained"
    );
    h.state_mut().panel.bindings = Some(detached);
    h.run_steps(3);
    text(
        &mut h,
        "Grid package destination",
        destination.to_str().unwrap(),
    );
    click(&mut h, "Inspect sprite PNG");
    wait(&mut h);
    if h.get_by_label("Grid rights acknowledgement")
        .accesskit_node()
        .toggled()
        == Some(egui::accesskit::Toggled::False)
    {
        click(&mut h, "Grid rights acknowledgement");
    }
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("sentinel.txt"), b"existing content").unwrap();
    let before = snapshot(fixture.temp.path());
    click(&mut h, "Create sprite grid package");
    wait(&mut h);
    assert!(h.state().panel.grid_package.created.is_none());
    assert!(h
        .state()
        .panel
        .grid_package
        .status
        .to_ascii_lowercase()
        .contains("exist"));
    assert_eq!(snapshot(fixture.temp.path()), before);
    assert!(!destination.join("sprites.json").exists());
    assert_eq!(
        h.state().panel.bindings.as_ref().unwrap().document(),
        &bindings
    );
    assert!(!h.state().panel.bindings.as_ref().unwrap().dirty());
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(h.state().editor.checksum(), checksum);
}
