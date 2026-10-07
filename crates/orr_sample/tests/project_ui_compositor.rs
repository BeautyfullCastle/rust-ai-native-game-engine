//! Software-GPU acceptance for the saved Korean UI on an authored-project view.
#![cfg(all(feature = "project", feature = "game-ui"))]
#![allow(clippy::float_arithmetic)]

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

use egui::{pos2, vec2, Color32, FontId, LayerId, Order, Pos2, Rect, TextureId};
use orr_bridge::Bridge;
use orr_render::{
    orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions},
    Camera, OffscreenTarget, RenderList,
};
use orr_sample::{
    game_ui::{Action, GameUi, Hud},
    game_ui_gpu::{GpuOverlay, PendingOverlay},
    project_compositor::{ProjectCompositor, SpriteDraw},
    project_runtime::PreparedRuntime,
    project_sprites::{Asset, AssetKey},
};
use orr_sprite::{SpriteDocument, SpriteInstance};

#[path = "common/project.rs"]
mod project_fixture;
use project_fixture::ProjectFixture;

fn install_saved_ui(project: &Path) -> Vec<u8> {
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/game_ui_font")
        .canonicalize()
        .unwrap();
    let installer = orr_package::Project::open_for_install(
        project,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap();
    let lock = installer.install(&[package]).unwrap();
    assert_eq!(
        lock.direct.len(),
        2,
        "the existing sprite and UI packages are selected once"
    );
    assert!(lock.direct.contains_key("sample-sprites"));
    assert!(lock.direct.contains_key("korean-game-ui"));

    let manifest_path = project.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["entry"]["ui"] = serde_json::json!({
        "profile": "arena-korean-v1",
        "font": {
            "package": "korean-game-ui",
            "asset": "OrreryKoreanUI.otf"
        }
    });
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let content =
        orr_package::Project::open(project, orr_sample::project_runtime::compiled_runtime())
            .unwrap()
            .read_asset("korean-game-ui", "OrreryKoreanUI.otf")
            .unwrap();
    assert!(!content.is_empty());
    content
}

fn asset(rgba: [u8; 4]) -> Asset {
    Asset {
        document: SpriteDocument::from_json(
            r#"{"format":"orr_sprite","version":1,"atlas":{"image":"opaque.rgba","width":1,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":1,"height":1}],"clips":[]}"#,
        )
        .unwrap(),
        rgba: rgba.to_vec(),
    }
}

fn pixel(bytes: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    bytes[(y * width + x) * 4..(y * width + x) * 4 + 4]
        .try_into()
        .unwrap()
}

fn sprite_bounds(
    camera: &Camera,
    viewport: (u32, u32),
    instance: SpriteInstance,
) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let center = camera.world_to_screen(instance.position, viewport);
    let pixels_per_unit = camera.pixels_per_unit(viewport.0, viewport.1);
    let half_width = instance.size[0] * pixels_per_unit * 0.5 + 2.0;
    let half_height = instance.size[1] * pixels_per_unit * 0.5 + 2.0;
    let x0 = (center[0] - half_width).floor().max(0.0) as usize;
    let x1 = (center[0] + half_width).ceil().min(viewport.0 as f32) as usize;
    let y0 = (center[1] - half_height).floor().max(0.0) as usize;
    let y1 = (center[1] + half_height).ceil().min(viewport.1 as f32) as usize;
    (x0..x1, y0..y1)
}

fn changed_in_sprite_bounds(
    before: &[u8],
    after: &[u8],
    viewport: (u32, u32),
    camera: &Camera,
    instance: SpriteInstance,
) -> usize {
    let (x_range, y_range) = sprite_bounds(camera, viewport, instance);
    y_range
        .flat_map(|y| x_range.clone().map(move |x| (x, y)))
        .filter(|(x, y)| {
            pixel(before, viewport.0 as usize, *x, *y) != pixel(after, viewport.0 as usize, *x, *y)
        })
        .count()
}

fn pointer_input(
    size: [f32; 2],
    pixels_per_point: f32,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::RawInput {
    let mut input = input(size, pixels_per_point, time);
    input.events = events;
    input
}

fn near(actual: [u8; 4], expected: [u8; 4]) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.abs_diff(expected) <= 4),
        "actual {actual:?}, expected {expected:?}"
    );
}

fn over(destination: [u8; 4], source: [u8; 4]) -> [u8; 4] {
    let alpha = f32::from(source[3]) / 255.0;
    let mut output = destination;
    for channel in 0..3 {
        let blended =
            f32::from(source[channel]) * alpha + f32::from(destination[channel]) * (1.0 - alpha);
        output[channel] = blended.round() as u8;
    }
    output[3] = (f32::from(source[3]) + f32::from(destination[3]) * (1.0 - alpha)).round() as u8;
    output
}

fn input(size: [f32; 2], pixels_per_point: f32, time: f64) -> egui::RawInput {
    let mut input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(size[0], size[1]))),
        time: Some(time),
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .expect("root viewport")
        .native_pixels_per_point = Some(pixels_per_point);
    input
}

fn image(color: Color32) -> egui::epaint::ImageDelta {
    egui::epaint::ImageDelta::full(
        egui::ColorImage::filled([2, 2], color),
        egui::TextureOptions::NEAREST,
    )
}

fn run_gpu() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions {
        force_software: true,
        ..Default::default()
    }) {
        Ok(gpu) => {
            assert!(gpu.is_software(), "acceptance requires a software adapter");
            eprintln!("saved-project UI software adapter: {}", gpu.adapter_name());
            Some(gpu)
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required software GPU unavailable: {error}"
            );
            eprintln!("SKIP: software GPU unavailable: {error}");
            None
        }
    }
}

#[test]
fn saved_korean_ui_composes_after_project_sprites_and_pending_epochs() {
    let fixture = ProjectFixture::new();
    let packaged_font = install_saved_ui(&fixture.root);

    // The saved descriptor and whole active package lock are admitted before GPU creation.
    let prepared = PreparedRuntime::open(&fixture.root).unwrap();
    let (seed, mut presentation, prepared_ui) = prepared.into_launch_parts();
    let prepared_ui = prepared_ui.expect("saved UI descriptor is admitted");
    assert_eq!(prepared_ui.font, packaged_font);
    assert_eq!(prepared_ui.descriptor.font.package, "korean-game-ui");

    let Some(gpu) = run_gpu() else {
        return;
    };
    let format = TextureFormat::Rgba8Unorm;
    let assets: BTreeMap<AssetKey, Asset> = BTreeMap::from([
        (
            ("atlas-a".into(), "sprites.json".into()),
            asset([255, 0, 0, 128]),
        ),
        (
            ("atlas-b".into(), "sprites.json".into()),
            asset([0, 255, 0, 128]),
        ),
    ]);
    let mut compositor = ProjectCompositor::new(gpu.clone(), format, &assets).unwrap();
    let sprite = |asset: AssetKey, position: [f32; 2], order: i32| SpriteDraw {
        asset,
        instance: SpriteInstance {
            position,
            size: [2.0, 2.0],
            tint: [1.0; 4],
            order,
            ..Default::default()
        },
    };
    let a: AssetKey = ("atlas-a".into(), "sprites.json".into());
    let b: AssetKey = ("atlas-b".into(), "sprites.json".into());
    let sprites = [
        sprite(a.clone(), [-8.0, -5.0], 900),
        sprite(b.clone(), [-8.0, -5.0], -900),
        sprite(a, [-8.0, -5.0], -1000),
        // Center this final sprite under the UI clip probe so the overlay must
        // paint over the last authored sprite run, not only an empty pixel.
        sprite(b, [-14.666_667, -10.0], 700),
    ];
    let shapes = RenderList::new();
    let camera = Camera::new([0.0, 0.0], 16.0);
    let mut ui = GameUi::from_font(prepared_ui.font.clone(), true).unwrap();
    ui.apply(Action::Play);
    let mut overlay = GpuOverlay::new(&gpu, format);
    let mut pending = PendingOverlay::default();
    let texture_id = TextureId::Managed(0xA11);

    // First frame establishes the real GameUi/font atlas at 1x scale.
    let (mut first, _) = ui.show(
        input([128.0, 96.0], 1.0, 1.0),
        Hud {
            tick: 7,
            verified_tick: 7,
            rollbacks: 0,
        },
    );
    assert_eq!(first.pixels_per_point, 1.0);
    first.textures_delta.push(texture_id, image(Color32::BLACK));
    pending.push(first).unwrap();

    // Paint an actual 1x frame first, establishing a previous target/view before
    // the resize and skipped-acquisition texture epochs below.
    let first_target = OffscreenTarget::new(&gpu, 128, 96, format);
    compositor
        .draw(
            first_target.render_view(),
            first_target.size(),
            &shapes,
            &sprites,
            &camera,
        )
        .unwrap();
    let first_underlay = first_target.read_rgba8();
    let mut first_overlay_called = false;
    compositor
        .draw_with_overlay(
            first_target.render_view(),
            first_target.size(),
            &shapes,
            &sprites,
            &camera,
            |rhi, view, size| {
                first_overlay_called = true;
                pending.paint(&mut overlay, rhi, view, size, &ui.context)
            },
        )
        .unwrap();
    assert!(first_overlay_called);
    let first_capture = first_target.read_rgba8();
    assert!(
        first_capture
            .chunks_exact(4)
            .zip(first_underlay.chunks_exact(4))
            .filter(|(after, before)| after != before)
            .count()
            > 100,
        "the real 1x GameUi frame should paint over its acquired project view"
    );

    // Model frames whose surface acquisition was skipped: preserve a texture
    // upload, then a free, then reuse of the same id in the latest geometry.
    let (mut skipped_upload, _) = ui.show(
        input([128.0, 96.0], 1.0, 1.5),
        Hud {
            tick: 8,
            verified_tick: 8,
            rollbacks: 0,
        },
    );
    skipped_upload
        .textures_delta
        .push(texture_id, image(Color32::BLACK));
    pending.push(skipped_upload).unwrap();
    let mut freed = egui::FullOutput {
        pixels_per_point: 1.0,
        ..Default::default()
    };
    freed.textures_delta.free(texture_id);
    pending.push(freed).unwrap();

    // Resize the target to 2x physical pixels and report 2x native DPI while the
    // logical UI rectangle stays 128x96 points.
    let (mut final_output, _) = ui.show(
        input([128.0, 96.0], 2.0, 2.0),
        Hud {
            tick: 8,
            verified_tick: 8,
            rollbacks: 0,
        },
    );
    assert_eq!(final_output.pixels_per_point, 2.0);
    let glyph_output = ui.context.run_ui(input([128.0, 96.0], 2.0, 2.1), |root| {
        let painter = root
            .ctx()
            .layer_painter(LayerId::new(Order::Foreground, "acceptance glyphs".into()));
        for (index, character) in ["오", "러", "리", "한", "글"].iter().enumerate() {
            painter.text(
                pos2(8.0 + index as f32 * 22.0, 30.0),
                egui::Align2::LEFT_TOP,
                *character,
                FontId::proportional(18.0),
                Color32::WHITE,
            );
        }
    });
    final_output
        .textures_delta
        .append(glyph_output.textures_delta);
    final_output.shapes.extend(glyph_output.shapes);

    // A large textured mesh is clipped to a small point-space rectangle. Its
    // texture id has been freed in the skipped frame and is now reused.
    let clip_rect = Rect::from_min_max(pos2(8.0, 74.0), pos2(24.0, 84.0));
    let mut mesh = egui::Mesh::with_texture(texture_id);
    mesh.add_rect_with_uv(
        Rect::from_min_max(pos2(4.0, 70.0), pos2(48.0, 88.0)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    final_output.shapes.push(egui::epaint::ClippedShape {
        clip_rect,
        shape: egui::Shape::mesh(mesh),
    });
    final_output
        .textures_delta
        .push(texture_id, image(Color32::from_rgb(20, 220, 40)));
    pending.push(final_output).unwrap();

    // Read a shape+sprite-only reference from the resized target. The final
    // composed draw repeats both passes before pending Korean UI is painted.
    let target = OffscreenTarget::new(&gpu, 256, 192, format);
    compositor
        .draw(target.render_view(), target.size(), &shapes, &[], &camera)
        .unwrap();
    let background = target.read_rgba8();
    compositor
        .draw(
            target.render_view(),
            target.size(),
            &shapes,
            &sprites,
            &camera,
        )
        .unwrap();
    let underlay = target.read_rgba8();
    let order_probe = (80, 126);
    let underlay_probe = pixel(&underlay, 256, order_probe.0, order_probe.1);
    let mut overlay_called = false;
    compositor
        .draw_with_overlay(
            target.render_view(),
            target.size(),
            &shapes,
            &sprites,
            &camera,
            |rhi, view, size| {
                overlay_called = true;
                pending.paint(&mut overlay, rhi, view, size, &ui.context)
            },
        )
        .unwrap();
    assert!(
        overlay_called,
        "an acquired target reaches the same-view overlay callback"
    );
    let capture = target.read_rgba8();

    // The pixel at the overlapping sprite sample remains A/B/A rather than a
    // regrouped A/A/B pass, proving the UI was submitted after the ordered runs.
    let base = pixel(&background, 256, 128, 96);
    let ordered = [[255, 0, 0, 128], [0, 255, 0, 128], [255, 0, 0, 128]]
        .into_iter()
        .fold(base, over);
    let regrouped = [[255, 0, 0, 128], [255, 0, 0, 128], [0, 255, 0, 128]]
        .into_iter()
        .fold(base, over);
    near(underlay_probe, ordered);
    near(pixel(&capture, 256, order_probe.0, order_probe.1), ordered);
    assert!(
        ordered
            .iter()
            .zip(regrouped)
            .any(|(ordered, regrouped)| ordered.abs_diff(regrouped) > 8),
        "A/B/A and A/A/B must be distinguishable"
    );

    // All five known Korean characters alter separate regions of the same view;
    // the masks differ so missing glyphs cannot silently become repeated tofu.
    let masks: Vec<Vec<bool>> = (0..5)
        .map(|index| {
            let x0 = ((8.0 + index as f32 * 22.0) * 2.0) as usize;
            (0..40)
                .flat_map(|y| (0..40).map(move |x| (x, y)))
                .map(|(x, y)| {
                    pixel(&capture, 256, x0 + x, 60 + y) != pixel(&underlay, 256, x0 + x, 60 + y)
                })
                .collect()
        })
        .collect();
    assert!(
        masks
            .iter()
            .all(|mask| mask.iter().filter(|changed| **changed).count() > 40),
        "each Korean glyph should render into the captured project view"
    );
    assert!(
        masks.windows(2).all(|pair| pair[0] != pair[1]),
        "distinct Korean glyphs should not repeat the same tofu mask"
    );

    // The screen-space clip survives the 2x resize: inside is the reused green
    // texture, while geometry outside the clip leaves the underlying scene alone.
    assert_ne!(
        pixel(&underlay, 256, 40, 156),
        pixel(&background, 256, 40, 156),
        "the UI clip probe must cover a real authored sprite texel"
    );
    near(pixel(&capture, 256, 40, 156), [20, 220, 40, 255]);
    assert_eq!(
        pixel(&capture, 256, 80, 156),
        pixel(&underlay, 256, 80, 156)
    );
    if let Some(directory) = std::env::var_os("ORR_PROJECT_UI_CAPTURE_DIR") {
        let directory = PathBuf::from(directory);
        assert!(directory.is_dir(), "capture directory must already exist");
        let path = directory.join("saved-project-korean-ui-compositor.png");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap_or_else(|error| panic!("create capture {}: {error}", path.display()));
        let mut encoder = png17::Encoder::new(file, 256, 192);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&capture)
            .unwrap();
    }
    eprintln!(
        "saved-project UI acceptance: A/B/A sample={:?}; 5 Korean glyphs; resize=256x192 @2x; clipped texture and skipped free/reuse epochs passed",
        pixel(&capture, 256, order_probe.0, order_probe.1)
    );

    // Keep an independent real saved-project readback alongside the synthetic
    // A/B/A probes above. Use the admitted seed, installed sprite assets, and
    // actual Playing HUD on the same acquired view.
    let bridge = seed.bridge().unwrap();
    let initial = bridge
        .snapshot()
        .expect("admitted PlayHost publishes a frame");
    presentation.update(Some(&initial)).unwrap();
    assert_eq!(presentation.sprites().len(), 2);
    let actual_size = (512, 512);
    let actual_format = TextureFormat::Rgba8UnormSrgb;
    let actual_target = OffscreenTarget::new(&gpu, actual_size.0, actual_size.1, actual_format);
    let mut actual_compositor =
        ProjectCompositor::new(gpu.clone(), actual_format, presentation.assets()).unwrap();

    actual_compositor
        .draw(
            actual_target.render_view(),
            actual_size,
            presentation.shapes(),
            &[],
            &presentation.camera,
        )
        .unwrap();
    let shape_only = actual_target.read_rgba8();
    actual_compositor
        .draw(
            actual_target.render_view(),
            actual_size,
            presentation.shapes(),
            presentation.sprites(),
            &presentation.camera,
        )
        .unwrap();
    let source_scene = actual_target.read_rgba8();
    let actual_sprites = presentation.sprites();
    for sprite in actual_sprites {
        assert!(
            changed_in_sprite_bounds(
                &shape_only,
                &source_scene,
                actual_size,
                &presentation.camera,
                sprite.instance,
            ) > 20,
            "both actual saved-project actors should be visible before the HUD"
        );
    }

    let mut playing_ui = GameUi::from_font(prepared_ui.font.clone(), true).unwrap();
    let hud = Hud {
        tick: initial.tick(),
        verified_tick: initial.verified_tick(),
        rollbacks: initial.stats().rollbacks,
    };
    let mut actual_pending = PendingOverlay::default();
    // Give the centered title window its normal egui sizing passes, retaining
    // the real font upload until the next available view.
    for frame in 0..3 {
        let (output, action) =
            playing_ui.show(input([512.0, 512.0], 1.0, 10.0 + frame as f64), hud);
        assert!(action.is_none());
        actual_pending.push(output).unwrap();
    }
    let play_point = playing_ui
        .buttons
        .iter()
        .find(|(action, _)| *action == Action::Play)
        .expect("title screen exposes the real Play button")
        .1
        .center();
    let (pressed, press_action) = playing_ui.show(
        pointer_input(
            [512.0, 512.0],
            1.0,
            13.0,
            vec![
                egui::Event::PointerMoved(play_point),
                egui::Event::PointerButton {
                    pos: play_point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        ),
        hud,
    );
    assert!(press_action.is_none());
    actual_pending.push(pressed).unwrap();
    let (released, play_action) = playing_ui.show(
        pointer_input(
            [512.0, 512.0],
            1.0,
            13.016,
            vec![egui::Event::PointerButton {
                pos: play_point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        ),
        hud,
    );
    assert_eq!(play_action, Some(Action::Play));
    actual_pending.push(released).unwrap();
    assert!(playing_ui.apply(play_action.unwrap()));
    assert_eq!(playing_ui.screen(), orr_sample::game_ui::Screen::Playing);
    let (playing_output, action) = playing_ui.show(input([512.0, 512.0], 1.0, 14.0), hud);
    assert!(action.is_none());
    actual_pending.push(playing_output).unwrap();
    let menu_button = playing_ui
        .buttons
        .iter()
        .find(|(action, _)| *action == Action::Menu)
        .expect("Playing HUD exposes its menu control")
        .1;

    let mut actual_overlay = GpuOverlay::new(&gpu, actual_format);
    let mut actual_overlay_called = false;
    actual_compositor
        .draw_with_overlay(
            actual_target.render_view(),
            actual_size,
            presentation.shapes(),
            actual_sprites,
            &presentation.camera,
            |rhi, view, size| {
                actual_overlay_called = true;
                actual_pending.paint(&mut actual_overlay, rhi, view, size, &playing_ui.context)
            },
        )
        .unwrap();
    assert!(actual_overlay_called);
    let playing_capture = actual_target.read_rgba8();

    for sprite in actual_sprites {
        assert_eq!(
            changed_in_sprite_bounds(
                &source_scene,
                &playing_capture,
                actual_size,
                &presentation.camera,
                sprite.instance,
            ),
            0,
            "Playing HUD must leave the saved-project actor pixels unchanged below it"
        );
    }
    let label_right = (menu_button.min.x.floor().max(0.0) as usize).min(actual_size.0 as usize);
    let label_bottom = (menu_button.max.y.ceil().max(0.0) as usize).min(64);
    let mut changed_label_pixels = 0;
    let mut visible_korean_glyph_pixels = 0;
    for y in 0..label_bottom {
        for x in 0..label_right {
            let before = pixel(&source_scene, actual_size.0 as usize, x, y);
            let after = pixel(&playing_capture, actual_size.0 as usize, x, y);
            if before != after {
                changed_label_pixels += 1;
                if after[..3].iter().all(|channel| *channel > 100) {
                    visible_korean_glyph_pixels += 1;
                }
            }
        }
    }
    assert!(
        changed_label_pixels > 100,
        "Playing HUD should add pixels to the real view"
    );
    assert!(
        visible_korean_glyph_pixels > 30,
        "Korean HUD glyph pixels should be visible before the Menu button"
    );

    if let Some(directory) = std::env::var_os("ORR_PROJECT_UI_CAPTURE_DIR") {
        let directory = PathBuf::from(directory);
        assert!(directory.is_dir(), "capture directory must already exist");
        let path = directory.join("saved-project-korean-playing.png");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap_or_else(|error| panic!("create capture {}: {error}", path.display()));
        let mut encoder = png17::Encoder::new(file, actual_size.0, actual_size.1);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&playing_capture)
            .unwrap();
    }
}
