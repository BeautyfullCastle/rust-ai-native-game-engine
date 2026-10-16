//! GPU tests: the viewport rendered offscreen and read back, and the whole
//! app rendered by egui_kittest's wgpu renderer.
//!
//! They skip (with a message) when no adapter exists. Set `ORR_REQUIRE_GPU=1`
//! to fail instead (CI runs them on lavapipe, a software Vulkan).
#![allow(clippy::float_arithmetic)]

mod common;

use common::*;
use egui_kittest::Harness;
use orr_editor::viewport::{build_list, ViewportGpu, HIGHLIGHT};
use orr_editor::EditorApp;
use orr_render::Camera;

/// sRGB byte of a linear shader color channel (the target is an sRGB texture).
fn enc(c: f32) -> u8 {
    let v = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn enc3(c: [f32; 3]) -> [u8; 3] {
    [enc(c[0]), enc(c[1]), enc(c[2])]
}

fn px(img: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * w + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2]]
}

fn near(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= tol)
}

fn body_world(ed: &mut orr_editor::Editor, name: &str) -> [f32; 2] {
    let t = target_named(ed, name);
    xy(&field(ed, &t, BODY, "pos"))
}

/// The frame entity of the selection (what the viewport highlights).
fn selected(ed: &orr_editor::Editor) -> Option<orr_ecs::Entity> {
    ed.selection().and_then(|t| ed.entity_of(t))
}

/// Brings the editor up to date with its host thread and draws a few frames.
fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..2 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

const STATIC_COLOR: [f32; 3] = [0.36, 0.38, 0.47];
const PADDLE_0_COLOR: [f32; 3] = [0.25, 0.6, 1.0];

#[test]
fn viewport_pixels_have_the_body_colors_and_the_selection_highlight() {
    let Some((_guard, rhi)) = gpu() else { return };
    let mut ed = demo_editor();
    let size = (800, 600);
    let mut vp = ViewportGpu::new(&rhi, size);

    // Whole scene, nothing selected: the floor and a paddle show their colors at their centers.
    let camera = ed.camera;
    let list = build_list(ed.bodies(), None, &camera, size);
    vp.render(size, &list, &camera);
    let img = vp.read_rgba8();
    assert_eq!(img.len(), (size.0 * size.1 * 4) as usize);
    let at = |w: [f32; 2]| {
        let s = camera.world_to_screen(w, size);
        (s[0].round() as u32, s[1].round() as u32)
    };
    let paddle = body_world(&mut ed, "paddle_0");
    let (x, y) = at(paddle);
    assert!(near(px(&img, size.0, x, y), enc3(PADDLE_0_COLOR), 4), "paddle_0 center {:?}", px(&img, size.0, x, y));
    // The floor: a point inside its box, away from the wall and the grid.
    let (x, y) = at([10.0, -1.0]);
    assert!(near(px(&img, size.0, x, y), enc3(STATIC_COLOR), 4), "floor {:?}", px(&img, size.0, x, y));
    // Empty space shows the background, not a body color.
    let (x, y) = at([0.5, 30.5]);
    let bg = px(&img, size.0, x, y);
    assert!(bg.iter().all(|&c| c < 80), "background {bg:?}");

    // Zoom on body_05 and select it: its outline is drawn in the highlight color.
    ed.select_named("body_05");
    let center = body_world(&mut ed, "body_05");
    let camera = Camera::new(center, 1.5);
    let size = (400, 400);
    let mut vp = ViewportGpu::new(&rhi, size);
    let with = build_list(ed.bodies(), selected(&ed), &camera, size);
    let without = build_list(ed.bodies(), None, &camera, size);
    assert!(with.lines.len() > without.lines.len(), "the highlight adds lines");
    let mut count_highlight = |list: &orr_render::RenderList| {
        vp.render(size, list, &camera);
        let img = vp.read_rgba8();
        let want = enc3([HIGHLIGHT[0], HIGHLIGHT[1], HIGHLIGHT[2]]);
        img.as_chunks::<4>().0.iter().filter(|p| near([p[0], p[1], p[2]], want, 6)).count()
    };
    let n_with = count_highlight(&with);
    let n_without = count_highlight(&without);
    assert!(n_with > 100, "highlight pixels with selection: {n_with}");
    assert_eq!(n_without, 0, "no highlight-colored pixels without a selection");
}

#[test]
fn the_app_shows_the_offscreen_viewport_as_an_egui_texture() {
    let Some((_guard, _probe)) = gpu() else { return };
    let mut h = Harness::builder()
        .with_size([1400.0, 850.0])
        .wgpu()
        .build_eframe(|cc| EditorApp::new(demo_editor(), cc.wgpu_render_state.clone()));
    settle(&mut h);
    assert!(h.state().has_gpu(), "the app got eframe's render state");
    assert!(h.state().viewport_gpu().is_some(), "the viewport drew");
    let image = h.render().expect("kittest wgpu render");
    let rect = h.state().ui.viewport_rect.expect("viewport rect");
    let vp = h.state().ui.viewport_px;
    let cam = h.state().editor.camera;
    let s = cam.world_to_screen([10.0, -1.0], vp);
    let (x, y) = ((rect.min.x + s[0]).round() as u32, (rect.min.y + s[1]).round() as u32);
    let p = image.get_pixel(x, y).0;
    assert!(near([p[0], p[1], p[2]], enc3(STATIC_COLOR), 6), "the floor pixel in the composed UI image: {p:?}");
    // Selecting a body draws the highlight into the same texture.
    h.state_mut().editor.select_named("body_05");
    settle(&mut h);
    let image = h.render().expect("render");
    let want = enc3([HIGHLIGHT[0], HIGHLIGHT[1], HIGHLIGHT[2]]);
    let found = image.pixels().any(|p| near([p.0[0], p.0[1], p.0[2]], want, 6));
    assert!(found, "a highlight-colored pixel appears in the UI image");
    // Play mode: the viewport follows the live frame (bodies fall).
    let before = image.clone();
    h.state_mut().editor.step(60);
    settle(&mut h);
    let after = h.render().expect("render");
    assert_ne!(before.as_raw(), after.as_raw(), "the picture changed after 60 ticks");
}

#[test]
fn arena_pixels_keep_player_colors_selection_and_proposal_ghosts() {
    let Some((_guard, rhi)) = gpu() else { return };
    let host = arena::ArenaHost::start(false);
    let mut agent = host.client();
    let mut ed = orr_editor::Editor::attach(&host.url, None).unwrap(); ed.sync();
    let size = (800, 600);
    let camera = ed.camera;
    let mut vp = ViewportGpu::new(&rhi, size);
    vp.render(size, &ed.viewport_list(size, &[]), &camera);
    let img = vp.read_rgba8();
    for (world, color) in [([-305.0,8.0], PADDLE_0_COLOR), ([295.0,8.0], [1.0,0.55,0.2])] {
        let at = camera.world_to_screen(world, size);
        assert!(near(px(&img,size.0,at[0] as u32,at[1] as u32), enc3(color), 5), "Arena player center color");
    }
    ed.select_named("hero"); ed.sync();
    vp.render(size, &ed.viewport_list(size, &[]), &camera);
    let highlighted = vp.read_rgba8();
    let highlight = enc3([HIGHLIGHT[0],HIGHLIGHT[1],HIGHLIGHT[2]]);
    assert!(highlighted.as_chunks::<4>().0.iter().filter(|p| near([p[0],p[1],p[2]],highlight,6)).count() > 30);
    let id = arena::proposal(&mut agent, -100, 100); ed.sync();
    assert!(ed.set_preview(Some(id))); ed.sync();
    vp.render(size, &ed.viewport_list(size, &[]), &camera);
    let preview = vp.read_rgba8();
    let cyan = orr_editor::viewport::PREVIEW_CHANGED;
    let cyan = enc3([cyan[0],cyan[1],cyan[2]]);
    assert!(preview.as_chunks::<4>().0.iter().any(|p| near([p[0],p[1],p[2]],cyan,6)), "proposal cyan outline");
    // Compare against exactly the same staged render with ghost lines removed;
    // grid/axis pixels cannot make this assertion pass.
    let mut no_ghost = ed.viewport_list(size, &[]);
    no_ghost.lines.retain(|line| line.color != orr_editor::viewport::PREVIEW_GHOST);
    vp.render(size, &no_ghost, &camera);
    let without_ghost = vp.read_rgba8();
    let old = camera.world_to_screen([-300.0,0.0], size);
    let mut changed = 0;
    for y in (old[1] as u32-20)..(old[1] as u32+20) {
        for x in (old[0] as u32-20)..(old[0] as u32+20) {
            if px(&preview,size.0,x,y) != px(&without_ghost,size.0,x,y) { changed += 1; }
        }
    }
    assert!(changed > 25, "ghost visibly occupies the previous player position");
}

#[test]
fn arena_app_composes_players_and_reflected_inspector() {
    use egui_kittest::kittest::Queryable;
    let Some((_guard, _probe)) = gpu() else { return };
    let host = arena::ArenaHost::start(false);
    let mut editor = orr_editor::Editor::attach(&host.url,None).unwrap();
    editor.sync(); editor.select_named("hero"); editor.sync();
    let mut h = Harness::builder().with_size([1400.0,850.0]).wgpu()
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    settle(&mut h);
    assert!(h.query_by_label("Arena").is_some());
    assert!(h.query_by_label_contains("Position").is_some());
    assert!(h.query_by_label_contains("PlayerTag").is_some());
    assert!(h.query_by_label_contains("Score").is_some());
    assert!(h.query_by_label("+ Body").is_none());
    let image = h.render().expect("Arena app framebuffer");
    let rect = h.state().ui.viewport_rect.unwrap();
    let at = h.state().editor.camera.world_to_screen([295.0,8.0],h.state().ui.viewport_px);
    let p = image.get_pixel((rect.min.x+at[0]).round() as u32,(rect.min.y+at[1]).round() as u32).0;
    assert!(near([p[0],p[1],p[2]],enc3([1.0,0.55,0.2]),6), "Arena player in composed UI: {p:?}");
    if let Some(dir) = std::env::var_os("ORR_EDITOR_SCREENSHOT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        image.save(dir.join("arena-egui-headless.png")).unwrap();
    }
}
