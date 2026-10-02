//! Headless GPU tests: render to a texture, read the pixels back, compare.
//!
//! They skip (with a message) when no adapter exists, for example on a CI
//! runner without a GPU. Set `ORR_REQUIRE_GPU=1` to fail instead of skipping.
//! Hardware is preferred; a software adapter (WARP, llvmpipe) is the fallback.
#![allow(clippy::float_arithmetic)] // a view-layer test: pixel colors are floats

use orr_render::orr_rhi::{TextureFormat, Wgpu, WgpuOptions};
use orr_render::{Camera, OffscreenTarget, RenderList, Renderer};
use std::sync::{Mutex, MutexGuard};

const BLACK: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

/// Tests run one at a time: several devices created and polled in parallel
/// on the same adapter hung on this machine (DX12).
static SERIAL: Mutex<()> = Mutex::new(());

fn gpu_with(opts: WgpuOptions) -> Option<(MutexGuard<'static, ()>, Wgpu)> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    match Wgpu::headless(opts) {
        Ok(g) => {
            eprintln!("gpu test adapter: {} (software: {})", orr_render::orr_rhi::Rhi::adapter_name(&g), g.is_software());
            Some((guard, g))
        }
        Err(e) => {
            assert!(std::env::var_os("ORR_REQUIRE_GPU").is_none(), "ORR_REQUIRE_GPU is set but no adapter: {e}");
            eprintln!("SKIP: no GPU adapter ({e})");
            None
        }
    }
}

fn gpu() -> Option<(MutexGuard<'static, ()>, Wgpu)> {
    gpu_with(WgpuOptions::default())
}

/// Renders `list` into a linear RGBA8 target of `w` x `h` and returns the pixels.
fn render(gpu: &Wgpu, w: u32, h: u32, list: &RenderList, camera: &Camera) -> Vec<u8> {
    let target = OffscreenTarget::new(gpu, w, h, TextureFormat::Rgba8Unorm);
    let mut renderer = Renderer::new(gpu.clone(), target.format());
    renderer.clear = Some(BLACK);
    target.render(&mut renderer, list, camera);
    target.read_rgba8()
}

fn px(bytes: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
}

/// Sum of coverage in 0..1 over the whole image, judged by the red channel of a red shape on black.
fn red_area(bytes: &[u8]) -> f32 {
    bytes.chunks_exact(4).map(|p| f32::from(p[0]) / 255.0).sum()
}

fn near(a: [u8; 4], b: [u8; 4], tol: u8) -> bool {
    a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= tol)
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

#[test]
fn circle_at_a_known_position_covers_the_expected_pixels() {
    let Some((_guard, gpu)) = gpu() else { return };
    // 64x64, half_extent 8: 4 px per world unit. Circle at (2, 0), r = 3: center pixel (40, 32), r = 12 px.
    let camera = Camera::new([0.0, 0.0], 8.0);
    let mut list = RenderList::new();
    list.circle([2.0, 0.0], 3.0, RED);
    let img = render(&gpu, 64, 64, &list, &camera);
    // Center pixel is the plain color: the rotation dot is 6.6 px to the right.
    assert!(near(px(&img, 64, 40, 32), [255, 0, 0, 255], 2), "{:?}", px(&img, 64, 40, 32));
    // The +x dot is darker.
    assert!(px(&img, 64, 47, 32)[0] < 200, "{:?}", px(&img, 64, 47, 32));
    // Just outside the circle on each side.
    for (x, y) in [(40 + 14, 32), (40 - 14, 32), (40, 32 + 14), (40, 32 - 14), (5, 5)] {
        assert_eq!(px(&img, 64, x, y), [0, 0, 0, 255], "pixel {x},{y}");
    }
    // Just inside.
    for (x, y) in [(40 - 10, 32), (40, 32 + 10), (40, 32 - 10)] {
        assert!(px(&img, 64, x, y)[0] > 240, "pixel {x},{y}");
    }
    // Area: pi * 11.5^2 = 415 (the edge fade sits inside the radius, the dot removes a little red).
    let area = red_area(&img);
    assert!((area - 405.0).abs() < 405.0 * 0.08, "area {area}");
}

#[test]
fn capsule_covers_its_stadium() {
    let Some((_guard, gpu)) = gpu() else { return };
    // 4 px per unit. Half length 4, radius 2: body 32 x 16 px plus two caps: 48 x 16 px overall.
    let camera = Camera::new([0.0, 0.0], 8.0);
    let expected_area = 32.0 * 16.0 + std::f32::consts::PI * 8.0 * 8.0; // 713
    let mut list = RenderList::new();
    list.capsule([0.0, 0.0], 4.0, 2.0, 0.0, RED);
    let img = render(&gpu, 64, 64, &list, &camera);
    assert!(near(px(&img, 64, 32, 32), [255, 0, 0, 255], 2));
    assert!(px(&img, 64, 32 + 22, 32)[0] > 200, "inside the +x cap");
    assert!(px(&img, 64, 32 - 22, 32)[0] > 200, "inside the -x cap");
    assert_eq!(px(&img, 64, 32 + 22, 32 + 7), [0, 0, 0, 255], "the cap is round: the corner is empty");
    assert_eq!(px(&img, 64, 32, 32 + 10), [0, 0, 0, 255]);
    assert_eq!(px(&img, 64, 2, 32), [0, 0, 0, 255]);
    let area = red_area(&img);
    assert!((area - expected_area).abs() < expected_area * 0.05, "area {area} vs {expected_area}");

    // Turned by 90 degrees it stands up.
    list.clear();
    list.capsule([0.0, 0.0], 4.0, 2.0, std::f32::consts::FRAC_PI_2, RED);
    let img = render(&gpu, 64, 64, &list, &camera);
    assert!(px(&img, 64, 32, 32 - 22)[0] > 200);
    assert!(px(&img, 64, 32, 32 + 22)[0] > 200);
    assert_eq!(px(&img, 64, 32 + 22, 32), [0, 0, 0, 255]);
    let area = red_area(&img);
    assert!((area - expected_area).abs() < expected_area * 0.05, "turned area {area}");

    // A capsule with a zero length segment is a circle of the radius.
    list.clear();
    list.capsule([0.0, 0.0], 0.0, 3.0, 0.0, RED);
    let img = render(&gpu, 64, 64, &list, &camera);
    let area = red_area(&img);
    let circle = std::f32::consts::PI * 12.0 * 12.0;
    assert!((area - circle).abs() < circle * 0.05, "round capsule area {area}");
}

#[test]
fn camera_transform_matches_the_pixels() {
    let Some((_guard, gpu)) = gpu() else { return };
    let (w, h) = (96u32, 64u32);
    let mut camera = Camera::new([5.0, -3.0], 8.0);
    let point = [6.5, -1.0];
    let check = |camera: &Camera, half_px: f32| {
        let mut list = RenderList::new();
        list.quad(point, [1.0, 1.0], 0.0, RED);
        let img = render(&gpu, w, h, &list, camera);
        let s = camera.world_to_screen(point, (w, h));
        let (x, y) = (s[0].floor() as u32, s[1].floor() as u32);
        assert!(near(px(&img, w, x, y), [255, 0, 0, 255], 1), "camera {camera:?}: screen {s:?}");
        // Area follows the zoom: (2 * pixels per unit)^2 pixels, if the box is inside the image.
        let ppu = camera.pixels_per_unit(w, h);
        let expected = (2.0 * ppu) * (2.0 * ppu);
        let area = red_area(&img);
        assert!((area - expected).abs() < expected * 0.06 + half_px, "area {area} vs {expected} at ppu {ppu}");
    };
    check(&camera, 0.0);
    // Zoom in 2x around the box: the box is 4x the area.
    camera.zoom_at(2.0, camera.world_to_screen(point, (w, h)), (w, h));
    check(&camera, 0.0);
    camera.pan_pixels([-6.0, 4.0], (w, h));
    check(&camera, 0.0);
    // Zoomed out far: the box is a few pixels but still at the predicted place.
    camera.zoom_at(0.25, [w as f32 / 2.0, h as f32 / 2.0], (w, h));
    let mut list = RenderList::new();
    list.quad(point, [1.0, 1.0], 0.0, RED);
    let img = render(&gpu, w, h, &list, &camera);
    let s = camera.world_to_screen(point, (w, h));
    assert!(px(&img, w, s[0] as u32, s[1] as u32)[0] > 100);
}

#[test]
fn ten_thousand_instances_each_draw_their_own_color() {
    let Some((_guard, gpu)) = gpu() else { return };
    // 100 x 100 unit cells, 4 px each: the quads tile the 400 x 400 image.
    let (w, h) = (400u32, 400u32);
    let camera = Camera::new([50.0, 50.0], 50.0);
    let mut list = RenderList::new();
    for j in 0..100u32 {
        for i in 0..100u32 {
            list.quad([i as f32 + 0.5, j as f32 + 0.5], [0.5, 0.5], 0.0, [i as f32 / 99.0, j as f32 / 99.0, 0.25, 1.0]);
        }
    }
    assert_eq!(list.shapes.len(), 10_000);
    let img = render(&gpu, w, h, &list, &camera);
    let mut bad = 0;
    for j in (0..100u32).step_by(3) {
        for i in (0..100u32).step_by(3) {
            let s = camera.world_to_screen([i as f32 + 0.5, j as f32 + 0.5], (w, h));
            let got = px(&img, w, s[0] as u32, s[1] as u32);
            let want = [(i as f32 / 99.0 * 255.0).round() as u8, (j as f32 / 99.0 * 255.0).round() as u8, 64, 255];
            if !near(got, want, 2) {
                bad += 1;
            }
        }
    }
    assert_eq!(bad, 0, "cells with a wrong color");
}

#[test]
fn lines_have_a_fixed_pixel_width() {
    let Some((_guard, gpu)) = gpu() else { return };
    let camera = Camera::new([0.0, 0.0], 8.0);
    // A 4 px wide horizontal line through the middle: rows 30..=33 at x = 32.
    let mut list = RenderList::new();
    list.line([-4.0, 0.0], [4.0, 0.0], 4.0, RED);
    let img = render(&gpu, 64, 64, &list, &camera);
    for y in 30..=33 {
        assert!(px(&img, 64, 32, y)[0] > 240, "row {y}: {:?}", px(&img, 64, 32, y));
    }
    for y in [27, 28, 35, 36] {
        assert!(px(&img, 64, 32, y)[0] < 10, "row {y}: {:?}", px(&img, 64, 32, y));
    }
    // Ends: from x = 16 to 48, with square caps of half the width.
    assert!(px(&img, 64, 15, 31)[0] > 240);
    assert!(px(&img, 64, 10, 31)[0] < 10);
    assert!(px(&img, 64, 52, 31)[0] < 10);

    // The width does not change with zoom.
    let zoomed = Camera::new([0.0, 0.0], 2.0);
    let img = render(&gpu, 64, 64, &list, &zoomed);
    let column: Vec<u8> = (0..64).map(|y| px(&img, 64, 32, y)[0]).collect();
    let lit: f32 = column.iter().map(|&v| f32::from(v) / 255.0).sum();
    assert!((lit - 4.0).abs() < 0.5, "line width in pixels {lit}");

    // An AABB gizmo draws its four edges and stays hollow.
    list.clear();
    list.aabb([-4.0, -4.0], [4.0, 4.0], 2.0, RED);
    let img = render(&gpu, 64, 64, &list, &camera);
    assert!(px(&img, 64, 32, 16)[0] > 240, "top edge");
    assert!(px(&img, 64, 16, 32)[0] > 240, "left edge");
    assert!(px(&img, 64, 48, 32)[0] > 240, "right edge");
    assert!(px(&img, 64, 32, 48)[0] > 240, "bottom edge");
    assert_eq!(px(&img, 64, 32, 32), [0, 0, 0, 255], "inside stays empty");
}

#[test]
fn lines_draw_over_shapes() {
    let Some((_guard, gpu)) = gpu() else { return };
    let camera = Camera::new([0.0, 0.0], 8.0);
    let mut list = RenderList::new();
    // The line is pushed first but still ends up on top.
    list.line([-6.0, 0.0], [6.0, 0.0], 3.0, [0.0, 1.0, 0.0, 1.0]);
    list.quad([0.0, 0.0], [5.0, 5.0], 0.0, RED);
    let img = render(&gpu, 64, 64, &list, &camera);
    let c = px(&img, 64, 32, 32);
    assert!(c[1] > 200 && c[0] < 50, "{c:?}");
    assert!(near(px(&img, 64, 32, 20), [255, 0, 0, 255], 1));
}

#[test]
fn srgb_target_encodes_like_a_window_and_exposes_a_raw_view() {
    let Some((_guard, gpu)) = gpu() else { return };
    let mut target = OffscreenTarget::new(&gpu, 32, 32, TextureFormat::Rgba8UnormSrgb);
    let mut renderer = Renderer::new(gpu.clone(), target.format());
    renderer.clear = Some(BLACK);
    let mut list = RenderList::new();
    list.quad([0.0, 0.0], [10.0, 10.0], 0.0, [0.5, 0.5, 0.5, 1.0]);
    target.render(&mut renderer, &list, &Camera::new([0.0, 0.0], 4.0));
    let img = target.read_rgba8();
    // 0.5 linear is 188 in sRGB.
    assert!(near(px(&img, 32, 16, 16), [188, 188, 188, 255], 2), "{:?}", px(&img, 32, 16, 16));
    // The sample view exists and is a different view from the render view.
    let _ = target.sample_view();
    assert_eq!(target.generation(), 0);
    assert!(!target.resize(32, 32));
    assert!(target.resize(48, 16));
    assert_eq!((target.size(), target.generation()), ((48, 16), 1));
    assert_eq!(target.read_rgba8().len(), 48 * 16 * 4);
}

#[test]
fn a_frame_can_hold_shapes_and_gizmos_and_the_list_is_reusable() {
    let Some((_guard, gpu)) = gpu() else { return };
    let camera = Camera::new([0.0, 0.0], 8.0);
    let target = OffscreenTarget::new(&gpu, 64, 64, TextureFormat::Rgba8Unorm);
    let mut renderer = Renderer::new(gpu.clone(), target.format());
    renderer.clear = Some(BLACK);
    let mut list = RenderList::new();
    for frame in 0..3 {
        list.clear();
        list.circle([0.0, 0.0], 2.0, RED);
        list.circle_outline([0.0, 0.0], 6.0, 32, 1.0, [1.0, 1.0, 1.0, 1.0]);
        list.arrow([0.0, 0.0], [6.0, 6.0], 1.5, 1.0, [1.0, 1.0, 0.0, 1.0]);
        list.cross([-5.0, -5.0], 1.0, 1.0, [0.0, 1.0, 1.0, 1.0]);
        list.capsule_outline([0.0, -4.0], 2.0, 1.0, 0.3, 16, 1.0, [1.0, 0.0, 1.0, 1.0]);
        target.render(&mut renderer, &list, &camera);
        let img = target.read_rgba8();
        assert!(near(px(&img, 64, 27, 34), [255, 0, 0, 255], 2), "frame {frame}");
        // The outline circle at radius 6 = 24 px: pixel (56, 32).
        assert!(px(&img, 64, 55, 32)[1].max(px(&img, 64, 56, 32)[1]) > 100, "frame {frame}");
    }
}

#[test]
fn software_adapter_renders_the_same_circle() {
    // WARP (Windows) or llvmpipe (Linux): the CI path when there is no GPU.
    let Some((_guard, gpu)) = gpu_with(WgpuOptions { force_software: true, ..Default::default() }) else { return };
    let camera = Camera::new([0.0, 0.0], 8.0);
    let mut list = RenderList::new();
    list.circle([2.0, 0.0], 3.0, RED);
    list.capsule([-3.0, -4.0], 2.0, 1.0, 0.5, [0.0, 1.0, 0.0, 1.0]);
    let img = render(&gpu, 64, 64, &list, &camera);
    assert!(near(px(&img, 64, 40, 32), [255, 0, 0, 255], 2));
    let area = red_area(&img);
    assert!((area - 405.0).abs() < 405.0 * 0.08, "area {area}");
}

#[test]
fn frame_stats_track_empty_growing_warm_and_resized_frames() {
    let Some((_guard, gpu)) = gpu() else { return };
    let camera = Camera::new([0.0, 0.0], 8.0);
    let mut target = OffscreenTarget::new(&gpu, 64, 64, TextureFormat::Rgba8Unorm);
    let mut renderer = Renderer::new(gpu.clone(), target.format());
    renderer.clear = Some(BLACK);
    assert_eq!(
        renderer.last_frame_stats(),
        orr_render::FrameStats::default()
    );
    let mut list = RenderList::new();
    target.render(&mut renderer, &list, &camera);
    let empty = renderer.last_frame_stats();
    assert_eq!(
        empty.main,
        orr_render::PassStats {
            passes: 1,
            draw_calls: 0,
            instances: 0
        }
    );
    assert_eq!(empty.shadow, orr_render::PassStats::default());
    assert_eq!(
        (
            empty.shape_instances,
            empty.mesh_instances,
            empty.line_instances
        ),
        (0, 0, 0)
    );
    assert_eq!((empty.upload_calls, empty.upload_bytes), (1, 32)); // 2D globals
    assert_eq!(empty.buffer_reallocations, 0);
    assert_eq!(
        (empty.attachment_allocations, empty.attachment_reallocations),
        (0, 0)
    );
    assert_eq!(empty.msaa_samples, 1);
    assert!(
        empty.cpu_prepare_time.is_some()
            && empty.cpu_encode_time.is_some()
            && empty.cpu_submit_time.is_some()
    );
    assert_eq!(px(&target.read_rgba8(), 64, 32, 32), [0, 0, 0, 255]);

    list.circle([2.0, 0.0], 3.0, RED);
    // Cross the initial capacity of both instance buffers; the extras are offscreen.
    for _ in 0..1025 {
        list.circle([100.0, 100.0], 1.0, RED);
        list.line([100.0, 100.0], [101.0, 101.0], 1.0, RED);
    }
    target.render(&mut renderer, &list, &camera);
    let cold = renderer.last_frame_stats();
    assert_eq!(
        (
            cold.shape_instances,
            cold.mesh_instances,
            cold.line_instances
        ),
        (1026, 0, 1025)
    );
    assert_eq!(
        cold.main,
        orr_render::PassStats {
            passes: 1,
            draw_calls: 2,
            instances: 2051
        }
    );
    assert_eq!(cold.shadow, orr_render::PassStats::default());
    assert_eq!(cold.upload_calls, 3);
    assert_eq!(
        cold.upload_bytes,
        32 + (1026 * std::mem::size_of::<orr_render::ShapeInstance>()
            + 1025 * std::mem::size_of::<orr_render::LineInstance>()) as u64
    );
    assert_eq!(cold.buffer_reallocations, 2);
    let first = target.read_rgba8();
    assert!(near(px(&first, 64, 40, 32), [255, 0, 0, 255], 2));

    target.render(&mut renderer, &list, &camera);
    let warm = renderer.last_frame_stats();
    assert_eq!(warm.main, cold.main);
    assert_eq!(
        (warm.upload_calls, warm.upload_bytes),
        (cold.upload_calls, cold.upload_bytes)
    );
    assert_eq!(warm.buffer_reallocations, 0);
    assert_eq!(
        target.read_rgba8(),
        first,
        "instrumentation preserves warm-frame pixels"
    );

    assert!(target.resize(80, 40));
    list.clear();
    target.render(&mut renderer, &list, &camera);
    let resized = renderer.last_frame_stats();
    assert_eq!(resized.main, empty.main);
    assert_eq!((resized.shape_instances, resized.line_instances), (0, 0));
    assert_eq!((resized.upload_calls, resized.upload_bytes), (1, 32));
    assert_eq!(resized.buffer_reallocations, 0);
    assert_eq!(
        (
            resized.attachment_allocations,
            resized.attachment_reallocations
        ),
        (0, 0),
        "the caller's target resize is not a renderer allocation"
    );
    assert_eq!(px(&target.read_rgba8(), 80, 40, 20), [0, 0, 0, 255]);
}
