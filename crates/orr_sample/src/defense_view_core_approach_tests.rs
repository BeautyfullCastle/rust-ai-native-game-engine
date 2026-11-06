//! CPU shape checks; no native window, device or video coverage.
use super::*;
type Sim = orr_sim::Simulation<game::DefenseGame>;
fn enemy(progress: u32) -> game::DefenseEnemy {
    game::DefenseEnemy {
        active: 1,
        id: 77,
        hp: 1000,
        max_hp: 1000,
        progress,
        ..Default::default()
    }
}
fn set_enemy(sim: &mut Sim, value: game::DefenseEnemy) {
    let entity = FrameView::of(sim.frame())
        .iter::<game::DefenseEnemy>()
        .next()
        .unwrap()
        .0;
    *sim.frame_mut()
        .get_mut::<game::DefenseEnemy>(entity)
        .unwrap() = value;
}
fn draw(
    ctx: &egui::Context,
    frame: &orr_ecs::Frame,
    screen: Rect,
    readonly: bool,
) -> egui::FullOutput {
    draw_clipped(ctx, frame, screen, screen, readonly)
}
fn draw_clipped(
    ctx: &egui::Context,
    frame: &orr_ecs::Frame,
    screen: Rect,
    clip: Rect,
    readonly: bool,
) -> egui::FullOutput {
    let before = (frame.tick(), frame.checksum());
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ui| {
            let painter = ui.painter().with_clip_rect(clip);
            if readonly {
                paint_view(&painter, Layout::new(screen), FrameView::of(frame), None);
            } else {
                paint(&painter, Layout::new(screen), frame, None);
            }
        },
    );
    out.textures_delta.clear();
    assert_eq!(before, (frame.tick(), frame.checksum()));
    out
}
fn count(out: &egui::FullOutput, label: &str) -> usize {
    out.shapes
        .iter()
        .filter(|s| matches!(&s.shape, Shape::Text(t) if t.galley.text() == label))
        .count()
}
fn screen() -> Rect {
    Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0))
}
#[test]
fn final_leg_boundaries_ignore_family_lane_and_slow_status() {
    for lane in 0..2 {
        for kind in 0..4 {
            for slow_ticks in [0, 100] {
                for progress in [0, 1599, 1600, 1999, 2000, u32::MAX] {
                    let e = game::DefenseEnemy {
                        lane,
                        kind,
                        slow_ticks,
                        ..enemy(progress)
                    };
                    assert_eq!(on_core_approach(&e), (1600..2000).contains(&progress));
                    assert!(!on_core_approach(&game::DefenseEnemy { active: 0, ..e }));
                }
            }
        }
    }
}
#[test]
fn real_movement_enters_warning_and_leak_removes_it() {
    let mut sim = Sim::new(game::DefenseConfig::default(), 60, 1);
    set_enemy(&mut sim, enemy(1599));
    sim.frame_mut()
        .singleton_mut::<game::DefenseRun>()
        .next_enemy_id = 100;
    let mut input = orr_sim::TickInputs::new(sim.frame().tick(), 1);
    input.set_input(orr_sim::PlayerSlot(0), game::DefenseInput::default());
    sim.step(&input);
    assert!(game::enemies(sim.frame())
        .iter()
        .any(|e| e.id == 77 && on_core_approach(e)));
    set_enemy(&mut sim, enemy(1999));
    let before = game::run(sim.frame());
    let mut input = orr_sim::TickInputs::new(sim.frame().tick(), 1);
    input.set_input(orr_sim::PlayerSlot(0), game::DefenseInput::default());
    sim.step(&input);
    let after = game::run(sim.frame());
    assert_eq!(after.leaks, before.leaks + 1);
    assert_eq!(after.reactor_hp, before.reactor_hp - 2);
    assert!(!game::enemies(sim.frame()).iter().any(on_core_approach));
}
#[test]
fn captions_and_marks_follow_active_playing_frame() {
    let mut sim = Sim::new(game::DefenseConfig::default(), 60, 1);
    let ctx = egui::Context::default();
    for progress in [1599, 1600, 1999, 2000] {
        set_enemy(&mut sim, enemy(progress));
        let out = draw(&ctx, sim.frame(), screen(), false);
        let expected = usize::from((1600..2000).contains(&progress));
        assert_eq!(count(&out, "!"), expected);
        assert_eq!(count(&out, "CORE APPROACH"), expected);
        assert_eq!(count(&out, "SHARED STORMGLASS CORE"), 1 - expected);
    }
    for phase in [game::WON, game::LOST, u32::MAX] {
        set_enemy(&mut sim, enemy(1800));
        sim.frame_mut().singleton_mut::<game::DefenseRun>().phase = phase;
        assert_eq!(count(&draw(&ctx, sim.frame(), screen(), false), "!"), 0);
    }
}
#[test]
fn both_lanes_and_boss_sizes_use_actual_positions_at_three_aspects() {
    let mut sim = Sim::new(game::DefenseConfig::default(), 60, 1);
    for size in [vec2(1200.0, 940.0), vec2(320.0, 230.0), vec2(900.0, 180.0)] {
        let screen = Rect::from_min_size(Pos2::new(37.0, 91.0), size);
        let layout = Layout::new(screen);
        for lane in 0..2 {
            for kind in [0, game::BOSS] {
                set_enemy(
                    &mut sim,
                    game::DefenseEnemy {
                        lane,
                        kind,
                        ..enemy(1900)
                    },
                );
                let out = draw(&egui::Context::default(), sim.frame(), screen, false);
                assert_eq!(count(&out, "!"), 1);
                let (x, y) = game::enemy_position(lane, 1900);
                let center = layout.world(x, y);
                let radius = (if kind == game::BOSS { 24.0 } else { 12.0 }) * layout.scale;
                assert!(out.shapes.iter().any(|s| matches!(&s.shape, Shape::Circle(c) if c.center == center && (c.radius-(radius*1.6+5.0*layout.scale)).abs()<0.001 && c.stroke.color==C::LIGHT_RED)));
            }
        }
    }
}
#[test]
fn native_and_readonly_paths_match_without_mutation() {
    let mut sim = Sim::new(game::DefenseConfig::default(), 60, 1);
    set_enemy(&mut sim, enemy(1999));
    let a = draw(&egui::Context::default(), sim.frame(), screen(), false);
    let b = draw(&egui::Context::default(), sim.frame(), screen(), true);
    assert_eq!(format!("{:?}", a.shapes), format!("{:?}", b.shapes));
}
#[test]
fn same_tick_replacement_rewind_and_paused_redraw_have_no_cached_warning() {
    let mut sim = Sim::new(game::DefenseConfig::default(), 60, 1);
    let ctx = egui::Context::default();
    let tick = sim.frame().tick();
    for progress in [1800, 1800, 1599, 1800, 0] {
        set_enemy(&mut sim, enemy(progress));
        assert_eq!(sim.frame().tick(), tick);
        assert_eq!(
            count(&draw(&ctx, sim.frame(), screen(), false), "!"),
            usize::from(progress >= 1600)
        );
    }
}
#[test]
fn warning_overlays_core_shell_and_inherits_parent_clip() {
    let mut sim = Sim::new(game::DefenseConfig::default(), 60, 1);
    set_enemy(
        &mut sim,
        game::DefenseEnemy {
            lane: 1,
            ..enemy(1999)
        },
    );
    let out = draw(&egui::Context::default(), sim.frame(), screen(), false);
    let shell = out
        .shapes
        .iter()
        .position(|s| matches!(&s.shape,Shape::Circle(c) if c.fill==C::from_rgb(37,54,51)))
        .unwrap();
    let warning = out
        .shapes
        .iter()
        .position(|s| matches!(&s.shape,Shape::Text(t) if t.galley.text()=="!"))
        .unwrap();
    let halo = out
        .shapes
        .iter()
        .position(|s| matches!(&s.shape,Shape::Circle(c) if c.stroke.color == C::LIGHT_RED))
        .unwrap();
    assert!(halo > shell && warning > halo);
    let bounds = out.shapes[halo].shape.visual_bounding_rect();
    let clip = Rect::from_min_max(bounds.center(), bounds.max);
    let partial = draw_clipped(
        &egui::Context::default(),
        sim.frame(),
        screen(),
        clip,
        false,
    );
    let ring = partial
        .shapes
        .iter()
        .find(|s| matches!(&s.shape,Shape::Circle(c) if c.stroke.color == C::LIGHT_RED))
        .unwrap();
    assert_eq!(ring.clip_rect, clip);
    assert!(ring.shape.visual_bounding_rect().intersects(clip));
    assert!(!clip.contains_rect(ring.shape.visual_bounding_rect()));
    assert_eq!(out.shapes[warning].clip_rect, screen());
}
