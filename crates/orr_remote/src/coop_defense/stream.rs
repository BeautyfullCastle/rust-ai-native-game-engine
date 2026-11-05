//! Stormglass's bounded latest-state projection. Exact gameplay values are u32
//! properties; shapes are approximate inspection markers, not native artwork.
//! This reads only the supplied Frame and does not retain simulation state.
use orr_ecs::{Entity, Frame};
use orr_games::coop_defense_game::{
    board_cell_position, enemy_position, Campaign, DefenseEnemy, DefenseEvent, DefenseInput,
    DefenseRun, DefenseUnit, NoCommand, Scenario, ARMORED, BOARD_COLS, BOARD_ROWS, BOSS,
    CELLS_PER_BOARD, CONDENSER, FAST, HASTE, HORDE, LOST, MAX_ENEMIES, MINER, MORTAR, NORMAL,
    PLAYING, PRISM, RIVET, SCENARIO_BUILD_ID, TICK_RATE, WON,
};
use orr_viewstream::{
    entity_id, input_layout_of, EntityRecord, EventDef, FrameMeta, KindDef, PropType, Schema,
    StreamProducer, ViewFrame, MODE_NONE, SHAPE_CIRCLE, SHAPE_QUAD,
};

const RUN: u16 = 0;
const EMPTY: u16 = 1;
const RUN_PROPS: &[&str] = &[
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
];
const CELL_PROPS: &[&str] = &["board", "cell", "unit_id", "tier", "cooldown"];
const ENEMY_PROPS: &[&str] = &["enemy_id", "lane", "progress", "hp", "max_hp", "slow_ticks"];

fn kind(id: u16, name: &str, props: &[&str]) -> KindDef {
    props.iter().fold(KindDef::new(id, name), |kind, name| {
        kind.with_prop(name, PropType::U32)
    })
}

fn schema() -> Schema {
    let mut kinds = vec![kind(RUN, "run", RUN_PROPS)];
    for (id, name) in [
        (EMPTY, "cell.empty"),
        (2, "cell.rivet"),
        (3, "cell.mortar"),
        (4, "cell.prism"),
        (5, "cell.condenser"),
        (6, "cell.haste"),
        (7, "cell.miner"),
    ] {
        kinds.push(kind(id, name, CELL_PROPS));
    }
    for (id, name) in [
        (8, "enemy.normal"),
        (9, "enemy.fast"),
        (10, "enemy.armored"),
        (11, "enemy.horde"),
        (12, "enemy.boss"),
    ] {
        kinds.push(kind(id, name, ENEMY_PROPS));
    }
    Schema {
        game: super::GAME.into(),
        dimensions: 2,
        build_id: SCENARIO_BUILD_ID,
        tick_rate: TICK_RATE,
        player_count: 1,
        kinds,
        input: input_layout_of::<DefenseInput>(),
        command_size: std::mem::size_of::<NoCommand>(),
        // ERP publishes existing events independently of frame pacing. State
        // consumers ignore event batches; these are not command receipts.
        events: vec![EventDef {
            id: 0,
            name: "defense_event_opaque".into(),
            payload_size: std::mem::size_of::<DefenseEvent>(),
        }],
    }
}

fn cell_kind(unit: &DefenseUnit) -> u16 {
    if unit.active == 0 {
        return EMPTY;
    }
    match unit.role {
        RIVET => 2,
        MORTAR => 3,
        PRISM => 4,
        CONDENSER => 5,
        HASTE => 6,
        MINER => 7,
        _ => unreachable!("invalid role in admitted Stormglass frame"),
    }
}

fn enemy_kind(enemy: &DefenseEnemy) -> u16 {
    match enemy.kind {
        NORMAL => 8,
        FAST => 9,
        ARMORED => 10,
        HORDE => 11,
        BOSS => 12,
        _ => unreachable!("invalid enemy kind in admitted Stormglass frame"),
    }
}

fn words(props: &mut Vec<u8>, values: &[u32]) {
    for value in values {
        props.extend_from_slice(&value.to_le_bytes());
    }
}

fn marker(id: Entity, kind: u16, position: (i32, i32), enemy: bool) -> EntityRecord {
    // Canvas coordinates are Y-down; OVS1 advertises Y-up. No interpolation
    // is implied: fixed pool slots can contain different logical actors.
    let pose = [position.0 as f32, (-position.1) as f32, 0.0];
    EntityRecord {
        id: entity_id(id),
        kind,
        shape: if enemy { SHAPE_CIRCLE } else { SHAPE_QUAD },
        mode: MODE_NONE,
        size: if enemy { 10.0 } else { 24.0 },
        half_y: if enemy { 0.0 } else { 24.0 },
        rgba: if enemy {
            [230, 90, 90, 255]
        } else if kind == EMPTY {
            [60, 70, 80, 120]
        } else {
            [80, 190, 210, 255]
        },
        prev: pose,
        cur: pose,
    }
}

pub(super) struct DefenseStreamProducer {
    schema: Schema,
    seq: u64,
}

impl DefenseStreamProducer {
    pub(super) fn new() -> Self {
        Self {
            schema: schema(),
            seq: 0,
        }
    }
}

impl StreamProducer for DefenseStreamProducer {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn encode_frame(&mut self, cur: &Frame, _prev: Option<&Frame>, meta: FrameMeta) -> Vec<u8> {
        let run = cur.singleton::<DefenseRun>();
        let checksum = cur.checksum();
        let mut entities = Vec::with_capacity(1 + 2 * CELLS_PER_BOARD as usize + MAX_ENEMIES);
        let mut props = Vec::new();
        // Entity::NONE cannot be allocated. This record is metadata, not an
        // authored/queryable ECS entity and not a control or command receipt.
        entities.push(EntityRecord {
            id: entity_id(Entity::NONE),
            kind: RUN,
            shape: SHAPE_QUAD,
            mode: MODE_NONE,
            size: 0.0,
            half_y: 0.0,
            rgba: [0; 4],
            prev: [0.0; 3],
            cur: [0.0; 3],
        });
        words(
            &mut props,
            &[
                u32::from(run.phase == PLAYING),
                u32::from(run.phase == WON),
                u32::from(run.phase == LOST),
                run.tick,
                run.generation,
                run.last_sequence,
                run.wave,
                cur.singleton::<Campaign>().last_wave,
                run.reactor_hp,
                cur.singleton::<Scenario>().reactor_hp,
                run.gold[0],
                run.gold[1],
                run.investment[0],
                run.investment[1],
                run.upgrades[0],
                run.upgrades[1],
                BOARD_COLS,
                BOARD_ROWS,
                checksum as u32,
                (checksum >> 32) as u32,
            ],
        );
        // Admitted Stormglass frames contain exactly 24 cell slots, 64 enemy
        // slots and 128 projectile slots. Only active enemies are projected;
        // projectile state is outside this bounded latest-state contract.
        for id in cur.entities() {
            if let Some(unit) = cur.get::<DefenseUnit>(id) {
                entities.push(marker(
                    id,
                    cell_kind(unit),
                    board_cell_position(unit.board, unit.cell),
                    false,
                ));
                let (logical_id, tier, cooldown) = if unit.active == 0 {
                    // A merge retires the actor but leaves old fields in its slot.
                    (0, 0, 0)
                } else {
                    (unit.id, unit.tier, unit.cooldown)
                };
                words(
                    &mut props,
                    &[unit.board, unit.cell, logical_id, tier, cooldown],
                );
            } else if let Some(enemy) = cur.get::<DefenseEnemy>(id).filter(|e| e.active != 0) {
                entities.push(marker(
                    id,
                    enemy_kind(enemy),
                    enemy_position(enemy.lane, enemy.progress),
                    true,
                ));
                words(
                    &mut props,
                    &[
                        enemy.id,
                        enemy.lane,
                        enemy.progress,
                        enemy.hp,
                        enemy.max_hp,
                        enemy.slow_ticks,
                    ],
                );
            }
        }
        self.seq += 1;
        ViewFrame {
            flags: meta.flags,
            tick: meta.tick,
            verified_tick: meta.verified_tick,
            seq: self.seq,
            rollback: meta.rollback,
            entities,
            props,
        }
        .encode()
    }
}

#[cfg(test)]
mod tests;
