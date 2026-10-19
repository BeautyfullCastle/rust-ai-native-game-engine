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
fn mode(h: &mut Harness<'_, App>, choice: &str) {
    h.get_by_role_and_label(Role::ComboBox, "Emission mode")
        .scroll_to_me();
    h.step();
    h.get_by_role_and_label(Role::ComboBox, "Emission mode")
        .click();
    h.step();
    click(h, choice);
}
fn configure(h: &mut Harness<'_, App>) {
    mode(h, "Timed (schema 2)");
    assert_eq!(h.state().panel().document().schema, 1);
    for (label, value) in [
        ("Burst", "0"),
        ("Rate per second", "4"),
        ("Emitter duration seconds", "1"),
        ("Lifetime seconds", "0.125"),
        ("Velocity X", "0"),
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
        ("End A", "1"),
    ] {
        text(h, label, value);
    }
    h.get_by_role_and_label(Role::ComboBox, "Blend").click();
    h.step();
    click(h, "Alpha");
    click(h, "Apply draft");
    let p = h.state().panel().document();
    assert_eq!(
        (p.schema, p.burst, p.rate, p.duration, p.lifetime),
        (2, 0, 4.0, 1.0, 0.125)
    );
}
fn author_and_save(h: &mut Harness<'_, App>, file: &std::path::Path) {
    configure(h);
    let accepted = h.state().panel().document().clone();
    click(h, "Undo preset edit");
    assert_eq!(h.state().panel().document().schema, 1);
    click(h, "Redo preset edit");
    assert_eq!(h.state().panel().document(), &accepted);
    mode(h, "Burst (schema 1)");
    click(h, "Apply draft");
    assert_eq!(h.state().panel().document(), &accepted);
    assert!(h.state().panel().has_pending_draft());
    click(h, "Discard draft");
    text(h, "Rate per second", "invalid");
    click(h, "Apply draft");
    assert_eq!(h.state().panel().document(), &accepted);
    text(h, "Preset file path", file.to_str().unwrap());
    click(h, "Save As new file");
    assert!(h.state().panel().has_pending_draft());
    assert_eq!(std::fs::read(file).unwrap(), accepted.to_bytes().unwrap());
    click(h, "Save As new file");
    assert!(!h.state().panel().message().starts_with("Saved"));
    click(h, "Discard draft");
    click(h, "Use defaults");
    assert_eq!(h.state().panel().document().schema, 1);
    click(h, "Open preset (undoable)");
    assert_eq!(h.state().panel().document(), &accepted);
    let bad = file.with_extension("invalid.json");
    std::fs::write(&bad, b"{").unwrap();
    text(h, "Preset file path", bad.to_str().unwrap());
    click(h, "Open preset (undoable)");
    assert_eq!(h.state().panel().document(), &accepted);
    std::fs::remove_file(bad).unwrap();
}
fn assert_repainting(h: &Harness<'_, App>) {
    assert_eq!(
        h.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay,
        std::time::Duration::ZERO,
        "live emitter must request another frame even with no particle draws"
    );
}
fn start_empty_and_check_repaint(h: &mut Harness<'_, App>) {
    click(h, "Preview accepted preset");
    // Consume incidental widget/pointer repaints without advancing presentation time.
    // A broken nonempty-draw-only scheduler settles here instead of continuing.
    for _ in 0..20 {
        h.step();
    }
    assert!(draws(h).is_empty());
    assert!(h.state().preview().playing());
    assert_repainting(h);
    click(h, "Pause preview");
    for _ in 0..20 {
        h.step();
    }
    assert_ne!(
        h.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay,
        std::time::Duration::ZERO,
        "paused negative control must settle"
    );
    click(h, "Resume preview");
    for _ in 0..20 {
        h.step();
    }
    assert!(draws(h).is_empty());
    assert_repainting(h);
}
#[test]
fn timed_widgets_save_versioned_draft_and_repaint_empty_emitter() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::builder()
        .with_size([1100.0, 1100.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(|cc| {
            cc.egui_ctx
                .all_styles_mut(|style| style.animation_time = 0.0);
            App::new(None)
        });
    at(&mut h, 10.0);
    author_and_save(&mut h, &dir.path().join("timed.json"));
    start_empty_and_check_repaint(&mut h);
    at(&mut h, 10.125);
    assert!(draws(&h).is_empty());
    assert_repainting(&h);
    at(&mut h, 10.25);
    assert_eq!(draws(&h).len(), 1);
    at(&mut h, 10.375);
    assert!(draws(&h).is_empty());
    for _ in 0..20 {
        h.step();
    }
    assert_repainting(&h);
    click(&mut h, "Pause preview");
    at(&mut h, 30.0);
    click(&mut h, "Resume preview");
    assert!(h.state().preview().playing());
    at(&mut h, 30.125);
    assert_eq!(draws(&h).len(), 1);
    let snapshot = h.state().preview().snapshot().cloned();
    text(&mut h, "Rate per second", "8");
    click(&mut h, "Apply draft");
    assert_eq!(h.state().preview().snapshot(), snapshot.as_ref());
    click(&mut h, "Restart preview");
    assert!(draws(&h).is_empty());
    assert_eq!(h.state().preview().snapshot(), snapshot.as_ref());
}
#[test]
#[ignore = "mandatory software Vulkan timed emission acceptance"]
fn timed_preview_widgets_have_real_gpu_pixels() {
    let evidence = std::path::PathBuf::from(
        std::env::var_os("ORR_PARTICLE_TIMED_EVIDENCE").expect("evidence directory"),
    );
    let file = std::path::PathBuf::from(
        std::env::var_os("ORR_PARTICLE_TIMED_ACCEPTANCE_FILE").expect("new authored file"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    assert!(file.is_absolute() && !file.exists());
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
        .with_size([1100.0, 1100.0])
        .wgpu_setup(setup.into())
        .build_eframe(|cc| {
            cc.egui_ctx
                .all_styles_mut(|style| style.animation_time = 0.0);
            state = cc.wgpu_render_state.clone();
            App::new(cc.wgpu_render_state.clone())
        });
    at(&mut h, 10.0);
    author_and_save(&mut h, &file);
    start_empty_and_check_repaint(&mut h);
    let initial = pixels(&h);
    let baseline =
        particle_preset_authoring::preview_gpu::PreviewGpu::new(state.as_ref().unwrap()).unwrap();
    assert_eq!(initial, baseline.read_rgba8());
    save_target(&evidence, "timed-initial-empty.png", &initial);
    at(&mut h, 10.125);
    assert_eq!(pixels(&h), initial);
    assert_repainting(&h);
    at(&mut h, 10.25);
    let emitted = pixels(&h);
    assert_ne!(emitted, initial);
    assert!(emitted
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[0] > 100 && p[1] < 30));
    save_target(&evidence, "timed-first-emission.png", &emitted);
    h.render()
        .unwrap()
        .save(evidence.join("timed-preview-full-window.png"))
        .unwrap();
    at(&mut h, 10.375);
    assert_eq!(pixels(&h), initial);
    for _ in 0..20 {
        h.step();
    }
    assert_repainting(&h);
    save_target(&evidence, "timed-live-gap.png", &pixels(&h));
    at(&mut h, 10.5);
    assert_eq!(pixels(&h), emitted);
    at(&mut h, 10.75);
    at(&mut h, 11.0);
    assert!(h.state().preview().playing());
    assert_eq!(draws(&h).len(), 1);
    at(&mut h, 11.125);
    assert!(!h.state().preview().playing());
    assert_eq!(pixels(&h), initial);
    save_target(&evidence, "timed-expired.png", &pixels(&h));
    assert_eq!(
        std::fs::read(&file).unwrap(),
        h.state().panel().document().to_bytes().unwrap()
    );
    std::fs::write(evidence.join("timed-preview-acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "production_eframe_app": true, "authored_file": file,
        "schema": 2, "burst": 0, "rate": 4, "duration": 1, "lifetime": 0.125,
        "empty_initial_repaint_requested": true, "empty_gap_repaint_requested": true,
        "gpu_empty_emission_expiry": true, "dirty_save_accepted_only": true,
        "undo_redo_defaults_invalid_open": true, "downgrade_rejected": true,
        "native_os_window": false, "adapter": format!("{:?}", state.as_ref().unwrap().adapter.get_info())
    })).unwrap()).unwrap();
}
