//! The connection to the ERP endpoint, made lazily and remade after a failure.

use orr_remote::{ClientError, ErpClient};
use serde_json::{json, Value as J};

#[path = "redact.rs"]
mod redact;

/// One ERP call. Implemented by [`ErpClient`]; tests can fake it.
pub trait ErpCall {
    /// Calls an ERP method and waits for its result.
    fn call(&mut self, method: &str, params: J) -> Result<J, ClientError>;
}

impl ErpCall for ErpClient {
    fn call(&mut self, method: &str, params: J) -> Result<J, ClientError> {
        ErpClient::call(self, method, params)
    }
}

/// A failed tool call: the text an agent reads (returned as `isError: true`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fail(pub String);

impl Fail {
    /// A failure with this text.
    pub fn new(text: impl Into<String>) -> Fail {
        Fail(text.into())
    }
}

// Internal only: the public Fail tuple is shared with CLI and external callers.
// The cause is captured at the failing call, not recovered from text or bridge state.
pub(crate) struct DetailedFail {
    pub(crate) legacy: Fail,
    erp: Option<ErpFailure>,
}

struct ErpFailure {
    code: i64,
    kind: Option<&'static str>,
}

impl DetailedFail {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Fail::new(text).into()
    }

    pub(crate) fn known_apply_rejection(&self) -> bool {
        self.erp.as_ref().is_some_and(|cause| {
            matches!(
                cause.kind,
                Some(
                    "invalid_params"
                        | "permission_denied"
                        | "unauthenticated"
                        | "too_many_ops"
                        | "unknown_proposal"
                        | "unknown_entity"
                        | "unknown_type"
                        | "no_component"
                        | "has_component"
                        | "guid_exists"
                        | "referenced"
                        | "invalid_value"
                        | "invalid_scene"
                        | "bake_failed"
                        | "proposal_conflict"
                )
            )
        })
    }

    pub(crate) fn into_legacy(self) -> Fail {
        self.legacy
    }

    pub(crate) fn structured(&self) -> Option<J> {
        let cause = self.erp.as_ref()?;
        let mut error = json!({"origin": "erp", "code": cause.code});
        if let Some(kind) = cause.kind {
            error["data"] = json!({"kind": kind});
        }
        Some(json!({"error": error}))
    }
}

impl From<Fail> for DetailedFail {
    fn from(legacy: Fail) -> Self {
        Self { legacy, erp: None }
    }
}

// Reviewed fixed ERP tags from error.rs, dispatch.rs, proposals.rs, server.rs and
// net.rs. Unknown/new tags are deliberately omitted, even if identifier-shaped.
// Never forward arbitrary data fields: they may contain credentials or paths.
fn safe_kind(kind: Option<&str>) -> Option<&'static str> {
    const KINDS: &[&str] = &[
        "bad_token",
        "bake_failed",
        "busy",
        "debug_refused",
        "disabled",
        "guid_exists",
        "has_component",
        "input_stale",
        "input_unavailable",
        "invalid_params",
        "invalid_request",
        "invalid_scene",
        "invalid_value",
        "io",
        "limit_exceeded",
        "method_not_found",
        "no_component",
        "no_bot",
        "no_last_play",
        "no_play",
        "no_scene_path",
        "no_tx",
        "not_a_client",
        "nothing_to_redo",
        "nothing_to_undo",
        "panic",
        "parse_error",
        "permission_denied",
        "play_edit_refused",
        "play_start_failed",
        "proposal_conflict",
        "read_only",
        "referenced",
        "registry_mismatch",
        "scene_path_fixed",
        "schema",
        "seek_failed",
        "sim_running",
        "stale_checksum",
        "stale_revision",
        "stale_verification",
        "step_limit",
        "too_large",
        "too_many_ops",
        "tx_busy",
        "tx_open",
        "unauthenticated",
        "unknown_entity",
        "unknown_proposal",
        "unknown_type",
        "verify_busy",
        "verify_cancelled",
        "verify_failed",
        "verify_limit",
        "verify_worker",
    ];
    KINDS.iter().copied().find(|known| Some(*known) == kind)
}

/// Who is reading the error hints: names the tools (`accept_proposal`) or the
/// commands (`orr accept`) to use next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// MCP tool names.
    Mcp,
    /// `orr` commands.
    Cli,
}

/// Opens a connection. Called again after a transport failure.
pub type Connector = Box<dyn FnMut() -> Result<Box<dyn ErpCall>, String>>;

/// The ERP endpoint as the tools see it.
pub struct Bridge {
    connect: Connector,
    conn: Option<Box<dyn ErpCall>>,
    /// Where it is, for error texts.
    pub endpoint: String,
    /// How error hints name the next step.
    pub style: Style,
    conn_error: Option<String>,
    // Only for exact known-token redaction; never serialized or logged.
    token: Option<String>,
}

impl Bridge {
    /// A bridge that connects with `connect` when first needed.
    pub fn new(endpoint: &str, connect: Connector) -> Bridge {
        Bridge { connect, conn: None, endpoint: endpoint.to_string(), style: Style::Mcp, conn_error: None, token: None }
    }

    /// A bridge to `url`, authenticating with `token` (none = a dev-mode host).
    /// `call_timeout` is how long one call may take (verification can be slow).
    pub fn to_url(url: &str, token: Option<String>, call_timeout: std::time::Duration) -> Bridge {
        let target = url.to_string();
        let known_token = token.clone();
        let mut bridge = Bridge::new(
            url,
            Box::new(move || {
                let mut c = ErpClient::connect(&target, token.as_deref()).map_err(|e| e.to_string())?;
                c.call_timeout = call_timeout;
                Ok(Box::new(c) as Box<dyn ErpCall>)
            }),
        );
        bridge.token = known_token;
        bridge
    }

    /// Calls an ERP method. A transport failure drops the connection (the
    /// next call reconnects); an ERP error is turned into a [`Fail`] with a hint.
    pub fn call(&mut self, method: &str, params: J) -> Result<J, Fail> {
        self.call_detailed(method, params).map_err(DetailedFail::into_legacy)
    }

    pub(crate) fn call_detailed(&mut self, method: &str, params: J) -> Result<J, DetailedFail> {
        self.conn_error = None;
        if self.conn.is_none() {
            let c = match (self.connect)() {
                Ok(c) => c,
                Err(e) => {
                    self.conn_error = Some(self.redact(&e));
                    return Err(self.unreachable(&e).into());
                }
            };
            self.conn = Some(c);
        }
        let conn = self.conn.as_mut().ok_or_else(|| Fail::new("not connected"))?;
        match conn.call(method, params) {
            Ok(v) => Ok(v),
            Err(ClientError::Rpc(e)) => {
                if e.kind() == Some("unauthenticated") {
                    // Connected, but the host wants a token this client did not send.
                    self.conn_error = Some("the host requires a token (none was given)".to_string());
                }
                let cause = ErpFailure { code: e.code, kind: safe_kind(e.kind()) };
                let legacy = Fail(self.redact(&rpc_text_styled(&e, self.style)));
                Err(DetailedFail { legacy, erp: Some(cause) })
            }
            Err(other) => {
                self.conn = None;
                self.conn_error = Some(self.redact(&other.to_string()));
                Err(self.unreachable(&other.to_string()).into())
            }
        }
    }

    /// Why the last [`call`](Self::call) failed to reach the host (connection
    /// or authentication failure), or `None` if it did reach it (it may still
    /// have failed with an ERP error).
    pub fn connection_error(&self) -> Option<&str> {
        self.conn_error.as_deref()
    }

    fn unreachable(&self, why: &str) -> Fail {
        Fail(self.redact(&format!(
            "cannot talk to the engine at {}: {why}. Is the host (orr_remote_host, or the editor started with --erp) running, and is the token right?",
            self.endpoint
        )))
    }

    pub(crate) fn redact(&self, text: &str) -> String {
        redact::text(text, self.token.as_deref())
    }
}

/// The text of an ERP error, with what to do about it (MCP tool names).
pub fn rpc_fail(e: &orr_remote::RpcError) -> Fail {
    rpc_fail_styled(e, Style::Mcp)
}

/// The text of an ERP error, with what to do about it, naming the next step in `style`.
pub fn rpc_fail_styled(e: &orr_remote::RpcError, style: Style) -> Fail {
    Fail(redact::text(&rpc_text_styled(e, style), None))
}

fn rpc_text_styled(e: &orr_remote::RpcError, style: Style) -> String {
    let kind = e.kind().unwrap_or("error");
    let cli = style == Style::Cli;
    let pick = |mcp: &'static str, cli_text: &'static str| if cli { cli_text } else { mcp };
    let hint = match kind {
        "permission_denied" => pick(
            " This token lacks the capability; ask the person running the host (for accept_proposal: they can accept it in the editor).",
            " This token lacks the capability; ask the person running the host for a token with it (for `orr accept`: they can accept in the editor).",
        ),
        "proposal_conflict" => " The scene changed since the proposal was made and one op no longer applies. Nothing was changed; reject it and propose again against the current scene.",
        "stale_verification" => pick(
            " Nothing was accepted. Run verify_proposal again and pass its new verified_state to accept_proposal.",
            " Nothing was accepted. Verify the current proposal again before accepting it.",
        ),
        "unknown_proposal" => pick(" It may have been accepted, rejected or dropped; see list_proposals.", " It may have been accepted, rejected or dropped; see `orr proposals`."),
        "sim_running" => pick(" A play session is running; stop it first (sim_run action=stop).", " A play session is running; stop it first (`orr sim stop`)."),
        "no_last_play" => pick(
            " Play in the editor (or sim_run start/step/stop) first, or use inputs kind=bot or idle.",
            " Play in the editor (or `orr sim start`, `orr sim step`, `orr sim stop`) first, or use `--bot N` or `--idle N`.",
        ),
        "verify_limit" | "step_limit" => " Use fewer ticks.",
        "unknown_entity" | "unknown_type" => pick(" Look the name up with scene_overview or get_schema.", " Look the name up with `orr scene` or `orr schema --types`."),
        _ => "",
    };
    format!("{} [{kind}]{hint}", e.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;
    impl ErpCall for Echo {
        fn call(&mut self, _: &str, _: J) -> Result<J, ClientError> {
            Err(ClientError::Rpc(orr_remote::RpcError::new(-42, "secret-token", "echo secret-token ws://user:password@host/path?token=query#fragment")))
        }
    }

    #[test]
    fn configured_token_is_removed_from_rpc_kind_message_and_local_errors() {
        let mut bridge = Bridge::to_url("ws://host", Some("secret-token".into()), std::time::Duration::from_secs(1));
        bridge.conn = Some(Box::new(Echo));
        let result = bridge.call_detailed("fake", J::Null).err().unwrap();
        assert_eq!(result.structured(), Some(json!({"error":{"origin":"erp", "code":-42}})));
        for secret in ["secret-token", "password", "query", "fragment"] {
            assert!(!result.legacy.0.contains(secret), "{}", result.legacy.0);
        }
        let fail = crate::tools::run(&mut bridge, "sim_run", json!({"action":"secret-token"}).as_object().unwrap()).err().unwrap();
        assert!(!fail.0.contains("secret-token"));
    }

    #[test]
    fn url_shaped_configured_token_is_redacted_before_rpc_formatting() {
        struct UrlToken;
        impl ErpCall for UrlToken {
            fn call(&mut self, _: &str, _: J) -> Result<J, ClientError> {
                Err(ClientError::Rpc(orr_remote::RpcError::new(-42, "ws://secret-bearer-value?fixed", "echo ws://secret-bearer-value?fixed")))
            }
        }
        let mut bridge = Bridge::to_url("ws://host", Some("ws://secret-bearer-value?fixed".into()), std::time::Duration::from_secs(1));
        bridge.conn = Some(Box::new(UrlToken));
        let result = bridge.call("fake", J::Null).unwrap_err();
        assert!(result.0.starts_with("echo *** [***"));
        assert!(!result.0.contains("secret-bearer-value"));
    }

    #[test]
    fn connection_and_public_diagnostics_remove_known_credentials() {
        let mut bridge = Bridge::to_url("ws://user:password@host/path?token=query#fragment", Some("secret-token".into()), std::time::Duration::from_secs(1));
        bridge.connect = Box::new(|| Err("echo secret-token wss://other:credential@host?token=other-query".into()));
        let fail = bridge.call("fake", J::Null).unwrap_err();
        for text in [fail.0.as_str(), bridge.connection_error().unwrap()] {
            for secret in ["secret-token", "password", "query", "fragment", "credential"] {
                assert!(!text.contains(secret), "{text}");
            }
        }
    }
}
