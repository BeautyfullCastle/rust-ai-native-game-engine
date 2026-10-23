//! Production widgets and native egui texture registration, with explicit frame times.
#![cfg(feature = "particle-preset-preview")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "../src/particle_preset_authoring.rs"]
mod particle_preset_authoring;
use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use particle_preset_authoring::App;
use std::sync::Arc;

// Keep event frames at the same explicit age; never settle a playing preview.
fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    h.step();
    h.get_by_role_and_label(Role::Button, label).click();
    h.step();
    h.step();
}
fn text(h: &mut Harness<'_, App>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.step();
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.step();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.step();
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.step();
}
fn at(h: &mut Harness<'_, App>, time: f64) {
    h.input_mut().time = Some(time);
    h.step();
}
fn draws(h: &Harness<'_, App>) -> Vec<orr_view::particles::ParticleDraw> {
    h.state().preview().draws().collect()
}
fn pixels(h: &Harness<'_, App>) -> Vec<u8> {
    let gpu = h.state().preview_gpu().expect("production preview GPU");
    assert_eq!(gpu.size(), (320, 240));
    gpu.read_rgba8()
}
fn save_target(dir: &std::path::Path, name: &str, pixels: &[u8]) {
    let file = std::fs::File::create(dir.join(name)).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 320, 240);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(pixels)
        .unwrap();
}
fn configure(h: &mut Harness<'_, App>) {
    for (label, value) in [
        ("Burst", "1"),
        ("Lifetime seconds", "1"),
        ("Velocity X", "2"),
        ("Velocity Y", "0"),
        ("Velocity Z", "0"),
        ("Jitter X", "0"),
        ("Jitter Y", "0"),
        ("Jitter Z", "0"),
        ("Start size", "2"),
        ("End size", "1"),
        ("Start R", "1"),
        ("Start G", "0"),
        ("Start B", "0"),
        ("Start A", "1"),
        ("End R", "1"),
        ("End G", "0"),
        ("End B", "0"),
        ("End A", "0"),
    ] {
        text(h, label, value);
    }
    click(h, "Apply draft");
}
#[test]
fn preview_widgets_capture_only_accepted_state_and_freeze_clock() {
    let mut h = Harness::builder()
        .with_size([1100.0, 1000.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(|_| App::new(None));
    at(&mut h, 10.0);
    configure(&mut h);
    let accepted = h.state().panel().document().clone();
    click(&mut h, "Preview accepted preset");
    assert_eq!(h.state().preview().snapshot(), Some(&accepted));
    assert!(h.state().preview_gpu().is_none());
    let initial = draws(&h);
    text(&mut h, "Lifetime seconds", "invalid");
    click(&mut h, "Apply draft");
    assert_eq!(h.state().preview().snapshot(), Some(&accepted));
    assert_eq!(draws(&h), initial);
    click(&mut h, "Pause preview");
    at(&mut h, 30.0);
    assert_eq!(draws(&h), initial);
    click(&mut h, "Resume preview");
    assert_eq!(draws(&h), initial);
    at(&mut h, 30.1);
    assert_ne!(draws(&h), initial);
    let later = draws(&h);
    h.input_mut().events.push(egui::Event::WindowFocused(false));
    at(&mut h, 40.0);
    assert_eq!(draws(&h), later);
    h.input_mut().events.push(egui::Event::WindowFocused(true));
    at(&mut h, 50.0);
    assert_eq!(draws(&h), later);
    h.input_mut().focused = false;
    at(&mut h, 100.0);
    assert_eq!(draws(&h), later);
    h.input_mut().focused = true;
    at(&mut h, 200.0);
    assert_eq!(draws(&h), later);
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .minimized = Some(true);
    at(&mut h, 300.0);
    assert_eq!(draws(&h), later);
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .minimized = Some(false);
    at(&mut h, 400.0);
    assert_eq!(draws(&h), later);
    click(&mut h, "Restart preview");
    assert_eq!(draws(&h), initial);
    click(&mut h, "Discard draft");
    text(&mut h, "Burst", "2");
    click(&mut h, "Apply draft");
    assert_eq!(h.state().preview().snapshot(), Some(&accepted));
    click(&mut h, "Preview accepted preset");
    assert_eq!(
        h.state().preview().snapshot(),
        Some(h.state().panel().document())
    );
    assert_eq!(draws(&h).len(), 2);
    click(&mut h, "Close preview");
    assert!(!h.state().preview().active());
    assert!(draws(&h).is_empty());
    assert_eq!(h.state().panel().document().burst, 2);
}

#[test]
#[ignore = "mandatory software Vulkan embedded preview acceptance"]
fn embedded_preview_widgets_have_real_gpu_pixels() {
    let evidence = std::path::PathBuf::from(
        std::env::var_os("ORR_PARTICLE_PREVIEW_EVIDENCE").expect("evidence directory"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|a| {
                let info = a.get_info();
                info.backend == wgpu::Backend::Vulkan && info.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "software Vulkan required".into())
    }));
    let mut state = None;
    let mut h = Harness::builder()
        .with_size([1100.0, 1000.0])
        .wgpu_setup(setup.into())
        .build_eframe(|cc| {
            state = cc.wgpu_render_state.clone();
            App::new(cc.wgpu_render_state.clone())
        });
    at(&mut h, 10.0);
    configure(&mut h);
    let accepted = h.state().panel().document().clone();
    let before_history = (h.state().panel().can_undo(), h.state().panel().can_redo());
    click(&mut h, "Preview accepted preset");
    let id = h.state().preview_gpu().unwrap().texture_id();
    let first = pixels(&h);
    save_target(&evidence, "preview-initial.png", &first);
    assert!(first
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[0] > 100 && p[1] < 30));
    h.render()
        .unwrap()
        .save(evidence.join("embedded-preview-full-window.png"))
        .unwrap();
    // Native registration must really be sampled by the full-window renderer.
    let full = h.render().unwrap();
    assert_eq!(full.dimensions(), (1100, 1000));
    assert!(
        full.as_raw()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 150 && p[1] < 30 && p[2] < 30)
            .count()
            > 100
    );
    click(&mut h, "Pause preview");
    at(&mut h, 30.0);
    assert_eq!(pixels(&h), first);
    text(&mut h, "Lifetime seconds", "invalid");
    click(&mut h, "Apply draft");
    assert_eq!(pixels(&h), first);
    assert_eq!(h.state().preview().snapshot(), Some(&accepted));
    assert_eq!(
        (h.state().panel().can_undo(), h.state().panel().can_redo()),
        before_history
    );
    click(&mut h, "Resume preview");
    assert_eq!(pixels(&h), first);
    at(&mut h, 30.2);
    let later = pixels(&h);
    save_target(&evidence, "preview-later.png", &later);
    assert_ne!(later, first);
    click(&mut h, "Pause preview");
    at(&mut h, 100.0);
    assert_eq!(pixels(&h), later);
    click(&mut h, "Restart preview");
    assert_eq!(pixels(&h), first);
    save_target(&evidence, "preview-restarted.png", &pixels(&h));
    click(&mut h, "Discard draft");
    text(&mut h, "Start R", "0");
    text(&mut h, "Start G", "1");
    click(&mut h, "Apply draft");
    assert_eq!(pixels(&h), first); // accepted edits mark stale, never live-replace
    assert_ne!(
        h.state().preview().snapshot(),
        Some(h.state().panel().document())
    );
    click(&mut h, "Preview accepted preset");
    let green = pixels(&h);
    save_target(&evidence, "preview-alternate.png", &green);
    assert_ne!(green, first);
    assert!(green
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[1] > 100 && p[0] < 30));
    for time in [100.25, 100.5, 100.75, 101.0, 101.25] {
        at(&mut h, time);
    }
    let empty = pixels(&h);
    save_target(&evidence, "preview-expired.png", &empty);
    assert!(!h.state().preview().playing());
    assert!(h.state().preview().draws().next().is_none());
    assert!(empty
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| pixel == &empty[..4]));
    // Exact independent empty production render target baseline on the same device.
    let gpu_state = state.as_ref().unwrap();
    let baseline = particle_preset_authoring::preview_gpu::PreviewGpu::new(gpu_state).unwrap();
    assert_eq!(empty, baseline.read_rgba8());
    save_target(
        &evidence,
        "preview-empty-baseline.png",
        &baseline.read_rgba8(),
    );
    let baseline_id = baseline.texture_id();
    drop(baseline);
    assert!(gpu_state.renderer.read().texture(&baseline_id).is_none());
    assert!(gpu_state.renderer.read().texture(&id).is_some());
    // Paint every event frame, including the exact close frame that still owns
    // its native texture, before the next update retires its registration.
    h.set_render_every_step(true);
    click(&mut h, "Close preview");
    h.render()
        .expect("close frame rendered before registration retirement");
    h.set_render_every_step(false);
    assert!(h.state().preview_gpu().is_none());
    assert!(gpu_state.renderer.read().texture(&id).is_none());
    click(&mut h, "Preview accepted preset");
    let reopened_id = h.state().preview_gpu().unwrap().texture_id();
    assert!(gpu_state.renderer.read().texture(&reopened_id).is_some());
    drop(h);
    assert!(gpu_state.renderer.read().texture(&reopened_id).is_none());
    std::fs::write(
        evidence.join("embedded-preview-acceptance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "production_eframe_app": true, "same_device_native_texture": true,
            "target_size": [320, 240], "accepted_burst": true, "different_color": true,
            "later_frame": true, "pause_freezes": true, "restart_exact": true,
            "dirty_draft_excluded_same_age_pixels": true, "accepted_edit_stale_until_refresh": true,
            "expiry_exact_empty_baseline": true, "full_window_texture_visible": true,
            "close_and_drop_unregister_texture": true, "native_os_window": false,
            "adapter": format!("{:?}", gpu_state.adapter.get_info())
        }))
        .unwrap(),
    )
    .unwrap();
}
