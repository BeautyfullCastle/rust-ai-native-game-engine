//! Client mode: the host is a relay client (it plays on an `orr_server` room) instead of owning
//! a simulation. Its ERP `viewstream` topic then streams the frames and events of the client
//! session, with the rollback flag and range and the predicted/verified/canceled event states
//! (see `docs/view-stream.md`, "Multiplayer (client sessions)").
//!
//! The server only knows the [`ClientSession`] trait; `orr_remote::sample` implements it for the
//! physics sample's `orr_sample::relay_view::RelayView`, the same session the C ABI uses.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use orr_viewstream::Schema;
use serde_json::{json, Value as J};

use crate::codec::hex_decode;
use crate::error::*;
use crate::wire::checksum_text;

/// Why a call of the session failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionErrorKind {
    /// Still joining.
    NotReady,
    /// Wrong arguments (another player's slot, a wrong size).
    Arg,
    /// The session is failed, disconnected or gone.
    Host,
}

/// A failed call of the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionError {
    pub kind: SessionErrorKind,
    pub message: String,
}

/// What one pump of the session produced: encoded view stream messages.
#[derive(Debug, Default)]
pub struct ClientPump {
    /// The newest frame message, if the session moved since the last pump.
    pub frame: Option<Vec<u8>>,
    /// An event batch message: the event states since the last pump.
    pub events: Option<Vec<u8>>,
}

/// A running relay client session.
pub trait ClientSession: Send {
    /// What the session produced since the last call.
    fn pump(&mut self) -> ClientPump;
    /// The schema of the stream (known once joined).
    fn schema(&self) -> Option<Schema>;
    /// The `session.status` result.
    fn status(&self) -> J;
    /// `(tick, checksum)` of the confirmed state at `tick` (0 = the newest checkpoint).
    fn confirmed_checksum(&self, tick: u64) -> Option<(u64, u64)>;
    /// Sets the held input of the joined slot.
    fn set_input(&mut self, player: u8, bytes: &[u8]) -> Result<(), SessionError>;
    /// Sends a game command with the next tick.
    fn send_command(&mut self, player: u8, bytes: &[u8]) -> Result<(), SessionError>;
}

/// A shared handle to the host's [`ClientSession`] (cloneable, so the settings stay `Clone`).
#[derive(Clone)]
pub struct ClientSessionHook(Arc<Mutex<dyn ClientSession>>);

impl ClientSessionHook {
    /// Wraps a session.
    pub fn new(session: impl ClientSession + 'static) -> Self {
        Self(Arc::new(Mutex::new(session)))
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, dyn ClientSession + 'static> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl core::fmt::Debug for ClientSessionHook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ClientSessionHook")
    }
}

fn session_error(e: SessionError) -> RpcError {
    match e.kind {
        SessionErrorKind::Arg => RpcError::params(e.message),
        SessionErrorKind::NotReady => RpcError::state("not_ready", e.message),
        SessionErrorKind::Host => RpcError::state("session_lost", e.message),
    }
}

/// The refusal of a method that needs a document or a local simulation.
pub(crate) fn unavailable(method: &str) -> RpcError {
    RpcError::state(
        "not_in_client_mode",
        format!("'{method}' is not available in client mode: this host is a relay client and plays on a server (see session.status, sim.input, sim.command, watch.subscribe viewstream, activity.list)"),
    )
}

/// Runs a method in client mode.
pub(crate) fn call(hook: &ClientSessionHook, method: &str, params: &J) -> Result<J, RpcError> {
    let obj = match params {
        J::Null => serde_json::Map::new(),
        J::Object(m) => m.clone(),
        _ => return Err(RpcError::params("params must be an object")),
    };
    let player = |required: bool| -> Result<u8, RpcError> {
        match obj.get("player") {
            None | Some(J::Null) if !required => Ok(0),
            None | Some(J::Null) => Err(RpcError::params("missing parameter 'player'")),
            Some(v) => v
                .as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .ok_or_else(|| RpcError::params("'player' must be a slot number")),
        }
    };
    let hex = |name: &str| -> Result<Vec<u8>, RpcError> {
        let text = obj.get(name).and_then(J::as_str).ok_or_else(|| RpcError::params(format!("missing parameter '{name}' (hex bytes)")))?;
        hex_decode(text).ok_or_else(|| RpcError::params(format!("'{name}' is not valid hex")))
    };
    match method {
        "sim.state" | "session.status" => Ok(hook.lock().status()),
        "sim.checksum" => {
            let tick = match obj.get("tick") {
                None | Some(J::Null) => 0,
                Some(v) => v.as_u64().ok_or_else(|| RpcError::params("'tick' must be a non-negative integer"))?,
            };
            match hook.lock().confirmed_checksum(tick) {
                Some((t, c)) => Ok(json!({"tick": t, "checksum": checksum_text(c), "confirmed": true})),
                None if tick == 0 => Err(RpcError::state("no_checkpoint", "no confirmed checkpoint yet")),
                None => Err(RpcError::new(NOT_FOUND, "no_checkpoint", format!("no confirmed checksum at tick {tick} (only checkpoint ticks have one)"))),
            }
        }
        "sim.input" => {
            let (slot, bytes) = (player(true)?, hex("input")?);
            hook.lock().set_input(slot, &bytes).map_err(session_error)?;
            Ok(json!({"ok": true}))
        }
        "sim.command" => {
            let (slot, bytes) = (player(false)?, hex("command")?);
            hook.lock().send_command(slot, &bytes).map_err(session_error)?;
            Ok(json!({"ok": true}))
        }
        other => Err(unavailable(other)),
    }
}
