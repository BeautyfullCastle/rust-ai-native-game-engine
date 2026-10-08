//! Real editor transaction acceptance. GPU proof uses the composed egui frame,
//! not the native viewport alone (which cannot see the sprite overlay).
#![cfg(all(
    feature = "image-reimport",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "common/saved_project.rs"]
#[allow(unused_imports)]
mod fixture;

use egui_kittest::Harness;
use fixture::*;
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    EditorApp, Mode,
};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use std::path::Path;

const REPLACE: &str = "Replace atlas with new package version";
const GREEN: [u8; 4] = [18, 238, 47, 255];

fn png_file(path: &Path, width: u32, height: u32) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&GREEN.repeat((width * height) as usize))
        .unwrap();
}

fn configure(h: &mut Harness<'_, EditorApp>, image: &Path) {
    settle(h);
    open_inspector(h, "target");
    let panel = &mut h.state_mut().sprites;
    panel.package = "sample-sprites".into();
    panel.document = "sprites.json".into();
    panel.replacement_image = image.to_string_lossy().into_owned();
    panel.replacement_version = "1.0.1".into();
    click(h, "Explicit PNG replacement");
}

struct Unchanged {
    scene: Vec<u8>,
    sidecar: Vec<u8>,
    lock: Vec<u8>,
    document: orr_editor::sprite_bindings::Document,
    checksum: u64,
    tick: u64,
    history: usize,
    camera: orr_render::Camera,
}
impl Unchanged {
    fn capture(f: &SavedFixture, h: &Harness<'_, EditorApp>) -> Self {
        Self {
            scene: std::fs::read(f.scene()).unwrap(),
            sidecar: std::fs::read(f.sidecar()).unwrap(),
            lock: std::fs::read(f.root.path().join("orr.packages.lock.json")).unwrap(),
            document: document(h).clone(),
            checksum: h.state().editor.checksum(),
            tick: h.state().editor.snapshot().unwrap().tick(),
            history: h.state().editor.history().entries.len(),
            camera: h.state().editor.camera,
        }
    }
    fn assert_scene(&self, f: &SavedFixture, h: &Harness<'_, EditorApp>) {
        assert_eq!(std::fs::read(f.scene()).unwrap(), self.scene);
        assert_eq!(std::fs::read(f.sidecar()).unwrap(), self.sidecar);
        assert_eq!(document(h), &self.document);
        assert_eq!(h.state().editor.checksum(), self.checksum);
        assert_eq!(h.state().editor.snapshot().unwrap().tick(), self.tick);
        assert_eq!(h.state().editor.history().entries.len(), self.history);
        assert_eq!(h.state().editor.camera, self.camera);
        assert_eq!(region(h.state(), HERO), Some(10));
        assert_eq!(region(h.state(), TARGET), Some(20));
    }
    fn assert_failed(&self, f: &SavedFixture, h: &Harness<'_, EditorApp>) {
        assert!(
            h.state().sprites.error().is_some(),
            "invalid replacement must report failure"
        );
        self.assert_scene(f, h);
        assert_eq!(
            std::fs::read(f.root.path().join("orr.packages.lock.json")).unwrap(),
            self.lock
        );
    }
}

#[test]
fn editor_reimport_preserves_saved_scene_and_binding_undo_and_reopens() {
    let f = SavedFixture::new();
    let input = tempdir();
    let image = input.path().join("replacement.png");
    png_file(&image, 64, 16);
    let mut h = f.headless();
    configure(&mut h, &image);
    click(&mut h, "Follow selected entity");
    click(&mut h, "Save bindings");
    assert_loaded(&h, TARGET);
    let before = Unchanged::capture(&f, &h);
    click(&mut h, REPLACE);
    no_error(&h);
    before.assert_scene(&f, &h);
    assert_ne!(
        std::fs::read(f.root.path().join("orr.packages.lock.json")).unwrap(),
        before.lock
    );
    let project = orr_editor::sprite_bindings::open_project(f.root.path()).unwrap();
    assert_eq!(project.verify().unwrap().direct["sample-sprites"], "1.0.1");
    let asset =
        orr_editor::sprite_bindings::load_asset(f.root.path(), "sample-sprites", "sprites.json")
            .unwrap();
    assert_eq!(asset.rgba, GREEN.repeat(64 * 16));
    click(&mut h, "Undo binding");
    no_error(&h);
    assert_eq!(document(&h).camera_follow.as_deref(), Some(HERO));
    click(&mut h, "Redo binding");
    no_error(&h);
    assert_loaded(&h, TARGET);
    before.assert_scene(&f, &h);
    drop(h);
    let mut reopened = f.headless();
    settle(&mut reopened);
    assert_loaded(&reopened, TARGET);
    before.assert_scene(&f, &reopened);
}

#[test]
fn editor_reimport_accepts_checked_relative_binding_path_with_absolute_host_scene() {
    let f = SavedFixture::new();
    let input = tempdir();
    let image = input.path().join("replacement.png");
    png_file(&image, 64, 16);
    let mut h = f.headless();
    configure(&mut h, &image);

    // Reach the filesystem root with parent segments, then descend to the
    // fixture. Do not change process-global cwd: libtests run concurrently.
    let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
    let mut relative = std::path::PathBuf::from(".");
    for component in cwd.components() {
        if matches!(component, std::path::Component::Normal(_)) {
            relative.push("..");
        }
    }
    for component in f.sidecar().canonicalize().unwrap().components() {
        if let std::path::Component::Normal(part) = component {
            relative.push(part);
        }
    }
    assert!(relative.is_relative());
    assert_eq!(relative.canonicalize().unwrap(), f.sidecar());
    assert!(Path::new(h.state().editor.sim().scene_path.as_deref().unwrap()).is_absolute());
    // Bindings::open exercises the same checked relative-path admission used
    // by the panel. Retain the existing loaded cache and displayed host frame.
    h.state_mut().sprites.bindings =
        Some(orr_editor::sprite_bindings::Bindings::open(relative.clone()).unwrap());
    assert_loaded(&h, HERO);
    let before = Unchanged::capture(&f, &h);
    click(&mut h, REPLACE);
    no_error(&h);
    before.assert_scene(&f, &h);
    assert_eq!(h.state().sprites.bindings.as_ref().unwrap().path, relative);
    assert_ne!(
        std::fs::read(f.root.path().join("orr.packages.lock.json")).unwrap(),
        before.lock
    );
    let project = orr_editor::sprite_bindings::open_project(f.root.path()).unwrap();
    assert_eq!(project.verify().unwrap().direct["sample-sprites"], "1.0.1");
    let asset =
        orr_editor::sprite_bindings::load_asset(f.root.path(), "sample-sprites", "sprites.json")
            .unwrap();
    assert_eq!(asset.rgba, GREEN.repeat(64 * 16));
}

#[test]
fn editor_reimport_bad_png_and_non_new_versions_leave_active_state_unchanged() {
    let f = SavedFixture::new();
    let input = tempdir();
    let image = input.path().join("replacement.png");
    png_file(&image, 32, 16);
    let mut h = f.headless();
    configure(&mut h, &image);
    let before = Unchanged::capture(&f, &h);
    click(&mut h, REPLACE);
    before.assert_failed(&f, &h);
    std::fs::write(&image, b"invalid PNG").unwrap();
    click(&mut h, REPLACE);
    before.assert_failed(&f, &h);
    png_file(&image, 64, 16);
    for version in ["1.0.0", "0.9.0", "not-semver"] {
        h.state_mut().sprites.replacement_version = version.into();
        click(&mut h, REPLACE);
        before.assert_failed(&f, &h);
    }
}

#[test]
fn editor_reimport_refuses_dirty_bindings_dirty_scene_and_play() {
    let f = SavedFixture::new();
    let input = tempdir();
    let image = input.path().join("replacement.png");
    png_file(&image, 64, 16);
    let mut h = f.headless();
    configure(&mut h, &image);
    click(&mut h, "Follow selected entity");
    let dirty_bindings = Unchanged::capture(&f, &h);
    click(&mut h, REPLACE);
    dirty_bindings.assert_failed(&f, &h);
    click(&mut h, "Undo binding");
    edit_hero_x(&mut h);
    let dirty_scene = Unchanged::capture(&f, &h);
    click(&mut h, REPLACE);
    dirty_scene.assert_failed(&f, &h);
    drop(h);
    let mut h = f.headless();
    configure(&mut h, &image);
    let lock = std::fs::read(f.root.path().join("orr.packages.lock.json")).unwrap();
    let camera = h.state().editor.camera;
    click(&mut h, LBL_PLAY);
    wait_for(&mut h, "running Play", |app| {
        app.editor.mode() == Mode::Play
    });
    click(&mut h, REPLACE);
    assert!(h.state().sprites.error().is_some());
    assert_eq!(
        std::fs::read(f.root.path().join("orr.packages.lock.json")).unwrap(),
        lock
    );
    click(&mut h, LBL_STOP);
    wait_stopped(&mut h, camera);
}

#[test]
fn editor_reimport_refuses_saved_sidecar_or_scene_changed_under_open_editor() {
    for change_sidecar in [true, false] {
        let f = SavedFixture::new();
        let input = tempdir();
        let image = input.path().join("replacement.png");
        png_file(&image, 64, 16);
        let mut h = f.headless();
        configure(&mut h, &image);
        if change_sidecar {
            let mut sidecar: serde_json::Value =
                serde_json::from_slice(&std::fs::read(f.sidecar()).unwrap()).unwrap();
            sidecar["camera_follow"] = serde_json::json!(TARGET);
            std::fs::write(f.sidecar(), serde_json::to_vec_pretty(&sidecar).unwrap()).unwrap();
        } else {
            let scene = std::fs::read_to_string(f.scene()).unwrap();
            assert!(scene.contains("-60"));
            std::fs::write(f.scene(), scene.replace("-60", "-48")).unwrap();
        }
        let before = Unchanged::capture(&f, &h);
        click(&mut h, REPLACE);
        before.assert_failed(&f, &h);
        assert!(h.state().sprites.error().unwrap().contains("differs"));
    }
}

fn gpu_available() -> bool {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => {
            eprintln!(
                "saved-project composed framebuffer GPU adapter: {} (software: {})",
                gpu.adapter_name(),
                gpu.is_software()
            );
            true
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "ORR_REQUIRE_GPU is set but saved-project GPU is unavailable: {error}"
            );
            eprintln!("SKIP: saved-project composed framebuffer GPU unavailable: {error}");
            false
        }
    }
}

struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn frame(h: &mut Harness<'_, EditorApp>, name: &str) -> Frame {
    // WgpuTestRenderer::render consumes the actual FullOutput through the same
    // RenderState.renderer supplied to EditorApp, then reads an offscreen target.
    // This contains both the native GPU viewport texture and egui atlas meshes.
    let image = h
        .render()
        .expect("compose the real EditorApp egui framebuffer");
    let frame = Frame {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    };
    assert!(frame
        .rgba
        .chunks_exact(4)
        .any(|pixel| pixel[0] > 40 || pixel[1] > 40 || pixel[2] > 40));
    if let Some(directory) = std::env::var_os("ORR_PROJECT_CAPTURE_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        let file =
            std::fs::File::create(Path::new(&directory).join(format!("{name}.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, frame.width, frame.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&frame.rgba)
            .unwrap();
    }
    frame
}

fn sprite_pixels(frame: &Frame, app: &EditorApp, guid: &str) -> Vec<[u8; 3]> {
    let rect = app.ui.viewport_rect.unwrap();
    let body = body_position(app, guid);
    let scale = app.sprites.bindings.as_ref().unwrap().document().bindings[guid].units_per_pixel;
    let mut pixels = Vec::new();
    for row in 0..16 {
        for column in 0..16 {
            let world = [
                body[0] + (column as f32 - 7.5) * scale,
                body[1] + (7.5 - row as f32) * scale,
            ];
            let at = app.editor.camera.world_to_screen(world, app.ui.viewport_px);
            let x = (rect.min.x + at[0]).floor() as u32;
            let y = (rect.min.y + at[1]).floor() as u32;
            assert!(
                x < frame.width && y < frame.height,
                "fixture sprite remains inside captured framebuffer"
            );
            let offset = ((y * frame.width + x) * 4) as usize;
            pixels.push(frame.rgba[offset..offset + 3].try_into().unwrap());
        }
    }
    pixels
}

/// Required acceptance lane:
/// ORR_REQUIRE_GPU=1 cargo test -p orr_editor --features image-reimport
///   --test image_reimport composed_reimport -- --ignored --nocapture
#[test]
#[ignore = "mandatory GPU acceptance lane; run explicitly with ORR_REQUIRE_GPU=1"]
fn composed_reimport_keeps_old_pixels_on_failure_then_swaps_and_reopens() {
    assert!(
        std::env::var_os("ORR_REQUIRE_GPU").is_some(),
        "this acceptance test requires the positive ORR_REQUIRE_GPU gate"
    );
    assert!(
        gpu_available(),
        "mandatory composed GPU evidence cannot skip"
    );
    let f = SavedFixture::new();
    let input = tempdir();
    let image = input.path().join("replacement.png");
    png_file(&image, 32, 16);
    let mut h = open_gpu(f.root.path());
    configure(&mut h, &image);
    assert!(h.state().has_gpu());
    click(&mut h, "Follow selected entity");
    click(&mut h, "Save bindings");
    let before = Unchanged::capture(&f, &h);
    let initial = frame(&mut h, "reimport-01-before");
    let initial_hero = sprite_pixels(&initial, h.state(), HERO);
    let initial_target = sprite_pixels(&initial, h.state(), TARGET);
    assert_ne!(
        initial_hero, initial_target,
        "original two frames must differ"
    );
    click(&mut h, REPLACE);
    before.assert_failed(&f, &h);
    let failed = frame(&mut h, "reimport-02-rejected");
    assert_eq!(
        sprite_pixels(&failed, h.state(), HERO),
        initial_hero,
        "failed admission retains the active hero texture"
    );
    assert_eq!(
        sprite_pixels(&failed, h.state(), TARGET),
        initial_target,
        "failed admission retains the active target texture"
    );
    png_file(&image, 64, 16);
    click(&mut h, REPLACE);
    no_error(&h);
    before.assert_scene(&f, &h);
    let replaced = frame(&mut h, "reimport-03-committed");
    for guid in [HERO, TARGET] {
        let pixels = sprite_pixels(&replaced, h.state(), guid);
        let green = pixels
            .iter()
            .filter(|pixel| {
                pixel
                    .iter()
                    .zip(GREEN)
                    .take(3)
                    .all(|(actual, expected)| actual.abs_diff(expected) <= 3)
            })
            .count();
        assert!(green > 220, "{guid} must visibly paint new PNG texels in actual composed egui output; saw {green}/256");
    }
    assert_ne!(sprite_pixels(&replaced, h.state(), HERO), initial_hero);
    assert_ne!(sprite_pixels(&replaced, h.state(), TARGET), initial_target);
    click(&mut h, "Undo binding");
    no_error(&h);
    assert_eq!(document(&h).camera_follow.as_deref(), Some(HERO));
    click(&mut h, "Redo binding");
    no_error(&h);
    before.assert_scene(&f, &h);
    drop(h);
    // Remove the external replacement input before reopening: the committed
    // package object must be sufficient to reconstruct the new texture.
    input.close().unwrap();
    let mut reopened = open_gpu(f.root.path());
    settle(&mut reopened);
    assert_loaded(&reopened, TARGET);
    before.assert_scene(&f, &reopened);
    let persisted = frame(&mut reopened, "reimport-04-reopened");
    for guid in [HERO, TARGET] {
        let pixels = sprite_pixels(&persisted, reopened.state(), guid);
        let green = pixels
            .iter()
            .filter(|pixel| {
                pixel
                    .iter()
                    .zip(GREEN)
                    .take(3)
                    .all(|(actual, expected)| actual.abs_diff(expected) <= 3)
            })
            .count();
        assert!(
            green > 220,
            "reopened {guid} must use committed replacement PNG"
        );
    }
}
