//! Actual production eframe widgets; saved bytes are the runtime acceptance input.
#![cfg(feature = "particle-preset-editor")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "../src/particle_preset_authoring.rs"]
mod particle_preset_authoring;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_sample::particle_preset::{ParticlePreset, PresetBlend};
use particle_preset_authoring::App;
use std::{fs, path::Path, sync::Arc};
fn settle(h: &mut Harness<'_, App>) {
    h.try_run().expect("production widgets settle");
}
fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    settle(h);
    assert!(!h
        .get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled());
    h.get_by_role_and_label(Role::Button, label).click();
    settle(h);
}
fn text(h: &mut Harness<'_, App>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    settle(h);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    settle(h);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    settle(h);
}
fn blend(h: &mut Harness<'_, App>, choice: &str) {
    h.get_by_role_and_label(Role::ComboBox, "Blend")
        .scroll_to_me();
    settle(h);
    h.get_by_role_and_label(Role::ComboBox, "Blend").click();
    settle(h);
    click(h, choice);
}
fn expected() -> ParticlePreset {
    ParticlePreset {
        burst: 24,
        lifetime: 0.4,
        velocity: [0.0; 3],
        velocity_jitter: [0.0; 3],
        size_start: 1.2,
        size_end: 0.4,
        color_start: [0.1, 1.0, 0.3, 1.0],
        color_end: [0.1, 0.3, 1.0, 0.0],
        blend: PresetBlend::Alpha,
        ..ParticlePreset::default()
    }
}
fn roundtrip(h: &mut Harness<'_, App>, file: &Path) {
    let default = h.state().panel().document().clone();
    for (label, value) in [
        ("Burst", "24"),
        ("Lifetime seconds", "0.4"),
        ("Velocity X", "0"),
        ("Velocity Y", "0"),
        ("Velocity Z", "0"),
        ("Jitter X", "0"),
        ("Jitter Y", "0"),
        ("Jitter Z", "0"),
        ("Start size", "1.2"),
        ("End size", "0.4"),
        ("Start R", "0.1"),
        ("Start G", "1"),
        ("Start B", "0.3"),
        ("Start A", "1"),
        ("End R", "0.1"),
        ("End G", "0.3"),
        ("End B", "1"),
        ("End A", "0"),
    ] {
        text(h, label, value);
    }
    blend(h, "Additive");
    blend(h, "Alpha");
    assert_eq!(h.state().panel().document(), &default);
    for label in [
        "Undo preset edit",
        "Redo preset edit",
        "Use defaults",
        "Open preset (undoable)",
    ] {
        assert!(h
            .get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled());
    }
    click(h, "Apply draft");
    assert_eq!(h.state().panel().document(), &expected());
    click(h, "Undo preset edit");
    assert_eq!(h.state().panel().document(), &default);
    click(h, "Redo preset edit");
    assert_eq!(h.state().panel().document(), &expected());
    text(h, "Lifetime seconds", "invalid");
    click(h, "Apply draft");
    assert_eq!(h.state().panel().document(), &expected());
    assert!(h.state().panel().message().contains("Invalid"));
    text(h, "Preset file path", file.to_str().unwrap());
    click(h, "Save As new file");
    assert!(h.state().panel().has_pending_draft());
    assert_eq!(ParticlePreset::open(file).unwrap(), expected());
    let bytes = fs::read(file).unwrap();
    assert_eq!(bytes, expected().to_bytes().unwrap());
    click(h, "Save As new file");
    assert!(!h.state().panel().message().starts_with("Saved"));
    assert_eq!(fs::read(file).unwrap(), bytes);
    click(h, "Discard draft");
    assert!(!h.state().panel().has_pending_draft());
    click(h, "Use defaults");
    click(h, "Open preset (undoable)");
    assert_eq!(h.state().panel().document(), &expected());
    let bad = file.with_extension("invalid.json");
    fs::write(&bad, b"{").unwrap();
    text(h, "Preset file path", bad.to_str().unwrap());
    click(h, "Open preset (undoable)");
    assert_eq!(h.state().panel().document(), &expected());
    fs::remove_file(bad).unwrap();
    text(h, "Preset file path", file.to_str().unwrap());
    click(h, "Open preset (undoable)");
    assert_eq!(h.state().panel().message(), "Opened validated preset.");
    assert_eq!(h.state().panel().document(), &expected());
    assert_eq!(fs::read(file).unwrap(), bytes);
}
#[test]
fn widget_edits_apply_discard_history_save_reopen_exact_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::builder()
        .with_size([760.0, 820.0])
        .with_max_steps(120)
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(|_| App::default());
    settle(&mut h);
    roundtrip(&mut h, &dir.path().join("authored.json"));
}
#[test]
#[ignore = "mandatory software Vulkan GUI acceptance with retained authored file"]
fn particle_preset_widgets_have_real_gpu_pixels() {
    let evidence = std::path::PathBuf::from(
        std::env::var_os("ORR_PARTICLE_PRESET_EVIDENCE").expect("evidence dir"),
    );
    let file = std::path::PathBuf::from(
        std::env::var_os("ORR_PARTICLE_PRESET_ACCEPTANCE_FILE").expect("new authored file"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    assert!(file.is_absolute() && !file.exists());
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|a| {
                let i = a.get_info();
                i.backend == wgpu::Backend::Vulkan && i.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "software Vulkan required".into())
    }));
    let mut adapter = String::new();
    let mut h = Harness::builder()
        .with_size([760.0, 820.0])
        .with_max_steps(120)
        .wgpu_setup(setup.into())
        .build_eframe(|cc| {
            let info = cc
                .wgpu_render_state
                .as_ref()
                .expect("GPU renderer")
                .adapter
                .get_info();
            assert_eq!(info.device_type, wgpu::DeviceType::Cpu);
            adapter = format!("{info:?}");
            App::default()
        });
    settle(&mut h);
    let before = h.render().expect("GPU readback");
    before
        .save(evidence.join("particle-preset-default.png"))
        .unwrap();
    roundtrip(&mut h, &file);
    h.get_by_label("Particle preset authoring").scroll_to_me();
    settle(&mut h);
    let after = h.render().expect("GPU authored readback");
    after
        .save(evidence.join("particle-preset-authored.png"))
        .unwrap();
    assert_eq!(after.dimensions(), (760, 820));
    assert_ne!(before.as_raw(), after.as_raw());
    assert!(after
        .as_raw()
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p != &after.as_raw()[..4]));
    fs::write(evidence.join("particle-preset-ui-acceptance.json"),serde_json::to_vec_pretty(&serde_json::json!({
        "production_eframe_app":true,"adapter":adapter,"authored_file":file,"all_numeric_fields_widget_edited":true,
        "alpha_additive_widget_edited":true,"invalid_apply_atomic":true,"dirty_save_accepted_only":true,
        "discard_undo_redo_open":true,"no_overwrite":true,"exact_saved_bytes":true,
        "native_os_window":false,"embedded_runtime_preview":false
    })).unwrap()).unwrap();
}
