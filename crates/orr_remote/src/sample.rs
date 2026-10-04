//! The sample physics game (`PhysGame`) as an ERP host: its type registry,
//! its hooks (metrics, scripted player) and a ready-made [`LocalHost`]. The
//! headless `orr_remote_host` and the editor use these, so both serve the
//! same game the same way. Needs the `sample-host` feature.

use std::path::PathBuf;

use orr_edit::{EditError, EditorDoc};
use orr_reflect::TypeRegistry;
use orr_sample::net_client::NetArgs;
use orr_sample::physics_game::{bot_input, register_reflect, PhysGame, PhysMetrics, TICK_RATE};
use orr_sample::physics_stream::phys_stream_source;
use orr_sample::relay_view::{RelayView, ViewError, ViewErrorKind};
use orr_sim::{PlayerSlot, Simulation};
use orr_viewstream::Schema;
use serde_json::{json, Value as J};

use crate::client_mode::{ClientPump, ClientSession, ClientSessionHook, SessionError, SessionErrorKind};
use crate::dispatch::HostLimits;
use crate::local::LocalHost;
use crate::proposals::{default_build_id, GameHooks};
use crate::viewstream::ViewStreamHook;
use crate::server::ServerConfig;
use crate::wire::checksum_text;

use orr_testgame::{Arena, ArenaMetrics};

/// Seed of the preview frame and of the play sessions.
pub const SEED: u64 = 7;
/// Players of a play session (`PhysGame` has one paddle per player).
pub const PLAYERS: u8 = 2;
/// Stable seed used by the editor's local Arena authoring host.
pub const ARENA_SEED: u64 = 42;
/// Default number of Arena player slots exposed by the local authoring host.
pub const ARENA_PLAYERS: u8 = 2;
/// Fixed update rate for the local Arena authoring host.
pub const ARENA_TICK_RATE: u32 = 60;

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

/// A document of the deterministic Arena sample game from scene text.
pub fn arena_doc(text: &str) -> Result<EditorDoc, EditError> {
    let mut types = TypeRegistry::new();
    orr_testgame::register_reflect(&mut types);
    EditorDoc::from_yaml(text, types, Simulation::<Arena>::build_registry(), ARENA_SEED)
}

/// The game-specific host configuration for the editor's local Arena.
pub fn configure_arena(limits: &mut HostLimits, scene_path: Option<PathBuf>) {
    limits.player_count = ARENA_PLAYERS;
    limits.tick_rate = ARENA_TICK_RATE;
    limits.scene_path = scene_path;
    limits.build_id = default_build_id("Arena");
    // Live input is installed on the ERP server after it starts below.
    limits.game = GameHooks::new("Arena").with_metrics(ArenaMetrics);
    limits.view_stream = None;
}

/// Starts a local Arena host with the same structured held-input contract as
/// the headless Arena host. Input derivation is installed after ERP startup
/// and before the host is made ready to clients.
pub fn spawn_arena_host(text: String, scene_path: Option<PathBuf>, mut cfg: ServerConfig) -> Result<LocalHost, String> {
    LocalHost::spawn_configured::<Arena>(
        move || {
            let doc = arena_doc(&text).map_err(|e| match &scene_path {
                Some(p) => format!("{}: {e}", p.display()),
                None => e.to_string(),
            })?;
            configure_arena(&mut cfg.limits, scene_path);
            Ok((doc, cfg))
        },
        |server| {
            server.set_structured_input::<Arena>("ArenaInput", 8, |slot, input| {
                orr_sample::arena_view::arena_fire_commands(u32::from(slot.0), input)
            });
            server.enable_managed_input();
        },
    )
}

/// The relay client session of `orr_sample` as an ERP client-mode session (`orr_remote_host --join`):
/// the same `RelayView` the C ABI uses, so the view stream bytes are the same.
pub struct PhysClientSession {
    view: RelayView,
    schema: Schema,
}

fn session_error(e: ViewError) -> SessionError {
    let kind = match e.kind {
        ViewErrorKind::NotReady => SessionErrorKind::NotReady,
        ViewErrorKind::Arg => SessionErrorKind::Arg,
        ViewErrorKind::Host => SessionErrorKind::Host,
    };
    SessionError { kind, message: e.message }
}

impl ClientSession for PhysClientSession {
    fn pump(&mut self) -> ClientPump {
        let out = self.view.pump();
        ClientPump { frame: out.frame, events: out.events }
    }

    fn schema(&self) -> Option<Schema> {
        Some(self.schema.clone())
    }

    fn status(&self) -> J {
        let s = self.view.status();
        let confirmed = self.view.confirmed_checksum(0).map(|(tick, c)| json!({"tick": tick, "checksum": checksum_text(c)}));
        json!({
            "mode": "client",
            "state": s.state.name(),
            "playing": s.state == orr_sample::relay_view::ViewState::Playing,
            "slot": s.slot,
            "player_count": s.player_count,
            "rtt_ms": s.rtt_ms,
            "input_delay": s.input_delay,
            "desyncs": s.desyncs,
            "head_tick": s.head_tick,
            "last_tick": s.head_tick,
            "verified_tick": s.verified_tick,
            "rollbacks": s.rollbacks,
            "resim_ticks": s.resim_ticks,
            "last_rollback_from": s.last_rollback_from,
            "last_rollback_to": s.last_rollback_to,
            "stall_episodes": s.stall_episodes,
            "stalled_ms": s.stalled_ms,
            "repeats": s.repeats,
            "confirmed": confirmed,
        })
    }

    fn confirmed_checksum(&self, tick: u64) -> Option<(u64, u64)> {
        self.view.confirmed_checksum(tick)
    }

    fn set_input(&mut self, player: u8, bytes: &[u8]) -> Result<(), SessionError> {
        self.view.set_input(player, bytes).map_err(session_error)
    }

    fn send_command(&mut self, player: u8, bytes: &[u8]) -> Result<(), SessionError> {
        self.view.send_command(player, bytes).map_err(session_error)
    }
}

/// Joins the room of `args` (blocks until it has started) and puts the session into `limits` as
/// the host's client mode: players, tick rate and build id come from the room.
pub fn join_phys_client(limits: &mut HostLimits, args: NetArgs) -> Result<(), String> {
    let (view, ready) = RelayView::join(args)?;
    limits.player_count = ready.player_count;
    limits.tick_rate = ready.schema.tick_rate;
    limits.build_id = ready.schema.build_id;
    limits.view_stream = None;
    limits.client_session = Some(ClientSessionHook::new(PhysClientSession { view, schema: ready.schema }));
    Ok(())
}
