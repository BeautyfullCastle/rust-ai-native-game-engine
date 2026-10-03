//! Headless GPU tests of the 3D renderer: render to a texture, read the
//! pixels back, check lighting, shadows, depth, MSAA, instancing, the camera
//! projection and output encoding.
//!
//! They skip (with a message) when no adapter exists. Set `ORR_REQUIRE_GPU=1`
//! to fail instead of skipping (CI runs them on lavapipe, software Vulkan).
#![allow(clippy::float_arithmetic)] // a view-layer test: pixel colors are floats

use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions};
use orr_render::{Camera3D, Lighting, Material, OffscreenTarget, RenderList3D, Renderer3D, Settings3D, IDENTITY_ROT};
use std::sync::{Mutex, MutexGuard};

static SERIAL: Mutex<()> = Mutex::new(());

fn gpu() -> Option<(MutexGuard<'static, ()>, Wgpu)> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(g) => {
            eprintln!("gpu test adapter: {} (software: {})", g.adapter_name(), g.is_software());
            Some((guard, g))
        }
        Err(e) => {
            assert!(std::env::var_os("ORR_REQUIRE_GPU").is_none(), "ORR_REQUIRE_GPU is set but no adapter: {e}");
            eprintln!("SKIP: no GPU adapter ({e})");
            None
        }
    }
}

fn baseline_gpu() -> Option<(MutexGuard<'static, ()>, Wgpu)> {
    let force_software = match std::env::var("ORR_BASELINE_GPU_MODE").as_deref() {
        Ok("software") => true,
        Ok("hardware") | Err(_) => false,
        Ok(other) => {
            panic!("ORR_BASELINE_GPU_MODE must be 'hardware' or 'software', got {other:?}")
        }
    };
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    match Wgpu::headless(WgpuOptions {
        force_software,
        ..WgpuOptions::default()
    }) {
        Ok(gpu) => Some((guard, gpu)),
        Err(e) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "ORR_REQUIRE_GPU is set but no adapter: {e}"
            );
            eprintln!("SKIP: no GPU adapter ({e})");
            None
        }
    }
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c.is_control() => escaped.push_str(&format!("\\u{:04x}", c as u32)),
            c => escaped.push(c),
        }
    }
    escaped
}

fn duration_ms(value: Option<std::time::Duration>) -> String {
    value.map_or_else(
        || "null".to_owned(),
        |d| format!("{:.6}", d.as_secs_f64() * 1000.0),
    )
}

#[expect(clippy::disallowed_types, reason = "Wall clocks measure this view-only benchmark and its separate readback latency.")]
fn run_release_baseline_3d(preset: &str, settings: Settings3D) {
    let Some((_guard, gpu)) = baseline_gpu() else {
        return;
    };
    const W: u32 = 640;
    const H: u32 = 360;
    let target = OffscreenTarget::new(&gpu, W, H, TextureFormat::Rgba8UnormSrgb);
    let mut renderer = Renderer3D::with_settings(gpu.clone(), target.format(), settings);
    renderer.clear = BLACK;
    let camera = Camera3D::perspective([0.0, 28.0, 34.0], [0.0, 0.0, 0.0], 55.0);
    let mut list = RenderList3D::new();
    for z in 0..25u32 {
        for x in 0..40u32 {
            let color = [
                x as f32 / 39.0,
                0.25 + z as f32 / 50.0,
                ((x * 7 + z * 13) % 40) as f32 / 39.0,
            ];
            list.sphere(
                [x as f32 - 19.5, 0.35, z as f32 - 12.0],
                IDENTITY_ROT,
                0.32,
                &Material::new(color),
            );
        }
    }
    assert_eq!(list.instance_count(), 1000);
    let info = gpu.adapter().get_info();
    eprintln!(
        "ORR_BASELINE {{\"schema_version\":1,\"suite\":\"renderer\",\"case\":\"metadata\",\"renderer\":\"3d\",\"preset\":\"{preset}\",\"scene_id\":\"sphere_grid_1000_v1\",\"instance_count\":1000,\"resolution_px\":[{W},{H}],\"target_format\":\"Rgba8UnormSrgb\",\"renderer_initialization_included\":false,\"target_creation_included\":false,\"cold_scope\":\"first_frame_after_renderer_construction\",\"settings\":{{\"requested_msaa\":{},\"shadow_map_size\":{},\"mesh_segments\":{}}},\"shadows\":{},\"adapter_name\":\"{}\",\"adapter_backend\":\"{:?}\",\"adapter_device_type\":\"{:?}\",\"adapter_vendor\":{},\"adapter_device\":{},\"driver\":\"{}\",\"driver_info\":\"{}\",\"software\":{}}}",
        settings.msaa,
        settings.shadow_map_size,
        settings.mesh_segments,
        list.lighting.shadows,
        json_escape(&info.name),
        info.backend,
        info.device_type,
        info.vendor,
        info.device,
        json_escape(&info.driver),
        json_escape(&info.driver_info),
        gpu.is_software(),
    );

    for (frame_class, count) in [("cold", 1), ("warmup", 10), ("steady", 30)] {
        for frame_index in 0..count {
            let start = std::time::Instant::now();
            target.render3d(&mut renderer, &list, &camera);
            let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
            let stats = renderer.last_frame_stats();
            eprintln!(
                "ORR_BASELINE {{\"schema_version\":1,\"suite\":\"renderer\",\"case\":\"frame\",\"renderer\":\"3d\",\"preset\":\"{preset}\",\"scene_id\":\"sphere_grid_1000_v1\",\"instance_count\":1000,\"resolution_px\":[{W},{H}],\"adapter_name\":\"{}\",\"adapter_backend\":\"{:?}\",\"adapter_device_type\":\"{:?}\",\"adapter_vendor\":{},\"adapter_device\":{},\"driver\":\"{}\",\"driver_info\":\"{}\",\"software\":{},\"frame_class\":\"{frame_class}\",\"frame_index\":{frame_index},\"wall_scope\":\"render_call_return_not_gpu_completion\",\"wall_ms\":{wall_ms:.6},\"cpu_prepare_ms\":{},\"cpu_encode_ms\":{},\"cpu_submit_ms\":{},\"shape_instances\":{},\"mesh_instances\":{},\"line_instances\":{},\"main\":{{\"passes\":{},\"draw_calls\":{},\"instances\":{}}},\"shadow\":{{\"passes\":{},\"draw_calls\":{},\"instances\":{}}},\"upload_calls\":{},\"upload_bytes\":{},\"buffer_reallocations\":{},\"attachment_allocations\":{},\"attachment_reallocations\":{},\"msaa_samples\":{}}}",
                json_escape(&info.name),
                info.backend,
                info.device_type,
                info.vendor,
                info.device,
                json_escape(&info.driver),
                json_escape(&info.driver_info),
                gpu.is_software(),
                duration_ms(stats.cpu_prepare_time),
                duration_ms(stats.cpu_encode_time),
                duration_ms(stats.cpu_submit_time),
                stats.shape_instances,
                stats.mesh_instances,
                stats.line_instances,
                stats.main.passes,
                stats.main.draw_calls,
                stats.main.instances,
                stats.shadow.passes,
                stats.shadow.draw_calls,
                stats.shadow.instances,
                stats.upload_calls,
                stats.upload_bytes,
                stats.buffer_reallocations,
                stats.attachment_allocations,
                stats.attachment_reallocations,
                stats.msaa_samples,
            );
        }
    }
    let read_start = std::time::Instant::now();
    let image = target.read_rgba8();
    let readback_ms = read_start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(image.len(), (W * H * 4) as usize);
    assert!(
        image
            .chunks_exact(4)
            .any(|px| px[0] != 0 || px[1] != 0 || px[2] != 0),
        "baseline scene produced no visible pixels"
    );
    let checksum = image.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    eprintln!(
        "ORR_BASELINE {{\"schema_version\":1,\"suite\":\"renderer\",\"case\":\"readback\",\"renderer\":\"3d\",\"preset\":\"{preset}\",\"scene_id\":\"sphere_grid_1000_v1\",\"resolution_px\":[{W},{H}],\"readback_ms\":{readback_ms:.6},\"pixel_bytes\":{},\"fnv1a64\":\"{checksum:016x}\",\"timing_included_in_frame_samples\":false}}",
        image.len()
    );
}

#[test]
#[ignore = "opt-in release baseline; run serially with --ignored --exact"]
fn release_baseline_3d_low() {
    run_release_baseline_3d("low", Settings3D::LOW);
}

#[test]
#[ignore = "opt-in release baseline; run serially with --ignored --exact"]
fn release_baseline_3d_default() {
    run_release_baseline_3d("default", Settings3D::default());
}

const BLACK: [f64; 4] = [0.0, 0.0, 0.0, 1.0];
const SETTINGS: Settings3D = Settings3D { msaa: 4, shadow_map_size: 1024, mesh_segments: 32 };

/// Renders into an sRGB RGBA8 target (stored bytes are sRGB encoded).
fn render_with(gpu: &Wgpu, size: (u32, u32), settings: Settings3D, list: &RenderList3D, cam: &Camera3D) -> (Vec<u8>, u32) {
    let target = OffscreenTarget::new(gpu, size.0, size.1, TextureFormat::Rgba8UnormSrgb);
    let mut r = Renderer3D::with_settings(gpu.clone(), target.format(), settings);
    r.clear = BLACK;
    target.render3d(&mut r, list, cam);
    assert_eq!(r.last_frame_stats().msaa_samples, r.samples());
    (target.read_rgba8(), r.samples())
}

fn render(gpu: &Wgpu, size: (u32, u32), list: &RenderList3D, cam: &Camera3D) -> Vec<u8> {
    render_with(gpu, size, SETTINGS, list, cam).0
}

fn px(img: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2], img[i + 3]]
}

/// Luminance of a stored sRGB pixel, in linear light (0..1).
fn luma(p: [u8; 4]) -> f32 {
    let lin = |b: u8| {
        let c = f32::from(b) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(p[0]) + 0.7152 * lin(p[1]) + 0.0722 * lin(p[2])
}

fn srgb_byte(linear: f32) -> f32 {
    let c = linear.clamp(0.0, 1.0);
    let e = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    e * 255.0
}

/// A light that adds nothing: the lit color is the base color times emissive.
fn unlit() -> Lighting {
    Lighting { intensity: 0.0, ambient: 0.0, shadows: false, tonemap: false, ..Lighting::default() }
}

fn white() -> Material {
    Material::new([0.8, 0.8, 0.8]).rough(0.8)
}

#[test]
fn lit_sphere_is_brighter_at_the_center_than_at_the_rim() {
    let Some((_g, gpu)) = gpu() else { return };
    let mut list = RenderList3D::new();
    // The sun travels along -z, toward a camera on +z: it hits the sphere head on.
    list.lighting =
        Lighting { direction: [0.0, 0.0, -1.0], shadows: false, ambient: 0.1, intensity: 1.0, tonemap: false, ..Lighting::default() };
    list.sphere([0.0; 3], IDENTITY_ROT, 1.0, &white());
    let cam = Camera3D::perspective([0.0, 0.0, 5.0], [0.0; 3], 40.0);
    let img = render(&gpu, (128, 128), &list, &cam);
    let center = px(&img, 128, 64, 64);
    // Radius on screen: 1 / (5 * tan 20 deg) * 64 = about 35 px. The rim is at 33 px.
    let rim = px(&img, 128, 64 + 33, 64);
    let outside = px(&img, 128, 64 + 45, 64);
    assert!(luma(center) > luma(rim) * 1.8, "center {center:?} rim {rim:?}");
    assert!(luma(rim) > luma(outside), "the rim is still lit more than the background: {rim:?} {outside:?}");
    // Specular: a glossy sphere shows a highlight at the center, a matte one less.
    let mut glossy = RenderList3D::new();
    glossy.lighting = list.lighting;
    glossy.sphere([0.0; 3], IDENTITY_ROT, 1.0, &Material::new([0.8, 0.8, 0.8]).rough(0.15));
    let g_center = px(&render(&gpu, (128, 128), &glossy, &cam), 128, 64, 64);
    assert!(luma(g_center) > luma(center), "glossy {g_center:?} matte {center:?}");
}

/// The shadow scene: a ground plane, a box, a low sun traveling toward -x.
fn shadow_scene(shadows: bool) -> (RenderList3D, Camera3D) {
    let mut list = RenderList3D::new();
    list.lighting = Lighting {
        direction: [-1.0, -0.5, 0.0],
        shadows,
        intensity: 3.0,
        ambient: 0.2,
        tonemap: false,
        shadow_center: [0.0; 3],
        shadow_radius: 15.0,
        ..Lighting::default()
    };
    list.plane([0.0, 0.0, 0.0], 30.0, 30.0, &Material::new([0.7, 0.7, 0.7]).rough(1.0));
    list.cuboid([0.0, 1.5, 0.0], IDENTITY_ROT, [1.0, 1.0, 1.0], &Material::new([0.8, 0.3, 0.3]));
    (list, Camera3D::perspective([0.0, 14.0, 9.0], [-1.5, 0.0, 0.0], 50.0))
}

#[test]
fn a_box_casts_a_shadow_on_the_ground() {
    let Some((_g, gpu)) = gpu() else { return };
    let vp = (256u32, 256u32);
    let (list, cam) = shadow_scene(true);
    let img = render(&gpu, vp, &list, &cam);
    // The sun travels (-1, -0.5, 0): a point at height h of the box throws its shadow 2h to -x.
    // The box spans y 0.5..2.5, so its shadow covers x about -6 .. -1 at z in -1..1.
    let at = |p: [f32; 3]| {
        let s = cam.world_to_screen(p, vp).expect("in front of the camera");
        px(&img, vp.0, s[0].round() as u32, s[1].round() as u32)
    };
    let shadowed = at([-3.5, 0.0, 0.0]);
    let lit_beside = at([-3.5, 0.0, 5.0]);
    let lit_far = at([6.0, 0.0, 0.0]);
    assert!(luma(shadowed) < luma(lit_beside) * 0.6, "shadowed {shadowed:?} lit {lit_beside:?}");
    assert!(luma(lit_far) > luma(shadowed) * 1.5, "{lit_far:?} vs {shadowed:?}");
    // Same pixel with shadows off is as bright as its lit surroundings.
    let (off_list, _) = shadow_scene(false);
    let off = render(&gpu, vp, &off_list, &cam);
    let s = cam.world_to_screen([-3.5, 0.0, 0.0], vp).unwrap();
    let unshadowed = px(&off, vp.0, s[0].round() as u32, s[1].round() as u32);
    assert!(luma(unshadowed) > luma(shadowed) * 1.5, "shadows off {unshadowed:?} vs on {shadowed:?}");
    assert!((luma(unshadowed) - luma(lit_beside)).abs() < luma(lit_beside) * 0.15, "{unshadowed:?} {lit_beside:?}");
    // The shadow is cast by the box only: a point lit at the same distance but on the sun side is not dark.
    let sun_side = at([3.5, 0.0, 0.0]);
    assert!(luma(sun_side) > luma(shadowed) * 1.5);
}

#[test]
fn a_ground_pixel_just_outside_the_shadow_is_lit_and_the_edge_is_soft() {
    let Some((_g, gpu)) = gpu() else { return };
    let vp = (320u32, 320u32);
    let (list, cam) = shadow_scene(true);
    let img = render(&gpu, vp, &list, &cam);
    // Walk along z through the shadow's side edge (the shadow spans z -1..1 near its far end).
    let values: Vec<f32> = (-30..=30)
        .map(|i| {
            let s = cam.world_to_screen([-4.0, 0.0, i as f32 * 0.1], vp).unwrap();
            luma(px(&img, vp.0, s[0].round() as u32, s[1].round() as u32))
        })
        .collect();
    let (min, max) = values.iter().fold((f32::MAX, 0.0f32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    assert!(min < max * 0.6, "no shadow found along the line: {min} {max}");
    // PCF: some samples lie between full shadow and full light.
    let mid = values.iter().filter(|&&v| v > min + (max - min) * 0.2 && v < min + (max - min) * 0.8).count();
    assert!(mid >= 1, "expected a soft edge, values {values:?}");
}

#[test]
fn near_object_hides_the_far_one_in_either_draw_order() {
    let Some((_g, gpu)) = gpu() else { return };
    let cam = Camera3D::perspective([0.0, 0.0, 10.0], [0.0; 3], 40.0);
    let red = Material::new([1.0, 0.0, 0.0]).glow(1.0);
    let blue = Material::new([0.0, 0.0, 1.0]).glow(1.0);
    for near_first in [true, false] {
        let mut list = RenderList3D::new();
        list.lighting = unlit();
        // Same mesh kind, so the list order is the draw order.
        let (near, far) = (([0.0, 0.0, 2.0], 1.0, &red), ([0.0, 0.0, -6.0], 3.0, &blue));
        let order = if near_first { [near, far] } else { [far, near] };
        for (p, r, m) in order {
            list.sphere(p, IDENTITY_ROT, r, m);
        }
        let img = render(&gpu, (128, 128), &list, &cam);
        let c = px(&img, 128, 64, 64);
        assert!(c[0] > 200 && c[2] < 30, "near_first={near_first}: center is {c:?}, expected the near red sphere");
        // Away from the near sphere the big far sphere shows.
        let side = px(&img, 128, 64 + 25, 64);
        assert!(side[2] > 150 && side[0] < 40, "near_first={near_first}: far sphere visible beside it, got {side:?}");
    }
}

#[test]
fn msaa_resolves_and_smooths_edges() {
    let Some((_g, gpu)) = gpu() else { return };
    let vp = (96u32, 96u32);
    let mut list = RenderList3D::new();
    list.lighting = unlit();
    let rot = [0.0, 0.0, (0.5f32).sin(), (0.5f32).cos()]; // a turn of 1 radian about z
    list.cuboid([0.0; 3], rot, [1.0, 1.0, 1.0], &Material::new([1.0, 1.0, 1.0]).glow(1.0));
    let cam = Camera3D::orthographic([0.0, 0.0, 10.0], [0.0; 3], 3.0);
    let partial = |img: &[u8]| {
        img.chunks_exact(4).filter(|p| p[0] > 25 && p[0] < 225 && (p[0] as i16 - p[1] as i16).abs() < 40).count()
    };
    let (one, s1) = render_with(&gpu, vp, Settings3D { msaa: 1, ..SETTINGS }, &list, &cam);
    assert_eq!(s1, 1);
    let (four, s4) = render_with(&gpu, vp, Settings3D { msaa: 4, ..SETTINGS }, &list, &cam);
    if s4 < 4 {
        eprintln!("adapter has no 4x MSAA (got {s4}x): checking only that it still renders");
        assert!(four.chunks_exact(4).any(|p| p[0] > 200));
        return;
    }
    // A flat white box on the background: without MSAA every pixel is fully in or out.
    let (p1, p4) = (partial(&one), partial(&four));
    assert!(p1 <= 4, "1x has {p1} partial pixels");
    assert!(p4 >= 30, "4x has {p4} partial pixels, expected an anti-aliased edge");
    // The interior is untouched by the resolve: fully white.
    let c = px(&four, vp.0, 48, 48);
    assert!(c[0] > 240 && c[1] > 240 && c[2] > 240, "{c:?}");
}

#[test]
fn an_unsupported_sample_count_falls_back() {
    let Some((_g, gpu)) = gpu() else { return };
    let mut list = RenderList3D::new();
    list.lighting = unlit();
    list.sphere([0.0; 3], IDENTITY_ROT, 1.0, &Material::new([1.0, 0.0, 0.0]).glow(1.0));
    let cam = Camera3D::perspective([0.0, 0.0, 5.0], [0.0; 3], 40.0);
    // 16x is never supported; the renderer must pick something valid and still draw.
    let (img, samples) = render_with(&gpu, (64, 64), Settings3D { msaa: 16, ..SETTINGS }, &list, &cam);
    assert!(matches!(samples, 1 | 2 | 4 | 8), "{samples}");
    assert!(px(&img, 64, 32, 32)[0] > 200);
}

/// The mobile preset renders the same scene: one sample, coarser spheres, nearly the same picture.
#[test]
fn low_preset_draws_the_same_scene() {
    let Some((_g, gpu)) = gpu() else { return };
    let mut list = RenderList3D::new();
    list.lighting = unlit();
    list.sphere([0.0; 3], IDENTITY_ROT, 1.0, &Material::new([1.0, 0.2, 0.1]).glow(1.0));
    list.sphere([1.8, 0.0, 0.0], IDENTITY_ROT, 0.6, &Material::new([0.1, 0.9, 0.2]).glow(1.0));
    let cam = Camera3D::perspective([0.0, 0.0, 6.0], [0.0; 3], 40.0);
    let (high, _) = render_with(&gpu, (96, 96), SETTINGS, &list, &cam);
    let (low, samples) = render_with(&gpu, (96, 96), Settings3D::LOW, &list, &cam);
    assert_eq!(samples, 1, "the low preset has no MSAA");
    assert!(px(&low, 96, 48, 48)[0] > 200, "the sphere is drawn");
    // Coverage differs by a few edge pixels only (a 12-segment sphere is inside the 32-segment one).
    let lit = |img: &[u8]| img.chunks_exact(4).filter(|p| p[0] > 100 || p[1] > 100).count();
    let (h, l) = (lit(&high) as i64, lit(&low) as i64);
    assert!(h > 300 && (h - l).abs() * 10 < h, "coverage {h} vs {l}");
}

fn grid_color(i: u32, j: u32) -> [f32; 3] {
    [i as f32 / 99.0, j as f32 / 99.0, ((i * 7 + j * 13) % 100) as f32 / 99.0]
}

#[test]
fn ten_thousand_instanced_boxes_each_get_their_own_color() {
    let Some((_g, gpu)) = gpu() else { return };
    let vp = (800u32, 800u32);
    let mut list = RenderList3D::new();
    list.lighting = unlit();
    // A 100 x 100 grid on the xz plane, seen from above: 8 px per cell, boxes 5 px wide.
    for i in 0..100u32 {
        for j in 0..100u32 {
            let pos = [(i as f32 - 49.5) * 0.1 * 1.0, 0.0, (j as f32 - 49.5) * 0.1];
            list.cuboid(pos, IDENTITY_ROT, [0.03, 0.03, 0.03], &Material::new(grid_color(i, j)).glow(1.0));
        }
    }
    assert_eq!(list.instance_count(), 10_000);
    // Looking down -y with +x to the right: screen up is -z.
    let mut cam = Camera3D::orthographic([0.0, 10.0, 0.0], [0.0; 3], 5.0);
    cam.up = [0.0, 0.0, -1.0];
    let img = render(&gpu, vp, &list, &cam);
    let mut bad = 0;
    let mut seen = std::collections::BTreeSet::new();
    let mut expected_distinct = std::collections::BTreeSet::new();
    for i in 0..100u32 {
        for j in 0..100u32 {
            let pos = [(i as f32 - 49.5) * 0.1, 0.0, (j as f32 - 49.5) * 0.1];
            let s = cam.world_to_screen(pos, vp).unwrap();
            let p = px(&img, vp.0, s[0].round() as u32, s[1].round() as u32);
            let c = grid_color(i, j);
            let want = [srgb_byte(c[0]), srgb_byte(c[1]), srgb_byte(c[2])];
            if (0..3).any(|k| (f32::from(p[k]) - want[k]).abs() > 2.0) {
                bad += 1;
            }
            seen.insert([p[0], p[1], p[2]]);
            expected_distinct.insert([want[0].round() as u8, want[1].round() as u8, want[2].round() as u8]);
        }
    }
    assert_eq!(bad, 0, "{bad} of 10000 cells have the wrong color");
    assert!(expected_distinct.len() > 5000, "test colors are not distinct enough: {}", expected_distinct.len());
    assert!(seen.len() * 10 >= expected_distinct.len() * 9, "{} distinct colors seen, expected about {}", seen.len(), expected_distinct.len());
}

#[test]
fn a_known_world_point_lands_on_the_expected_pixel() {
    let Some((_g, gpu)) = gpu() else { return };
    let vp = (256u32, 192u32);
    let cam = Camera3D::perspective([0.0, 0.0, 10.0], [0.0; 3], 60.0);
    // By hand: tan(30 deg) * 10 = 5.7735 units fill half the height (96 px), so 1 unit = 16.63 px.
    // The point (2, 1, 0) is at x = 128 + 2 * 16.63 = 161.3 and y = 96 - 16.63 = 79.4.
    let p = cam.world_to_screen([2.0, 1.0, 0.0], vp).unwrap();
    assert!((p[0] - 161.26).abs() < 0.05 && (p[1] - 79.37).abs() < 0.05, "{p:?}");
    let mut list = RenderList3D::new();
    list.lighting = unlit();
    list.sphere([2.0, 1.0, 0.0], IDENTITY_ROT, 0.25, &Material::new([1.0, 1.0, 1.0]).glow(1.0));
    let img = render(&gpu, vp, &list, &cam);
    let (mut sx, mut sy, mut n) = (0.0f32, 0.0f32, 0.0f32);
    for y in 0..vp.1 {
        for x in 0..vp.0 {
            let w = f32::from(px(&img, vp.0, x, y)[0]) / 255.0;
            sx += w * (x as f32 + 0.5);
            sy += w * (y as f32 + 0.5);
            n += w;
        }
    }
    assert!(n > 20.0, "the marker sphere is not visible");
    let (cx, cy) = (sx / n, sy / n);
    assert!((cx - p[0]).abs() < 1.0 && (cy - p[1]).abs() < 1.0, "centroid ({cx}, {cy}) vs projected {p:?}");
}

#[test]
fn debug_lines_draw_with_depth_and_survive_the_near_plane() {
    let Some((_g, gpu)) = gpu() else { return };
    let vp = (128u32, 128u32);
    let cam = Camera3D::perspective([0.0, 0.0, 6.0], [0.0; 3], 50.0);
    let mut list = RenderList3D::new();
    list.lighting = unlit();
    // A horizontal 4 px line through the center, and one that starts behind the camera.
    list.line([-2.0, 0.0, 0.0], [2.0, 0.0, 0.0], 4.0, [0.0, 1.0, 0.0, 1.0]);
    list.line([0.0, 1.0, 0.0], [0.0, 1.0, 20.0], 3.0, [1.0, 0.0, 0.0, 1.0]);
    // An opaque box in front of the right half of the line hides it.
    list.cuboid([1.2, 0.0, 2.0], IDENTITY_ROT, [0.5, 0.5, 0.5], &Material::new([0.0, 0.0, 1.0]).glow(1.0));
    let img = render(&gpu, vp, &list, &cam);
    let left = cam.world_to_screen([-1.0, 0.0, 0.0], vp).unwrap();
    let c = px(&img, vp.0, left[0].round() as u32, left[1].round() as u32);
    assert!(c[1] > 200 && c[0] < 40, "left of the line is green: {c:?}");
    let right = cam.world_to_screen([1.2, 0.0, 0.0], vp).unwrap();
    let h = px(&img, vp.0, right[0].round() as u32, right[1].round() as u32);
    assert!(h[2] > 200 && h[1] < 40, "the box hides the line: {h:?}");
    // Width: the line is about 4 px thick.
    let x = left[0].round() as u32;
    let thick = (0..vp.1).filter(|&y| px(&img, vp.0, x, y)[1] > 150).count();
    assert!((3..=6).contains(&thick), "line thickness {thick}");
    // The clipped red line toward the camera is drawn from the near plane up to its far end.
    let red_pixels = img.chunks_exact(4).filter(|p| p[0] > 200 && p[1] < 60 && p[2] < 60).count();
    assert!(red_pixels > 5, "the red line crossing the near plane is visible ({red_pixels})");
}

#[test]
fn srgb_and_linear_targets_look_the_same() {
    let Some((_g, gpu)) = gpu() else { return };
    let mut list = RenderList3D::new();
    list.lighting = Lighting { direction: [0.3, -0.6, -1.0], shadows: false, ..Lighting::default() };
    list.sphere([0.0; 3], IDENTITY_ROT, 1.0, &Material::new([0.2, 0.5, 0.9]));
    let cam = Camera3D::perspective([0.0, 0.0, 5.0], [0.0; 3], 40.0);
    let mut shots = Vec::new();
    for format in [TextureFormat::Rgba8UnormSrgb, TextureFormat::Rgba8Unorm, TextureFormat::Bgra8UnormSrgb] {
        let target = OffscreenTarget::new(&gpu, 64, 64, format);
        let mut r = Renderer3D::with_settings(gpu.clone(), format, SETTINGS);
        r.clear = [0.05, 0.05, 0.05, 1.0];
        target.render3d(&mut r, &list, &cam);
        shots.push(target.read_rgba8());
    }
    // Hardware encoding (sRGB target) and shader encoding (plain target) agree to a few levels,
    // and the RGB/BGRA order is handled by `read_rgba8`.
    // Only the interior is compared: the MSAA resolve averages in encoded space on the plain target
    // and in linear space on the sRGB ones, so silhouette pixels legitimately differ.
    for other in &shots[1..] {
        let mut worst = 0;
        let mut at = (0, 0);
        for y in 0..64u32 {
            for x in 0..64u32 {
                let (dx, dy) = (x as f32 - 32.0, y as f32 - 32.0);
                if dx * dx + dy * dy < 13.0 * 13.0 {
                    let (a, b) = (px(&shots[0], 64, x, y), px(other, 64, x, y));
                    let d = (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap();
                    if d > worst {
                        worst = d;
                        at = (x, y);
                    }
                }
            }
        }
        assert!(worst <= 4, "targets differ by {worst} levels at {at:?}: {:?} vs {:?}", px(&shots[0], 64, at.0, at.1), px(other, 64, at.0, at.1));
    }
    let center = px(&shots[0], 64, 32, 32);
    assert!(center[2] > center[0], "a blue sphere is blue: {center:?}");
}

#[test]
fn render_into_a_resized_target_and_an_empty_list() {
    let Some((_g, gpu)) = gpu() else { return };
    let cam = Camera3D::perspective([0.0, 0.0, 5.0], [0.0; 3], 40.0);
    let mut target = OffscreenTarget::new(&gpu, 48, 48, TextureFormat::Rgba8UnormSrgb);
    let mut r = Renderer3D::with_settings(gpu.clone(), target.format(), SETTINGS);
    assert_eq!(r.last_frame_stats(), orr_render::FrameStats::default());
    target.render3d(&mut r, &RenderList3D::new(), &cam);
    let empty = r.last_frame_stats();
    let attachments = 1 + u32::from(r.samples() > 1);
    assert_eq!(
        empty.main,
        orr_render::PassStats {
            passes: 1,
            draw_calls: 0,
            instances: 0
        }
    );
    assert_eq!(
        empty.shadow,
        orr_render::PassStats {
            passes: 1,
            draw_calls: 0,
            instances: 0
        }
    );
    assert_eq!(
        (
            empty.shape_instances,
            empty.mesh_instances,
            empty.line_instances
        ),
        (0, 0, 0)
    );
    assert_eq!((empty.upload_calls, empty.upload_bytes), (2, 512)); // two 3D globals
    assert_eq!(empty.buffer_reallocations, 0);
    assert_eq!(
        (empty.attachment_allocations, empty.attachment_reallocations),
        (attachments, 0)
    );
    assert_eq!(empty.msaa_samples, r.samples());
    assert!(
        empty.cpu_prepare_time.is_some()
            && empty.cpu_encode_time.is_some()
            && empty.cpu_submit_time.is_some()
    );
    let img = target.read_rgba8();
    assert!(img.chunks_exact(4).all(|p| p == &img[0..4]), "an empty list shows only the background");
    assert!(target.resize(80, 40));
    let mut list = RenderList3D::new();
    list.sphere([0.0; 3], IDENTITY_ROT, 1.0, &white());
    target.render3d(&mut r, &list, &cam);
    let img = target.read_rgba8();
    assert_eq!(img.len(), 80 * 40 * 4);
    assert_ne!(px(&img, 80, 40, 20), px(&img, 80, 2, 2));
    let resized = r.last_frame_stats();
    assert_eq!(
        resized.main,
        orr_render::PassStats {
            passes: 1,
            draw_calls: 1,
            instances: 1
        }
    );
    assert_eq!(resized.shadow, resized.main);
    assert_eq!((resized.upload_calls, resized.upload_bytes), (3, 512 + 80));
    assert_eq!(resized.buffer_reallocations, 0);
    assert_eq!(
        (
            resized.attachment_allocations,
            resized.attachment_reallocations
        ),
        (attachments, attachments)
    );

    target.render3d(&mut r, &list, &cam);
    let warm = r.last_frame_stats();
    assert_eq!(warm.main, resized.main);
    assert_eq!(warm.shadow, resized.shadow);
    assert_eq!(
        (warm.attachment_allocations, warm.attachment_reallocations),
        (0, 0)
    );
    assert_eq!(warm.buffer_reallocations, 0);
    assert_eq!(
        target.read_rgba8(),
        img,
        "instrumentation preserves warm-frame pixels"
    );

    list.clear();
    list.lighting.shadows = false;
    target.render3d(&mut r, &list, &cam);
    let cleared = r.last_frame_stats();
    assert_eq!(cleared.main, empty.main);
    assert_eq!(
        cleared.shadow, empty.shadow,
        "disabled shadows still encode a clear pass"
    );
    assert_eq!((cleared.mesh_instances, cleared.line_instances), (0, 0));
    assert_eq!((cleared.upload_calls, cleared.upload_bytes), (2, 512));
    assert_eq!(
        (
            cleared.attachment_allocations,
            cleared.attachment_reallocations
        ),
        (0, 0)
    );
    assert_eq!(cleared.buffer_reallocations, 0);
    let img = target.read_rgba8();
    assert!(img.chunks_exact(4).all(|p| p == &img[0..4]));
}

#[test]
fn frame_stats_count_mesh_batches_shadow_exclusions_and_buffer_growth() {
    let Some((_g, gpu)) = gpu() else { return };
    let target = OffscreenTarget::new(&gpu, 32, 32, TextureFormat::Rgba8UnormSrgb);
    let mut r = Renderer3D::with_settings(gpu.clone(), target.format(), Settings3D::LOW);
    r.clear = BLACK;
    let cam = Camera3D::perspective([0.0, 0.0, 5.0], [0.0; 3], 40.0);
    let mut list = RenderList3D::new();
    list.lighting = unlit();
    let red = Material::new([1.0, 0.0, 0.0]).glow(1.0);
    list.sphere([0.0; 3], IDENTITY_ROT, 1.0, &red);
    list.capsule([100.0; 3], IDENTITY_ROT, 1.0, 1.0, &red);
    list.plane([100.0; 3], 1.0, 1.0, &red);
    for _ in 0..1025 {
        list.cuboid([100.0; 3], IDENTITY_ROT, [1.0; 3], &red);
        list.line([100.0; 3], [101.0; 3], 1.0, [1.0; 4]);
    }
    target.render3d(&mut r, &list, &cam);
    let cold = r.last_frame_stats();
    assert_eq!(
        (
            cold.shape_instances,
            cold.mesh_instances,
            cold.line_instances
        ),
        (0, 1028, 1025)
    );
    assert_eq!(
        cold.main,
        orr_render::PassStats {
            passes: 1,
            draw_calls: 5,
            instances: 2053
        }
    );
    assert_eq!(
        cold.shadow,
        orr_render::PassStats {
            passes: 1,
            draw_calls: 0,
            instances: 0
        },
        "disabled shadows still clear, without drawing casters"
    );
    assert_eq!(cold.upload_calls, 7); // two uniforms, four mesh kinds, lines
    assert_eq!(cold.upload_bytes, 512 + 1028 * 80 + 1025 * 48);
    assert_eq!(cold.buffer_reallocations, 2);
    assert_eq!(
        (cold.attachment_allocations, cold.attachment_reallocations),
        (1, 0)
    );
    assert_eq!(cold.msaa_samples, 1);
    let image = target.read_rgba8();
    assert!(px(&image, 32, 16, 16)[0] > 200);

    list.lighting.shadows = true;
    target.render3d(&mut r, &list, &cam);
    let warm = r.last_frame_stats();
    assert_eq!(warm.main, cold.main);
    assert_eq!(
        warm.shadow,
        orr_render::PassStats {
            passes: 1,
            draw_calls: 3,
            instances: 1027
        },
        "planes and lines are excluded from the shadow pass"
    );
    assert_eq!(
        (warm.upload_calls, warm.upload_bytes),
        (cold.upload_calls, cold.upload_bytes)
    );
    assert_eq!(warm.buffer_reallocations, 0);
    assert_eq!(
        (warm.attachment_allocations, warm.attachment_reallocations),
        (0, 0)
    );
    assert_eq!(
        target.read_rgba8(),
        image,
        "the emissive object remains unchanged"
    );

    list.clear();
    list.plane([100.0; 3], 1.0, 1.0, &red);
    target.render3d(&mut r, &list, &cam);
    let plane = r.last_frame_stats();
    assert_eq!(
        plane.main,
        orr_render::PassStats {
            passes: 1,
            draw_calls: 1,
            instances: 1
        }
    );
    assert_eq!(
        plane.shadow,
        orr_render::PassStats {
            passes: 1,
            draw_calls: 0,
            instances: 0
        },
        "the shadow pass runs with no caster draws for a plane-only list"
    );
    assert_eq!((plane.mesh_instances, plane.line_instances), (1, 0));
    assert_eq!((plane.upload_calls, plane.upload_bytes), (3, 512 + 80));
    assert_eq!(plane.buffer_reallocations, 0);
}
