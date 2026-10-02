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
//! - a [`RemoteBridge`] (`Bridge` + `SimControl` + `Snapshot`) for the
//!   frames the viewport draws.
//!
//! For a local host both channels are in-process links (no sockets, frames
//! as shared copies); for a remote one they are two WebSocket connections.

use std::path::PathBuf;
use std::time::Duration;

use orr_remote::sample::spawn_phys_host;
use orr_remote::{Auth, Caps, ErpClient, LocalHost, RemoteBridge, RemoteConfig, ServerConfig, ViewDeliveryMode, USER_CLIENT};
use orr_sample::physics_game::PhysGame;
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

    /// Attach to `url`.
    pub fn remote(url: &str, token: Option<&str>) -> HostSpec {
        HostSpec::Remote { url: url.to_string(), token: token.map(str::to_string) }
    }

    /// True for a host thread of this process.
    pub fn is_local(&self) -> bool {
        matches!(self, HostSpec::Local { .. })
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
    pub bridge: RemoteBridge<PhysGame>,
    /// The host thread, for a local host.
    pub host: Option<LocalHost>,
    /// The name this editor's connections have on the host.
    pub own_client: String,
    /// The ERP address agents (or this editor) can connect to: the listener
    /// of a local host started with `--erp`, or the remote host's address.
    pub url: Option<String>,
    /// The game the host runs (`rpc.discover`).
    pub game: String,
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
            HostSpec::Local { scene, listen, debug_hooks } => {
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
                let host = spawn_phys_host(text, Some(scene.clone()), cfg)?;
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
                let bridge = RemoteBridge::<PhysGame>::connect_transport(Box::new(link("frames")?), rc)?;
                let url = host.url().map(str::to_string);
                let mut b = Backend { spec: spec.clone(), erp, bridge, host: Some(host), own_client: USER_CLIENT.to_string(), url, game: String::new() };
                b.handshake()?;
                Ok(b)
            }
            HostSpec::Remote { url, token } => {
                let full = ws_url(url, token.as_deref());
                let mut erp = match token {
                    Some(t) => ErpClient::connect_pumped(url, Some(t)),
                    None => ErpClient::connect_pumped(&full, None),
                }
                .map_err(|e| format!("cannot connect to {url}: {e}"))?;
                erp.call_timeout = CALL_TIMEOUT;
                let mut rc = RemoteConfig::new(if token.is_some() { url } else { &full });
                rc.token.clone_from(token);
                rc.source = "view".to_string();
                rc.view_delivery = ViewDeliveryMode::PreferFenced;
                let bridge = RemoteBridge::<PhysGame>::connect(rc).map_err(|e| format!("frame stream of {url}: {e}"))?;
                let mut b = Backend { spec: spec.clone(), erp, bridge, host: None, own_client: USER_CLIENT.to_string(), url: Some(url.clone()), game: String::new() };
                b.handshake()?;
                Ok(b)
            }
        }
    }

    /// Learns who this connection is and what the host runs.
    fn handshake(&mut self) -> Result<(), String> {
        let d = self.erp.call("rpc.discover", J::Null).map_err(|e| format!("rpc.discover: {e}"))?;
        if let Some(c) = d.pointer("/you/client").and_then(J::as_str) {
            self.own_client = c.to_string();
        }
        self.game = d.pointer("/engine/game").and_then(J::as_str).unwrap_or_default().to_string();
        if !self.game.is_empty() && self.game != "PhysGame" {
            return Err(format!("the host runs '{}'; this editor draws PhysGame scenes", self.game));
        }
        Ok(())
    }

    /// Another frame stream, for what the viewport previews (`proposal:p3`).
    pub fn frame_stream(&self, source: &str) -> Result<RemoteBridge<PhysGame>, String> {
        let mut rc = match &self.spec {
            HostSpec::Local { .. } => RemoteConfig::new(""),
            HostSpec::Remote { url, token } => {
                let mut c = RemoteConfig::new(&if token.is_some() { url.clone() } else { ws_url(url, None) });
                c.token.clone_from(token);
                c
            }
        };
        rc.source = source.to_string();
        rc.view_delivery = if self.spec.is_local() { ViewDeliveryMode::RequireFenced } else { ViewDeliveryMode::PreferFenced };
        match &self.host {
            Some(h) => {
                let t = h.connector().connect(USER_CLIENT, Caps::ALL).map_err(|e| e.to_string())?;
                RemoteBridge::<PhysGame>::connect_transport(Box::new(t), rc)
            }
            None => RemoteBridge::<PhysGame>::connect(rc),
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
