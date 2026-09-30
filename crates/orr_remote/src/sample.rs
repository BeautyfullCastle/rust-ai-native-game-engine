//! The sample physics game (`PhysGame`) as an ERP host: its type registry,
//! its hooks (metrics, scripted player) and a ready-made [`LocalHost`]. The
//! headless `orr_remote_host` and the editor use these, so both serve the
//! same game the same way. Needs the `sample-host` feature.

use std::path::PathBuf;

use orr_edit::{EditError, EditorDoc};
use orr_reflect::TypeRegistry;
use orr_sample::physics_game::{bot_input, register_reflect, PhysGame, PhysMetrics, TICK_RATE};
use orr_sample::physics_stream::phys_stream_source;
use orr_sim::{PlayerSlot, Simulation};

use crate::dispatch::HostLimits;
use crate::local::LocalHost;
use crate::proposals::{default_build_id, GameHooks};
use crate::viewstream::ViewStreamHook;
use crate::server::ServerConfig;

/// Seed of the preview frame and of the play sessions.
pub const SEED: u64 = 7;
/// Players of a play session (`PhysGame` has one paddle per player).
pub const PLAYERS: u8 = 2;

/// The type registry of `PhysGame` (physics types, `PaddleTag`, `Scene`).
pub fn phys_types() -> TypeRegistry {
    let mut t = TypeRegistry::new();
    register_reflect(&mut t);
    t
}

/// A document of `PhysGame` from scene text.
pub fn phys_doc(text: &str) -> Result<EditorDoc, EditError> {
    EditorDoc::from_yaml(text, phys_types(), Simulation::<PhysGame>::build_registry(), SEED)
}

/// The game-specific parts of a host for `PhysGame`: players, tick rate,
/// build id, metrics, the scripted player and the view stream (`viewstream` topic). Sets `scene_path` as the file
/// `scene.save {write: true}` writes.
pub fn configure_phys(limits: &mut HostLimits, scene_path: Option<PathBuf>) {
    limits.player_count = PLAYERS;
    limits.tick_rate = TICK_RATE;
    limits.scene_path = scene_path;
    limits.build_id = default_build_id("PhysGame");
    limits.game = GameHooks::new("PhysGame").with_metrics(PhysMetrics).with_bot(|seed, tick, slot| bot_input(seed, tick, PlayerSlot(slot)));
    limits.view_stream = Some(ViewStreamHook::new(phys_stream_source(limits.build_id, PLAYERS)));
}

/// Starts a host thread of `PhysGame` on the scene `text`. `cfg.limits` get
/// the game's settings ([`configure_phys`]). The scene's file name is
/// `scene_path`; the editor's host also lets clients name files
/// (`allow_scene_paths`).
pub fn spawn_phys_host(text: String, scene_path: Option<PathBuf>, mut cfg: ServerConfig) -> Result<LocalHost, String> {
    LocalHost::spawn::<PhysGame>(move || {
        let doc = phys_doc(&text).map_err(|e| match &scene_path {
            Some(p) => format!("{}: {e}", p.display()),
            None => e.to_string(),
        })?;
        configure_phys(&mut cfg.limits, scene_path);
        Ok((doc, cfg))
    })
}
