//! Actual settings widgets rendered over the admitted authored project.
//! Set ORR_REQUIRE_GPU=1 for mandatory software-GPU readback and
//! ORR_PLAYER_SETTINGS_CAPTURE_DIR to an existing directory to retain PNGs.
#![cfg(all(feature = "player-settings", target_os = "linux"))]
#![allow(clippy::float_arithmetic)]

use egui::{pos2, vec2, Event, FullOutput, Pos2, RawInput, Rect};
use orr_bridge::Bridge;
use orr_render::{
    orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions},
    OffscreenTarget,
};
use orr_sample::{
    arena_input::ArenaControls,
    game_ui::{Action, GameUi, Hud},
    game_ui_gpu::{GpuOverlay, PendingOverlay},
    player_controls::PlayerSettingsSession,
    player_settings::{FireBinding, SettingsPaths, SettingsStore},
    project_compositor::ProjectCompositor,
    project_runtime::{PreparedRuntime, ProjectPresentation},
};
use std::{fs, fs::OpenOptions, path::Path};

#[path = "common/project.rs"]
mod project_fixture;

fn input(time: f64, events: Vec<Event>) -> RawInput {
    let mut input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        time: Some(time),
        events,
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .native_pixels_per_point = Some(1.0);
    input
}

fn labels(output: &FullOutput) -> String {
    fn collect(shape: &egui::Shape, text: &mut String) {
        match shape {
            egui::Shape::Text(shape) => {
                text.push_str(&shape.galley.job.text);
                text.push('\n');
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, text);
                }
            }
            _ => {}
        }
    }
    let mut text = String::new();
    for shape in &output.shapes {
        collect(&shape.shape, &mut text);
    }
    text
}

fn frame(ui: &mut GameUi, pending: &mut PendingOverlay, hud: Hud, time: f64) -> String {
    let (output, action) = ui.show(input(time, vec![]), hud);
    assert!(action.is_none());
    let text = labels(&output);
    pending.push(output).unwrap();
    text
}

fn click(ui: &mut GameUi, pending: &mut PendingOverlay, hud: Hud, action: Action, time: f64) {
    let point = ui
        .buttons
        .iter()
        .find(|(candidate, _)| *candidate == action)
        .unwrap()
        .1
        .center();
    let (pressed, on_press) = ui.show(
        input(
            time,
            vec![
                Event::PointerMoved(point),
                Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        ),
        hud,
    );
    assert!(on_press.is_none());
    pending.push(pressed).unwrap();
    let (released, clicked) = ui.show(
        input(
            time + 0.016,
            vec![Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        ),
        hud,
    );
    assert_eq!(
        clicked,
        Some(action),
        "the real widget must generate its action"
    );
    pending.push(released).unwrap();
    assert!(ui.apply(clicked.unwrap()));
}

fn capture(
    ui: &GameUi,
    presentation: &ProjectPresentation,
    pending: &mut PendingOverlay,
    compositor: &mut ProjectCompositor<Wgpu>,
    overlay: &mut GpuOverlay,
    target: &OffscreenTarget<Wgpu>,
) -> Vec<u8> {
    let mut called = false;
    compositor
        .draw_with_overlay(
            target.render_view(),
            target.size(),
            presentation.shapes(),
            presentation.sprites(),
            &presentation.camera,
            |rhi, view, size| {
                called = true;
                pending.paint(overlay, rhi, view, size, &ui.context)
            },
        )
        .unwrap();
    assert!(called);
    target.read_rgba8()
}

fn bright_pixels(image: &[u8], bounds: Rect) -> usize {
    let minimum = bounds.min.max(pos2(0.0, 0.0));
    let maximum = bounds.max.min(pos2(800.0, 600.0));
    (minimum.y.floor() as usize..maximum.y.ceil() as usize)
        .flat_map(|y| (minimum.x.floor() as usize..maximum.x.ceil() as usize).map(move |x| (x, y)))
        .filter(|(x, y)| {
            image[(y * 800 + x) * 4..][..3]
                .iter()
                .all(|channel| *channel > 110)
        })
        .count()
}

fn retain(name: &str, image: &[u8]) {
    let Some(directory) = std::env::var_os("ORR_PLAYER_SETTINGS_CAPTURE_DIR") else {
        return;
    };
    let directory = Path::new(&directory);
    assert!(directory.is_dir(), "capture directory must already exist");
    let path = directory.join(name);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap_or_else(|error| panic!("create {}: {error}", path.display()));
    let mut encoder = png17::Encoder::new(file, 800, 600);
    encoder.set_color(png17::ColorType::Rgba);
    encoder.set_depth(png17::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(image)
        .unwrap();
}

#[test]
fn korean_settings_widgets_render_saved_left_mouse_over_authored_scene() {
    let fixture = project_fixture::ProjectFixture::new();
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/game_ui_font")
        .canonicalize()
        .unwrap();
    orr_package::Project::open_for_install(
        &fixture.root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[package])
    .unwrap();
    let manifest_path = fixture.root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["entry"]["ui"] = serde_json::json!({"profile": "arena-korean-v1", "font": {
        "package": "korean-game-ui", "asset": "OrreryKoreanUI.otf"
    }});
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let manifest_before = fs::read(&manifest_path).unwrap();
    let (seed, mut presentation, prepared_ui) = PreparedRuntime::open(&fixture.root)
        .unwrap()
        .into_launch_parts();
    let prepared_ui = prepared_ui.expect("saved Korean UI must be admitted before GPU creation");
    let gpu = match Wgpu::headless(WgpuOptions {
        force_software: true,
        ..Default::default()
    }) {
        Ok(gpu) => gpu,
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none()
                    && std::env::var_os("ORR_PLAYER_SETTINGS_CAPTURE_DIR").is_none(),
                "required software GPU unavailable: {error}"
            );
            eprintln!("SKIP: software GPU unavailable: {error}");
            return;
        }
    };
    assert!(gpu.is_software());
    eprintln!("player settings software adapter: {}", gpu.adapter_name());
    let bridge = seed.bridge().unwrap();
    let snapshot = bridge.snapshot().unwrap();
    presentation.update(Some(&snapshot)).unwrap();
    assert_eq!(presentation.sprites().len(), 2);
    let hud = Hud {
        tick: snapshot.tick(),
        verified_tick: snapshot.verified_tick(),
        rollbacks: snapshot.stats().rollbacks,
    };
    let format = TextureFormat::Rgba8UnormSrgb;
    let target = OffscreenTarget::new(&gpu, 800, 600, format);
    let mut compositor =
        ProjectCompositor::new(gpu.clone(), format, presentation.assets()).unwrap();
    compositor
        .draw(
            target.render_view(),
            target.size(),
            presentation.shapes(),
            presentation.sprites(),
            &presentation.camera,
        )
        .unwrap();
    let underlay = target.read_rgba8();
    let mut overlay = GpuOverlay::new(&gpu, format);
    let mut pending = PendingOverlay::default();
    let profile = tempfile::Builder::new()
        .prefix("orr player settings ui ")
        .tempdir()
        .unwrap();
    let paths = SettingsPaths::from_directory(profile.path().join("external profile")).unwrap();
    let session = PlayerSettingsSession::open(Ok(paths.clone()));
    let mut controls = ArenaControls::new(session.action_map()).unwrap();
    controls.set_ui_capture(true);
    let mut ui = GameUi::from_font(prepared_ui.font, true).unwrap();
    ui.set_player_settings(session);
    let mut text = String::new();
    for frame_index in 0..4 {
        text = frame(&mut ui, &mut pending, hud, 1.0 + f64::from(frame_index));
    }
    for label in [
        "발사 조작 설정",
        "마우스 왼쪽 버튼",
        "적용",
        "취소",
        "기본값",
    ] {
        assert!(text.contains(label), "actual UI geometry omits {label}");
    }
    assert!(!paths.primary().exists());
    let initial = capture(
        &ui,
        &presentation,
        &mut pending,
        &mut compositor,
        &mut overlay,
        &target,
    );
    retain("player-settings-default.png", &initial);

    click(&mut ui, &mut pending, hud, Action::FireLeftMouse, 5.0);
    frame(&mut ui, &mut pending, hud, 6.0);
    assert_eq!(
        ui.player_settings().unwrap().draft(),
        FireBinding::LeftMouse
    );
    click(&mut ui, &mut pending, hud, Action::SettingsApply, 7.0);
    assert!(ui.apply_player_settings(&mut controls));
    assert!(
        !controls.keys().fire,
        "Apply cannot become a gameplay mouse press"
    );
    let settings = ui.player_settings().unwrap();
    assert_eq!(settings.active(), FireBinding::LeftMouse);
    assert!(!settings.dirty());
    let status = settings.status().to_owned();
    for frame_index in 0..4 {
        text = frame(&mut ui, &mut pending, hud, 8.0 + f64::from(frame_index));
    }
    assert!(text.contains("발사: 마우스 왼쪽 버튼"));
    assert!(
        text.contains(&status),
        "saved status is missing from actual rendered text"
    );
    assert!(status.contains("저장하고 적용"), "{status}");
    let selected = ui
        .buttons
        .iter()
        .find(|(action, _)| *action == Action::FireLeftMouse)
        .unwrap()
        .1;
    let saved = capture(
        &ui,
        &presentation,
        &mut pending,
        &mut compositor,
        &mut overlay,
        &target,
    );
    assert_ne!(
        saved, initial,
        "selection and save status must change visible output"
    );
    assert!(
        bright_pixels(&saved, selected) > 30,
        "Korean firing selector glyphs must be readable"
    );
    assert!(
        saved
            .chunks_exact(4)
            .zip(underlay.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count()
            > 2000,
        "the settings panel must be composed over the authored scene"
    );
    retain("player-settings-saved-left-mouse.png", &saved);
    assert_eq!(
        SettingsStore::new(paths).load().settings.fire_binding,
        FireBinding::LeftMouse
    );
    assert_eq!(fs::read(manifest_path).unwrap(), manifest_before);
}
