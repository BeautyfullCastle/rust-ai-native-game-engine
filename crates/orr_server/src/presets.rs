//! Room settings of the sample games, so a server can be started with
//! `--game arena` or `--game physics`.
//!
//! The server does not know the games. What the players must agree on
//! travels in the room config: build hash, input size, seed, and an opaque
//! config blob that every client hands to its `Game::Config`. For the sample
//! games these values are constants here and in `orr_sample` (a test in
//! `orr_sample` checks that they match).

use crate::RoomConfig;

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
