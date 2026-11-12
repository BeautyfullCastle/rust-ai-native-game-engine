use super::*;
use orr_games::coop_defense_game::{
    DefenseConfig, DefenseGame, DefenseProjectile, MAX_PROJECTILES, MOVE_OR_MERGE, RESTART, SUMMON,
};
use orr_sim::{PlayerSlot, SimCommand, Simulation, TickInputs};
use orr_viewstream::{FLAG_DISCONTINUITY, FLAG_EVENTS_RESET, FLAG_PAUSED, FLAG_ROLLED_BACK};
use std::collections::BTreeSet;

fn simulation() -> Simulation<DefenseGame> {
    let config = DefenseConfig::default();
    Simulation::new(config, TICK_RATE, u64::from(config.seed))
}

fn step(sim: &mut Simulation<DefenseGame>, input: DefenseInput) {
    let mut inputs = TickInputs::new(sim.tick(), 1);
    inputs.set_input(PlayerSlot(0), input);
    sim.step(&inputs);
}

fn decode(producer: &mut DefenseStreamProducer, frame: &Frame) -> ViewFrame {
    let meta = FrameMeta {
        tick: frame.tick(),
        verified_tick: frame.tick(),
        ..Default::default()
    };
    ViewFrame::decode(&producer.encode_frame(frame, None, meta)).unwrap()
}

fn properties(frame: &ViewFrame, schema: &Schema) -> Vec<Vec<u32>> {
    frame
        .props_by_entity(|kind| schema.props_words(kind))
        .expect("schema must account for every property byte")
        .into_iter()
        .map(|bytes| {
            bytes
                .chunks_exact(4)
                .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
                .collect()
        })
        .collect()
}

fn record(frame: &ViewFrame, schema: &Schema, id: Entity) -> (EntityRecord, Vec<u32>) {
    let index = frame
        .entities
        .iter()
        .position(|record| record.id == entity_id(id))
        .unwrap();
    (
        frame.entities[index],
        properties(frame, schema)[index].clone(),
    )
}

fn cells(frame: &Frame) -> Vec<Entity> {
    frame
        .entities()
        .filter(|id| frame.get::<DefenseUnit>(*id).is_some())
        .collect()
}

fn enemy_slots(frame: &Frame) -> Vec<Entity> {
    frame
        .entities()
        .filter(|id| frame.get::<DefenseEnemy>(*id).is_some())
        .collect()
}

#[test]
fn schema_preserves_identity_input_offsets_command_and_opaque_event_layout() {
    let producer = DefenseStreamProducer::new();
    let schema = producer.schema();
    assert_eq!(schema.game, "CoopDefense");
    assert_eq!(schema.dimensions, 2);
    assert_eq!(schema.build_id, SCENARIO_BUILD_ID);
    assert_eq!(schema.tick_rate, 60);
    assert_eq!(schema.player_count, 1);
    assert_eq!(schema.input.size, 16);
    assert_eq!(schema.input.size, std::mem::size_of::<DefenseInput>());
    let offsets = [
        ("action", std::mem::offset_of!(DefenseInput, action)),
        (
            "source_cell",
            std::mem::offset_of!(DefenseInput, source_cell),
        ),
        (
            "target_cell",
            std::mem::offset_of!(DefenseInput, target_cell),
        ),
        ("sequence", std::mem::offset_of!(DefenseInput, sequence)),
    ];
    assert_eq!(schema.input.fields.len(), offsets.len());
    for (field, (name, offset)) in schema.input.fields.iter().zip(offsets) {
        assert_eq!(field["name"], name);
        assert_eq!(field["type"], "u32");
        assert_eq!(field["offset"], offset);
        assert_eq!(field["size"], 4);
    }
    let mut command = Vec::new();
    NoCommand::default().encode(&mut command);
    assert_eq!(command, [0; 4]);
    assert_eq!(schema.command_size, command.len());
    assert_eq!(
        schema.events,
        [EventDef {
            id: 0,
            name: "defense_event_opaque".into(),
            payload_size: 48,
        }]
    );
    assert_eq!(
        schema.events[0].payload_size,
        std::mem::size_of::<DefenseEvent>()
    );
    assert_eq!(
        producer.event_type(bytemuck::bytes_of(&DefenseEvent::default())),
        0
    );
    let wire = schema.to_json();
    assert_eq!(wire["world"]["units"], "game units, y up");
    assert_eq!(wire["command"]["size"], 4);
    assert_eq!(wire["events"][0]["payload_size"], 48);
}

#[test]
fn schema_names_every_kind_and_exact_u32_properties() {
    let schema = schema();
    assert_eq!(
        schema
            .kinds
            .iter()
            .map(|kind| kind.name.as_str())
            .collect::<Vec<_>>(),
        [
            "run",
            "cell.empty",
            "cell.rivet",
            "cell.mortar",
            "cell.prism",
            "cell.condenser",
            "cell.haste",
            "cell.miner",
            "enemy.normal",
            "enemy.fast",
            "enemy.armored",
            "enemy.horde",
            "enemy.boss",
        ]
    );
    assert_eq!(
        schema.kinds.iter().map(|kind| kind.id).collect::<Vec<_>>(),
        (0..13).collect::<Vec<_>>()
    );
    for kind in &schema.kinds {
        let expected: &[&str] = match kind.id {
            0 => &[
                "playing",
                "won",
                "lost",
                "run_tick",
                "generation",
                "last_sequence",
                "wave",
                "last_wave",
                "reactor_hp",
                "max_reactor_hp",
                "gold_board_0",
                "gold_board_1",
                "investment_board_0",
                "investment_board_1",
                "upgrades_board_0",
                "upgrades_board_1",
                "board_cols",
                "board_rows",
                "checksum_lo",
                "checksum_hi",
            ],
            1..=7 => &["board", "cell", "unit_id", "tier", "cooldown"],
            8..=12 => &["enemy_id", "lane", "progress", "hp", "max_hp", "slow_ticks"],
            _ => unreachable!(),
        };
        assert_eq!(
            kind.props
                .iter()
                .map(|prop| prop.name.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(kind.props.iter().all(|prop| prop.ty == PropType::U32));
    }
}

#[test]
fn initial_projection_has_unique_real_cell_ids_exact_checksum_and_no_enemy_or_projectile_records() {
    let sim = simulation();
    let mut producer = DefenseStreamProducer::new();
    let frame = decode(&mut producer, sim.frame());
    assert_eq!(frame.entities.len(), 25);
    let props = properties(&frame, producer.schema());
    assert_eq!(frame.props.len(), (20 + 24 * 5) * 4);
    let checksum = sim.frame().checksum();
    assert_eq!(
        props[0],
        [
            1,
            0,
            0,
            0,
            0,
            0,
            1,
            10,
            30,
            30,
            150,
            150,
            0,
            0,
            0,
            0,
            4,
            3,
            checksum as u32,
            (checksum >> 32) as u32,
        ]
    );
    assert_eq!(
        u64::from(props[0][18]) | (u64::from(props[0][19]) << 32),
        checksum
    );
    let run = frame.entities[0];
    assert_eq!(run.id, entity_id(Entity::NONE));
    assert_eq!(run.id, 0x0000_0000_ffff_ffff);
    assert_eq!(run.kind, RUN);
    assert_eq!((run.size, run.half_y, run.rgba), (0.0, 0.0, [0; 4]));
    let source_ids: Vec<_> = cells(sim.frame()).into_iter().map(entity_id).collect();
    assert_eq!(
        frame.entities[1..]
            .iter()
            .map(|record| record.id)
            .collect::<Vec<_>>(),
        source_ids
    );
    let ids: BTreeSet<_> = frame.entities.iter().map(|record| record.id).collect();
    assert_eq!(ids.len(), frame.entities.len());
    for (index, record) in frame.entities.iter().enumerate().skip(1) {
        let board = (index - 1) as u32 / CELLS_PER_BOARD;
        let cell = (index - 1) as u32 % CELLS_PER_BOARD;
        let (x, y) = board_cell_position(board, cell);
        assert_eq!(record.kind, EMPTY);
        assert_eq!(props[index], [board, cell, 0, 0, 0]);
        assert_eq!(record.cur, [x as f32, (-y) as f32, 0.0]);
        assert_eq!(record.shape, SHAPE_QUAD);
        assert_eq!(record.mode, MODE_NONE);
        assert_eq!(record.prev, record.cur);
    }
}

#[test]
fn every_actor_kind_preserves_full_width_properties_and_normalizes_inactive_cells() {
    let mut sim = simulation();
    let cells = cells(sim.frame());
    for (index, id) in cells.iter().take(6).enumerate() {
        let unit = sim.frame_mut().get_mut::<DefenseUnit>(*id).unwrap();
        unit.active = 1;
        unit.role = index as u32;
        unit.id = u32::MAX - index as u32;
        unit.tier = 1 + index as u32 % 5;
        unit.cooldown = u32::MAX - 10 - index as u32;
    }
    let retired = sim.frame_mut().get_mut::<DefenseUnit>(cells[6]).unwrap();
    retired.id = 12345;
    retired.role = u32::MAX;
    retired.tier = 5;
    retired.cooldown = 99;
    let enemies = enemy_slots(sim.frame());
    for (index, id) in enemies.iter().enumerate() {
        *sim.frame_mut().get_mut::<DefenseEnemy>(*id).unwrap() = DefenseEnemy {
            active: u32::from(index < 5),
            id: u32::MAX - index as u32,
            kind: if index < 5 { index as u32 } else { u32::MAX },
            lane: index as u32 % 2,
            progress: 321 + index as u32,
            hp: u32::MAX - 100,
            max_hp: u32::MAX,
            slow_ticks: u32::MAX - 200,
        };
    }
    let mut producer = DefenseStreamProducer::new();
    let frame = decode(&mut producer, sim.frame());
    assert_eq!(frame.entities.len(), 30);
    assert_eq!(frame.props.len(), (20 + 24 * 5 + 5 * 6) * 4);
    for (index, id) in cells.iter().take(6).enumerate() {
        let (record, props) = record(&frame, producer.schema(), *id);
        assert_eq!(record.kind, 2 + index as u16);
        assert_eq!(
            props,
            [
                0,
                index as u32,
                u32::MAX - index as u32,
                1 + index as u32 % 5,
                u32::MAX - 10 - index as u32,
            ]
        );
    }
    let (empty, props) = record(&frame, producer.schema(), cells[6]);
    assert_eq!(empty.kind, EMPTY);
    assert_eq!(props, [0, 6, 0, 0, 0]);
    for (index, id) in enemies.iter().take(5).enumerate() {
        let (record, props) = record(&frame, producer.schema(), *id);
        let enemy = sim.frame().get::<DefenseEnemy>(*id).unwrap();
        assert_eq!(record.kind, 8 + index as u16);
        assert_eq!(
            props,
            [
                enemy.id,
                enemy.lane,
                enemy.progress,
                enemy.hp,
                enemy.max_hp,
                enemy.slow_ticks
            ]
        );
        let (x, y) = enemy_position(enemy.lane, enemy.progress);
        assert_eq!(record.cur, [x as f32, (-y) as f32, 0.0]);
        assert_eq!(record.prev, record.cur);
        assert_eq!(record.mode, MODE_NONE);
        assert_eq!(record.shape, SHAPE_CIRCLE);
    }
    assert!(enemies[5..].iter().all(|id| !frame
        .entities
        .iter()
        .any(|record| record.id == entity_id(*id))));
}

#[test]
fn active_enemy_pool_is_bounded_and_slot_identity_does_not_claim_actor_lifetime() {
    let mut sim = simulation();
    let enemies = enemy_slots(sim.frame());
    assert_eq!(enemies.len(), MAX_ENEMIES);
    for (index, id) in enemies.iter().enumerate() {
        *sim.frame_mut().get_mut::<DefenseEnemy>(*id).unwrap() = DefenseEnemy {
            active: 1,
            id: 100 + index as u32,
            kind: index as u32 % 5,
            hp: 10,
            max_hp: 10,
            ..Default::default()
        };
    }
    let projectile_ids: Vec<_> = sim
        .frame()
        .entities()
        .filter(|id| sim.frame().get::<DefenseProjectile>(*id).is_some())
        .collect();
    assert_eq!(projectile_ids.len(), MAX_PROJECTILES);
    for id in &projectile_ids {
        sim.frame_mut()
            .get_mut::<DefenseProjectile>(*id)
            .unwrap()
            .active = 1;
    }
    let mut producer = DefenseStreamProducer::new();
    let full = decode(&mut producer, sim.frame());
    assert_eq!(full.entities.len(), 1 + 24 + MAX_ENEMIES);
    assert_eq!(full.props.len(), (20 + 24 * 5 + MAX_ENEMIES * 6) * 4);
    assert_eq!(
        full.entities
            .iter()
            .map(|record| record.id)
            .collect::<BTreeSet<_>>()
            .len(),
        full.entities.len()
    );
    assert!(projectile_ids.iter().all(|id| !full
        .entities
        .iter()
        .any(|record| record.id == entity_id(*id))));
    assert_eq!(record(&full, producer.schema(), enemies[0]).1[0], 100);
    sim.frame_mut()
        .get_mut::<DefenseEnemy>(enemies[0])
        .unwrap()
        .active = 0;
    let absent = decode(&mut producer, sim.frame());
    assert_eq!(absent.entities.len(), full.entities.len() - 1);
    assert!(!absent
        .entities
        .iter()
        .any(|record| record.id == entity_id(enemies[0])));
    let reused = sim.frame_mut().get_mut::<DefenseEnemy>(enemies[0]).unwrap();
    reused.active = 1;
    reused.id = 900;
    let reused = decode(&mut producer, sim.frame());
    let (slot, props) = record(&reused, producer.schema(), enemies[0]);
    assert_eq!(slot.id, entity_id(enemies[0]));
    assert_eq!(props[0], 900);
    assert_eq!(slot.prev, slot.cur);
}

#[test]
fn actual_move_merge_and_restart_preserve_slot_and_logical_identity_distinctions() {
    let mut sim = simulation();
    let slots = cells(sim.frame());
    for (index, id) in slots.iter().take(2).enumerate() {
        let unit = sim.frame_mut().get_mut::<DefenseUnit>(*id).unwrap();
        unit.active = 1;
        unit.role = RIVET;
        unit.tier = 1;
        unit.id = 41 + index as u32;
        unit.cooldown = 7;
    }
    sim.frame_mut().singleton_mut::<DefenseRun>().next_unit_id = 43;
    let mut producer = DefenseStreamProducer::new();
    let initial = decode(&mut producer, sim.frame());
    assert_eq!(record(&initial, producer.schema(), slots[0]).1[2], 41);
    step(
        &mut sim,
        DefenseInput {
            action: MOVE_OR_MERGE,
            source_cell: 0,
            target_cell: 2,
            sequence: 1,
        },
    );
    let moved = decode(&mut producer, sim.frame());
    assert_eq!(
        record(&moved, producer.schema(), slots[0]).1,
        [0, 0, 0, 0, 0]
    );
    assert_eq!(record(&moved, producer.schema(), slots[2]).1[2], 41);
    assert_eq!(record(&moved, producer.schema(), slots[1]).1[2], 42);
    step(
        &mut sim,
        DefenseInput {
            action: MOVE_OR_MERGE,
            source_cell: 2,
            target_cell: 1,
            sequence: 2,
        },
    );
    // The source really retains its retired logical ID; the projection must not.
    assert_eq!(sim.frame().get::<DefenseUnit>(slots[2]).unwrap().id, 41);
    let merged = decode(&mut producer, sim.frame());
    assert_eq!(
        record(&merged, producer.schema(), slots[2]).1,
        [0, 2, 0, 0, 0]
    );
    let (_, survivor) = record(&merged, producer.schema(), slots[1]);
    assert_eq!(&survivor[..4], [0, 1, 42, 2]);
    step(
        &mut sim,
        DefenseInput {
            action: RESTART,
            sequence: 3,
            ..Default::default()
        },
    );
    let restarted = decode(&mut producer, sim.frame());
    let props = properties(&restarted, producer.schema());
    assert_eq!(&props[0][3..6], [0, 1, 3]);
    assert_eq!(
        restarted
            .entities
            .iter()
            .map(|record| record.id)
            .collect::<Vec<_>>(),
        initial
            .entities
            .iter()
            .map(|record| record.id)
            .collect::<Vec<_>>()
    );
    assert!(restarted.entities[1..]
        .iter()
        .all(|record| record.kind == EMPTY));
    assert!(props[1..].iter().all(|words| words[2..] == [0, 0, 0]));
    step(
        &mut sim,
        DefenseInput {
            action: SUMMON,
            sequence: 4,
            ..Default::default()
        },
    );
    let new_run = decode(&mut producer, sim.frame());
    let props = properties(&new_run, producer.schema());
    assert_eq!(props[0][4], 1);
    assert_eq!(record(&new_run, producer.schema(), slots[0]).1[2], 1);
}

#[test]
fn phase_economy_metadata_and_checksum_are_exact_without_claiming_host_lifecycle() {
    let mut sim = simulation();
    let mut producer = DefenseStreamProducer::new();
    for (phase, flags) in [(PLAYING, [1, 0, 0]), (WON, [0, 1, 0]), (LOST, [0, 0, 1])] {
        let run = sim.frame_mut().singleton_mut::<DefenseRun>();
        run.phase = phase;
        run.tick = 123;
        run.generation = u32::MAX;
        run.last_sequence = u32::MAX - 1;
        run.wave = 3;
        run.reactor_hp = 21;
        run.gold = [54321, 67890];
        run.investment = [2, 4];
        run.upgrades = [3, 6];
        sim.frame_mut().set_singleton(Campaign { last_wave: 3 });
        sim.frame_mut().set_singleton(Scenario {
            reactor_hp: 51,
            starting_gold: 500,
        });
        let meta = FrameMeta {
            tick: 1000,
            verified_tick: 998,
            flags: FLAG_PAUSED | FLAG_DISCONTINUITY | FLAG_EVENTS_RESET,
            rollback: Some((1002, 997)),
        };
        let frame = ViewFrame::decode(&producer.encode_frame(sim.frame(), None, meta)).unwrap();
        let props = properties(&frame, producer.schema());
        let checksum = sim.frame().checksum();
        assert_eq!(&props[0][..3], flags);
        assert_eq!(
            &props[0][3..],
            [
                123,
                u32::MAX,
                u32::MAX - 1,
                3,
                3,
                21,
                51,
                54321,
                67890,
                2,
                4,
                3,
                6,
                4,
                3,
                checksum as u32,
                (checksum >> 32) as u32,
            ]
        );
        assert_eq!(frame.tick, meta.tick);
        assert_eq!(frame.verified_tick, meta.verified_tick);
        assert_eq!(frame.flags, meta.flags | FLAG_ROLLED_BACK);
        assert_eq!(frame.rollback, meta.rollback);
        assert!(frame
            .entities
            .iter()
            .all(|record| record.mode == MODE_NONE && record.prev == record.cur));
    }
}

#[test]
fn repeated_projection_is_deterministic_and_cannot_mutate_current_previous_or_authored_source() {
    let text = "# retain exact authored source\nschema: orr.scene/1\nentities: {}\nsingletons:\n  CoopDefense::Scenario: {starting_gold: 487, reactor_hp: 83}\n  CoopDefense::Campaign: {last_wave: 3}\n";
    let doc = super::super::document(text).unwrap();
    let source_bytes = doc.frame().to_bytes();
    let source_checksum = doc.checksum();
    let mut sim = simulation();
    step(
        &mut sim,
        DefenseInput {
            action: SUMMON,
            sequence: 1,
            ..Default::default()
        },
    );
    let previous_bytes = sim.frame().to_bytes();
    let previous_checksum = sim.frame().checksum();
    let mut a = DefenseStreamProducer::new();
    let mut b = DefenseStreamProducer::new();
    let meta = FrameMeta {
        tick: 44,
        verified_tick: 42,
        ..Default::default()
    };
    let first = a.encode_frame(doc.frame(), Some(sim.frame()), meta);
    let independent = b.encode_frame(doc.frame(), None, meta);
    assert_eq!(
        first, independent,
        "previous-frame actors must not affect a latest-state projection"
    );
    let decoded = ViewFrame::decode(&first).unwrap();
    let props = properties(&decoded, a.schema());
    assert_eq!(&props[0][7..12], [3, 83, 83, 487, 487]);
    let second = a.encode_frame(doc.frame(), Some(sim.frame()), meta);
    let mut second = ViewFrame::decode(&second).unwrap();
    assert_eq!((decoded.seq, second.seq), (1, 2));
    second.seq = decoded.seq;
    assert_eq!(
        decoded, second,
        "only the producer sequence changes on repeated extraction"
    );
    assert_eq!(doc.frame().to_bytes(), source_bytes);
    assert_eq!(doc.checksum(), source_checksum);
    assert_eq!(sim.frame().to_bytes(), previous_bytes);
    assert_eq!(sim.frame().checksum(), previous_checksum);
    assert_eq!(doc.to_yaml(), text);
    assert_eq!(doc.revision(), 0);
    assert!(doc.history().is_empty());
    assert!(!doc.is_dirty() && !doc.can_undo() && !doc.can_redo());
}

#[test]
fn coop_defense_preset_installs_the_game_owned_stream() {
    let mut limits = crate::HostLimits::default();
    super::super::configure(&mut limits, None);
    let hook = limits
        .view_stream
        .expect("CoopDefense must provide latest-state observation");
    assert_eq!(hook.lock().schema(), &schema());
    assert_eq!(limits.build_id, SCENARIO_BUILD_ID);
    assert_eq!(limits.tick_rate, 60);
    assert_eq!(limits.player_count, 1);
}
