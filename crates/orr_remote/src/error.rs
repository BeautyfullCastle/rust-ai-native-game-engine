//! JSON-RPC errors and the mapping from editor errors.

use orr_edit::EditError;
use serde_json::{json, Value as J};

/// The text is not valid JSON.
pub const PARSE_ERROR: i64 = -32700;
/// The JSON is not a valid request object.
pub const INVALID_REQUEST: i64 = -32600;
/// No such method.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// Missing or wrong parameters.
pub const INVALID_PARAMS: i64 = -32602;
/// A bug on the server side (a panic was caught).
pub const INTERNAL_ERROR: i64 = -32603;
/// Not authenticated, or the token was refused.
pub const UNAUTHENTICATED: i64 = -32001;
/// The token lacks the capability the method needs.
pub const PERMISSION_DENIED: i64 = -32002;
/// The call is valid but not now (no play session, a transaction is open, ...).
pub const INVALID_STATE: i64 = -32003;
/// The entity, component or type does not exist.
pub const NOT_FOUND: i64 = -32004;
/// The edit conflicts with the document (already exists, still referenced).
pub const CONFLICT: i64 = -32005;
/// The value was refused (type, range, path) or the scene text is invalid.
pub const INVALID_VALUE: i64 = -32010;
/// The play session refused a debug command.
pub const DEBUG_REFUSED: i64 = -32011;
/// The request was too large or the server is too busy.
pub const LIMIT_EXCEEDED: i64 = -32020;

/// An error result of a call.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    /// JSON-RPC code (see the constants).
    pub code: i64,
    /// Human-readable message.
    pub message: String,
    /// Machine-readable extra (always an object with a `kind` text).
    pub data: Option<J>,
}

impl RpcError {
    /// An error with a `kind` tag in `data`.
    pub fn new(code: i64, kind: &str, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: Some(json!({ "kind": kind })) }
    }
    /// Adds a field to `data`.
    pub fn with(mut self, key: &str, value: J) -> Self {
        if let Some(J::Object(m)) = &mut self.data {
            m.insert(key.to_string(), value);
        }
        self
    }
    /// `-32602` with a message.
    pub fn params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, "invalid_params", message)
    }
    /// `-32003` with a message.
    pub fn state(kind: &str, message: impl Into<String>) -> Self {
        Self::new(INVALID_STATE, kind, message)
    }
    /// The `error` member of a response.
    pub fn to_json(&self) -> J {
        let mut o = serde_json::Map::new();
        o.insert("code".into(), json!(self.code));
        o.insert("message".into(), json!(self.message));
        if let Some(d) = &self.data {
            o.insert("data".into(), d.clone());
        }
        J::Object(o)
    }
    /// The error of a response, back to a value (client side).
    pub fn from_json(j: &J) -> RpcError {
        RpcError {
            code: j.get("code").and_then(J::as_i64).unwrap_or(INTERNAL_ERROR),
            message: j.get("message").and_then(J::as_str).unwrap_or("").to_string(),
            data: j.get("data").cloned(),
        }
    }
    /// The `data.kind` tag, if any.
    pub fn kind(&self) -> Option<&str> {
        self.data.as_ref()?.get("kind")?.as_str()
    }
}

impl core::fmt::Display for RpcError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}
impl std::error::Error for RpcError {}

impl From<EditError> for RpcError {
    fn from(e: EditError) -> Self {
        let msg = e.to_string();
        match &e {
            EditError::UnknownEntity(_) => RpcError::new(NOT_FOUND, "unknown_entity", msg),
            EditError::UnknownType(_) => RpcError::new(NOT_FOUND, "unknown_type", msg),
            EditError::NoComponent { .. } => RpcError::new(NOT_FOUND, "no_component", msg),
            EditError::HasComponent { .. } => RpcError::new(CONFLICT, "has_component", msg),
            EditError::GuidExists(_) => RpcError::new(CONFLICT, "guid_exists", msg),
            EditError::Referenced { .. } => RpcError::new(CONFLICT, "referenced", msg),
            EditError::Reflect(_) | EditError::Invalid(_) => RpcError::new(INVALID_VALUE, "invalid_value", msg),
            EditError::Parse(_) => RpcError::new(INVALID_VALUE, "invalid_scene", msg),
            EditError::Bake(_) => RpcError::new(INVALID_VALUE, "bake_failed", msg),
            EditError::TxOpen => RpcError::state("tx_open", msg),
            EditError::NoTx => RpcError::state("no_tx", msg),
            EditError::TxBusy { owner } => RpcError::state("tx_busy", msg).with("owner", json!(owner)),
            EditError::NothingToUndo => RpcError::state("nothing_to_undo", msg),
            EditError::NothingToRedo => RpcError::state("nothing_to_redo", msg),
            EditError::RegistryMismatch(_) => RpcError::new(INTERNAL_ERROR, "registry_mismatch", msg),
            EditError::Debug(_) => RpcError::new(DEBUG_REFUSED, "debug_refused", msg),
            EditError::PlayStart(_) => RpcError::state("play_start_failed", msg),
            EditError::UnknownProposal(_) => RpcError::new(NOT_FOUND, "unknown_proposal", msg),
            EditError::ProposalConflict { .. } => RpcError::new(CONFLICT, "proposal_conflict", msg),
            EditError::StaleVerification { .. } => RpcError::new(CONFLICT, "stale_verification", msg),
            EditError::Verify(_) => RpcError::new(INVALID_VALUE, "verify_failed", msg),
        }
    }
}
