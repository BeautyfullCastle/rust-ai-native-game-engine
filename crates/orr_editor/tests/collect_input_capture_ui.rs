//! Mandatory production authoring pixels and physical egui event acceptance.
//! This is software Vulkan, not native OS keyboard/window injection.
#![cfg(feature = "collect-input-map-editor")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "../src/collect_input_map_authoring.rs"]
mod collect_input_map_authoring;
use collect_input_map_authoring::App;
use egui::{accesskit::Role, Key};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_input::{ActionMap, Button, Key as MapKey};
use std::{fs, path::Path, sync::Arc};

fn settle(h: &mut Harness<'_, App>) {
    h.try_run().expect("capture UI must settle");
}
fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    settle(h);
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "disabled {label}"
    );
    h.get_by_role_and_label(Role::Button, label).click();
    settle(h);
}
fn combo(h: &mut Harness<'_, App>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::ComboBox, label)
        .scroll_to_me();
    settle(h);
    h.get_by_role_and_label(Role::ComboBox, label).click();
    settle(h);
    click(h, value);
}
fn event(
    h: &mut Harness<'_, App>,
    physical: Option<Key>,
    logical: Key,
    pressed: bool,
    repeat: bool,
) {
    h.input_mut().events.push(egui::Event::Key {
        key: logical,
        physical_key: physical,
        pressed,
        repeat,
        modifiers: egui::Modifiers::NONE,
    });
}
fn tap(h: &mut Harness<'_, App>, physical: Option<Key>, logical: Key) {
    // Both edges in one RawInput batch; Harness::event would queue separate frames.
    event(h, physical, logical, true, false);
    event(h, physical, logical, false, false);
    settle(h);
}
fn capture(h: &mut Harness<'_, App>, row: usize, physical: Key, expected: MapKey) {
    let accepted = h.state().panel().document().clone();
    click(h, &format!("Capture binding {row}"));
    assert!(h.state().panel().capture_is_listening());
    tap(h, Some(physical), Key::F1);
    assert!(!h.state().panel().capture_is_listening());
    assert_eq!(
        h.state().panel().draft()[row - 1],
        Button::Keyboard { key: expected }
    );
    assert_eq!(
        h.state().panel().document(),
        &accepted,
        "capture is draft-only"
    );
}
fn save(h: &mut Harness<'_, App>, path: &Path) {
    h.get_by_role(Role::TextInput).scroll_to_me();
    settle(h);
    h.get_by_role(Role::TextInput).focus();
    settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    settle(h);
    h.get_by_role(Role::TextInput)
        .type_text(path.to_str().unwrap());
    settle(h);
    click(h, "Save As new file");
    assert!(h
        .state()
        .panel()
        .message()
        .starts_with("Saved accepted map."));
}
fn frame(
    h: &mut Harness<'_, App>,
    dir: &Path,
    name: &str,
    labels: &[&str],
    semantic: &str,
) -> serde_json::Value {
    let mut positions = Vec::new();
    for label in labels {
        let rect = h.get_by_label(label).rect();
        assert!(
            rect.min.x >= 0.0 && rect.max.x <= 720.0 && rect.min.y >= 0.0 && rect.max.y <= 650.0,
            "{label} clipped: {rect:?}"
        );
        assert!(
            rect.width() >= 5.0 && rect.height() >= 8.0,
            "{label} unreadable bounds"
        );
        positions.push(serde_json::json!({"label": label, "rect": [rect.min.x, rect.min.y, rect.max.x, rect.max.y]}));
    }
    let pixels = h.render().expect("required software Vulkan readback");
    assert_eq!(pixels.dimensions(), (720, 650));
    let filename = format!("{name}.png");
    pixels.save(dir.join(&filename)).unwrap();
    serde_json::json!({"filename": filename, "widgets": positions, "proof": semantic})
}

#[test]
#[ignore = "mandatory software Vulkan capture acceptance with retained PNGs"]
fn collect_capture_widgets_have_real_gpu_pixels() {
    let dir = std::path::PathBuf::from(
        std::env::var_os("ORR_COLLECT_INPUT_CAPTURE_EVIDENCE")
            .expect("retained evidence directory required"),
    );
    assert!(dir.is_absolute() && dir.is_dir());
    assert!(
        !dir.join("collect-input-capture-acceptance.json").exists(),
        "preserve previous evidence"
    );
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|a| {
                let i = a.get_info();
                i.backend == wgpu::Backend::Vulkan
                    && i.device_type == wgpu::DeviceType::Cpu
                    && i.name.to_lowercase().contains("llvmpipe")
            })
            .cloned()
            .ok_or_else(|| "mandatory llvmpipe software Vulkan unavailable".into())
    }));
    let mut adapter = serde_json::Value::Null;
    let mut h = Harness::builder().with_size([720.0,650.0]).with_max_steps(120).wgpu_setup(setup.into()).build_eframe(|cc| {
        let info = cc.wgpu_render_state.as_ref().unwrap().adapter.get_info();
        assert_eq!(info.backend, wgpu::Backend::Vulkan); assert_eq!(info.device_type, wgpu::DeviceType::Cpu);
        adapter = serde_json::json!({"name": info.name, "backend": "Vulkan", "device_type": "Cpu", "driver": info.driver, "driver_info": info.driver_info}); App::default()
    });
    settle(&mut h);
    let mut frames = vec![frame(
        &mut h,
        &dir,
        "capture-initial",
        &["Capture binding 1", "Binding 1"],
        "Capture and dropdown visible at 720x650",
    )];
    let defaults = h.state().panel().document().clone();
    click(&mut h, "Capture binding 1");
    frames.push(frame(
        &mut h,
        &dir,
        "capture-listening",
        &["Capture physical key", "Cancel capture"],
        "Production modal listening; accepted map unchanged",
    ));
    let draft = h.state().panel().draft().to_vec();
    click(&mut h, "Cancel capture");
    assert_eq!(h.state().panel().draft(), draft);
    assert_eq!(h.state().panel().document(), &defaults);
    click(&mut h, "Capture binding 1");
    tap(&mut h, Some(Key::Escape), Key::F1);
    assert!(!h.state().panel().capture_is_listening());
    assert_eq!(h.state().panel().draft(), draft);
    // Missing physical identity must never silently fall back to a logical letter.
    click(&mut h, "Capture binding 1");
    tap(&mut h, None, Key::J);
    assert!(h.state().panel().capture_is_listening());
    assert_eq!(h.state().panel().draft(), draft);
    frames.push(frame(&mut h, &dir, "capture-physical-unavailable", &["Capture physical key", "Cancel capture"], "physical_key=None refused; previous draft retained; dropdown fallback available after Cancel"));
    click(&mut h, "Cancel capture");
    combo(&mut h, "Binding 1", "Key J");
    assert_eq!(
        h.state().panel().draft()[0],
        Button::Keyboard { key: MapKey::J }
    );
    click(&mut h, "Discard draft");
    // Repeat-only and an already-held activation key cannot be accepted.
    event(&mut h, Some(Key::J), Key::F1, true, false);
    settle(&mut h);
    click(&mut h, "Capture binding 1");
    event(&mut h, Some(Key::J), Key::F1, true, true);
    settle(&mut h);
    assert!(h.state().panel().capture_is_listening());
    assert_eq!(h.state().panel().draft(), draft);
    event(&mut h, Some(Key::J), Key::F1, false, false);
    settle(&mut h);
    tap(&mut h, Some(Key::J), Key::F1);
    assert!(!h.state().panel().capture_is_listening());
    click(&mut h, "Discard draft");
    click(&mut h, "Capture binding 1");
    h.input_mut().focused = false;
    h.input_mut().events.push(egui::Event::WindowFocused(false));
    settle(&mut h);
    assert!(!h.state().panel().capture_is_listening());
    assert_eq!(h.state().panel().draft(), draft);
    h.input_mut().focused = true;
    h.input_mut().events.push(egui::Event::WindowFocused(true));
    settle(&mut h);
    // Modal text/paste never edits the file path; logical Enter cannot activate Apply.
    let path_before = h.get_by_role(Role::TextInput).accesskit_node().value();
    click(&mut h, "Capture binding 1");
    h.input_mut()
        .events
        .push(egui::Event::Text("DO_NOT_LEAK".into()));
    h.input_mut()
        .events
        .push(egui::Event::Paste("DO_NOT_PASTE".into()));
    event(&mut h, Some(Key::Tab), Key::Enter, true, false);
    event(&mut h, Some(Key::Tab), Key::Enter, false, false);
    settle(&mut h);
    assert_eq!(
        h.state().panel().draft()[0],
        Button::Keyboard { key: MapKey::Tab }
    );
    assert_eq!(h.state().panel().document(), &defaults);
    assert_eq!(
        h.get_by_role(Role::TextInput).accesskit_node().value(),
        path_before
    );
    click(&mut h, "Discard draft");
    // Six accepted rows are authored exclusively through physical capture.
    for (action, physical, expected) in [
        ("left", Key::J, MapKey::J),
        ("right", Key::L, MapKey::L),
        ("up", Key::I, MapKey::I),
        ("down", Key::K, MapKey::K),
        ("restart", Key::Enter, MapKey::Enter),
        ("quit", Key::Q, MapKey::Q),
    ] {
        combo(&mut h, "Action", action);
        click(&mut h, "Clear action draft");
        click(&mut h, "Add binding");
        capture(&mut h, 1, physical, expected);
        click(&mut h, "Apply action");
        assert!(h.state().panel().message().starts_with("Action applied"));
    }
    let accepted = h.state().panel().document().clone();
    for _ in 0..6 {
        click(&mut h, "Undo map edit");
    }
    assert_eq!(h.state().panel().document(), &defaults);
    for _ in 0..6 {
        click(&mut h, "Redo map edit");
    }
    assert_eq!(h.state().panel().document(), &accepted);
    combo(&mut h, "Action", "right");
    capture(&mut h, 1, Key::J, MapKey::J);
    assert!(h
        .state()
        .panel()
        .validate_draft()
        .unwrap_err()
        .contains("duplicate physical binding"));
    h.get_by_label_contains("duplicate physical binding")
        .scroll_to_me();
    settle(&mut h);
    let conflict = format!(
        "Draft error: {}",
        h.state().panel().validate_draft().unwrap_err()
    );
    frames.push(frame(
        &mut h,
        &dir,
        "capture-live-conflict",
        &[&conflict, "Apply action"],
        "Duplicate physical binding diagnosed before Apply; accepted map and history unchanged",
    ));
    assert_eq!(h.state().panel().document(), &accepted);
    let file = dir.join("capture-accepted-only.json");
    save(&mut h, &file);
    assert_eq!(ActionMap::load_file(&file).unwrap(), accepted);
    assert!(h.state().panel().has_pending_draft());
    click(&mut h, "Discard draft");
    // Alias keys and eight rows exercise scrolling, row identity and readable controls.
    combo(&mut h, "Action", "left");
    click(&mut h, "Clear action draft");
    for (index, (physical, expected)) in [
        (Key::A, MapKey::A),
        (Key::ArrowLeft, MapKey::ArrowLeft),
        (Key::ArrowRight, MapKey::ArrowRight),
        (Key::ArrowUp, MapKey::ArrowUp),
        (Key::ArrowDown, MapKey::ArrowDown),
        (Key::Space, MapKey::Space),
        (Key::Tab, MapKey::Tab),
        (Key::B, MapKey::B),
    ]
    .into_iter()
    .enumerate()
    {
        click(&mut h, "Add binding");
        capture(&mut h, index + 1, physical, expected);
    }
    assert_eq!(h.state().panel().draft().len(), 8);
    h.get_by_label("Capture binding 8").scroll_to_me();
    settle(&mut h);
    frames.push(frame(
        &mut h,
        &dir,
        "capture-eight-rows",
        &["Binding 8", "Capture binding 8", "Remove binding 8"],
        "Eighth binding scrolled into view with bounded readable row controls",
    ));
    assert!(h
        .get_by_role_and_label(Role::Button, "Add binding")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Discard draft");
    combo(&mut h, "Action", "left");
    combo(&mut h, "Binding 1", "Key Escape");
    click(&mut h, "Apply action");
    assert!(h.state().panel().message().starts_with("Action applied"));
    assert_eq!(
        h.state().panel().selected_action().bindings,
        vec![Button::Keyboard {
            key: MapKey::Escape
        }]
    );
    h.get_by_label_contains("Compatibility warning:")
        .scroll_to_me();
    settle(&mut h);
    frames.push(frame(
        &mut h,
        &dir,
        "capture-escape-policy",
        &["Compatibility warning: Escape on movement or restart is valid for standalone Collect, but cannot be loaded by the main editor, where Escape releases controls.", "Apply action"],
        "Dropdown Escape accepted for standalone; main Editor compatibility warning rendered",
    ));
    fs::write(dir.join("collect-input-capture-acceptance.json"),serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1,"production_eframe_app":true,"physical_egui_events":true,"native_os_keyboard":false,"window_pixels":[720,650],"adapter_details":adapter,"frames":frames,
        "six_physical_actions":true,"logical_keys_differ":true,"capture_draft_only":true,"live_conflict_before_apply":true,"cancel_escape_held_repeat_focus_loss":true,"modal_text_paste_tab_enter_isolation":true,"physical_none_refused_dropdown_fallback":true,"eight_rows_scrolled":true,"accepted_only_save":true,"exact_undo_redo":true,"standalone_escape_dropdown":true
    })).unwrap()).unwrap();
}
