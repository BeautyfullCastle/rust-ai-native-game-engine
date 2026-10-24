//! Original, integer-only cooperative defense. Two private boards defend one reactor.
//! Fixed actor pools and the complete economy/RNG live in Frame for exact rollback.
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::FrameRng;
use orr_sim::{decode_pod, encode_pod, Game, PlayerSlot, SimCommand, SimContext, System};

pub const TICK_RATE: u32 = 60;
pub const BOARD_COLS: u32 = 4;
pub const BOARD_ROWS: u32 = 3;
pub const CELLS_PER_BOARD: u32 = BOARD_COLS * BOARD_ROWS;
pub const MAX_ENEMIES: usize = 64;
pub const MAX_PROJECTILES: usize = 128;
pub const NOOP: u32 = 0;
pub const SUMMON: u32 = 1;
pub const MOVE_OR_MERGE: u32 = 2;
pub const UPGRADE: u32 = 3;
pub const INVEST: u32 = 4;
pub const RESTART: u32 = 5;
pub const PLAYING: u32 = 0;
pub const WON: u32 = 1;
pub const LOST: u32 = 2;
pub const RIVET: u32 = 0;
pub const MORTAR: u32 = 1;
pub const PRISM: u32 = 2;
pub const CONDENSER: u32 = 3;
pub const HASTE: u32 = 4;
pub const MINER: u32 = 5;
pub const NORMAL: u32 = 0;
pub const FAST: u32 = 1;
pub const ARMORED: u32 = 2;
pub const HORDE: u32 = 3;
pub const BOSS: u32 = 4;
pub const EVENT_SUMMON: u32 = 1;
pub const EVENT_MERGE: u32 = 2;
pub const EVENT_KILL: u32 = 3;
pub const EVENT_LEAK: u32 = 4;
pub const EVENT_WAVE: u32 = 5;
pub const EVENT_FINISH: u32 = 6;
pub const EVENT_RESTART: u32 = 7;
pub const EVENT_RELEASE: u32 = 8;
pub const EVENT_HIT: u32 = 9;
pub const EVENT_PROJECTILE_EXPIRED: u32 = 10;
pub const EVENT_MOVE: u32 = 11;
pub const EVENT_ENEMY_SPAWN: u32 = 12;
/// Conservative structural bound: each projectile can hit every enemy and retire,
/// each enemy can die/leak once, each unit can fire once, plus actions/wave events.
pub const MAX_EVENTS_PER_TICK: usize =
    MAX_PROJECTILES * (MAX_ENEMIES + 1) + 2 * MAX_ENEMIES + 2 * CELLS_PER_BOARD as usize + 8;

/// Closed semantic event tags. The Pod payload stores the tag as u32, not a Rust
/// enum, so every bit pattern remains safe to decode. Unknown tags are rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum DefenseEventKind {
    Summon = EVENT_SUMMON,
    Merge = EVENT_MERGE,
    Death = EVENT_KILL,
    Leak = EVENT_LEAK,
    Wave = EVENT_WAVE,
    Finish = EVENT_FINISH,
    Restart = EVENT_RESTART,
    Release = EVENT_RELEASE,
    Hit = EVENT_HIT,
    ProjectileExpired = EVENT_PROJECTILE_EXPIRED,
    Move = EVENT_MOVE,
    EnemySpawn = EVENT_ENEMY_SPAWN,
}
impl DefenseEventKind {
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            EVENT_SUMMON => Self::Summon,
            EVENT_MERGE => Self::Merge,
            EVENT_KILL => Self::Death,
            EVENT_LEAK => Self::Leak,
            EVENT_WAVE => Self::Wave,
            EVENT_FINISH => Self::Finish,
            EVENT_RESTART => Self::Restart,
            EVENT_RELEASE => Self::Release,
            EVENT_HIT => Self::Hit,
            EVENT_PROJECTILE_EXPIRED => Self::ProjectileExpired,
            EVENT_MOVE => Self::Move,
            EVENT_ENEMY_SPAWN => Self::EnemySpawn,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Summon => "summon",
            Self::Merge => "merge",
            Self::Death => "death",
            Self::Leak => "leak",
            Self::Wave => "wave",
            Self::Finish => "finish",
            Self::Restart => "restart",
            Self::Release => "release",
            Self::Hit => "hit",
            Self::ProjectileExpired => "projectile_expired",
            Self::Move => "move",
            Self::EnemySpawn => "enemy_spawn",
        }
    }
}
pub const SUMMON_COST: u32 = 25;
pub const MAX_TIER: u32 = 5;
pub const MAX_UPGRADE: u32 = 8;
pub const MAX_INVESTMENT: u32 = 5;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct DefenseInput {
    pub action: u32,
    pub source_cell: u32,
    pub target_cell: u32,
    /// Nonzero increasing action ID. Held/predicted samples cannot spend twice.
    pub sequence: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct DefenseRun {
    pub phase: u32,
    pub tick: u32,
    pub wave: u32,
    pub reactor_hp: u32,
    pub gold: [u32; 2],
    pub investment: [u32; 2],
    pub upgrades: [u32; 2],
    pub kills: u32,
    pub leaks: u32,
    pub summons: u32,
    pub merges: u32,
    pub wave_tick: u32,
    pub spawned: u32,
    pub next_enemy_id: u32,
    pub last_sequence: u32,
    pub seed: u32,
    pub rng_draws: u32,
    /// Reset/RESTART namespace, rolled back and checksummed with the game.
    pub generation: u32,
    pub next_unit_id: u32,
    pub next_projectile_id: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct DefenseUnit {
    pub active: u32,
    pub board: u32,
    pub cell: u32,
    pub role: u32,
    pub tier: u32,
    pub cooldown: u32,
    /// Logical actor identity, not the fixed ECS pool/cell entity. Zero is absent.
    pub id: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct DefenseEnemy {
    pub active: u32,
    pub id: u32,
    pub lane: u32,
    pub kind: u32,
    pub progress: u32,
    pub hp: u32,
    pub max_hp: u32,
    pub slow_ticks: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct DefenseProjectile {
    pub active: u32,
    pub board: u32,
    pub role: u32,
    pub target_id: u32,
    pub x: i32,
    pub y: i32,
    pub damage: u32,
    pub speed: u32,
    pub id: u32,
    /// Source remains meaningful even after this unit moves or is merged away.
    pub source_unit_id: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct DefenseEvent {
    pub kind: u32,
    pub board: u32,
    pub value: u32,
    pub generation: u32,
    pub unit_id: u32,
    /// Merge consumes this ID and preserves unit_id (the destination actor).
    pub other_unit_id: u32,
    pub enemy_id: u32,
    pub projectile_id: u32,
    pub role: u32,
    pub hp_after: u32,
    pub x: i32,
    pub y: i32,
}
impl DefenseEvent {
    pub fn event_kind(&self) -> Option<DefenseEventKind> {
        DefenseEventKind::from_code(self.kind)
    }
    fn new(kind: DefenseEventKind, run: &DefenseRun, board: u32, value: u32) -> Self {
        Self {
            kind: kind as u32,
            generation: run.generation,
            board,
            value,
            ..Self::default()
        }
    }
    fn unit(mut self, unit: &DefenseUnit) -> Self {
        self.unit_id = unit.id;
        self.role = unit.role;
        (self.x, self.y) = board_cell_position(unit.board, unit.cell);
        self
    }
    fn projectile(mut self, projectile: &DefenseProjectile) -> Self {
        self.projectile_id = projectile.id;
        self.unit_id = projectile.source_unit_id;
        self.enemy_id = projectile.target_id;
        self.role = projectile.role;
        (self.x, self.y) = (projectile.x, projectile.y);
        self
    }
    fn enemy(mut self, enemy: &DefenseEnemy) -> Self {
        self.enemy_id = enemy.id;
        self.hp_after = enemy.hp;
        (self.x, self.y) = enemy_position(enemy.lane, enemy.progress);
        self
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct NoCommand {
    pub reserved: u32,
}
impl SimCommand for NoCommand {
    fn encode(&self, out: &mut Vec<u8>) {
        encode_pod(self, out);
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        let c: Self = decode_pod(bytes)?;
        (c.reserved == 0).then_some(c)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct DefenseConfig {
    pub seed: u32,
}
impl Default for DefenseConfig {
    fn default() -> Self {
        Self { seed: 0x0c00_2026 }
    }
}
pub struct DefenseGame;
impl Game for DefenseGame {
    type Input = DefenseInput;
    type Command = NoCommand;
    type Event = DefenseEvent;
    type Config = DefenseConfig;
    fn register(b: &mut ComponentRegistryBuilder) {
        b.register_singleton::<DefenseRun>("CoopDefense::Run");
        b.register_component::<DefenseUnit>("CoopDefense::Unit");
        b.register_component::<DefenseEnemy>("CoopDefense::Enemy");
        b.register_component::<DefenseProjectile>("CoopDefense::Projectile");
    }
    fn setup(f: &mut Frame, c: &DefenseConfig) {
        f.set_singleton(initial_run(c.seed));
        f.set_singleton(FrameRng::new(c.seed as u64));
        for board in 0..2 {
            for cell in 0..CELLS_PER_BOARD {
                let e = f.spawn();
                f.add(
                    e,
                    DefenseUnit {
                        board,
                        cell,
                        ..Default::default()
                    },
                );
            }
        }
        for _ in 0..MAX_ENEMIES {
            let e = f.spawn();
            f.add(e, DefenseEnemy::default());
        }
        for _ in 0..MAX_PROJECTILES {
            let e = f.spawn();
            f.add(e, DefenseProjectile::default());
        }
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(DefenseSystem)]
    }
}
fn initial_run(seed: u32) -> DefenseRun {
    DefenseRun {
        reactor_hp: 30,
        gold: [150; 2],
        wave: 1,
        seed,
        next_enemy_id: 1,
        next_unit_id: 1,
        next_projectile_id: 1,
        ..Default::default()
    }
}
pub fn run(f: &Frame) -> DefenseRun {
    *f.singleton::<DefenseRun>()
}
pub fn units(f: &Frame) -> Vec<DefenseUnit> {
    let mut v: Vec<_> = f
        .entities()
        .filter_map(|e| f.get::<DefenseUnit>(e).copied())
        .collect();
    v.sort_by_key(|u| (u.board, u.cell));
    v
}
pub fn enemies(f: &Frame) -> Vec<DefenseEnemy> {
    f.entities()
        .filter_map(|e| f.get::<DefenseEnemy>(e).copied())
        .collect()
}
pub fn projectiles(f: &Frame) -> Vec<DefenseProjectile> {
    f.entities()
        .filter_map(|e| f.get::<DefenseProjectile>(e).copied())
        .collect()
}
/// UI coordinates in a 1000 by 700 logical canvas. Invalid indices are clamped.
pub fn board_cell_position(board: u32, cell: u32) -> (i32, i32) {
    let c = cell.min(CELLS_PER_BOARD - 1);
    (
        120 + board.min(1) as i32 * 500 + (c % BOARD_COLS) as i32 * 85,
        265 + (c / BOARD_COLS) as i32 * 75,
    )
}
/// Each lane wraps around its board, then converges on the shared reactor.
pub fn enemy_position(lane: u32, progress: u32) -> (i32, i32) {
    let p = progress.min(2000) as i32;
    let offset = lane.min(1) as i32 * 500;
    let (x, y) = if p < 500 {
        (50 + p * 4 / 5, 190)
    } else if p < 900 {
        (450, 190 + (p - 500) * 3 / 4)
    } else if p < 1400 {
        (450 - (p - 900) * 4 / 5, 490)
    } else if p < 1600 {
        (50, 490 + (p - 1400) / 2)
    } else {
        (50 + (p - 1600) * (450 - offset) / 400, 590)
    };
    (x + offset, y)
}
pub fn upgrade_cost(level: u32) -> u32 {
    60 + level.min(MAX_UPGRADE) * 40
}
pub fn investment_cost(level: u32) -> u32 {
    60 + level.min(MAX_INVESTMENT) * 35
}
pub fn role_name(role: u32) -> &'static str {
    match role {
        RIVET => "Rivet gunner",
        MORTAR => "Arc mortar",
        PRISM => "Prism lancer",
        CONDENSER => "Slow condenser",
        HASTE => "Haste beacon",
        MINER => "Alloy miner",
        _ => "Unknown",
    }
}
fn allocate_id(next: &mut u32) -> u32 {
    // Never alias an old lifetime. Exhaustion is explicit, never wrapping reuse.
    let id = *next;
    *next = next
        .checked_add(1)
        .expect("defense logical identity exhausted");
    id
}
fn action(
    r: &mut DefenseRun,
    u: &mut [DefenseUnit],
    board: u32,
    input: DefenseInput,
    events: &mut Vec<DefenseEvent>,
    rng: &mut FrameRng,
) {
    let b = board as usize;
    match input.action {
        SUMMON if r.gold[b] >= SUMMON_COST => {
            if let Some(slot) = u.iter_mut().find(|u| u.board == board && u.active == 0) {
                slot.id = allocate_id(&mut r.next_unit_id);
                slot.active = 1;
                slot.tier = 1;
                slot.role = rng.next_u32() % 6;
                r.rng_draws += 1;
                slot.cooldown = 0;
                r.gold[b] -= SUMMON_COST;
                r.summons += 1;
                events.push(
                    DefenseEvent::new(DefenseEventKind::Summon, r, board, slot.role).unit(slot),
                );
            }
        }
        MOVE_OR_MERGE
            if input.source_cell < CELLS_PER_BOARD
                && input.target_cell < CELLS_PER_BOARD
                && input.source_cell != input.target_cell =>
        {
            let a = (board * CELLS_PER_BOARD + input.source_cell) as usize;
            let b = (board * CELLS_PER_BOARD + input.target_cell) as usize;
            if u[a].active == 0 {
                return;
            }
            if u[b].active == 0 {
                let cell_a = u[a].cell;
                let cell_b = u[b].cell;
                u.swap(a, b);
                u[a].cell = cell_a;
                u[b].cell = cell_b;
                events
                    .push(DefenseEvent::new(DefenseEventKind::Move, r, board, cell_b).unit(&u[b]));
            } else if u[a].role == u[b].role && u[a].tier == u[b].tier && u[b].tier < MAX_TIER {
                u[b].tier += 1;
                u[b].cooldown = 0;
                u[a].active = 0;
                r.merges += 1;
                let mut event =
                    DefenseEvent::new(DefenseEventKind::Merge, r, board, u[b].tier).unit(&u[b]);
                event.other_unit_id = u[a].id;
                events.push(event);
            }
        }
        UPGRADE if r.upgrades[b] < MAX_UPGRADE => {
            let cost = upgrade_cost(r.upgrades[b]);
            if r.gold[b] >= cost {
                r.gold[b] -= cost;
                r.upgrades[b] += 1;
            }
        }
        INVEST if r.investment[b] < MAX_INVESTMENT => {
            let cost = investment_cost(r.investment[b]);
            if r.gold[b] >= cost {
                r.gold[b] -= cost;
                r.investment[b] += 1;
            }
        }
        _ => {}
    }
}
/// Deterministic, ordinary economy/merge decisions, also useful for a smoke-test autoplayer.
pub fn strategy_input(
    r: &DefenseRun,
    u: &[DefenseUnit],
    board: u32,
    sequence: u32,
) -> DefenseInput {
    let b = board.min(1) as usize;
    let mut result = DefenseInput {
        sequence,
        ..Default::default()
    };
    let count = u
        .iter()
        .filter(|x| x.board == board && x.active != 0)
        .count();
    if count >= 8 {
        for a in u
            .iter()
            .filter(|x| x.board == board && x.active != 0 && x.tier < MAX_TIER)
        {
            if let Some(c) = u.iter().find(|x| {
                x.board == board
                    && x.active != 0
                    && x.cell > a.cell
                    && x.tier == a.tier
                    && x.role == a.role
            }) {
                result.action = MOVE_OR_MERGE;
                result.source_cell = a.cell;
                result.target_cell = c.cell;
                return result;
            }
        }
    }
    if count < CELLS_PER_BOARD as usize && r.gold[b] >= SUMMON_COST {
        result.action = SUMMON;
    } else if r.upgrades[b] < MAX_UPGRADE && r.gold[b] >= upgrade_cost(r.upgrades[b]) {
        result.action = UPGRADE;
    } else if r.investment[b] < MAX_INVESTMENT && r.gold[b] >= investment_cost(r.investment[b]) {
        result.action = INVEST;
    }
    result
}
struct DefenseSystem;
impl System<DefenseGame> for DefenseSystem {
    fn name(&self) -> &'static str {
        "CoopDefense"
    }
    fn run(&mut self, ctx: &mut SimContext<DefenseGame>) {
        let input =
            if ctx.inputs.player_count() > 0 && !ctx.inputs.flags(PlayerSlot(0)).disconnected {
                *ctx.inputs.input(PlayerSlot(0))
            } else {
                DefenseInput::default()
            };
        let mut r = run(ctx.frame);
        let mut rng = *ctx.frame.singleton::<FrameRng>();
        let mut u = units(ctx.frame);
        let mut e = enemies(ctx.frame);
        let mut p = projectiles(ctx.frame);
        let mut events = Vec::new();
        // Strict monotonic IDs deduplicate prediction/repeat. No wrapping is needed within a run.
        let fresh = input.sequence > r.last_sequence
            && input.action <= RESTART
            && input.source_cell < CELLS_PER_BOARD
            && input.target_cell < CELLS_PER_BOARD;
        if fresh {
            r.last_sequence = input.sequence;
        }
        if fresh && input.action == RESTART {
            let sequence = r.last_sequence;
            let generation = r
                .generation
                .checked_add(1)
                .expect("defense generation exhausted");
            r = initial_run(r.seed);
            r.generation = generation;
            r.last_sequence = sequence;
            rng = FrameRng::new(r.seed as u64);
            for unit in &mut u {
                unit.id = 0;
                unit.active = 0;
                unit.role = 0;
                unit.tier = 0;
                unit.cooldown = 0;
            }
            e.fill(DefenseEnemy::default());
            p.fill(DefenseProjectile::default());
            events.push(DefenseEvent::new(DefenseEventKind::Restart, &r, 0, 0));
        } else if r.phase == PLAYING {
            if fresh {
                action(&mut r, &mut u, 0, input, &mut events, &mut rng);
            }
            r.tick += 1;
            r.wave_tick += 1;
            if r.tick % 45 == 0 {
                let ai = strategy_input(&r, &u, 1, 1);
                action(&mut r, &mut u, 1, ai, &mut events, &mut rng);
            }
            if r.tick % 60 == 0 {
                for board in 0..2 {
                    let miners: u32 = u
                        .iter()
                        .filter(|x| x.active != 0 && x.board == board && x.role == MINER)
                        .map(|x| x.tier)
                        .sum();
                    r.gold[board as usize] =
                        (r.gold[board as usize] + 3 + r.investment[board as usize] * 2 + miners)
                            .min(99999);
                }
            }
            spawn_wave(&mut r, &mut e, &mut events);
            move_enemies(&mut r, &mut e, &mut events);
            advance_projectiles(&mut r, &mut e, &mut p, &mut events);
            fire(&mut r, &mut u, &e, &mut p, &mut events);
            if r.reactor_hp == 0 {
                r.phase = LOST;
                events.push(DefenseEvent::new(DefenseEventKind::Finish, &r, 0, LOST));
            } else if r.spawned >= wave_count(r.wave) && e.iter().all(|x| x.active == 0) {
                if r.wave == 10 {
                    r.phase = WON;
                    events.push(DefenseEvent::new(DefenseEventKind::Finish, &r, 0, WON));
                } else {
                    r.wave += 1;
                    r.wave_tick = 0;
                    r.spawned = 0;
                    for gold in &mut r.gold {
                        *gold = (*gold + 40).min(99999);
                    }
                    events.push(DefenseEvent::new(DefenseEventKind::Wave, &r, 0, r.wave));
                }
            }
        }
        // Pools never despawn; write in their fixed setup order, not enemy IDs.
        let mut ids: Vec<_> = ctx
            .frame
            .query::<(&DefenseUnit,)>()
            .map(|(id, (v,))| (v.board, v.cell, id))
            .collect();
        ids.sort_by_key(|x| (x.0, x.1));
        for ((_, _, id), value) in ids.into_iter().zip(u) {
            *ctx.frame.get_mut::<DefenseUnit>(id).unwrap() = value;
        }
        let ids: Vec<_> = ctx
            .frame
            .query::<(&DefenseEnemy,)>()
            .map(|(id, _)| id)
            .collect();
        for (id, value) in ids.into_iter().zip(e) {
            *ctx.frame.get_mut::<DefenseEnemy>(id).unwrap() = value;
        }
        let ids: Vec<_> = ctx
            .frame
            .query::<(&DefenseProjectile,)>()
            .map(|(id, _)| id)
            .collect();
        for (id, value) in ids.into_iter().zip(p) {
            *ctx.frame.get_mut::<DefenseProjectile>(id).unwrap() = value;
        }
        ctx.frame.set_singleton(r);
        ctx.frame.set_singleton(rng);
        assert!(
            events.len() <= MAX_EVENTS_PER_TICK,
            "defense event structural bound exceeded"
        );
        for ev in events {
            ctx.emit(ev);
        }
    }
}
fn wave_count(wave: u32) -> u32 {
    (8 + wave * 2) * 2
}
fn spawn_wave(r: &mut DefenseRun, e: &mut [DefenseEnemy], events: &mut Vec<DefenseEvent>) {
    if r.wave_tick < 180 || r.wave_tick % 35 != 0 || r.spawned >= wave_count(r.wave) {
        return;
    }
    for lane in 0..2 {
        let Some(slot) = e.iter_mut().find(|x| x.active == 0) else {
            return;
        };
        let ordinal = r.spawned / 2;
        let kind = if (r.wave == 5 || r.wave == 10) && ordinal == 0 {
            BOSS
        } else {
            (ordinal + r.wave) % 4
        };
        let base = 24 + r.wave * 15;
        let hp = match kind {
            BOSS => base * 14,
            ARMORED => base * 2,
            HORDE => base * 2 / 3,
            _ => base,
        };
        *slot = DefenseEnemy {
            active: 1,
            id: allocate_id(&mut r.next_enemy_id),
            lane,
            kind,
            hp,
            max_hp: hp,
            ..Default::default()
        };
        events.push(DefenseEvent::new(DefenseEventKind::EnemySpawn, r, lane, kind).enemy(slot));
        r.spawned += 1;
    }
}
fn move_enemies(r: &mut DefenseRun, e: &mut [DefenseEnemy], events: &mut Vec<DefenseEvent>) {
    for enemy in e.iter_mut().filter(|x| x.active != 0) {
        let speed = match enemy.kind {
            FAST => 7,
            BOSS => 2,
            _ => 4,
        };
        enemy.progress += if enemy.slow_ticks > 0 {
            enemy.slow_ticks -= 1;
            (speed / 2).max(1)
        } else {
            speed
        };
        if enemy.progress >= 2000 {
            enemy.active = 0;
            r.leaks += 1;
            r.reactor_hp = r
                .reactor_hp
                .saturating_sub(if enemy.kind == BOSS { 10 } else { 2 });
            events.push(
                DefenseEvent::new(DefenseEventKind::Leak, r, enemy.lane, enemy.kind).enemy(enemy),
            );
        }
    }
}
fn distance(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}
fn advance_projectiles(
    r: &mut DefenseRun,
    e: &mut [DefenseEnemy],
    p: &mut [DefenseProjectile],
    events: &mut Vec<DefenseEvent>,
) {
    for projectile in p.iter_mut().filter(|p| p.active != 0) {
        let Some(target) = e
            .iter()
            .position(|x| x.active != 0 && x.id == projectile.target_id)
        else {
            projectile.active = 0;
            events.push(
                DefenseEvent::new(DefenseEventKind::ProjectileExpired, r, projectile.board, 0)
                    .projectile(projectile),
            );
            continue;
        };
        let target_lane = e[target].lane;
        let pos = enemy_position(target_lane, e[target].progress);
        let d = distance((projectile.x, projectile.y), pos);
        if d > projectile.speed as i32 {
            let dx = pos.0 - projectile.x;
            let dy = pos.1 - projectile.y;
            projectile.x += dx * projectile.speed as i32 / d;
            projectile.y += dy * projectile.speed as i32 / d;
            continue;
        }
        projectile.x = pos.0;
        projectile.y = pos.1;
        projectile.active = 0;
        for enemy in e.iter_mut().filter(|x| x.active != 0) {
            let splash = projectile.role == MORTAR
                && enemy.lane == target_lane
                && distance(enemy_position(enemy.lane, enemy.progress), pos) <= 110;
            if enemy.id != projectile.target_id && !splash {
                continue;
            }
            let damage = if enemy.kind == ARMORED && projectile.role != PRISM {
                projectile.damage.saturating_sub(7).max(1)
            } else {
                projectile.damage
            };
            let hp_before = enemy.hp;
            enemy.hp = enemy.hp.saturating_sub(damage);
            events.push(
                DefenseEvent::new(DefenseEventKind::Hit, r, enemy.lane, hp_before - enemy.hp)
                    .projectile(projectile)
                    .enemy(enemy),
            );
            if projectile.role == CONDENSER {
                enemy.slow_ticks = 100;
            }
            if enemy.hp == 0 {
                enemy.active = 0;
                r.kills += 1;
                r.gold[projectile.board as usize] = (r.gold[projectile.board as usize]
                    + if enemy.kind == BOSS { 60 } else { 4 })
                .min(99999);
                events.push(
                    DefenseEvent::new(DefenseEventKind::Death, r, enemy.lane, enemy.kind)
                        .projectile(projectile)
                        .enemy(enemy),
                );
            }
        }
    }
}

fn fire(
    r: &mut DefenseRun,
    u: &mut [DefenseUnit],
    e: &[DefenseEnemy],
    p: &mut [DefenseProjectile],
    events: &mut Vec<DefenseEvent>,
) {
    let haste: [u32; 2] = core::array::from_fn(|b| {
        u.iter()
            .filter(|x| x.active != 0 && x.board == b as u32 && x.role == HASTE)
            .map(|x| x.tier)
            .sum()
    });
    for unit in u.iter_mut().filter(|x| x.active != 0) {
        if unit.cooldown > 0 {
            unit.cooldown -= 1;
            continue;
        }
        if unit.role == HASTE || unit.role == MINER {
            continue;
        }
        let pos = board_cell_position(unit.board, unit.cell);
        let range = if unit.role == MORTAR { 600 } else { 390 };
        let target = e
            .iter()
            .filter(|x| {
                x.active != 0
                    && (x.lane == unit.board || (unit.role == MORTAR && x.progress >= 1600))
                    && distance(pos, enemy_position(x.lane, x.progress)) <= range
            })
            .max_by_key(|x| (x.progress, core::cmp::Reverse(x.id)));
        let Some(target) = target else {
            continue;
        };
        let Some(slot) = p.iter_mut().find(|x| x.active == 0) else {
            break;
        };
        let base = match unit.role {
            RIVET => 14,
            MORTAR => 28,
            PRISM => 23,
            _ => 9,
        };
        let multiplier = 1u32 << (unit.tier - 1);
        *slot = DefenseProjectile {
            active: 1,
            id: allocate_id(&mut r.next_projectile_id),
            source_unit_id: unit.id,
            board: unit.board,
            role: unit.role,
            target_id: target.id,
            x: pos.0,
            y: pos.1,
            damage: (base + r.upgrades[unit.board as usize] * 6) * multiplier,
            speed: if unit.role == MORTAR { 10 } else { 18 },
        };
        events.push(
            DefenseEvent::new(DefenseEventKind::Release, r, unit.board, slot.damage)
                .projectile(slot),
        );
        let cooldown = match unit.role {
            RIVET => 25,
            MORTAR => 75,
            PRISM => 42,
            _ => 45,
        };
        unit.cooldown = (cooldown * 10 / (10 + haste[unit.board as usize] * 2)).max(8);
    }
}

#[cfg(test)]
mod tests;
