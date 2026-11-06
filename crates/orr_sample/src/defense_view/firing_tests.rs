use super::*;

fn actor() -> game::DefenseUnit {
    game::DefenseUnit {
        active: 1,
        id: 41,
        role: game::RIVET,
        tier: 1,
        ..Default::default()
    }
}

fn shot_at(unit: &game::DefenseUnit) -> game::DefenseProjectile {
    let (x, y) = game::board_cell_position(unit.board, unit.cell);
    game::DefenseProjectile {
        active: 1,
        id: 7,
        source_unit_id: unit.id,
        board: unit.board,
        role: unit.role,
        x,
        y,
        ..Default::default()
    }
}

#[test]
fn neighboring_actor_projectile_passing_through_does_not_fire_this_actor() {
    let unit = actor();
    let mut shot = shot_at(&unit);
    shot.source_unit_id = unit.id + 1;
    for offset in [-24, 0, 24] {
        shot.x = game::board_cell_position(unit.board, unit.cell).0 + offset;
        assert!(!unit_has_nearby_projectile(&unit, &[shot]));
    }
}

#[test]
fn own_live_projectile_requires_role_board_and_strict_manhattan_proximity() {
    let unit = actor();
    let shot = shot_at(&unit);
    assert!(unit_has_nearby_projectile(&unit, &[shot]));
    assert!(!unit_has_nearby_projectile(&unit, &[]));
    for invalid in [
        game::DefenseProjectile { active: 0, ..shot },
        game::DefenseProjectile {
            source_unit_id: 0,
            ..shot
        },
        game::DefenseProjectile {
            role: game::MORTAR,
            ..shot
        },
        game::DefenseProjectile { board: 1, ..shot },
        game::DefenseProjectile {
            x: shot.x + 25,
            ..shot
        },
        game::DefenseProjectile {
            x: shot.x - 12,
            y: shot.y + 13,
            ..shot
        },
    ] {
        assert!(!unit_has_nearby_projectile(&unit, &[invalid]));
    }
    assert!(unit_has_nearby_projectile(
        &unit,
        &[
            game::DefenseProjectile { active: 0, ..shot },
            game::DefenseProjectile {
                x: shot.x - 12,
                y: shot.y + 12,
                ..shot
            },
        ]
    ));
}

#[test]
fn absent_or_inactive_actor_never_fires_even_with_matching_source() {
    for unit in [
        game::DefenseUnit { id: 0, ..actor() },
        game::DefenseUnit {
            active: 0,
            ..actor()
        },
    ] {
        assert!(!unit_has_nearby_projectile(&unit, &[shot_at(&unit)]));
    }
}

#[test]
fn moved_actor_requires_its_projectile_near_its_present_cell() {
    let original = actor();
    let moved = game::DefenseUnit {
        cell: 1,
        ..original
    };
    assert!(!unit_has_nearby_projectile(&moved, &[shot_at(&original)]));
    assert!(unit_has_nearby_projectile(&moved, &[shot_at(&moved)]));
    let replacement = game::DefenseUnit { id: 99, ..original };
    assert!(!unit_has_nearby_projectile(
        &replacement,
        &[shot_at(&original)]
    ));
}

#[test]
fn merge_destination_keeps_only_its_own_projectile_cue() {
    // Post-merge snapshot: source is inactive, destination identity survives.
    let source = game::DefenseUnit {
        active: 0,
        ..actor()
    };
    let destination = game::DefenseUnit {
        id: 42,
        cell: 1,
        tier: 2,
        ..actor()
    };
    let own = shot_at(&destination);
    let consumed = game::DefenseProjectile {
        source_unit_id: source.id,
        ..own
    };
    assert!(!unit_has_nearby_projectile(&source, &[shot_at(&source)]));
    assert!(!unit_has_nearby_projectile(&destination, &[consumed]));
    assert!(unit_has_nearby_projectile(&destination, &[own]));
}

// Full frame/egui checks below require Cargo and are not part of the extracted
// rustc-only predicate harness used in resource-constrained environments.
#[test]
fn snapshot_restore_and_both_skin_paths_are_readonly_and_share_the_cue() {
    let sim = orr_sim::Simulation::<game::DefenseGame>::new(game::DefenseConfig::default(), 60, 1);
    let mut frame = sim.frame().clone();
    let unit = actor();
    let unit_entity = frame
        .entities()
        .find(|&e| {
            frame
                .get::<game::DefenseUnit>(e)
                .is_some_and(|u| u.board == 0 && u.cell == 0)
        })
        .unwrap();
    let shot_entity = frame
        .entities()
        .find(|&e| frame.has::<game::DefenseProjectile>(e))
        .unwrap();
    *frame.get_mut::<game::DefenseUnit>(unit_entity).unwrap() = unit;
    *frame
        .get_mut::<game::DefenseProjectile>(shot_entity)
        .unwrap() = shot_at(&unit);
    let snapshot = frame.to_bytes();
    let snapshot_checksum = frame.checksum();
    let context = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0));
    let layout = Layout::new(rect);
    let cue_center = layout.cell(unit.board, unit.cell).center() - vec2(0.0, 25.0);
    let assert_paint = |frame: &orr_ecs::Frame, expected: usize| {
        let bytes = frame.to_bytes();
        let checksum = frame.checksum();
        for skinned in [false, true] {
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                paint_view_with_unit_skin(
                    &ui.painter().with_clip_rect(rect),
                    layout,
                    FrameView::of(frame),
                    None,
                    |_, _, _, _| skinned,
                );
            });
            let rings = output
                .shapes
                .iter()
                .filter(|shape| {
                    matches!(&shape.shape, Shape::Circle(circle)
                    if circle.center == cue_center && circle.radius == 11.0
                        && circle.stroke == Stroke::new(2.0, PAPER))
                })
                .count();
            assert_eq!(rings, expected, "skin replacement enabled: {skinned}");
            assert_eq!(frame.to_bytes(), bytes);
            assert_eq!(frame.checksum(), checksum);
        }
    };
    assert_paint(&frame, 1);
    frame
        .get_mut::<game::DefenseProjectile>(shot_entity)
        .unwrap()
        .source_unit_id += 1;
    assert_paint(&frame, 0);
    let restored = orr_ecs::Frame::from_bytes(frame.registry().clone(), &snapshot).unwrap();
    frame.copy_from(&restored);
    assert_eq!(frame.checksum(), snapshot_checksum);
    assert_eq!(frame.to_bytes(), snapshot);
    assert_paint(&frame, 1);
    frame
        .get_mut::<game::DefenseProjectile>(shot_entity)
        .unwrap()
        .active = 0;
    assert_paint(&frame, 0);
    frame.copy_from(&restored);
    assert_paint(&frame, 1);
}

#[test]
fn simulation_move_and_merge_preserve_only_the_destination_identity() {
    let mut sim =
        orr_sim::Simulation::<game::DefenseGame>::new(game::DefenseConfig::default(), 60, 1);
    let source = actor();
    let destination = game::DefenseUnit {
        id: 42,
        cell: 2,
        ..source
    };
    let entities: Vec<_> = sim.frame().entities().collect();
    for entity in entities {
        if let Some(unit) = sim.frame_mut().get_mut::<game::DefenseUnit>(entity) {
            if unit.board == 0 && unit.cell == source.cell {
                *unit = source;
            } else if unit.board == 0 && unit.cell == destination.cell {
                *unit = destination;
            }
        }
    }
    for (tick, source_cell, target_cell) in [(0, 0, 1), (1, 1, 2)] {
        let mut inputs = orr_sim::TickInputs::new(tick, 1);
        inputs.set_input(
            orr_sim::PlayerSlot(0),
            game::DefenseInput {
                action: game::MOVE_OR_MERGE,
                sequence: tick as u32 + 1,
                source_cell,
                target_cell,
                ..Default::default()
            },
        );
        sim.step(&inputs);
        let units = game::units(sim.frame());
        let moved_or_merged = units
            .iter()
            .find(|u| u.board == 0 && u.cell == target_cell)
            .unwrap();
        assert_ne!(moved_or_merged.active, 0);
        assert_eq!(
            moved_or_merged.id,
            if tick == 0 { source.id } else { destination.id }
        );
        assert!(unit_has_nearby_projectile(
            moved_or_merged,
            &[shot_at(moved_or_merged)]
        ));
        if tick == 1 {
            assert_eq!(moved_or_merged.tier, 2);
            let consumed = units
                .iter()
                .find(|u| u.board == 0 && u.cell == source_cell)
                .unwrap();
            assert_eq!(consumed.active, 0);
            assert!(!unit_has_nearby_projectile(consumed, &[shot_at(consumed)]));
            let stale = game::DefenseProjectile {
                source_unit_id: source.id,
                ..shot_at(moved_or_merged)
            };
            assert!(!unit_has_nearby_projectile(moved_or_merged, &[stale]));
        }
    }
}
