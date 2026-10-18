//! The production eframe app, driven through real widgets. Rendering is an
//! explicit mandatory software-GPU acceptance, never a skip-on-missing-adapter.
//! The saved map crosses CollectBindings and a real Bridge; this does not call
//! the private runtime Keys policy or create/inject a native OS window.
#![cfg(feature = "collect-input-map-editor")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "../src/collect_input_map_authoring.rs"]
mod collect_input_map_authoring;

use collect_input_map_authoring::App;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_bridge::{Bridge, BridgeConfig, InProc, PlayHost, PlayerSlot};
use orr_fp::{FPVec2, FP};
use orr_input::{ActionMap, Button, Key};
use orr_sample::{
    collect_app::input_actions::{default_map, CollectAction, CollectBindings},
    collect_game::{self as game, CollectDodgeV1, CollectInput},
};
use std::{collections::BTreeSet, fs, path::Path, sync::Arc, time::Duration};
use winit::keyboard::KeyCode;

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size([720.0, 650.0])
        .with_max_steps(120)
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(|_| App::default());
    settle(&mut h, "initial layout");
    h
}

fn settle(h: &mut Harness<'_, App>, context: &str) {
    h.try_run()
        .unwrap_or_else(|error| panic!("layout did not settle after {context}: {error}"));
}
fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    settle(h, label);
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "disabled {label}"
    );
    h.get_by_role_and_label(Role::Button, label).click();
    settle(h, label);
}
fn combo(h: &mut Harness<'_, App>, label: &str, choice: &str) {
    h.get_by_role_and_label(Role::ComboBox, label)
        .scroll_to_me();
    settle(h, label);
    h.get_by_role_and_label(Role::ComboBox, label).click();
    settle(h, label);
    h.get_by_role_and_label(Role::Button, choice).scroll_to_me();
    settle(h, choice);
    h.get_by_role_and_label(Role::Button, choice).click();
    settle(h, choice);
    assert_eq!(
        h.get_by_role_and_label(Role::ComboBox, label)
            .accesskit_node()
            .value()
            .as_deref(),
        Some(choice)
    );
}
fn set_action(h: &mut Harness<'_, App>, name: &str, binding: &str) {
    combo(h, "Action", name);
    click(h, "Clear action draft");
    click(h, "Add binding");
    combo(h, "Binding 1", binding);
    click(h, "Apply action");
    assert!(
        h.state().panel().message().starts_with("Action applied"),
        "{name}: {}",
        h.state().panel().message()
    );
}
fn author_all(h: &mut Harness<'_, App>) {
    for (action, binding) in [
        ("left", "Key J"),
        ("right", "Key L"),
        ("up", "Key I"),
        ("down", "Key K"),
        ("restart", "Key Enter"),
        ("quit", "Key Q"),
    ] {
        set_action(h, action, binding);
    }
}
fn path(h: &mut Harness<'_, App>, value: &Path) {
    h.get_by_role(Role::TextInput).scroll_to_me();
    settle(h, "path scroll");
    h.get_by_role(Role::TextInput).focus();
    settle(h, "path focus");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    settle(h, "select path");
    h.get_by_role(Role::TextInput)
        .type_text(value.to_str().unwrap());
    settle(h, "path input");
}
fn save(h: &mut Harness<'_, App>, file: &Path) {
    path(h, file);
    click(h, "Save As new file");
    assert!(
        h.state()
            .panel()
            .message()
            .starts_with("Saved accepted map."),
        "{}",
        h.state().panel().message()
    );
}
fn key(key: Key) -> Button {
    Button::Keyboard { key }
}

fn roundtrip(h: &mut Harness<'_, App>, file: &Path) {
    author_all(h);
    let expected = h.state().panel().document().clone();
    assert_ne!(expected, default_map());
    for _ in 0..6 {
        click(h, "Undo map edit");
    }
    assert_eq!(h.state().panel().document(), &default_map());
    assert!(!h.state().panel().can_undo());
    for _ in 0..6 {
        click(h, "Redo map edit");
    }
    assert_eq!(h.state().panel().document(), &expected);
    assert!(!h.state().panel().can_redo());
    save(h, file);
    assert_eq!(ActionMap::load_file(file).unwrap(), expected);
    let bytes = fs::read(file).unwrap();
    click(h, "Save As new file");
    assert!(!h.state().panel().message().starts_with("Saved"));
    assert_eq!(fs::read(file).unwrap(), bytes);
    click(h, "Use Collect defaults");
    click(h, "Open map (undoable)");
    assert_eq!(h.state().panel().document(), &expected);
    click(h, "Undo map edit");
    assert_eq!(h.state().panel().document(), &default_map());
    click(h, "Redo map edit");
    assert_eq!(h.state().panel().document(), &expected);
}

#[test]
fn six_action_widgets_save_reopen_and_exact_undo_redo() {
    let dir = tempfile::tempdir().unwrap();
    roundtrip(&mut harness(), &dir.path().join("collect-input.json"));
}

#[test]
fn conflicting_draft_blocks_navigation_and_save_preserves_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness();
    let original = h.state().panel().document().clone();
    combo(&mut h, "Action", "right");
    click(&mut h, "Clear action draft");
    click(&mut h, "Add binding");
    combo(&mut h, "Binding 1", "Key A");
    click(&mut h, "Apply action");
    assert!(h
        .state()
        .panel()
        .message()
        .contains("duplicate physical binding"));
    assert_eq!(h.state().panel().document(), &original);
    assert!(!h.state().panel().can_undo());
    assert!(h.state().panel().has_pending_draft());
    for label in [
        "Undo map edit",
        "Redo map edit",
        "Use Collect defaults",
        "Open map (undoable)",
    ] {
        assert!(
            h.get_by_role_and_label(Role::Button, label)
                .accesskit_node()
                .is_disabled(),
            "{label}"
        );
    }
    assert!(h
        .get_by_role_and_label(Role::ComboBox, "Action")
        .accesskit_node()
        .is_disabled());
    let file = dir.path().join("accepted-only.json");
    save(&mut h, &file);
    assert_eq!(ActionMap::load_file(file).unwrap(), original);
    assert_eq!(h.state().panel().draft(), &[key(Key::A)]);
    assert!(h.state().panel().has_pending_draft());
    click(&mut h, "Discard draft");
    assert!(!h.state().panel().has_pending_draft());
    assert_eq!(h.state().panel().document(), &original);
    combo(&mut h, "Action", "left");
    assert_eq!(h.state().panel().selected_action().name, "left");
}

#[test]
fn empty_system_actions_warn_and_eight_keyboard_bindings_are_editable() {
    let mut h = harness();
    for name in ["restart", "quit"] {
        combo(&mut h, "Action", name);
        click(&mut h, "Clear action draft");
        click(&mut h, "Apply action");
        assert!(h.state().panel().selected_action().bindings.is_empty());
    }
    h.get_by_label("Warning: restart is Unbound. The keyboard cannot restart Collect.");
    h.get_by_label("Warning: quit is Unbound. Use the normal window close control to exit.");
    combo(&mut h, "Action", "left");
    click(&mut h, "Clear action draft");
    for (index, binding) in [
        "Key B", "Key C", "Key E", "Key F", "Key G", "Key H", "Key I", "Key J",
    ]
    .into_iter()
    .enumerate()
    {
        click(&mut h, "Add binding");
        combo(&mut h, &format!("Binding {}", index + 1), binding);
    }
    assert_eq!(h.state().panel().draft().len(), 8);
    assert!(h
        .get_by_role_and_label(Role::Button, "Add binding")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Apply action");
    assert_eq!(h.state().panel().selected_action().bindings.len(), 8);
    CollectBindings::from_map(h.state().panel().document()).unwrap();
    click(&mut h, "Remove binding 8");
    assert_eq!(h.state().panel().draft().len(), 7);
    assert_eq!(h.state().panel().selected_action().bindings.len(), 8);
    click(&mut h, "Discard draft");
    assert_eq!(h.state().panel().draft().len(), 8);
}

// Adapter-only translation in this test, deliberately not a replacement runtime
// input policy. Private Keys owns focus/capture/restart-edge queues, tested in
// orr_sample's production-policy tests. Here each held sample is explicit.
fn adapted_input(bindings: &CollectBindings, pressed: &BTreeSet<KeyCode>) -> CollectInput {
    let held = |action| bindings.is_held(action, pressed);
    CollectInput {
        x: FP::from_int(
            i32::from(held(CollectAction::Right)) - i32::from(held(CollectAction::Left)),
        ),
        y: FP::from_int(i32::from(held(CollectAction::Up)) - i32::from(held(CollectAction::Down))),
        buttons: if held(CollectAction::Restart) {
            game::RESTART
        } else {
            0
        },
        ..Default::default()
    }
}
#[derive(Debug, PartialEq, Eq)]
struct Sample {
    input: CollectInput,
    tick: u64,
    checksum: u64,
    run: game::CollectRun,
}
fn trace(bindings: &CollectBindings, right: KeyCode, restart: KeyCode) -> Vec<Sample> {
    let level = game::CollectLevel::new(
        FPVec2::ZERO,
        vec![FPVec2::new(FP::from_int(20), FP::ZERO)],
        vec![],
        600,
    )
    .unwrap();
    let simulation = orr_sim::Simulation::<CollectDodgeV1>::new(level, 60, 42);
    let mut config = orr_bridge::PlayConfig::new(1, 42, 60);
    config.start_paused = false;
    let session =
        orr_bridge::PlaySession::<CollectDodgeV1>::from_frame(config, simulation.frame()).unwrap();
    let mut bridge = InProc::new(
        PlayHost::new(session, PlayerSlot(0)),
        BridgeConfig::default(),
    );
    let mut out = Vec::new();
    for n in 0..24 {
        let pressed = match n {
            0..=19 => BTreeSet::from([right]),
            20 => BTreeSet::from([restart]),
            _ => BTreeSet::new(),
        };
        let input = adapted_input(bindings, &pressed);
        bridge.set_input(PlayerSlot(0), input).unwrap();
        bridge.update(Duration::from_millis(17));
        let snapshot = bridge.poll_view().snapshot.unwrap();
        out.push(Sample {
            input,
            tick: snapshot.tick(),
            checksum: snapshot.predicted().checksum(),
            run: *snapshot.predicted().singleton::<game::CollectRun>(),
        });
    }
    assert_eq!(out[19].run.phase, game::WON);
    assert_eq!(out[19].run.score, 1);
    assert_eq!(out[20].run.phase, game::PLAYING);
    assert_eq!(out[20].run.score, 0);
    assert_eq!(out[20].run.elapsed_ticks, 0);
    assert!(out[23].run.elapsed_ticks > 0);
    out
}
fn verify_saved_adapter(file: &Path) -> Vec<Sample> {
    let before = fs::read(file).unwrap();
    let bindings = CollectBindings::load_file(file).unwrap();
    for (code, action, x, y) in [
        (KeyCode::KeyJ, CollectAction::Left, -1, 0),
        (KeyCode::KeyL, CollectAction::Right, 1, 0),
        (KeyCode::KeyI, CollectAction::Up, 0, 1),
        (KeyCode::KeyK, CollectAction::Down, 0, -1),
    ] {
        assert_eq!(bindings.action_for_key(code), Some(action));
        let input = adapted_input(&bindings, &BTreeSet::from([code]));
        assert_eq!((input.x, input.y), (FP::from_int(x), FP::from_int(y)));
    }
    assert_eq!(
        bindings.action_for_key(KeyCode::Enter),
        Some(CollectAction::Restart)
    );
    assert_eq!(
        bindings.action_for_key(KeyCode::KeyQ),
        Some(CollectAction::Quit)
    );
    for old in [
        KeyCode::KeyA,
        KeyCode::KeyD,
        KeyCode::KeyW,
        KeyCode::KeyS,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
        KeyCode::Space,
        KeyCode::Escape,
    ] {
        assert_eq!(bindings.action_for_key(old), None);
    }
    let mapped = trace(&bindings, KeyCode::KeyL, KeyCode::Enter);
    assert_eq!(
        mapped,
        trace(&CollectBindings::default(), KeyCode::KeyD, KeyCode::Space)
    );
    assert_eq!(
        mapped,
        trace(
            &CollectBindings::load_file(file).unwrap(),
            KeyCode::KeyL,
            KeyCode::Enter
        )
    );
    assert_eq!(fs::read(file).unwrap(), before);
    mapped
}
#[test]
fn saved_widget_map_drives_collect_loader_and_real_bridge_trace() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("authored.json");
    let mut h = harness();
    author_all(&mut h);
    save(&mut h, &file);
    verify_saved_adapter(&file);
}

#[test]
#[ignore = "mandatory software Vulkan GPU acceptance with retained full-window PNGs"]
fn collect_map_widgets_have_real_gpu_pixels() {
    let evidence = std::path::PathBuf::from(
        std::env::var_os("ORR_COLLECT_INPUT_MAP_EVIDENCE")
            .expect("retained external evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan && info.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "software Vulkan adapter required".into())
    }));
    let mut adapter = String::new();
    let mut h = Harness::builder()
        .with_size([720.0, 650.0])
        .with_max_steps(120)
        .wgpu_setup(setup.into())
        .build_eframe(|cc| {
            let info = cc
                .wgpu_render_state
                .as_ref()
                .expect("real eframe GPU renderer")
                .adapter
                .get_info();
            assert_eq!(info.device_type, wgpu::DeviceType::Cpu);
            assert_eq!(info.backend, wgpu::Backend::Vulkan);
            adapter = format!("{info:?}");
            App::default()
        });
    settle(&mut h, "GPU initial layout");
    let before = h.render().expect("mandatory baseline GPU readback");
    before
        .save(evidence.join("collect-input-map-default.png"))
        .unwrap();
    assert_eq!(before.dimensions(), (720, 650));
    let file = evidence.join("collect-input-map-authored.json");
    roundtrip(&mut h, &file);
    combo(&mut h, "Action", "right");
    let accepted = h.state().panel().document().clone();
    combo(&mut h, "Binding 1", "Key J");
    click(&mut h, "Apply action");
    assert!(h
        .state()
        .panel()
        .message()
        .contains("duplicate physical binding"));
    assert_eq!(h.state().panel().document(), &accepted);
    h.render()
        .expect("conflict GPU readback")
        .save(evidence.join("collect-input-map-conflict.png"))
        .unwrap();
    click(&mut h, "Discard draft");
    // Restore the top of the real production window before retaining the frame.
    h.get_by_label("Collect input map").scroll_to_me();
    settle(&mut h, "full window top");
    let after = h.render().expect("mandatory authored GPU readback");
    after
        .save(evidence.join("collect-input-map-authored.png"))
        .unwrap();
    assert_eq!(after.dimensions(), (720, 650));
    assert_ne!(before.as_raw(), after.as_raw());
    assert!(
        after
            .as_raw()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel != &after.as_raw()[..4]),
        "nonblank production GUI required"
    );
    let trace = verify_saved_adapter(&file);
    let trace_json: Vec<_> = trace.iter().map(|sample| serde_json::json!({
        "tick": sample.tick, "checksum": format!("{:016x}", sample.checksum),
        "phase": sample.run.phase, "score": sample.run.score, "elapsed_ticks": sample.run.elapsed_ticks,
    })).collect();
    fs::write(evidence.join("collect-input-map-acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema": 1, "production_eframe_app": true, "window_pixels": [720, 650], "adapter": adapter,
        "six_actions_authored": true, "conflicts_refused": true, "exact_undo_redo": true,
        "save_reopen_no_overwrite": true, "real_collect_bindings_loader": true,
        "real_bridge_movement_win_restart_trace": trace_json,
        "native_os_window": false, "physical_keyboard_events": false, "private_runtime_keys_policy": false,
    })).unwrap()).unwrap();
}
