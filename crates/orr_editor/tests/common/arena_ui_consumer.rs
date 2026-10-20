//! Isolated production-module consumer, not an OS-window or exported-App driver.
use egui::{Event, PointerButton, Pos2, Rect};
use orr_bridge::Bridge;
use orr_render::{
    orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions},
    OffscreenTarget,
};
use orr_sample::{
    game_ui::{Action, GameUi, Hud, Screen},
    game_ui_gpu::{GpuOverlay, PendingOverlay},
    project_compositor::ProjectCompositor,
    project_runtime::PreparedRuntime,
};
use std::{fs, path::Path};

fn frame(
    ui: &mut GameUi,
    pending: &mut PendingOverlay,
    time: &mut f64,
    events: Vec<Event>,
    hud: Hud,
) -> Option<Action> {
    *time += 0.5;
    let (output, action) = ui.show(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(512.0, 512.0))),
            time: Some(*time),
            events,
            ..Default::default()
        },
        hud,
    );
    pending.push(output).unwrap();
    action
}
fn click(ui: &mut GameUi, pending: &mut PendingOverlay, time: &mut f64, action: Action, hud: Hud) {
    for _ in 0..3 {
        assert!(frame(ui, pending, time, vec![], hud).is_none());
    }
    let point = ui
        .buttons
        .iter()
        .find(|(a, _)| *a == action)
        .expect("actual preset widget")
        .1
        .center();
    assert!(frame(
        ui,
        pending,
        time,
        vec![
            Event::PointerMoved(point),
            Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default()
            }
        ],
        hud
    )
    .is_none());
    assert_eq!(
        frame(
            ui,
            pending,
            time,
            vec![Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Default::default()
            }],
            hud
        ),
        Some(action)
    );
    // Apply only the action emitted by the real widget, never assign screen state.
    assert!(ui.apply(action));
    for _ in 0..3 {
        assert!(frame(ui, pending, time, vec![], hud).is_none());
    }
}
fn write_png(path: &Path, bytes: &[u8]) {
    let mut encoder = png::Encoder::new(fs::File::create_new(path).unwrap(), 512, 512);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(bytes)
        .unwrap();
}
fn visible_preset(base: &[u8], with_ui: &[u8]) -> bool {
    // Fixed top HUD area before the right-side Menu control. Both the background
    // and bright glyph pixels must differ, so identical/no-UI images fail.
    let mut changed = 0;
    let mut bright = 0;
    for y in 0..48 {
        for x in 0..320 {
            let i = (y * 512 + x) * 4;
            if base[i..i + 4] != with_ui[i..i + 4] {
                changed += 1;
                if with_ui[i..i + 3].iter().all(|v| *v > 100) {
                    bright += 1;
                }
            }
        }
    }
    changed > 100 && bright > 30
}
pub fn consume(bundle: &Path, evidence: &Path) {
    let project = bundle.join("project");
    let prepared = PreparedRuntime::open(&project).unwrap();
    let (seed, mut presentation, font) = prepared.into_launch_parts();
    let font = font.expect("bundle's admitted saved Korean preset");
    let mut ui = GameUi::from_font(font.font, true).unwrap();
    let bridge = seed.bridge().unwrap();
    let snapshot = bridge.snapshot().unwrap();
    presentation.update(Some(&snapshot)).unwrap();
    assert!(!presentation.sprites().is_empty());
    let hud = Hud {
        tick: snapshot.tick(),
        verified_tick: snapshot.tick(),
        rollbacks: 0,
    };
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("required GPU adapter");
    fs::write(evidence.join("ui-consumer-adapter.json"), serde_json::to_vec_pretty(&serde_json::json!({"name":gpu.adapter_name(), "software":gpu.is_software(), "coverage":"production modules, not OS-window input"})).unwrap()).unwrap();
    let format = TextureFormat::Rgba8UnormSrgb;
    let target = OffscreenTarget::new(&gpu, 512, 512, format);
    let mut compositor =
        ProjectCompositor::new(gpu.clone(), format, presentation.assets()).unwrap();
    compositor
        .draw(
            target.render_view(),
            (512, 512),
            presentation.shapes(),
            presentation.sprites(),
            &presentation.camera,
        )
        .unwrap();
    let no_ui = target.read_rgba8();
    write_png(&evidence.join("ui-negative-no-overlay.png"), &no_ui);
    let mut overlay = GpuOverlay::new(&gpu, format);
    let mut pending = PendingOverlay::default();
    let mut time = 0.0;
    for _ in 0..3 {
        assert!(frame(&mut ui, &mut pending, &mut time, vec![], hud).is_none());
    }
    let mut capture = |name: &str, ui: &GameUi, pending: &mut PendingOverlay| {
        compositor
            .draw_with_overlay(
                target.render_view(),
                (512, 512),
                presentation.shapes(),
                presentation.sprites(),
                &presentation.camera,
                |rhi, view, size| pending.paint(&mut overlay, rhi, view, size, &ui.context),
            )
            .unwrap();
        let pixels = target.read_rgba8();
        write_png(&evidence.join(name), &pixels);
        pixels
    };
    assert_eq!(ui.screen(), Screen::Title);
    let title = capture("ui-title.png", &ui, &mut pending);
    assert_ne!(title, no_ui);
    click(&mut ui, &mut pending, &mut time, Action::Play, hud);
    assert_eq!(ui.screen(), Screen::Playing);
    let playing = capture("ui-playing.png", &ui, &mut pending);
    assert!(
        visible_preset(&no_ui, &playing),
        "Korean HUD must visibly affect captured scene"
    );
    assert!(
        !visible_preset(&no_ui, &no_ui),
        "same scene with absent UI must fail the exact visible-preset assertion"
    );
    click(&mut ui, &mut pending, &mut time, Action::Menu, hud);
    assert_eq!(ui.screen(), Screen::Menu);
    let menu = capture("ui-menu.png", &ui, &mut pending);
    assert_ne!(menu, playing);
    click(&mut ui, &mut pending, &mut time, Action::Continue, hud);
    assert_eq!(ui.screen(), Screen::Playing);
    let continued = capture("ui-continued.png", &ui, &mut pending);
    assert!(visible_preset(&no_ui, &continued));
    click(&mut ui, &mut pending, &mut time, Action::Menu, hud);
    click(&mut ui, &mut pending, &mut time, Action::Restart, hud);
    assert_eq!(ui.screen(), Screen::Playing);
    let restarted = capture("ui-restart-action.png", &ui, &mut pending);
    assert!(visible_preset(&no_ui, &restarted));
    // App::dispatch_ui_action owns simulation restart and is private. This
    // consumer proves button dispatch + UI, not that separate bridge-reset route.
    println!(
        "UI controls and visible negative passed; Restart button/UI only, not App bridge reset"
    );
}
