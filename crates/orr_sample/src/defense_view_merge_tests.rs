//! CPU geometry and actual simulation admission, not a native/GPU playtest.
use super::*;
type Sim = orr_sim::Simulation<game::DefenseGame>;
fn unit(board: u32, cell: u32, role: u32, tier: u32) -> game::DefenseUnit {
    game::DefenseUnit {
        active: 1,
        board,
        cell,
        role,
        tier,
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
        let value = match (old.board, old.cell) {
            (0, 0) | (0, 1) | (1, 1) => Some(unit(old.board, old.cell, game::RIVET, 2)),
            (0, 2) => Some(unit(0, 2, game::MORTAR, 2)),
            _ => None,
        };
        if let Some(value) = value {
            *sim.frame_mut()
                .get_mut::<game::DefenseUnit>(entity)
                .unwrap() = value;
        }
    }
    sim.frame_mut()
        .singleton_mut::<game::DefenseRun>()
        .next_unit_id = 100;
    sim
}
fn change_unit(frame: &mut orr_ecs::Frame, cell: u32, f: impl FnOnce(&mut game::DefenseUnit)) {
    let entity = FrameView::of(frame)
        .iter::<game::DefenseUnit>()
        .find(|(_, u)| u.board == 0 && u.cell == cell)
        .unwrap()
        .0;
    f(frame.get_mut::<game::DefenseUnit>(entity).unwrap());
}
fn draw(
    frame: &orr_ecs::Frame,
    selected: Option<u32>,
    screen: Rect,
    custom_skin: bool,
    readonly: bool,
) -> egui::FullOutput {
    let context = egui::Context::default();
    draw_with_context(&context, frame, selected, screen, custom_skin, readonly)
}
fn draw_with_context(
    context: &egui::Context,
    frame: &orr_ecs::Frame,
    selected: Option<u32>,
    screen: Rect,
    custom_skin: bool,
    readonly: bool,
) -> egui::FullOutput {
    let before = (frame.tick(), frame.checksum());
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ui| {
            let painter = ui.painter().with_clip_rect(screen);
            let layout = Layout::new(screen);
            if custom_skin {
                paint_view_with_unit_skin(
                    &painter,
                    layout,
                    FrameView::of(frame),
                    selected,
                    |p, center, scale, _| {
                        // Opaque fixture skin proves the later guide isn't covered by the skin.
                        p.rect_filled(
                            Rect::from_center_size(center, vec2(90.0, 90.0) * scale),
                            0.0,
                            C::from_rgb(230, 0, 230),
                        );
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
fn has_guide(output: &egui::FullOutput, layout: Layout, cell: u32) -> bool {
    output.shapes.iter().any(|s| {
        matches!(&s.shape, Shape::Rect(r)
        if r.rect == layout.cell(0,cell).shrink(6.0*layout.scale)
            && r.stroke == Stroke::new(2.0*layout.scale,TEAL))
    })
}

#[test]
fn matching_roles_and_tiers_only_mark_other_active_local_cells() {
    let mut units = vec![
        unit(0, 0, game::RIVET, 2),
        unit(0, 1, game::RIVET, 2),
        unit(1, 1, game::RIVET, 2),
        unit(0, 2, game::MORTAR, 2),
        unit(0, 3, game::RIVET, 3),
        unit(0, 4, game::RIVET, 2),
    ];
    units.last_mut().unwrap().active = 0;
    assert_eq!(compatible_merge_cells(&units, Some(0)), vec![1]);
    assert!(compatible_merge_cells(&units, None).is_empty());
    assert!(compatible_merge_cells(&units, Some(game::CELLS_PER_BOARD)).is_empty());
    assert!(compatible_merge_cells(&units, Some(4)).is_empty());
}

#[test]
fn support_roles_are_mergeable_but_max_tier_and_duplicate_slots_are_safe() {
    for role in game::RIVET..=game::MINER {
        let units = [
            unit(0, 0, role, game::MAX_TIER - 1),
            unit(0, 2, role, game::MAX_TIER - 1),
            unit(0, 1, role, game::MAX_TIER - 1),
            unit(0, 1, role, game::MAX_TIER - 1),
        ];
        assert_eq!(compatible_merge_cells(&units, Some(0)), vec![1, 2]);
        let max = [
            unit(0, 0, role, game::MAX_TIER),
            unit(0, 1, role, game::MAX_TIER),
        ];
        assert!(compatible_merge_cells(&max, Some(0)).is_empty());
    }
}

#[test]
fn suggested_pair_is_accepted_by_real_game_and_disappears_after_merge() {
    let mut sim = fixture();
    assert_eq!(
        compatible_merge_cells(&game::units(sim.frame()), Some(0)),
        vec![1]
    );
    let mut inputs = orr_sim::TickInputs::new(sim.frame().tick(), 1);
    inputs.set_input(
        orr_sim::PlayerSlot(0),
        game::DefenseInput {
            action: game::MOVE_OR_MERGE,
            source_cell: 0,
            target_cell: 1,
            sequence: 1,
        },
    );
    sim.step(&inputs);
    let units = game::units(sim.frame());
    assert_eq!(
        units
            .iter()
            .find(|u| u.board == 0 && u.cell == 0)
            .unwrap()
            .active,
        0
    );
    assert_eq!(
        units
            .iter()
            .find(|u| u.board == 0 && u.cell == 1)
            .unwrap()
            .tier,
        3
    );
    assert!(compatible_merge_cells(&units, Some(0)).is_empty());
}

#[test]
fn full_painter_guides_stay_in_local_cells_and_survive_custom_skins_responsively() {
    let sim = fixture();
    for size in [vec2(1200.0, 940.0), vec2(320.0, 230.0), vec2(900.0, 180.0)] {
        let screen = Rect::from_min_size(egui::pos2(37.0, 91.0), size);
        let layout = Layout::new(screen);
        for skin in [false, true] {
            let output = draw(sim.frame(), Some(0), screen, skin, true);
            assert!(has_guide(&output, layout, 1));
            assert!(!has_guide(&output, layout, 0));
            assert!(!has_guide(&output, layout, 2));
            let target = layout.cell(0, 1);
            let marked = output.shapes.iter().find(|s| matches!(&s.shape, Shape::Rect(r) if r.rect == target.shrink(6.0*layout.scale) && r.stroke.color==TEAL)).unwrap();
            assert_eq!(marked.clip_rect, screen.intersect(target));
            if skin {
                let skin_index = output
                    .shapes
                    .iter()
                    .rposition(
                        |s| matches!(&s.shape,Shape::Rect(r) if r.fill==C::from_rgb(230,0,230)),
                    )
                    .unwrap();
                let guide_index = output
                    .shapes
                    .iter()
                    .position(|s| std::ptr::eq(s, marked))
                    .unwrap();
                assert!(
                    guide_index > skin_index,
                    "guide was hidden behind custom skin"
                );
            }
        }
    }
}

#[test]
fn frame_and_readonly_wrappers_produce_identical_candidate_shapes() {
    let sim = fixture();
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0));
    let native = draw(sim.frame(), Some(0), screen, false, false);
    let readonly = draw(sim.frame(), Some(0), screen, false, true);
    assert_eq!(native.shapes, readonly.shapes);
}

#[test]
fn embedded_partial_and_hidden_parent_clips_never_leak_candidate_outlines() {
    let sim = fixture();
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0));
    let layout = Layout::new(screen);
    let target = layout.cell(0, 1);
    let partial = Rect::from_min_max(target.center(), target.right_bottom());
    let context = egui::Context::default();
    let render = |clip| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| {
                paint_view(
                    &ui.painter().with_clip_rect(clip),
                    layout,
                    FrameView::of(sim.frame()),
                    Some(0),
                );
            },
        );
        output.textures_delta.clear();
        output
    };
    let before = sim.frame().checksum();
    let output = render(partial);
    let guide = output
        .shapes
        .iter()
        .find(|s| {
            matches!(&s.shape,Shape::Rect(r)
        if r.rect==target.shrink(6.0*layout.scale) && r.stroke==Stroke::new(2.0*layout.scale,TEAL))
        })
        .unwrap();
    assert_eq!(guide.clip_rect, partial.intersect(target));
    assert!(!has_guide(&render(layout.cell(0, 0)), layout, 1));
    assert_eq!(sim.frame().checksum(), before);
}

#[test]
fn changed_snapshot_rewind_and_terminal_phase_never_keep_stale_guides() {
    let sim = fixture();
    let original = sim.frame().clone();
    let mut changed = original.clone();
    change_unit(&mut changed, 1, |u| u.role = game::MORTAR);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0));
    let layout = Layout::new(screen);
    let context = egui::Context::default();
    for (frame, expected) in [(&original, true), (&changed, false), (&original, true)] {
        assert_eq!(
            has_guide(
                &draw_with_context(&context, frame, Some(0), screen, false, true),
                layout,
                1
            ),
            expected
        );
    }
    for phase in [game::WON, game::LOST] {
        changed = original.clone();
        changed.singleton_mut::<game::DefenseRun>().phase = phase;
        assert!(!has_guide(
            &draw(&changed, Some(0), screen, false, true),
            layout,
            1
        ));
    }
}
