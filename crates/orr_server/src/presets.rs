//! Room settings of the sample games, so a server can be started with
//! `--game arena` or `--game physics`.
//!
//! The server does not know the games. What the players must agree on
//! travels in the room config: build hash, input size, seed, and an opaque
//! config blob that every client hands to its `Game::Config`. For the sample
//! games these values are constants here and in `orr_sample` (a test in
//! `orr_sample` checks that they match).

use orr_ecs::Frame;
use orr_fp::{FPVec2, FP};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, PlayerTag, Position, SpawnBulletCmd, FIRE, PLAYER_SPEED};

use crate::{Audit, GameSim, RoomConfig, Violation};

/// Build id of the arena sample (`Simulation::with_build_id`). The room
/// build hash is `orr_sim::build_hash_of(build_id, 0)`.
pub const ARENA_BUILD_ID: u64 = 0x0A2E_4A00_0001;
/// Build id of the physics sample.
pub const PHYSICS_BUILD_ID: u64 = 0x0A2E_4A00_0002;
/// Byte size of `ArenaInput` and of `PhysInput`.
pub const SAMPLE_INPUT_SIZE: u32 = 24;
/// Default sim seed of the sample rooms.
pub const SAMPLE_SEED: u64 = 0x5EED_0A2E;

/// A game the server can host by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Game {
    Arena,
    Physics,
    /// Everything comes from the command line.
    Custom,
}

impl core::str::FromStr for Game {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "arena" => Ok(Game::Arena),
            "physics" => Ok(Game::Physics),
            "custom" => Ok(Game::Custom),
            other => Err(format!("unknown game '{other}' (arena, physics or custom)")),
        }
    }
}

/// Scene settings of the physics sample, encoded in its config blob.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicsScene {
    pub bodies: u32,
    /// 0 rain, 1 pile, 2 mixer.
    pub mode: u8,
    pub spawn_rate: u32,
    pub max_entities: u32,
    pub layout_seed: u64,
}

impl Default for PhysicsScene {
    fn default() -> Self {
        Self { bodies: 1000, mode: 0, spawn_rate: 0, max_entities: 20_000, layout_seed: 0x0DDB_A110 }
    }
}

const PHYS_MAGIC: &[u8; 4] = b"PHY1";

impl PhysicsScene {
    /// The config blob: `"PHY1"`, then little-endian `bodies u32, mode u8,
    /// spawn_rate u32, max_entities u32, layout_seed u64`.
    pub fn to_blob(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(25);
        b.extend_from_slice(PHYS_MAGIC);
        b.extend_from_slice(&self.bodies.to_le_bytes());
        b.push(self.mode);
        b.extend_from_slice(&self.spawn_rate.to_le_bytes());
        b.extend_from_slice(&self.max_entities.to_le_bytes());
        b.extend_from_slice(&self.layout_seed.to_le_bytes());
        b
    }
}

/// Room settings for a named game. `players` clients, `tick_rate` Hz.
pub fn room_config(game: Game, players: u8, tick_rate: u32, seed: u64, scene: PhysicsScene) -> RoomConfig {
    let mut cfg = RoomConfig::new(players, tick_rate, seed, SAMPLE_INPUT_SIZE);
    match game {
        Game::Arena => cfg.build_hash = orr_sim::build_hash_of(ARENA_BUILD_ID, 0),
        Game::Physics => {
            cfg.build_hash = orr_sim::build_hash_of(PHYSICS_BUILD_ID, 0);
            cfg.config_blob = scene.to_blob();
        }
        Game::Custom => {}
    }
    cfg
}

// ---- authoritative arena ---------------------------------------------------

/// The server-side AI of the arena sample: moves toward the nearest other
/// player and fires every 12 ticks (staggered by slot). It reads only the
/// server's frame, so it is deterministic; its input and `SpawnBulletCmd`
/// ride in the confirmed bundle like a human's.
pub fn arena_ai(frame: &Frame, tick: u64, slot: u8) -> (ArenaInput, Vec<SpawnBulletCmd>) {
    let (entities, tags) = frame.dense::<PlayerTag>();
    let mut me: Option<FPVec2> = None;
    for (e, tag) in entities.iter().zip(tags) {
        if tag.slot == u32::from(slot) {
            me = frame.get::<Position>(*e).map(|p| p.pos);
        }
    }
    let mut target: Option<(FP, FPVec2)> = None;
    if let Some(me) = me {
        for (e, tag) in entities.iter().zip(tags) {
            if tag.slot == u32::from(slot) {
                continue;
            }
            let Some(p) = frame.get::<Position>(*e) else { continue };
            let d = (p.pos - me).length_sq();
            if target.is_none_or(|(best, _)| d < best) {
                target = Some((d, p.pos));
            }
        }
    }
    let (ax, ay) = match (me, target) {
        (Some(me), Some((_, t))) => {
            let dead = FP::from_int(10);
            let dir = |d: FP| if d > dead { FP::ONE } else if d < -dead { -FP::ONE } else { FP::ZERO };
            (dir(t.x - me.x), dir(t.y - me.y))
        }
        _ => (FP::ZERO, FP::ZERO),
    };
    let fire = tick % 12 == u64::from(slot) % 12;
    let input = ArenaInput::new(ax, ay, fire);
    let cmds = if input.buttons & FIRE != 0 { vec![SpawnBulletCmd { owner: u32::from(slot) }] } else { Vec::new() };
    (input, cmds)
}

/// The arena's state audit: a player cannot move more than `max_step` units
/// in one tick (honest input moves at most `PLAYER_SPEED * sqrt(2)`, about
/// 8.5; a forged axis value moves farther). Computed from the server's frames
/// before and after the tick.
pub fn arena_speed_audit(max_step: FP) -> impl FnMut(&Audit<'_, Arena>) -> Vec<Violation> + Send {
    move |a| {
        let mut out = Vec::new();
        let (entities, tags) = a.after.dense::<PlayerTag>();
        let limit = max_step * max_step;
        for (e, tag) in entities.iter().zip(tags) {
            let (Some(now), Some(then)) = (a.after.get::<Position>(*e), a.before.get::<Position>(*e)) else { continue };
            let d2 = (now.pos - then.pos).length_sq();
            if d2 > limit {
                out.push(Violation::new(tag.slot as u8, format!("moved {d2:?} (squared) in one tick, limit {limit:?}")));
            }
        }
        out
    }
}

/// The server sim of an authoritative arena room. `ai_slots` are played by
/// [`arena_ai`]; every slot is audited with [`arena_speed_audit`].
/// `build_id` must be the clients' (`ARENA_BUILD_ID` for the sample app).
pub fn arena_sim(players: u8, tick_rate: u32, seed: u64, build_id: u64, ai_slots: &[u8]) -> GameSim<Arena> {
    let audit_limit = PLAYER_SPEED * FP::from_raw(3 << 15); // 1.5 * 6 = 9
    let mut sim = GameSim::<Arena>::new(ArenaConfig { player_count: players }, tick_rate, seed, build_id, players)
        .with_audit(arena_speed_audit(audit_limit));
    if !ai_slots.is_empty() {
        sim = sim.with_brain(arena_ai);
    }
    sim
}
