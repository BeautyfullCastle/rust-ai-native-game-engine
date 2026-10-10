//! Mandatory-with-ORR_REQUIRE_GPU composed EditorApp framebuffer proof.
//! The installed PNG is painted by SpritePanel after the core viewport texture;
//! reading GpuViewport::read_rgba8 alone would entirely miss this feature.
#![cfg(feature = "sprites")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "common/arena_sprites.rs"]
mod fixture;

use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use fixture::*;
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP},
    editor::input::Phase,
    EditorApp, Mode,
};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use serde_json::json;
use std::path::Path;

fn gpu_available() -> bool {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => {
            eprintln!(
                "Arena composed framebuffer GPU adapter: {} (software: {})",
                gpu.adapter_name(),
                gpu.is_software()
            );
            true
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "ORR_REQUIRE_GPU is set but Arena GPU is unavailable: {error}"
            );
            eprintln!("SKIP: Arena composed framebuffer GPU unavailable: {error}");
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
    if let Some(directory) = std::env::var_os("ORR_SPRITE_CAPTURE_DIR") {
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

#[test]
fn composed_arena_png_bindings_follow_real_control_pause_seek_and_stop() {
    if !gpu_available() {
        return;
    }
    let fixture = Fixture::new();
    let editor = fixture.editor();
    let mut h = Harness::builder()
        .with_size([1500.0, 1400.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    settle(&mut h);
    assert!(h.state().has_gpu());
    assert!(h.state().viewport_gpu().is_some());
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    let scene_bytes = std::fs::read(&fixture.scene).unwrap();
    let procedural = frame(&mut h, "arena-01-procedural");
    two_locomotion_bindings(&mut h, &fixture);
    // The inspector's long section may scroll, but the viewport geometry stays
    // fixed. Move the cursor out of it before pixel comparisons.
    h.event(egui::Event::PointerMoved(egui::pos2(2.0, 2.0)));
    settle(&mut h);
    let bound = frame(&mut h, "arena-02-bound-idle");
    let rect = h.state().ui.viewport_rect.unwrap();
    let changed = viewport_difference(&procedural, &bound, rect);
    assert!(
        changed > 200,
        "sprite meshes must change the composed viewport: {changed}"
    );
    assert!(
        atlas_color_count(&bound, rect) > 100,
        "composed pixels must contain the installed PNG colors"
    );
    assert_eq!(region(h.state(), HERO), Some(10));
    assert_eq!(region(h.state(), TARGET), Some(20));
    let idle_sprite = sprite_pixels(&bound, h.state(), HERO);
    let second_sprite = sprite_pixels(&bound, h.state(), TARGET);
    assert_ne!(
        idle_sprite, second_sprite,
        "independent idle/walk assignments paint different PNG frames"
    );

    click(&mut h, "target");
    click(&mut h, "Follow selected entity");
    assert_eq!(document(&h).camera_follow.as_deref(), Some(TARGET));
    click(&mut h, LBL_PLAY);
    wait_for(
        &mut h,
        "follow the chosen second actor rather than keyboard slot zero",
        |app| {
            app.editor.mode() == Mode::Play
                && app.editor.camera.center == body_position(app, TARGET)
        },
    );
    click(&mut h, LBL_PAUSE);
    assert_eq!(
        h.state().editor.camera.center,
        body_position(h.state(), TARGET)
    );
    frame(&mut h, "arena-02b-follow-second-actor");
    click(&mut h, LBL_STOP);
    click(&mut h, "hero");
    click(&mut h, "Follow selected entity");
    click(&mut h, "Save bindings");
    no_error(&h);
    let authored = document(&h).clone();
    let saved = std::fs::read(&fixture.sidecar).unwrap();
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);

    let edit_camera = h.state().editor.camera;
    click(&mut h, LBL_PLAY);
    wait_for(&mut h, "real-time Arena play", |app| {
        app.editor.can_take_control()
    });
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert!(h
        .get_by_label("Assign sprite to selection")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .get_by_label("Follow selected entity")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Take control");
    wait_for(&mut h, "managed keyboard control", |app| {
        app.editor.input_phase() == Phase::Active
    });
    let start = body_position(h.state(), HERO);
    key(&h, egui::Key::ArrowRight, true);
    wait_for(&mut h, "hero walking from actual key events", |app| {
        moving(app, HERO) && body_position(app, HERO)[0] > start[0]
    });
    assert!(!moving(h.state(), TARGET));
    assert!(matches!(region(h.state(), HERO), Some(20 | 21)));
    assert!(
        matches!(region(h.state(), TARGET), Some(20 | 21)),
        "stationary second actor uses its own inverted mapping"
    );
    assert_eq!(
        h.state().editor.camera.center,
        body_position(h.state(), HERO)
    );
    let walking = frame(&mut h, "arena-03-controlled-walk-follow");
    let walking_sprite = sprite_pixels(&walking, h.state(), HERO);
    assert_ne!(
        idle_sprite, walking_sprite,
        "movement changes the PNG pose in the composed framebuffer"
    );
    assert!(atlas_color_count(&walking, h.state().ui.viewport_rect.unwrap()) > 100);
    assert_eq!(document(&h), &authored);
    assert_eq!(std::fs::read(&fixture.sidecar).unwrap(), saved);

    key(&h, egui::Key::ArrowRight, false);
    wait_for(&mut h, "released key returns hero to idle", |app| {
        !moving(app, HERO) && matches!(region(app, HERO), Some(10 | 11))
    });
    frame(&mut h, "arena-04-key-release-idle");
    key(&h, egui::Key::ArrowUp, true);
    wait_for(&mut h, "second walking gesture", |app| moving(app, HERO));
    h.input_mut().focused = false;
    h.event(egui::Event::WindowFocused(false));
    wait_for(
        &mut h,
        "focus loss releases held input and idles actor",
        |app| app.editor.input_phase() == Phase::Off && !moving(app, HERO),
    );
    let focus_lost_position = body_position(h.state(), HERO);
    let focus_lost_tick = h.state().editor.timeline().unwrap().tick;
    wait_for(
        &mut h,
        "released actor stays still on later host ticks",
        |app| app.editor.timeline().unwrap().tick >= focus_lost_tick + 3,
    );
    assert_eq!(body_position(h.state(), HERO), focus_lost_position);
    frame(&mut h, "arena-05-focus-loss-idle");
    h.input_mut().focused = true;
    h.event(egui::Event::WindowFocused(true));
    key(&h, egui::Key::ArrowUp, false);
    settle(&mut h);
    assert_eq!(
        h.state().editor.input_phase(),
        Phase::Off,
        "focus regain does not silently reclaim input"
    );

    for release_key in [egui::Key::Tab, egui::Key::Escape] {
        click(&mut h, "Take control");
        wait_for(&mut h, "explicit control reacquisition", |app| {
            app.editor.input_phase() == Phase::Active
        });
        key(&h, release_key, true);
        h.run_steps(1);
        key(&h, release_key, false);
        wait_for(&mut h, "Tab/Escape still releases managed control", |app| {
            app.editor.input_phase() == Phase::Off
        });
        settle(&mut h);
    }
    click(&mut h, LBL_PAUSE);
    let paused_tick = h.state().editor.timeline().unwrap().tick;
    let paused_elapsed = h.state().sprites.playback.state(HERO).elapsed_ms;
    let paused_region = region(h.state(), HERO);
    h.run_steps(30);
    assert_eq!(h.state().editor.timeline().unwrap().tick, paused_tick);
    assert_eq!(
        h.state().sprites.playback.state(HERO).elapsed_ms,
        paused_elapsed
    );
    assert_eq!(region(h.state(), HERO), paused_region);
    assert!(h
        .get_by_label("Take control")
        .accesskit_node()
        .is_disabled());
    frame(&mut h, "arena-06-paused");

    pan_viewport(&mut h);
    assert!(h.state().sprites.playback.follow_suspended());
    assert_ne!(
        h.state().editor.camera.center,
        body_position(h.state(), HERO)
    );
    assert_eq!(
        document(&h),
        &authored,
        "manual navigation suspends live following, without rewriting saved configuration"
    );
    click(&mut h, "Resume camera follow");
    assert!(!h.state().sprites.playback.follow_suspended());
    assert_eq!(
        h.state().editor.camera.center,
        body_position(h.state(), HERO)
    );
    let zoom_before = h.state().editor.camera.half_extent;
    let zoom_at = h.state().ui.viewport_rect.unwrap().center() + egui::vec2(60.0, 20.0);
    h.event(egui::Event::PointerMoved(zoom_at));
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        phase: egui::TouchPhase::Move,
        delta: egui::vec2(0.0, 70.0),
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    assert!(
        h.state().sprites.playback.follow_suspended(),
        "wheel zoom also suspends following"
    );
    assert_ne!(h.state().editor.camera.half_extent, zoom_before);
    assert_eq!(document(&h), &authored);
    click(&mut h, "Resume camera follow");
    assert!(!h.state().sprites.playback.follow_suspended());
    assert_eq!(
        h.state().editor.camera.center,
        body_position(h.state(), HERO)
    );
    click(&mut h, LBL_STEP);
    assert_eq!(h.state().editor.timeline().unwrap().tick, paused_tick + 1);
    assert!(!moving(h.state(), HERO));
    assert_eq!(
        h.state().editor.camera.center,
        body_position(h.state(), HERO)
    );
    frame(&mut h, "arena-07-stepped");

    h.state_mut().editor.seek(0);
    settle(&mut h);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 0);
    assert_eq!(h.state().sprites.playback.state(HERO).elapsed_ms, 0);
    assert!(!moving(h.state(), HERO));
    assert_eq!(region(h.state(), HERO), Some(10));
    assert_eq!(region(h.state(), TARGET), Some(20));
    assert_eq!(
        h.state().editor.camera.center,
        body_position(h.state(), HERO)
    );
    frame(&mut h, "arena-08-rewound");
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(
        h.state().editor.camera,
        edit_camera,
        "Stop restores the pre-Play camera and zoom"
    );
    assert_eq!(region(h.state(), HERO), Some(10));
    assert_eq!(region(h.state(), TARGET), Some(20));
    frame(&mut h, "arena-09-stopped");
    assert_eq!(document(&h), &authored);
    assert_eq!(std::fs::read(&fixture.sidecar).unwrap(), saved);
    assert_eq!(std::fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert!(!h.state().sprites.bindings.as_ref().unwrap().dirty());
    let replay = h
        .state_mut()
        .editor
        .host_call(
            "verify.self",
            json!({
                "inputs": {"kind": "last_play"}, "checks": ["recording_matches"],
            }),
        )
        .unwrap();
    assert_eq!(
        replay["passed"], true,
        "presentation leaves recorded Arena replay checksums intact"
    );
}
