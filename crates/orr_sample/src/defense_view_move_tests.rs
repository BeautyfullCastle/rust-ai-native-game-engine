//! CPU-only move affordance and actual game admission; no native window/video.
use super::*;
type Sim = orr_sim::Simulation<game::DefenseGame>;
fn unit(board: u32, cell: u32, active: u32) -> game::DefenseUnit {
    game::DefenseUnit {
        board,
        cell,
        active,
        role: game::RIVET,
        tier: game::MAX_TIER,
        id: 1 + board * game::CELLS_PER_BOARD + cell,
        ..Default::default()
    }
}
fn fixture() -> Sim {
    let mut sim = Sim::new(game::DefenseConfig::default(), 60, 1);
    let entries: Vec<_> = FrameView::of(sim.frame())
        .iter::<game::DefenseUnit>()
        .map(|(e, u)| (e, *u))
        .collect();
    for (entity, old) in entries {
        if [(0, 0), (0, 1), (0, 3), (1, 2)].contains(&(old.board, old.cell)) {
            *sim.frame_mut()
                .get_mut::<game::DefenseUnit>(entity)
                .unwrap() = unit(old.board, old.cell, 1);
        }
    }
    sim.frame_mut()
        .singleton_mut::<game::DefenseRun>()
        .next_unit_id = 100;
    sim
}
fn change(frame: &mut orr_ecs::Frame, cell: u32, active: u32) {
    let entity = FrameView::of(frame)
        .iter::<game::DefenseUnit>()
        .find(|(_, u)| u.board == 0 && u.cell == cell)
        .unwrap()
        .0;
    *frame.get_mut::<game::DefenseUnit>(entity).unwrap() = unit(0, cell, active);
}
fn draw(
    ctx: &egui::Context,
    frame: &orr_ecs::Frame,
    selected: Option<u32>,
    screen: Rect,
    clip: Rect,
    skin: bool,
    readonly: bool,
) -> egui::FullOutput {
    let before = (frame.tick(), frame.checksum());
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ui| {
            let painter = ui.painter().with_clip_rect(clip);
            let layout = Layout::new(screen);
            if skin {
                paint_view_with_unit_skin(
                    &painter,
                    layout,
                    FrameView::of(frame),
                    selected,
                    |p, _, _, _| {
                        p.rect_filled(p.clip_rect(), 0.0, C::from_rgb(230, 0, 230));
                        true
                    },
                );
            } else if readonly {
                paint_view(&painter, layout, FrameView::of(frame), selected);
            } else {
                paint(&painter, layout, frame, selected);
            }
        },
    );
    output.textures_delta.clear();
    assert_eq!((frame.tick(), frame.checksum()), before);
    output
}
fn labels(output: &egui::FullOutput) -> Vec<&egui::epaint::ClippedShape> {
    output
        .shapes
        .iter()
        .filter(|s| matches!(&s.shape,Shape::Text(t) if t.galley.text()=="MOVE"))
        .collect()
}
fn has(output: &egui::FullOutput, layout: Layout, cell: u32) -> bool {
    labels(output)
        .iter()
        .any(|s| s.clip_rect == layout.cell(0, cell))
}

#[test]
fn only_existing_empty_local_slots_are_move_destinations_even_at_max_tier() {
    let units = [unit(0, 0, 1), unit(0, 1, 1), unit(0, 2, 0), unit(1, 3, 0)];
    assert_eq!(available_move_cells(&units, Some(0)), vec![2]);
    assert!(compatible_merge_cells(&units, Some(0)).is_empty());
    assert!(available_move_cells(&units, None).is_empty());
    assert!(available_move_cells(&units, Some(2)).is_empty());
    assert!(available_move_cells(&units, Some(game::CELLS_PER_BOARD)).is_empty());
}

#[test]
fn missing_and_conflicting_duplicate_slots_are_never_guessed_empty() {
    let units = [
        unit(0, 0, 1),
        unit(0, 2, 0),
        unit(0, 2, 1),
        unit(0, 4, 0),
        unit(0, 4, 0),
    ];
    assert_eq!(available_move_cells(&units, Some(0)), vec![4]);
    assert!(available_move_cells(&[unit(0, 0, 0), unit(0, 1, 0)], Some(0)).is_empty());
}

#[test]
fn actual_game_moves_a_max_tier_actor_without_merging_or_changing_its_identity() {
    let mut sim = fixture();
    let before = game::units(sim.frame())
        .into_iter()
        .find(|u| u.board == 0 && u.cell == 0)
        .unwrap();
    assert!(available_move_cells(&game::units(sim.frame()), Some(0)).contains(&2));
    let mut inputs = orr_sim::TickInputs::new(sim.frame().tick(), 1);
    inputs.set_input(
        orr_sim::PlayerSlot(0),
        game::DefenseInput {
            action: game::MOVE_OR_MERGE,
            source_cell: 0,
            target_cell: 2,
            sequence: 1,
        },
    );
    sim.step(&inputs);
    let units = game::units(sim.frame());
    let moved = units.iter().find(|u| u.board == 0 && u.cell == 2).unwrap();
    assert_eq!(
        units
            .iter()
            .find(|u| u.board == 0 && u.cell == 0)
            .unwrap()
            .active,
        0
    );
    assert_eq!(
        (moved.id, moved.role, moved.tier),
        (before.id, before.role, before.tier)
    );
    assert_eq!(game::run(sim.frame()).merges, 0);
    assert!(available_move_cells(&units, Some(0)).is_empty());
    assert!(available_move_cells(&units, Some(2)).contains(&0));
}

#[test]
fn responsive_labels_are_local_empty_cells_and_above_opaque_custom_skins() {
    let sim = fixture();
    for size in [vec2(1200.0, 940.0), vec2(320.0, 230.0), vec2(900.0, 180.0)] {
        let screen = Rect::from_min_size(egui::pos2(37.0, 91.0), size);
        let layout = Layout::new(screen);
        for skin in [false, true] {
            let output = draw(
                &egui::Context::default(),
                sim.frame(),
                Some(0),
                screen,
                screen,
                skin,
                true,
            );
            assert_eq!(labels(&output).len(), 9);
            for cell in 0..game::CELLS_PER_BOARD {
                assert_eq!(has(&output, layout, cell), ![0, 1, 3].contains(&cell));
            }
            for label in labels(&output) {
                assert!(label
                    .clip_rect
                    .contains_rect(label.shape.visual_bounding_rect()));
            }
            if skin {
                let last_skin = output
                    .shapes
                    .iter()
                    .rposition(
                        |s| matches!(&s.shape,Shape::Rect(r) if r.fill==C::from_rgb(230,0,230)),
                    )
                    .unwrap();
                let first_move = output
                    .shapes
                    .iter()
                    .position(|s| matches!(&s.shape,Shape::Text(t) if t.galley.text()=="MOVE"))
                    .unwrap();
                assert!(first_move > last_skin);
            }
        }
    }
}

#[test]
fn readonly_and_native_wrappers_keep_identical_move_labels() {
    let sim = fixture();
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0));
    let context = egui::Context::default();
    let native = draw(&context, sim.frame(), Some(0), screen, screen, false, false);
    let readonly = draw(&context, sim.frame(), Some(0), screen, screen, false, true);
    assert_eq!(native.shapes, readonly.shapes);
}

#[test]
fn same_tick_snapshot_replacement_and_terminal_phase_do_not_keep_stale_moves() {
    let sim = fixture();
    let original = sim.frame().clone();
    let mut filled = original.clone();
    change(&mut filled, 2, 1);
    let context = egui::Context::default();
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0));
    let layout = Layout::new(screen);
    for (frame, expected) in [(&original, true), (&filled, false), (&original, true)] {
        assert_eq!(
            has(
                &draw(&context, frame, Some(0), screen, screen, false, true),
                layout,
                2
            ),
            expected
        );
    }
    for phase in [game::WON, game::LOST, u32::MAX] {
        filled = original.clone();
        filled.singleton_mut::<game::DefenseRun>().phase = phase;
        assert!(labels(&draw(
            &context,
            &filled,
            Some(0),
            screen,
            screen,
            false,
            true
        ))
        .is_empty());
    }
}

#[test]
fn partial_and_hidden_parent_clips_never_expose_other_empty_cells() {
    let sim = fixture();
    let context = egui::Context::default();
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0));
    let target = Layout::new(screen).cell(0, 2);
    let partial = Rect::from_min_max(target.center(), target.right_bottom());
    let output = draw(&context, sim.frame(), Some(0), screen, partial, false, true);
    assert_eq!(labels(&output).len(), 1);
    assert!(labels(&output)
        .iter()
        .all(|s| s.clip_rect == partial.intersect(target)));
    assert!(labels(&draw(
        &context,
        sim.frame(),
        Some(0),
        screen,
        Layout::new(screen).cell(0, 0),
        false,
        true
    ))
    .is_empty());
}
