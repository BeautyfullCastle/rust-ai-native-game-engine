//! `orr_testgame`: a small deterministic arena game used to exercise
//! `orr_sim`/`orr_session` (prediction, rollback, replay, checksums).
//!
//! N players move circles with FP velocity from input axes; holding the
//! fire button spawns a bullet (via `Commands`, so spawns land after the
//! movement system in a deterministic order); bullets despawn on hit or
//! timeout; a `Hit` event fires on impact; spawn jitter comes from the
//! frame RNG; a `Score` singleton tracks kills per player.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FPVec2, FP};
use orr_sim::{decode_pod, encode_pod, Game, PlayerSlot, SimCommand, SimContext, System};

pub const MAX_PLAYERS: usize = 8;
pub const ARENA_HALF: FP = FP(2_000 << 16); // +-2000 units
pub const PLAYER_SPEED: FP = FP((6 << 16) as i64);
pub const PLAYER_RADIUS: FP = FP((20 << 16) as i64);
pub const BULLET_SPEED: FP = FP((30 << 16) as i64);
pub const BULLET_RADIUS: FP = FP((6 << 16) as i64);
pub const BULLET_LIFETIME_TICKS: u32 = 90;

// ---- input ----

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Pod, Zeroable)]
pub struct ArenaInput {
    /// -1, 0 or 1 on each axis, packed as raw FP for determinism/Pod.
    pub axis_x: FP,
    pub axis_y: FP,
    pub buttons: u32,
    pub _pad: u32,
}
pub const FIRE: u32 = 1 << 0;

impl ArenaInput {
    pub fn new(axis_x: FP, axis_y: FP, fire: bool) -> Self {
        Self { axis_x, axis_y, buttons: if fire { FIRE } else { 0 }, _pad: 0 }
    }
}

// ---- commands ----

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct SpawnBulletCmd {
    pub owner: u32,
}
impl SimCommand for SpawnBulletCmd {
    fn encode(&self, out: &mut Vec<u8>) {
        encode_pod(self, out)
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        decode_pod(bytes)
    }
}

// ---- events ----

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Hit {
    pub victim_slot: u32,
    pub shooter_slot: u32,
}

// ---- components ----

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Position {
    pub pos: FPVec2,
}
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct PlayerTag {
    pub slot: u32,
}
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Bullet {
    pub velocity: FPVec2,
    pub owner_slot: u32,
    pub ttl: u32,
}
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Score {
    pub kills: [u32; MAX_PLAYERS],
}

pub struct Arena;

#[derive(Default)]
pub struct ArenaConfig {
    pub player_count: u8,
}

impl Game for Arena {
    type Input = ArenaInput;
    type Command = SpawnBulletCmd;
    type Event = Hit;
    type Config = ArenaConfig;

    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_component::<Position>("Position");
        builder.register_component::<PlayerTag>("PlayerTag");
        builder.register_component::<Bullet>("Bullet");
        builder.register_singleton::<Score>("Score");
    }

    fn setup(frame: &mut Frame, config: &Self::Config) {
        for slot in 0..config.player_count {
            let e = frame.spawn();
            // Deterministic, spread-out starting positions.
            let angle_step = FP::TWO_PI / FP::from_raw((config.player_count as i64) << 16);
            let angle = angle_step * FP::from_raw((slot as i64) << 16);
            let radius = fp!(500);
            let pos = FPVec2::new(radius * angle.cos(), radius * angle.sin());
            frame.add(e, Position { pos });
            frame.add(e, PlayerTag { slot: slot as u32 });
        }
        frame.set_singleton(Score { kills: [0; MAX_PLAYERS] });
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(MoveSystem), Box::new(FireSystem), Box::new(BulletSystem)]
    }
}

struct MoveSystem;
impl System<Arena> for MoveSystem {
    fn name(&self) -> &'static str {
        "MoveSystem"
    }
    fn run(&mut self, ctx: &mut SimContext<Arena>) {
        let player_count = ctx.inputs.player_count();
        for slot in 0..player_count {
            let input = *ctx.inputs.input(PlayerSlot(slot));
            let mut found: Option<Entity> = None;
            for (e, (tag,)) in ctx.frame.query::<(&PlayerTag,)>() {
                if tag.slot == slot as u32 {
                    found = Some(e);
                    break;
                }
            }
            if let Some(e) = found {
                let delta = FPVec2::new(input.axis_x, input.axis_y) * PLAYER_SPEED;
                if let Some(p) = ctx.frame.get_mut::<Position>(e) {
                    let mut np = p.pos + delta;
                    np.x = np.x.clamp(-ARENA_HALF, ARENA_HALF);
                    np.y = np.y.clamp(-ARENA_HALF, ARENA_HALF);
                    p.pos = np;
                }
            }
        }
    }
}

struct FireSystem;
impl System<Arena> for FireSystem {
    fn name(&self) -> &'static str {
        "FireSystem"
    }
    fn run(&mut self, ctx: &mut SimContext<Arena>) {
        // Commands submitted this tick (SpawnBulletCmd) spawn bullets.
        // (Fire *decisions* are made by the caller building `TickInputs`,
        // which pushes a `SpawnBulletCmd` when a player's `FIRE` bit is
        // set; this system only has to consume already-submitted commands
        // so it stays a pure function of `(Frame, TickInputs)`.)
        for (owner_slot, cmd) in ctx.inputs.commands() {
            let _ = owner_slot;
            let owner = cmd.owner;
            let mut origin = FPVec2::ZERO;
            let mut nearest_enemy: Option<FPVec2> = None;
            for (_e, (tag, pos)) in ctx.frame.query::<(&PlayerTag, &Position)>() {
                if tag.slot == owner {
                    origin = pos.pos;
                } else {
                    nearest_enemy = Some(pos.pos);
                }
            }
            // Auto-aim at the (first found) other player, falling back to
            // input axis direction, then a fixed default; this keeps the
            // test game's hit rate high enough to exercise `Hit` events
            // reliably in a short scripted test run.
            let mut aim = FPVec2::new(FP::ONE, FP::ZERO);
            if let Some(enemy_pos) = nearest_enemy {
                let to_enemy = (enemy_pos - origin).normalize_or_zero();
                if to_enemy != FPVec2::ZERO {
                    aim = to_enemy;
                }
            } else {
                let input = *ctx.inputs.input(PlayerSlot(owner as u8));
                if input.axis_x != FP::ZERO || input.axis_y != FP::ZERO {
                    let a = FPVec2::new(input.axis_x, input.axis_y).normalize_or_zero();
                    if a != FPVec2::ZERO {
                        aim = a;
                    }
                }
            }
            // Small deterministic RNG-driven spawn-offset jitter.
            let jitter = ctx.rng().range_fp(-fp!(2), fp!(2));
            let spawn_pos = origin + aim * fp!(25) + FPVec2::new(jitter, FP::ZERO);
            let e = ctx.frame.spawn();
            ctx.frame.add(e, Position { pos: spawn_pos });
            ctx.frame.add(e, Bullet { velocity: aim * BULLET_SPEED, owner_slot: owner, ttl: BULLET_LIFETIME_TICKS });
        }
    }
}

struct BulletSystem;
impl System<Arena> for BulletSystem {
    fn name(&self) -> &'static str {
        "BulletSystem"
    }
    fn run(&mut self, ctx: &mut SimContext<Arena>) {
        let mut to_despawn: Vec<Entity> = Vec::new();
        let mut hits: Vec<(u32, u32)> = Vec::new(); // (victim_slot, shooter_slot)

        // advance bullets
        let mut bullet_positions: Vec<(Entity, FPVec2, u32)> = Vec::new();
        for (e, (bullet, pos)) in ctx.frame.query::<(&mut Bullet, &mut Position)>() {
            pos.pos += bullet.velocity;
            if bullet.ttl == 0 {
                to_despawn.push(e);
                continue;
            }
            bullet.ttl -= 1;
            if pos.pos.x.abs() > ARENA_HALF || pos.pos.y.abs() > ARENA_HALF {
                to_despawn.push(e);
                continue;
            }
            bullet_positions.push((e, pos.pos, bullet.owner_slot));
        }

        // collide vs players
        let mut player_positions: Vec<(u32, FPVec2)> = Vec::new();
        for (_e, (tag, pos)) in ctx.frame.query::<(&PlayerTag, &Position)>() {
            player_positions.push((tag.slot, pos.pos));
        }

        let hit_radius = PLAYER_RADIUS + BULLET_RADIUS;
        for (be, bpos, owner) in &bullet_positions {
            for (pslot, ppos) in &player_positions {
                if *pslot == *owner {
                    continue;
                }
                let d2 = (*bpos - *ppos).length_sq();
                if d2 <= hit_radius * hit_radius {
                    to_despawn.push(*be);
                    hits.push((*pslot, *owner));
                    break;
                }
            }
        }

        for e in to_despawn {
            ctx.frame.despawn(e);
        }
        for (victim, shooter) in hits {
            let score = ctx.frame.singleton_mut::<Score>();
            score.kills[shooter as usize] += 1;
            ctx.emit(Hit { victim_slot: victim, shooter_slot: shooter });
        }
    }
}
