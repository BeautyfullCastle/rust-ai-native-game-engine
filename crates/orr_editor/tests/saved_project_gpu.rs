//! Composed egui + wgpu proof for automatic saved-project loading and relocation.
//! Core viewport readback alone cannot see the sprite overlay.
#![cfg(feature = "sprites")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "common/saved_project.rs"]
mod fixture;

use egui_kittest::Harness;
use fixture::*;
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    editor::input::Phase,
    EditorApp, Mode,
};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use std::path::Path;

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
        .as_chunks::<4>().0.iter()
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

fn viewport_difference(a: &Frame, b: &Frame, rect: egui::Rect) -> usize {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.rgba
        .as_chunks::<4>().0.iter()
        .zip(b.rgba.as_chunks::<4>().0.iter())
        .enumerate()
        .filter(|(index, (left, right))| {
            let x = (*index as u32 % a.width) as f32;
            let y = (*index as u32 / a.width) as f32;
            // Exclude status text and all inspector/toolbar changes.
            rect.shrink(5.0).contains(egui::pos2(x, y)) && y > rect.min.y + 40.0 && left != right
        })
        .count()
}

fn atlas_color_count(frame: &Frame, rect: egui::Rect) -> usize {
    frame
        .rgba
        .as_chunks::<4>().0.iter()
        .enumerate()
        .filter(|(index, pixel)| {
            let point = egui::pos2(
                (*index as u32 % frame.width) as f32,
                (*index as u32 / frame.width) as f32,
            );
            // Two distinctive opaque colors from the real lantern_keeper.png. These
            // do not occur in the procedural red/blue Arena collider proxies.
            rect.shrink(5.0).contains(point)
                && ((pixel[0].abs_diff(54) <= 3
                    && pixel[1].abs_diff(154) <= 3
                    && pixel[2].abs_diff(165) <= 3)
                    || (pixel[0].abs_diff(255) <= 3
                        && pixel[1].abs_diff(201) <= 3
                        && pixel[2].abs_diff(76) <= 3))
        })
        .count()
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

fn atlas_pixels(asset: &orr_editor::sprite_bindings::Asset, id: u32) -> Vec<[u8; 4]> {
    let region = asset.document.region(id).unwrap();
    assert_eq!((region.width, region.height), (16, 16));
    (0..16)
        .flat_map(|y| {
            (0..16).map(move |x| {
                let offset =
                    (((region.y + y) * asset.document.atlas().width + region.x + x) * 4) as usize;
                asset.rgba[offset..offset + 4].try_into().unwrap()
            })
        })
        .collect()
}

fn matches_color(actual: &[u8; 3], expected: &[u8]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, b)| a.abs_diff(*b) <= 3)
}

/// Check the real installed atlas, not a screenshot checksum or the procedural
/// background visible through transparent texels. Every opaque texel must paint.
fn assert_opaque_pose(pixels: &[[u8; 3]], asset: &orr_editor::sprite_bindings::Asset, id: u32) {
    let atlas = atlas_pixels(asset, id);
    let mut checked = 0;
    for (index, expected) in atlas
        .iter()
        .enumerate()
        .filter(|(_, pixel)| pixel[3] == 255)
    {
        assert!(
            matches_color(&pixels[index], &expected[..3]),
            "region {id} opaque texel ({}, {}): expected {:?}, composed {:?}",
            index % 16,
            index / 16,
            &expected[..3],
            pixels[index]
        );
        checked += 1;
    }
    assert!(checked > 80, "fixture has enough opaque PNG evidence");
}

fn assert_independent_opaque_frames(
    hero: &[[u8; 3]],
    target: &[[u8; 3]],
    asset: &orr_editor::sprite_bindings::Asset,
) {
    let idle = atlas_pixels(asset, 10);
    let walk = atlas_pixels(asset, 20);
    assert_opaque_pose(hero, asset, 10);
    assert_opaque_pose(target, asset, 20);
    // Frames 10 and 20 deliberately share their body colors. Their independent
    // identity is established by unique opaque leg texels in *both* directions.
    // Rendering one actor's frame for both, or omitting either sprite, fails.
    for (actual, other_actual, expected, other_expected) in
        [(hero, target, &idle, &walk), (target, hero, &walk, &idle)]
    {
        let mut distinguishing = 0;
        for (index, pixel) in expected.iter().enumerate() {
            if pixel[3] == 255
                && (other_expected[index][3] != 255 || pixel[..3] != other_expected[index][..3])
            {
                assert!(matches_color(&actual[index], &pixel[..3]));
                assert!(
                    !matches_color(&other_actual[index], &pixel[..3]),
                    "actors must differ at PNG-defined opaque frame-identity texel ({}, {})",
                    index % 16,
                    index / 16
                );
                distinguishing += 1;
            }
        }
        assert!(
            distinguishing >= 6,
            "both frames have independent opaque identity evidence"
        );
    }
}

#[test]
fn composed_saved_project_autoload_walk_follow_stop_and_relocate() {
    if !gpu_available() {
        return;
    }
    let fixture = SavedFixture::new();
    let asset = orr_editor::sprite_bindings::load_asset(
        fixture.root.path(),
        "sample-sprites",
        "sprites.json",
    )
    .unwrap();
    let mut h = open_gpu(fixture.root.path());
    settle(&mut h);
    assert!(h.state().has_gpu());
    assert!(h.state().viewport_gpu().is_some());
    assert_loaded(&h, HERO);
    let authored = document(&h).clone();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    let scene = std::fs::read(fixture.scene()).unwrap();
    let sidecar = std::fs::read(fixture.sidecar()).unwrap();
    let initial = frame(&mut h, "project-01-initial-autoload");
    assert!(
        atlas_color_count(&initial, h.state().ui.viewport_rect.unwrap()) > 100,
        "automatic startup must paint real installed PNG colors"
    );
    let hero_pixels = sprite_pixels(&initial, h.state(), HERO);
    let target_pixels = sprite_pixels(&initial, h.state(), TARGET);
    assert_ne!(
        hero_pixels, target_pixels,
        "two authored actors must paint their independently bound idle frames"
    );
    assert_independent_opaque_frames(&hero_pixels, &target_pixels, &asset);
    frame(&mut h, "project-02-independent-idle-frames");
    let expected = serde_json::json!({"hero":hero_pixels,"target":target_pixels});
    let edit_camera = h.state().editor.camera;

    click(&mut h, LBL_PLAY);
    wait_for(&mut h, "saved-project live control capability", |app| {
        app.editor.can_take_control()
    });
    click(&mut h, "Take control");
    wait_for(&mut h, "saved-project managed keyboard claim", |app| {
        app.editor.input_phase() == Phase::Active
    });
    let start = body_position(h.state(), HERO);
    key(&h, egui::Key::ArrowRight, true);
    wait_for(
        &mut h,
        "saved-project actual walk and saved follow",
        |app| {
            moving(app, HERO)
                && body_position(app, HERO)[0] > start[0]
                && app.editor.camera.center == body_position(app, HERO)
        },
    );
    assert!(!moving(h.state(), TARGET));
    assert!(matches!(region(h.state(), HERO), Some(20 | 21)));
    assert!(
        matches!(region(h.state(), TARGET), Some(20 | 21)),
        "stationary actor retains its own inverted idle mapping"
    );
    let walking = frame(&mut h, "project-03-walk-follow");
    let walking_hero = sprite_pixels(&walking, h.state(), HERO);
    let walking_target = sprite_pixels(&walking, h.state(), TARGET);
    assert_opaque_pose(&walking_hero, &asset, region(h.state(), HERO).unwrap());
    assert_opaque_pose(&walking_target, &asset, region(h.state(), TARGET).unwrap());
    assert_ne!(hero_pixels, walking_hero);
    assert!(viewport_difference(&initial, &walking, h.state().ui.viewport_rect.unwrap()) > 200);
    assert!(atlas_color_count(&walking, h.state().ui.viewport_rect.unwrap()) > 100);
    assert_eq!(document(&h), &authored);
    key(&h, egui::Key::ArrowRight, false);
    wait_for(&mut h, "saved-project key release", |app| {
        !moving(app, HERO)
    });
    click(&mut h, LBL_STOP);
    wait_stopped(&mut h, edit_camera);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(h.state().editor.camera, edit_camera);
    assert_loaded(&h, HERO);
    let stopped = frame(&mut h, "project-04-stopped");
    assert_independent_opaque_frames(
        &sprite_pixels(&stopped, h.state(), HERO),
        &sprite_pixels(&stopped, h.state(), TARGET),
        &asset,
    );
    assert_eq!(sprite_pixels(&stopped, h.state(), HERO), hero_pixels);
    assert_eq!(sprite_pixels(&stopped, h.state(), TARGET), target_pixels);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(document(&h), &authored);
    assert_eq!(std::fs::read(fixture.scene()).unwrap(), scene);
    assert_eq!(std::fs::read(fixture.sidecar()).unwrap(), sidecar);
    drop(h);

    let moved = fixture.relocate();
    std::fs::write(
        moved.root.path().join("expected-test-pixels.json"),
        serde_json::to_vec(&expected).unwrap(),
    )
    .unwrap();
    relocated_child(
        moved.root.path(),
        "relocated_saved_project_gpu_child",
        checksum,
    );
}

#[test]
fn relocated_saved_project_gpu_child() {
    let Some(root) = child_root() else { return };
    assert!(gpu_available(), "relocated GPU proof cannot silently skip");
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("expected-test-pixels.json")).unwrap())
            .unwrap();
    let asset =
        orr_editor::sprite_bindings::load_asset(&root, "sample-sprites", "sprites.json").unwrap();
    let mut h = open_gpu(&root);
    settle(&mut h);
    assert_loaded(&h, HERO);
    assert_eq!(h.state().editor.checksum(), child_checksum());
    let reopened = frame(&mut h, "project-05-relocated-reopen");
    assert_independent_opaque_frames(
        &sprite_pixels(&reopened, h.state(), HERO),
        &sprite_pixels(&reopened, h.state(), TARGET),
        &asset,
    );
    assert!(atlas_color_count(&reopened, h.state().ui.viewport_rect.unwrap()) > 100);
    assert_eq!(
        serde_json::to_value(sprite_pixels(&reopened, h.state(), HERO)).unwrap(),
        expected["hero"]
    );
    assert_eq!(
        serde_json::to_value(sprite_pixels(&reopened, h.state(), TARGET)).unwrap(),
        expected["target"]
    );
    let camera = h.state().editor.camera;
    click(&mut h, LBL_PLAY);
    wait_for(&mut h, "relocated saved follow GUID", |app| {
        app.editor.mode() == Mode::Play && app.editor.camera.center == body_position(app, HERO)
    });
    click(&mut h, LBL_STOP);
    wait_stopped(&mut h, camera);
    assert_eq!(h.state().editor.camera, camera);
    assert_eq!(h.state().editor.checksum(), child_checksum());
    assert_loaded(&h, HERO);
}
