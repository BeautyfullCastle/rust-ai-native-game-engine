//! CPU egui/frame acceptance only. No native window, GPU or playtest video.
use super::*;
type Sim = orr_sim::Simulation<game::DefenseGame>;
fn sim() -> Sim {
    Sim::new(game::DefenseConfig::default(), 60, 1)
}
fn label(frame: &orr_ecs::Frame) -> String {
    wave_pressure_label(
        &game::run(frame),
        frame.singleton::<game::WaveCounts>(),
        &game::enemies(frame),
    )
}
fn advance(sim: &mut Sim, action: game::DefenseInput) {
    let mut input = orr_sim::TickInputs::new(sim.frame().tick(), 1);
    input.set_input(orr_sim::PlayerSlot(0), action);
    sim.step(&input);
}
fn first_spawn(sim: &mut Sim) {
    for _ in 0..600 {
        if game::run(sim.frame()).spawned != 0 {
            return;
        }
        advance(sim, game::DefenseInput::default());
    }
    panic!("real simulation never spawned its first pair");
}
fn text_bounds(shape: &egui::Shape, needle: &str) -> Option<Rect> {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == needle => Some(shape.visual_bounding_rect()),
        egui::Shape::Vec(v) => v.iter().find_map(|s| text_bounds(s, needle)),
        _ => None,
    }
}
fn draw(ui: &mut DefenseUi, frame: &orr_ecs::Frame, size: egui::Vec2) -> egui::FullOutput {
    let checksum = frame.checksum();
    let tick = frame.tick();
    let (mut output, command) = ui.show(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        },
        frame,
    );
    output.textures_delta.clear(); // CPU-only renderer consumes no uploads.
    assert!(command.is_none());
    assert_eq!(frame.checksum(), checksum);
    assert_eq!(frame.tick(), tick);
    output
}

#[test]
fn incoming_uses_authored_pairs_and_route_counts_only_active_enemies_on_both_lanes() {
    let mut run = game::run(sim().frame());
    let mut counts = game::WaveCounts::default();
    counts.pair_counts[0] = 3;
    run.spawned = 1;
    let enemies = [
        game::DefenseEnemy {
            active: 1,
            lane: 0,
            ..Default::default()
        },
        game::DefenseEnemy {
            active: 1,
            lane: 1,
            ..Default::default()
        },
        game::DefenseEnemy {
            active: 0,
            hp: 100,
            ..Default::default()
        },
    ];
    assert_eq!(
        wave_pressure_label(&run, &counts, &enemies),
        "INCOMING 5  /  ON ROUTE 2"
    );
    run.spawned = 6;
    assert_eq!(
        wave_pressure_label(&run, &counts, &enemies),
        "INCOMING 0  /  ON ROUTE 2"
    );
}

#[test]
fn invalid_wave_and_excess_spawn_counters_do_not_panic_or_underflow() {
    let mut run = game::run(sim().frame());
    let counts = game::WaveCounts::default();
    for wave in [0, 11, u32::MAX] {
        run.wave = wave;
        assert_eq!(
            wave_pressure_label(&run, &counts, &[]),
            "WAVE STATUS UNAVAILABLE"
        );
    }
    run.wave = 1;
    run.spawned = u32::MAX;
    assert_eq!(
        wave_pressure_label(&run, &counts, &[]),
        "INCOMING 0  /  ON ROUTE 0"
    );
}

#[test]
fn terminal_and_unknown_phases_do_not_suggest_future_spawns() {
    let mut run = game::run(sim().frame());
    let counts = game::WaveCounts::default();
    for (phase, expected) in [
        (game::WON, "CAMPAIGN CLEARED"),
        (game::LOST, "CORE LOST"),
        (u32::MAX, "WAVE STATUS UNAVAILABLE"),
    ] {
        run.phase = phase;
        assert_eq!(wave_pressure_label(&run, &counts, &[]), expected);
    }
}

#[test]
fn real_spawn_updates_authored_totals_without_modifying_frame() {
    let mut config = game::DefenseConfig::default();
    config.wave_counts.pair_counts[0] = 3;
    let mut sim = Sim::new(config, 60, 1);
    assert_eq!(label(sim.frame()), "INCOMING 6  /  ON ROUTE 0");
    first_spawn(&mut sim);
    assert_eq!(game::run(sim.frame()).spawned, 2);
    let before = sim.frame().checksum();
    let active = game::enemies(sim.frame())
        .iter()
        .filter(|e| e.active != 0)
        .count();
    assert_eq!(
        label(sim.frame()),
        format!("INCOMING 4  /  ON ROUTE {active}")
    );
    assert_eq!(sim.frame().checksum(), before);
}

#[test]
fn rendered_label_tracks_rewind_and_fits_responsive_header_without_commands() {
    let mut sim = sim();
    let early = sim.frame().clone();
    first_spawn(&mut sim);
    let late = sim.frame().clone();
    let mut ui = DefenseUi::new();
    ui.started = true;
    for size in [vec2(1200.0, 940.0), vec2(750.0, 900.0), vec2(1800.0, 800.0)] {
        for frame in [&late, &early, &late] {
            let expected = label(frame);
            let output = draw(&mut ui, frame, size);
            let bounds = output
                .shapes
                .iter()
                .find_map(|s| text_bounds(&s.shape, &expected))
                .expect("HUD label not painted");
            assert!(Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(bounds));
            assert!(
                bounds.max.x
                    < Layout::new(Rect::from_min_size(egui::Pos2::ZERO, size))
                        .p(785.0, 67.0)
                        .x
            );
        }
    }
}

#[test]
fn title_pause_and_real_restart_preserve_truthful_frame_status() {
    let mut sim = sim();
    let size = vec2(1200.0, 940.0);
    let mut ui = DefenseUi::new();
    let output = draw(&mut ui, sim.frame(), size);
    assert!(output.shapes.iter().any(|s| text_bounds(
        &s.shape,
        "A COOPERATIVE DEFENSE OF THE LIVING ENGINE"
    )
    .is_some()));
    assert!(!output
        .shapes
        .iter()
        .any(|s| text_bounds(&s.shape, &label(sim.frame())).is_some()));
    ui.started = true;
    first_spawn(&mut sim);
    ui.paused = true;
    for _ in 0..2 {
        let output = draw(&mut ui, sim.frame(), size);
        assert!(output
            .shapes
            .iter()
            .any(|s| text_bounds(&s.shape, &label(sim.frame())).is_some()));
    }
    advance(
        &mut sim,
        game::DefenseInput {
            action: game::RESTART,
            sequence: 1,
            ..Default::default()
        },
    );
    assert_eq!(label(sim.frame()), "INCOMING 20  /  ON ROUTE 0");
    let output = draw(&mut ui, sim.frame(), size);
    assert!(output
        .shapes
        .iter()
        .any(|s| text_bounds(&s.shape, "INCOMING 20  /  ON ROUTE 0").is_some()));
}
