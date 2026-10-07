//! The connection between the editor and the simulation host.
//!
//! The editor owns no document and no play session. It talks to a **host**
//! (see `orr_remote::Host`): either a host thread it starts itself
//! ([`HostSpec::Local`]) or a host in another process it attaches to
//! ([`HostSpec::Remote`], `orr_remote_host` or another editor's ERP).
//! Either way it uses the same two channels:
//!
//! - an [`ErpClient`] for everything a person can do (edits, transactions,
//!   undo, history, proposals, sim control, schema, queries, activity);
//! - a [`orr_remote::RemoteBridge`] (`Bridge` + `SimControl` + `Snapshot`) for the
//!   frames the viewport draws.
//!
//! For a local host both channels are in-process links (no sockets, frames
//! as shared copies); for a remote one they are two WebSocket connections.

use std::path::PathBuf;
use std::time::Duration;

use orr_remote::sample::{spawn_arena_host, spawn_phys_host};
use orr_remote::{Auth, Caps, ErpClient, LocalHost, RemoteConfig, RemoteIdentity, ScreenshotOwner, ScreenshotService, ServerConfig, ViewDeliveryMode, USER_CLIENT};
use crate::game::{EditorGame, EditorStream};
use serde_json::{json, Value as J};

/// Where the simulation runs.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)] // one per editor, never in a collection
pub enum HostSpec {
    /// A host thread of this process on a scene file.
    Local {
        /// The scene file the host loads and saves.
        scene: PathBuf,
        /// Also listen for agents on this address (the `--erp` flag): the same
        /// host serves them, so they share the person's document and undo history.
        listen: Option<ServerConfig>,
        /// Enable the `debug.panic` test hook (tests of crash isolation).
        debug_hooks: bool,
    },
    /// A local host of the selected editor game, preserving `Local`'s legacy
    /// PhysGame behavior and public struct-literal shape.
    LocalGame {
        /// The scene file the host loads and saves.
        scene: PathBuf,
        /// The compiled game hosted on this thread.
        game: EditorGame,
        /// Optional ERP listener configuration.
        listen: Option<ServerConfig>,
        /// Enable the `debug.panic` test hook.
        debug_hooks: bool,
    },
    /// A host in another process.
    Remote {
        /// `ws://host:port` of its ERP server.
        url: String,
        /// Its token (none for a dev-mode host).
        token: Option<String>,
    },
}

impl HostSpec {
    /// A local host on `scene`, no listener.
    pub fn local(scene: impl Into<PathBuf>) -> HostSpec {
        HostSpec::Local { scene: scene.into(), listen: None, debug_hooks: false }
    }

    /// A local host using a particular compiled game and scene.
    pub fn local_game(scene: impl Into<PathBuf>, game: EditorGame) -> HostSpec {
        HostSpec::LocalGame { scene: scene.into(), game, listen: None, debug_hooks: false }
    }

    /// Adds an ERP listener to an editor-owned local host.
    pub fn with_listener(mut self, listen: ServerConfig) -> HostSpec {
        match &mut self {
            HostSpec::Local { listen: current, .. } | HostSpec::LocalGame { listen: current, .. } => *current = Some(listen),
            HostSpec::Remote { .. } => {}
        }
        self
    }

    /// Updates the persisted scene path of a local host spec after open/save.
    pub fn set_local_scene_path(&mut self, path: PathBuf) -> bool {
        match self {
            HostSpec::Local { scene, .. } | HostSpec::LocalGame { scene, .. } => {
                *scene = path;
                true
            }
            HostSpec::Remote { .. } => false,
        }
    }

    /// Attach to `url`.
    pub fn remote(url: &str, token: Option<&str>) -> HostSpec {
        HostSpec::Remote { url: url.to_string(), token: token.map(str::to_string) }
    }

    /// True for a host thread of this process.
    pub fn is_local(&self) -> bool {
        matches!(self, HostSpec::Local { .. } | HostSpec::LocalGame { .. })
    }
}

/// How long a blocking call may take before the editor gives up.
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// A live connection to a host. See the module docs.
pub struct Backend {
    /// How it was made (to restart or reconnect).
    pub spec: HostSpec,
    /// The ERP channel.
    pub erp: ErpClient,
    /// The frame channel of the scene (play frame, else the scene's preview frame).
    pub bridge: EditorStream,
    /// The host thread, for a local host.
    pub host: Option<LocalHost>,
    /// The name this editor's connections have on the host.
    pub own_client: String,
    /// The ERP address agents (or this editor) can connect to: the listener
    /// of a local host started with `--erp`, or the remote host's address.
    pub url: Option<String>,
    /// The game the host runs (`rpc.discover`).
    pub game: EditorGame,
    /// Checked descriptor identity shared by control and frame connections.
    identity: RemoteIdentity,
    pub(crate) managed_input: bool,
    /// The app-framebuffer capture endpoint, present only for an editor-owned local host.
    pub(crate) screenshot_owner: Option<ScreenshotOwner>,
}

/// `ws://host:port` plus the path and query a dev-mode host needs to know
/// this is a person's view.
fn ws_url(url: &str, token: Option<&str>) -> String {
    if token.is_some() {
        return url.to_string();
    }
    let authority_end = url.find("://").map_or(0, |i| i + 3);
    let url = if url[authority_end..].contains('/') { url.to_string() } else { format!("{url}/") };
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}client={USER_CLIENT}")
}

impl Backend {
    /// Starts the host thread or attaches to the host, and opens both channels.
    pub fn connect(spec: &HostSpec) -> Result<Backend, String> {
        match spec {
            HostSpec::Local { scene, listen, debug_hooks } | HostSpec::LocalGame { scene, listen, debug_hooks, .. } => {
                let selected_game = match spec {
                    HostSpec::Local { .. } => EditorGame::PhysGame,
                    HostSpec::LocalGame { game, .. } => *game,
                    HostSpec::Remote { .. } => unreachable!("matched local host spec"),
                };
                let text = std::fs::read_to_string(scene).map_err(|e| format!("cannot read {}: {e}", scene.display()))?;
                let mut cfg = match listen {
                    Some(c) => c.clone(),
                    None => {
                        let mut c = ServerConfig::new(Auth::DevNoAuth);
                        c.listen = false;
                        c
                    }
                };
                cfg.listen = listen.is_some();
                cfg.limits.allow_scene_paths = true;
                cfg.limits.debug_hooks = *debug_hooks;
                cfg.limits.max_step_per_call = 20_000;
                let (screenshot_service, screenshot_owner) = ScreenshotService::pair();
                cfg.screenshot = Some(screenshot_service);
                let host = match selected_game {
                    EditorGame::PhysGame => spawn_phys_host(text, Some(scene.clone()), cfg)?,
                    EditorGame::Arena => spawn_arena_host(text, Some(scene.clone()), cfg)?,
                    EditorGame::Yard3D => orr_remote::yard3d::spawn_yard3d_scene_host(scene.clone(), cfg)?,
                };
                let connector = host.connector();
                let link = |what: &str| connector.connect(USER_CLIENT, Caps::ALL).map_err(|e| format!("{what}: {e}"));
                let mut erp = ErpClient::with_transport(Box::new(link("ERP")?));
                erp.call_timeout = CALL_TIMEOUT;
                let mut rc = RemoteConfig::new("");
                rc.source = "view".to_string();
                rc.view_delivery = ViewDeliveryMode::RequireFenced;
                // In process a frame is a copy, not a message: let an edit show at once instead of
                // waiting out a 60 Hz cap (the window draws at most as often as it likes anyway).
                rc.max_fps = 240;
                let (game, identity, own_client, managed_input) = discover(&mut erp)?;
                rc.expected_identity = Some(identity.clone());
                let bridge = EditorStream::connect_transport(game, Box::new(link("frames")?), rc)?;
                let url = host.url().map(str::to_string);
                Ok(Backend { spec: spec.clone(), erp, bridge, host: Some(host), own_client, url, game, identity, managed_input, screenshot_owner: Some(screenshot_owner) })
            }
            HostSpec::Remote { url, token } => {
                let full = ws_url(url, token.as_deref());
                let mut erp = match token {
                    Some(t) => ErpClient::connect_pumped(url, Some(t)),
                    None => ErpClient::connect_pumped(&full, None),
                }
                .map_err(|e| format!("cannot connect to {url}: {e}"))?;
                erp.call_timeout = CALL_TIMEOUT;
                let (game, identity, own_client, managed_input) = discover(&mut erp)?;
                let mut rc = RemoteConfig::new(if token.is_some() { url } else { &full });
                rc.token.clone_from(token);
                rc.source = "view".to_string();
                rc.view_delivery = delivery(game, false);
                rc.expected_identity = Some(identity.clone());
                let bridge = EditorStream::connect(game, rc).map_err(|e| format!("frame stream of {url}: {e}"))?;
                Ok(Backend { spec: spec.clone(), erp, bridge, host: None, own_client, url: Some(url.clone()), game, identity, managed_input, screenshot_owner: None })
            }
        }
    }

    /// Another frame stream, for what the viewport previews (`proposal:p3`).
    pub fn frame_stream(&self, source: &str) -> Result<EditorStream, String> {
        let mut rc = match &self.spec {
            HostSpec::Local { .. } | HostSpec::LocalGame { .. } => RemoteConfig::new(""),
            HostSpec::Remote { url, token } => {
                let mut c = RemoteConfig::new(&if token.is_some() { url.clone() } else { ws_url(url, None) });
                c.token.clone_from(token);
                c
            }
        };
        rc.source = source.to_string();
        rc.view_delivery = delivery(self.game, self.spec.is_local());
        rc.expected_identity = Some(self.identity.clone());
        match &self.host {
            Some(h) => {
                let t = h.connector().connect(USER_CLIENT, Caps::ALL).map_err(|e| e.to_string())?;
                EditorStream::connect_transport(self.game, Box::new(t), rc)
            }
            None => EditorStream::connect(self.game, rc),
        }
    }

    /// A second ERP client with its own name, like an agent's. Local hosts
    /// only (scripts use it to stage proposals the way an agent would).
    pub fn agent_client(&self, name: &str) -> Result<ErpClient, String> {
        let host = self.host.as_ref().ok_or("staging proposals as an agent needs the editor's own host")?;
        let caps = Caps::parse("read,scene_edit,sim_control").map_err(|e| e.to_string())?;
        let t = host.connector().connect(name, caps).map_err(|e| e.to_string())?;
        let mut c = ErpClient::with_transport(Box::new(t));
        c.call_timeout = CALL_TIMEOUT;
        Ok(c)
    }

    /// Subscribes to the notifications the editor shows.
    pub fn subscribe(&mut self) -> Result<(), String> {
        self.erp
            .call("watch.subscribe", json!({"topics": ["tick", "history", "proposals", "activity"], "include_reads": true}))
            .map(|_| ())
            .map_err(|e| format!("watch.subscribe: {e}"))
    }
}

// Discover before choosing a native frame decoder. Schema comparison includes
// fields, ranges and descriptors, not just type names. The stream repeats the
// identity check on its own connection before subscribing. This checks game and
// build/schema compatibility, not a unique host instance or source-content hash.
fn discover(erp: &mut ErpClient) -> Result<(EditorGame, RemoteIdentity, String, bool), String> {
    let d = erp.call("rpc.discover", J::Null).map_err(|e| format!("rpc.discover: {e}"))?;
    if d["erp_version"].as_u64() != Some(1) {
        return Err("editor requires ERP version 1".into());
    }
    let name = d.pointer("/engine/game").and_then(J::as_str).ok_or("host discovery is missing explicit engine.game")?;
    let game = EditorGame::from_name(name)?;
    let build_id = d.pointer("/engine/build_id").and_then(J::as_str).filter(|s| !s.is_empty()).ok_or("host discovery is missing engine.build_id")?.to_string();
    let schema: J = serde_json::from_str(&game.types().json_schema()).map_err(|e| format!("compiled schema: {e}"))?;
    let actual = erp.call("registry.schema", J::Null).map_err(|e| format!("registry.schema: {e}"))?;
    if actual.get("schema") != Some(&schema) {
        return Err(format!("{name} reflected schema mismatch: host descriptors differ from this editor"));
    }
    let own_client = d.pointer("/you/client").and_then(J::as_str).unwrap_or(USER_CLIENT).to_string();
    let input = if game == EditorGame::Arena { erp.call("registry.input", J::Null).ok() } else { None };
    let mut input_types = orr_reflect::TypeRegistry::new();
    input_types.register_component::<orr_sample::arena_game::ArenaInput>("ArenaInput");
    let expected_input: J = serde_json::from_str(&input_types.type_json_schema("ArenaInput").expect("registered input")).expect("valid reflected schema");
    let managed_input = game == EditorGame::Arena && input.as_ref().is_some_and(|v| v["schema"] == expected_input) && input.as_ref().and_then(|v| v.get("managed_held")).is_some_and(|v| v["version"] == 1 && v["lease_ms"] == 2000 && v["heartbeat_ms"] == 500);
    Ok((game, RemoteIdentity { game: name.to_string(), build_id, schema }, own_client, managed_input))
}

fn delivery(game: EditorGame, local: bool) -> ViewDeliveryMode {
    if local || matches!(game, EditorGame::Arena | EditorGame::Yard3D) { ViewDeliveryMode::RequireFenced } else { ViewDeliveryMode::PreferFenced }
}
