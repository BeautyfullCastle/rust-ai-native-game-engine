//! Real native App/egui/Session path. CPU-only, no window or GPU.
use super::*;

fn ready() -> App {
    let mut app = pause_tests::app();
    app.ui.pixel_motion.prepare(&app.ui.context).unwrap();
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
fn frame(app: &mut App, events: Vec<egui::Event>, delta: f64) -> Vec<egui::Pos2> {
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
    assert!(app.error.is_none(), "{:?}", app.error);
    let texture = app.ui.pixel_motion.test_texture_id();
    let uv = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Mesh(m) if m.texture_id == texture => Some(m.vertices[0].uv),
            _ => None,
        })
        .collect();
    output.textures_delta.clear();
    uv
}
fn expected(app: &App) -> egui::Pos2 {
    let run = game::run(app.session.predicted_frame());
    let region = if app.ui.started && run.phase == game::PLAYING {
        run.tick % 84 / 6
    } else {
        0
    };
    egui::pos2(
        (region % 7 * 96) as f32 / 672.0,
        (region / 7 * 112) as f32 / 224.0,
    )
}
fn summon_rivet(app: &mut App) {
    for tick in 0..360 {
        let events = if tick % 30 == 0 {
            pulse(egui::Key::Space)
        } else {
            vec![]
        };
        frame(app, events, 1.0 / 60.0);
        if game::units(app.session.predicted_frame())
            .iter()
            .any(|u| u.active != 0 && u.role == game::RIVET)
        {
            let uv = frame(app, vec![], 0.0);
            assert!(!uv.is_empty());
            assert!(uv.iter().all(|&v| v == expected(app)));
            return;
        }
    }
    panic!("actual App failed to summon and paint a Rivet");
}
#[test]
fn actual_app_pause_focus_resume_and_inert_redraws_freeze_pixel_frame() {
    let mut app = ready();
    summon_rivet(&mut app);
    for _ in 0..7 {
        frame(&mut app, vec![], 1.0 / 60.0);
    }
    let before = app.session.predicted_frame().to_bytes();
    let uv = frame(&mut app, vec![], 0.0);
    assert_eq!(frame(&mut app, pulse(egui::Key::P), 10.0), uv);
    assert!(app.ui.paused);
    for _ in 0..3 {
        assert_eq!(frame(&mut app, vec![], 10.0), uv);
    }
    app.set_focused(false);
    assert_eq!(frame(&mut app, pulse(egui::Key::Enter), 10.0), uv);
    app.set_focused(true);
    assert_eq!(frame(&mut app, pulse(egui::Key::Enter), 10.0), uv);
    assert_eq!(app.session.predicted_frame().to_bytes(), before);
    for _ in 0..6 {
        frame(&mut app, vec![], 1.0 / 60.0);
    }
    let resumed = frame(&mut app, vec![], 0.0);
    assert!(resumed.iter().all(|&v| v == expected(&app)));
    assert_ne!(resumed, uv);
}
#[test]
fn actual_app_prestart_is_neutral_and_restart_uses_new_run_clock() {
    let mut app = ready();
    app.ui.started = false;
    let before = app.session.predicted_frame().to_bytes();
    for _ in 0..3 {
        assert!(frame(&mut app, vec![], 10.0).is_empty());
    }
    assert_eq!(app.session.predicted_frame().to_bytes(), before);
    frame(&mut app, pulse(egui::Key::Enter), 10.0);
    assert!(app.ui.started);
    assert_eq!(app.session.predicted_frame().to_bytes(), before);
    summon_rivet(&mut app);
    for _ in 0..27 {
        frame(&mut app, vec![], 1.0 / 60.0);
    }
    frame(&mut app, pulse(egui::Key::R), 1.0 / 60.0);
    for _ in 0..4 {
        frame(&mut app, vec![], 1.0 / 60.0);
    }
    let run = game::run(app.session.predicted_frame());
    assert_eq!(run.generation, 1);
    assert!(run.tick < 6);
    assert!(
        frame(&mut app, vec![], 0.0).is_empty(),
        "removed units cannot leave ghost skins"
    );
    summon_rivet(&mut app);
    assert_eq!(game::run(app.session.predicted_frame()).generation, 1);
}
#[cfg(all(
    feature = "coop-defense-pixelfx",
    not(feature = "coop-defense-attack-clip")
))]
#[test]
fn actual_release_moves_only_its_skin_and_pause_freezes_recovery() {
    fn positions(app: &mut App) -> Vec<egui::Pos2> {
        let texture = app.ui.pixel_motion.test_texture_id();
        let mut output = app.process_input(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 940.0),
                )),
                ..Default::default()
            },
            0.0,
        );
        assert!(app.error.is_none(), "{:?}", app.error);
        let points = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Mesh(m) if m.texture_id == texture => Some(m.vertices[0].pos),
                _ => None,
            })
            .collect();
        output.textures_delta.clear();
        points
    }
    let mut app = ready();
    let mut found = false;
    for tick in 0..1800 {
        frame(
            &mut app,
            if tick % 45 == 0 {
                pulse(egui::Key::Space)
            } else {
                vec![]
            },
            1.0 / 60.0,
        );
        let run = game::run(app.session.predicted_frame());
        let mut units = game::units(app.session.predicted_frame());
        units.sort_by_key(|unit| (unit.board, unit.cell));
        let offsets: Vec<_> = units
            .iter()
            .filter(|unit| unit.active != 0 && unit.role == game::RIVET)
            .map(|unit| app.ui.vfx.unit_recoil(unit, &run))
            .collect();
        if !offsets.iter().any(|offset| *offset > 0.0) {
            continue;
        }
        let before = app.session.predicted_frame().to_bytes();
        let animated = positions(&mut app);
        let vfx = std::mem::take(&mut app.ui.vfx);
        let neutral = positions(&mut app);
        app.ui.vfx = vfx;
        assert_eq!(animated.len(), offsets.len());
        assert_eq!(neutral.len(), offsets.len());
        for ((animated, neutral), offset) in animated.iter().zip(neutral.iter()).zip(offsets) {
            assert!((animated.x - neutral.x).abs() < 0.001);
            assert!((animated.y - neutral.y - offset).abs() < 0.001);
        }
        assert_eq!(app.session.predicted_frame().to_bytes(), before);
        frame(&mut app, pulse(egui::Key::P), 0.0);
        frame(&mut app, vec![], 10.0);
        assert_eq!(positions(&mut app), animated);
        assert_eq!(app.session.predicted_frame().to_bytes(), before);
        found = true;
        break;
    }
    assert!(found, "actual App must produce a Rivet Release event");
}

#[cfg(feature = "coop-defense-attack-clip")]
#[test]
fn actual_attack_clip_release_pause_idle_fallback_and_restart_keep_frame_bytes() {
    fn attack_uv(app: &mut App) -> Vec<egui::Pos2> {
        let texture = app.ui.pixel_motion.test_attack_texture_id();
        let mut output = app.process_input(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 940.0),
                )),
                ..Default::default()
            },
            0.0,
        );
        assert!(app.error.is_none(), "{:?}", app.error);
        let result = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Mesh(m) if m.texture_id == texture => Some(m.vertices[0].uv),
                _ => None,
            })
            .collect();
        output.textures_delta.clear();
        result
    }
    let mut app = ready();
    assert!(attack_uv(&mut app).is_empty());
    for tick in 0..1800 {
        frame(
            &mut app,
            if tick % 45 == 0 {
                pulse(egui::Key::Space)
            } else {
                vec![]
            },
            1.0 / 60.0,
        );
        let run = game::run(app.session.predicted_frame());
        let before = app.session.predicted_frame().to_bytes();
        let actual = attack_uv(&mut app);
        let mut units = game::units(app.session.predicted_frame());
        units.sort_by_key(|u| (u.board, u.cell));
        let expected: Vec<_> = units
            .iter()
            .filter(|u| u.active != 0 && u.role == game::RIVET)
            .filter_map(|u| app.ui.vfx.unit_release_pose(u, &run))
            .filter(|(age, _)| *age < 20)
            .map(|(age, flip)| {
                let region = (age * 1000 / 60 / 40) as usize;
                egui::pos2(
                    if flip {
                        [444, 887, 1330, 1774][region % 4]
                    } else {
                        [0, 444, 887, 1330][region % 4]
                    } as f32
                        / 1774.0,
                    if region < 4 { 0.0 } else { 444.0 / 887.0 },
                )
            })
            .collect();
        if expected.is_empty() {
            continue;
        }
        assert_eq!(actual, expected);
        let shots = game::projectiles(app.session.predicted_frame());
        let enemies = game::enemies(app.session.predicted_frame());
        let mut checked_target = false;
        for unit in units
            .iter()
            .filter(|u| u.active != 0 && u.role == game::RIVET)
        {
            let Some((0, flip)) = app.ui.vfx.unit_release_pose(unit, &run) else {
                continue;
            };
            if let Some(enemy) = shots
                .iter()
                .filter(|s| s.active != 0 && s.source_unit_id == unit.id)
                .max_by_key(|s| s.id)
                .and_then(|shot| {
                    enemies
                        .iter()
                        .find(|enemy| enemy.active != 0 && enemy.id == shot.target_id)
                })
            {
                assert_eq!(
                    flip,
                    game::enemy_position(enemy.lane, enemy.progress).0
                        < game::board_cell_position(unit.board, unit.cell).0
                );
                checked_target = true;
            }
        }
        assert!(
            checked_target,
            "first displayed shot has a live independently resolved target"
        );
        let vfx = std::mem::take(&mut app.ui.vfx);
        assert!(attack_uv(&mut app).is_empty());
        assert!(
            !frame(&mut app, vec![], 0.0).is_empty(),
            "no shot uses original idle texture"
        );
        app.ui.vfx = vfx;
        frame(&mut app, pulse(egui::Key::P), 0.0);
        frame(&mut app, vec![], 10.0);
        assert_eq!(attack_uv(&mut app), expected);
        assert_eq!(app.session.predicted_frame().to_bytes(), before);
        frame(&mut app, pulse(egui::Key::Enter), 0.0);
        frame(&mut app, pulse(egui::Key::R), 1.0 / 60.0);
        for _ in 0..3 {
            frame(&mut app, vec![], 1.0 / 60.0);
        }
        assert_eq!(
            game::run(app.session.predicted_frame()).generation,
            run.generation + 1
        );
        assert!(attack_uv(&mut app).is_empty());
        return;
    }
    panic!("real native App must produce a Rivet shot clip");
}

#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
#[test]
fn actual_app_loaded_save_restores_pixel_frame_without_backlog_or_catchup() {
    use crate::defense_run_save::{Candidate, Journal, Origin};
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let mut app = ready();
    app.saves = Some(RunSaves {
        store: crate::defense_run_save_store::RunStore::open(dir.path(), &[]).unwrap(),
        identity: "a".repeat(64),
        journal: Journal::new(Origin { scene: None }, &app.session),
        candidate: None,
    });
    summon_rivet(&mut app);
    for _ in 0..19 {
        frame(&mut app, vec![], 1.0 / 60.0);
    }
    frame(&mut app, pulse(egui::Key::P), 0.0);
    let saved_frame = app.session.predicted_frame().to_bytes();
    let saved_uv = frame(&mut app, vec![], 0.0);
    let saves = app.saves.as_mut().unwrap();
    let bytes = saves
        .journal
        .encode(&app.session, app.ui.high_water(), &saves.identity)
        .unwrap();
    frame(&mut app, pulse(egui::Key::Enter), 0.0);
    for _ in 0..24 {
        frame(&mut app, vec![], 1.0 / 60.0);
    }
    frame(&mut app, pulse(egui::Key::P), 0.0);
    assert_ne!(app.session.predicted_frame().to_bytes(), saved_frame);
    app.saves.as_mut().unwrap().candidate =
        Some(Candidate::parse(&bytes, &"a".repeat(64)).unwrap());
    app.ui.run_save.mode = crate::defense_ui::RunSaveMode::Loading;
    for _ in 0..10 {
        frame(&mut app, vec![], 10.0);
        if app.saves.as_ref().unwrap().candidate.is_none() {
            break;
        }
    }
    assert!(app.saves.as_ref().unwrap().candidate.is_none());
    assert!(app.ui.paused);
    assert_eq!(app.session.predicted_frame().to_bytes(), saved_frame);
    assert_eq!(frame(&mut app, vec![], 10.0), saved_uv);
    assert_eq!(frame(&mut app, pulse(egui::Key::Enter), 10.0), saved_uv);
    assert_eq!(app.session.predicted_frame().to_bytes(), saved_frame);
    for _ in 0..6 {
        frame(&mut app, vec![], 1.0 / 60.0);
    }
    let uv = frame(&mut app, vec![], 0.0);
    assert!(uv.iter().all(|&v| v == expected(&app)));
    assert_ne!(uv, saved_uv);
}
