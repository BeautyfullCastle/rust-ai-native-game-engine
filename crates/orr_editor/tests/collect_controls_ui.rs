//! Real standalone authoring and main EditorApp widgets, mandatory llvmpipe readback.
//! Injected egui physical-key events are not native OS keyboard/window evidence.
#![cfg(all(feature = "collect-ui", feature = "collect-input-map-editor"))]
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
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    editor::input::Phase,
    Editor, EditorApp, HostSpec, Mode,
};
use orr_sample::{
    collect_game::{CollectActor, CollectRun, PLAYER, PLAYING, WON},
    collect_project::PreparedProject,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

fn software_vulkan() -> egui_wgpu::WgpuSetup {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan
                    && info.device_type == wgpu::DeviceType::Cpu
                    && info.name.to_lowercase().contains("llvmpipe")
            })
            .cloned()
            .ok_or_else(|| "mandatory llvmpipe software Vulkan adapter unavailable".into())
    }));
    setup.into()
}
fn author_settle(h: &mut Harness<'_, App>, context: &str) {
    h.try_run()
        .unwrap_or_else(|error| panic!("layout did not settle after {context}: {error}"));
}
fn author_click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    author_settle(h, label);
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "disabled {label}"
    );
    h.get_by_role_and_label(Role::Button, label).click();
    author_settle(h, label);
}
fn combo(h: &mut Harness<'_, App>, label: &str, choice: &str) {
    h.get_by_role_and_label(Role::ComboBox, label)
        .scroll_to_me();
    author_settle(h, label);
    h.get_by_role_and_label(Role::ComboBox, label).click();
    author_settle(h, label);
    h.get_by_role_and_label(Role::Button, choice).scroll_to_me();
    author_settle(h, choice);
    h.get_by_role_and_label(Role::Button, choice).click();
    author_settle(h, choice);
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
    author_click(h, "Clear action draft");
    author_click(h, "Add binding");
    combo(h, "Binding 1", binding);
    author_click(h, "Apply action");
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
    author_settle(h, "path scroll");
    h.get_by_role(Role::TextInput).focus();
    author_settle(h, "path focus");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    author_settle(h, "select path");
    h.get_by_role(Role::TextInput)
        .type_text(value.to_str().unwrap());
    author_settle(h, "path input");
}
fn save(h: &mut Harness<'_, App>, file: &Path) {
    path(h, file);
    author_click(h, "Save As new file");
    assert!(
        h.state()
            .panel()
            .message()
            .starts_with("Saved accepted map."),
        "{}",
        h.state().panel().message()
    );
}
fn author_map(file: &Path, evidence: &Path) {
    let mut h = Harness::builder()
        .with_size([720.0, 650.0])
        .with_max_steps(120)
        .wgpu_setup(software_vulkan())
        .build_eframe(|_| App::default());
    author_settle(&mut h, "initial");
    author_all(&mut h);
    save(&mut h, file);
    let accepted = h.state().panel().document().clone();
    author_click(&mut h, "Use Collect defaults");
    author_click(&mut h, "Open map (undoable)");
    assert_eq!(h.state().panel().document(), &accepted);
    h.get_by_label("Collect input map").scroll_to_me();
    author_settle(&mut h, "top");
    h.render()
        .unwrap()
        .save(evidence.join("collect-controls-authoring.png"))
        .unwrap();
}
fn open(root: &Path, evidence: &Path) -> Harness<'static, EditorApp> {
    let evidence = evidence.to_owned();
    let p = PreparedProject::open(root).unwrap();
    let mut editor = Editor::start(&HostSpec::PreparedCollect {
        scene: p.path().into(),
        text: p.scene().text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    Harness::builder()
        .with_size([1600.0, 1000.0])
        .wgpu_setup(software_vulkan())
        .build_eframe(move |cc| {
            let info = cc.wgpu_render_state.as_ref().unwrap().adapter.get_info();
            assert_eq!(info.device_type, wgpu::DeviceType::Cpu);
            assert_eq!(info.backend, wgpu::Backend::Vulkan);
            eprintln!("main EditorApp GPU: {info:?}");
            fs::write(
                evidence.join("collect-controls-adapter.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "name": info.name, "backend": format!("{:?}", info.backend),
                    "device_type": format!("{:?}", info.device_type),
                    "driver": info.driver, "driver_info": info.driver_info,
                }))
                .unwrap(),
            )
            .unwrap();
            EditorApp::new(editor, cc.wgpu_render_state.clone())
        })
}
fn wait(h: &mut Harness<'_, EditorApp>, reason: &str, ready: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        h.run_steps(1);
        assert!(
            h.output().viewport_output.values().all(|output| output
                .commands
                .iter()
                .all(|command| !matches!(command, egui::ViewportCommand::Close))),
            "mapped Quit must never request application exit"
        );
        if ready(&h.state().editor) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{reason}: {:?}",
            h.state().editor.status()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    h.run_steps(3);
}
fn physical(h: &mut Harness<'_, EditorApp>, physical: Key, logical: Key, pressed: bool) {
    h.event(egui::Event::Key {
        key: logical,
        physical_key: Some(physical),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
}
fn tap(h: &mut Harness<'_, EditorApp>, key: Key) {
    // One RawInput event batch: neither egui nor the simulation sees a held GUI frame.
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
}
fn ticks(h: &mut Harness<'_, EditorApp>, n: u64) {
    let tick = h.state().editor.snapshot().unwrap().tick();
    wait(h, "simulation advances", |e| {
        e.snapshot().unwrap().tick() >= tick + n
    });
}
fn run(h: &Harness<'_, EditorApp>) -> CollectRun {
    *h.state()
        .editor
        .snapshot()
        .unwrap()
        .verified()
        .expect("confirmed authoritative frame")
        .singleton::<CollectRun>()
}
fn player(h: &Harness<'_, EditorApp>) -> orr_fp::FPVec2 {
    h.state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .dense::<CollectActor>()
        .1
        .iter()
        .find(|actor| actor.kind == PLAYER)
        .unwrap()
        .position
}
fn load(h: &mut Harness<'_, EditorApp>, file: &Path) {
    h.get_by_role_and_label(Role::TextInput, "Collect controls")
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "Collect controls")
        .type_text(file.to_str().unwrap());
    h.run_steps(2);
    click(h, "Load controls");
}
fn play(h: &mut Harness<'_, EditorApp>) {
    click(h, LBL_PLAY);
    wait(h, "Play", |e| e.can_take_control());
    click(h, "Take control");
    wait(h, "Take control", |e| e.input_phase() == Phase::Active);
    h.get_by(|n| {
        n.role() == Role::Label
            && n.value()
                .or_else(|| n.label())
                .is_some_and(|s| s.contains("Right L") && s.contains("Restart Enter"))
    });
    assert!(h
        .get_by_role_and_label(Role::Button, "Load controls")
        .accesskit_node()
        .is_disabled());
}
fn stop(h: &mut Harness<'_, EditorApp>) {
    click(h, LBL_STOP);
    wait(h, "Stop", |e| {
        e.mode() == Mode::Edit && e.input_phase() == Phase::Off
    });
}
fn capture(h: &mut Harness<'_, EditorApp>, evidence: &Path, name: &str) -> Vec<u8> {
    h.run_steps(2);
    assert!(h.state().has_gpu());
    assert_eq!(h.ctx.pixels_per_point(), 1.0);
    let image = h.render().expect("mandatory real GPU readback");
    assert_eq!(image.dimensions(), (1600, 1000));
    image.save(evidence.join(format!("{name}.png"))).unwrap();
    let snapshot = h.state().editor.snapshot().unwrap();
    let verified = snapshot
        .verified()
        .expect("confirmed authoritative capture");
    assert_eq!(verified.tick(), snapshot.tick());
    assert_eq!(verified.checksum(), snapshot.predicted().checksum());
    let active_actors = verified
        .dense::<CollectActor>()
        .1
        .iter()
        .filter(|actor| actor.active != 0)
        .count();
    assert_eq!(
        active_actors,
        4 - run(h).score as usize,
        "confirmed collected actors disappear and return after restart"
    );
    // Exact scene pixels: remove surrounding panels and the top 24px PLAY/tick overlay.
    // Full-window captures retain that overlay and the mapping/victory/restart hints.
    let rect = h.state().ui.viewport_rect.unwrap();
    let (x, y, width, height) = (
        rect.min.x.ceil() as u32,
        rect.min.y.ceil() as u32 + 24,
        rect.max.x.floor() as u32 - rect.min.x.ceil() as u32,
        rect.max.y.floor() as u32 - rect.min.y.ceil() as u32 - 24,
    );
    let mut pixels = Vec::new();
    for py in y..y + height {
        for px in x..x + width {
            pixels.extend(image.get_pixel(px, py).0);
        }
    }
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[2] > 180 && p[0] < 160),
        "blue Collect player pixels"
    );
    let file = fs::File::create(evidence.join(format!("{name}-viewport.png"))).unwrap();
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    fs::write(evidence.join(format!("{name}.json")), serde_json::to_vec_pretty(&serde_json::json!({
        "tick": h.state().editor.snapshot().unwrap().tick(),
        "checksum": format!("{:016x}", h.state().editor.snapshot().unwrap().predicted().checksum()),
        "verified_tick": h.state().editor.snapshot().unwrap().verified_tick(),
        "verified_checksum": format!("{:016x}", h.state().editor.snapshot().unwrap().verified().unwrap().checksum()),
        "phase": run(h).phase, "score": run(h).score, "active_actors": active_actors, "viewport": [x,y,width,height],
    })).unwrap()).unwrap();
    pixels
}
#[test]
#[ignore = "mandatory llvmpipe main EditorApp acceptance with retained PNG evidence"]
fn authored_map_drives_actual_editor_gpu_victory_restart_reopen() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_COLLECT_CONTROLS_EVIDENCE").expect("retained evidence directory"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let map = evidence.join("collect-controls-authored.json");
    author_map(&map, &evidence);
    let map_bytes = fs::read(&map).unwrap();
    let project = tempfile::tempdir().unwrap();
    let scene = project.path().join("level.scene.yaml");
    let manifest = project.path().join("orr.project.json");
    fs::write(
        &scene,
        include_bytes!("../../../scenes/collect_dodge_v1.scene.yaml"),
    )
    .unwrap();
    fs::write(
        &manifest,
        include_bytes!("../../../assets/collect_dodge_project/orr.project.json"),
    )
    .unwrap();
    let initial_scene = fs::read(&scene).unwrap();
    let initial_manifest = fs::read(&manifest).unwrap();
    let mut h = open(project.path(), &evidence);
    h.run_steps(3);
    let initial_checksum = h.state().editor.checksum();
    load(&mut h, &map);
    assert_eq!(h.state().editor.checksum(), initial_checksum);
    assert!(!h.state().editor.is_dirty());
    assert_eq!(fs::read(&scene).unwrap(), initial_scene);
    assert_eq!(fs::read(&manifest).unwrap(), initial_manifest);
    play(&mut h);
    let initial = capture(&mut h, &evidence, "collect-controls-mapped");
    physical(&mut h, Key::D, Key::L, true);
    ticks(&mut h, 20);
    assert_eq!(
        player(&h),
        orr_fp::FPVec2::ZERO,
        "old physical D cannot move even logical L"
    );
    physical(&mut h, Key::D, Key::L, false);
    physical(&mut h, Key::L, Key::D, true);
    wait(&mut h, "physical L authoritative victory", |e| {
        e.snapshot()
            .unwrap()
            .predicted()
            .singleton::<CollectRun>()
            .phase
            == WON
    });
    physical(&mut h, Key::L, Key::D, false);
    assert_eq!(run(&h).score, 2);
    let won = capture(&mut h, &evidence, "collect-controls-victory");
    assert_ne!(initial, won);
    tap(&mut h, Key::Space);
    ticks(&mut h, 4);
    assert_eq!(run(&h).phase, WON, "old Space cannot restart");
    tap(&mut h, Key::Enter);
    wait(&mut h, "short Enter tap restart", |e| {
        let s = e.snapshot().unwrap();
        let r = s.predicted().singleton::<CollectRun>();
        r.phase == PLAYING && r.score == 0
    });
    assert_eq!(player(&h), orr_fp::FPVec2::ZERO);
    let restart = capture(&mut h, &evidence, "collect-controls-restart");
    assert!(
        initial == restart,
        "normalized restart viewport restores all initial actors"
    );
    // Mapped Quit releases the lease and leaves the actual application running.
    tap(&mut h, Key::Q);
    wait(&mut h, "mapped Quit releases", |e| {
        e.input_phase() == Phase::Off
    });
    assert_eq!(h.state().editor.mode(), Mode::Play);
    click(&mut h, "Take control");
    wait(&mut h, "reclaim", |e| e.input_phase() == Phase::Active);
    tap(&mut h, Key::Escape);
    wait(&mut h, "Escape still releases", |e| {
        e.input_phase() == Phase::Off
    });
    stop(&mut h);
    assert_eq!(fs::read(&scene).unwrap(), initial_scene);
    // An ordinary real inspector edit preserves session bindings, then explicit Save persists only the scene.
    h.get_by_label("first_collectible").click();
    wait(&mut h, "inspector", |e| e.inspect().is_some());
    h.run_steps(3);
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("10"))
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("10"))
        .type_text("12");
    h.run_steps(2);
    h.key_press(Key::Enter);
    wait(&mut h, "ordinary inspector edit", |e| {
        e.bodies().iter().any(|b| b.pos == [12.0, 0.0])
    });
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by(|n| {
        n.role() == Role::Button
            && n.label()
                .is_some_and(|s| s.starts_with("Save ") && !s.starts_with("Save As"))
    })
    .click();
    wait(&mut h, "explicit Save", |e| !e.is_dirty());
    let saved = fs::read(&scene).unwrap();
    assert_ne!(saved, initial_scene);
    play(&mut h); // Mapping survived ordinary edit/save.
    stop(&mut h);
    drop(h);
    let mut fresh = open(project.path(), &evidence);
    fresh.run_steps(3);
    assert_eq!(
        fresh
            .get_by_role_and_label(Role::TextInput, "Collect controls")
            .accesskit_node()
            .value()
            .as_deref(),
        Some("")
    );
    load(&mut fresh, &map); // Session setting must be explicitly restored in a fresh app.
    play(&mut fresh);
    physical(&mut fresh, Key::L, Key::D, true);
    wait(&mut fresh, "reopened gameplay", |e| {
        e.snapshot()
            .unwrap()
            .predicted()
            .singleton::<CollectRun>()
            .phase
            == WON
    });
    physical(&mut fresh, Key::L, Key::D, false);
    capture(&mut fresh, &evidence, "collect-controls-reopened-victory");
    assert_eq!(fs::read(&scene).unwrap(), saved);
    assert_eq!(fs::read(&manifest).unwrap(), initial_manifest);
    assert_eq!(fs::read(&map).unwrap(), map_bytes);
    let adapter: serde_json::Value =
        serde_json::from_slice(&fs::read(evidence.join("collect-controls-adapter.json")).unwrap())
            .unwrap();
    fs::write(evidence.join("collect-controls-acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema": 1, "adapter": adapter["name"], "adapter_details": adapter, "backend": "Vulkan", "device_type": "Cpu",
        "main_editor_app": true, "standalone_authoring_app": true, "physical_egui_events": true,
        "native_os_keyboard": false, "window_pixels": [1600,1000], "authoring_pixels": [720,650],
        "mapped_victory_restart": true, "authoritative_victory": true, "restart_acknowledged": true, "normalized_restart_viewport_equal": true,
        "fresh_app_explicit_reload": true, "configuration_bytes_unchanged": true,
        "viewport_normalization": "Crop editor panels and top 24px PLAY/global-tick overlay; full-window PNGs preserve all text. Restart resets gameplay, not the global tick.",
    })).unwrap()).unwrap();
}
