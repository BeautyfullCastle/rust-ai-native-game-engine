//! Real native App/Session/egui input path, not a mocked event producer or GPU.
use super::*;

fn ready() -> App {
    let mut app = pause_tests::app();
    app.ui.vfx.prepare(&app.ui.context).unwrap();
    app
}
fn pulse(key: egui::Key) -> Vec<egui::Event> {
    [true, false]
        .into_iter()
        .map(|pressed| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: Default::default(),
        })
        .collect()
}
fn frame(app: &mut App, events: Vec<egui::Event>, delta: f64) {
    let mut output = app.process_input(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 940.0),
            )),
            events,
            ..Default::default()
        },
        delta,
    );
    output.textures_delta.clear(); // CPU egui path has no renderer to consume uploads.
}
fn combat(app: &mut App) {
    for tick in 0..1200 {
        let events = if tick % 45 == 0 {
            pulse(egui::Key::Space)
        } else {
            vec![]
        };
        frame(app, events, 1.0 / 60.0);
        assert!(app.error.is_none(), "{:?}", app.error);
        if app.ui.vfx.test_stats().2 > 0 {
            return;
        }
    }
    panic!("actual native App did not deliver any combat VFX");
}
#[test]
fn actual_app_combat_pause_focus_and_resume_freeze_visual_clock() {
    let mut app = ready();
    combat(&mut app);
    let before = app.ui.vfx.test_stats();
    let frame_before = app.session.predicted_frame().to_bytes();
    frame(&mut app, pulse(egui::Key::P), 10.0);
    assert!(app.ui.paused);
    frame(&mut app, vec![], 10.0);
    assert_eq!(app.ui.vfx.test_stats(), before);
    assert_eq!(app.session.predicted_frame().to_bytes(), frame_before);
    app.set_focused(false);
    frame(&mut app, pulse(egui::Key::Enter), 10.0);
    assert_eq!(app.ui.vfx.test_stats(), before);
    app.set_focused(true);
    frame(&mut app, pulse(egui::Key::Enter), 10.0);
    assert_eq!(
        app.ui.vfx.test_stats(),
        before,
        "resume frame must not catch up"
    );
    frame(&mut app, vec![], 1.0 / 60.0);
    assert_eq!(app.ui.vfx.test_stats().0, before.0 + 1);
}
#[test]
fn actual_app_restart_clears_old_effects_and_changes_generation_once() {
    let mut app = ready();
    combat(&mut app);
    frame(&mut app, pulse(egui::Key::R), 1.0 / 60.0);
    for _ in 0..4 {
        frame(&mut app, vec![], 1.0 / 60.0);
    }
    let stats = app.ui.vfx.test_stats();
    assert_eq!((stats.2, stats.3), (0, 1));
    assert_eq!(game::run(app.session.predicted_frame()).generation, 1);
    assert!(app.error.is_none());
    frame(&mut app, vec![], 1.0 / 60.0);
    assert_eq!(app.ui.vfx.test_stats().3, 1);
}
#[test]
fn actual_app_prestart_and_inert_redraws_cannot_create_effects() {
    let mut app = ready();
    app.ui.started = false;
    for _ in 0..3 {
        frame(&mut app, vec![], 10.0);
    }
    assert_eq!(app.ui.vfx.test_stats(), (0, 0, 0, 0));
    frame(&mut app, pulse(egui::Key::Enter), 0.0);
    assert!(app.ui.started);
    for _ in 0..3 {
        frame(&mut app, vec![], 0.0);
    }
    assert_eq!(app.ui.vfx.test_stats(), (0, 0, 0, 0));
}
