//! The connection to the ERP endpoint, made lazily and remade after a failure.

use orr_remote::{ClientError, ErpClient};
use serde_json::Value as J;

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

/// Opens a connection. Called again after a transport failure.
pub type Connector = Box<dyn FnMut() -> Result<Box<dyn ErpCall>, String>>;

/// The ERP endpoint as the tools see it.
pub struct Bridge {
    connect: Connector,
    conn: Option<Box<dyn ErpCall>>,
    /// Where it is, for error texts.
    pub endpoint: String,
}

impl Bridge {
    /// A bridge that connects with `connect` when first needed.
    pub fn new(endpoint: &str, connect: Connector) -> Bridge {
        Bridge { connect, conn: None, endpoint: endpoint.to_string() }
    }

    /// A bridge to `url`, authenticating with `token` (none = a dev-mode host).
    /// `call_timeout` is how long one call may take (verification can be slow).
    pub fn to_url(url: &str, token: Option<String>, call_timeout: std::time::Duration) -> Bridge {
        let target = url.to_string();
        Bridge::new(
            url,
            Box::new(move || {
                let mut c = ErpClient::connect(&target, token.as_deref()).map_err(|e| e.to_string())?;
                c.call_timeout = call_timeout;
                Ok(Box::new(c) as Box<dyn ErpCall>)
            }),
        )
    }

    /// Calls an ERP method. A transport failure drops the connection (the
    /// next call reconnects); an ERP error is turned into a [`Fail`] with a hint.
    pub fn call(&mut self, method: &str, params: J) -> Result<J, Fail> {
        if self.conn.is_none() {
            let c = (self.connect)().map_err(|e| self.unreachable(&e))?;
            self.conn = Some(c);
        }
        let conn = self.conn.as_mut().ok_or_else(|| Fail::new("not connected"))?;
        match conn.call(method, params) {
            Ok(v) => Ok(v),
            Err(ClientError::Rpc(e)) => Err(rpc_fail(&e)),
            Err(other) => {
                self.conn = None;
                Err(self.unreachable(&other.to_string()))
            }
        }
    }

    fn unreachable(&self, why: &str) -> Fail {
        Fail(format!(
            "cannot talk to the engine at {}: {why}. Is the host (orr_remote_host, or the editor started with --erp) running, and is the token right?",
            self.endpoint
        ))
    }
}

/// The text of an ERP error, with what to do about it.
pub fn rpc_fail(e: &orr_remote::RpcError) -> Fail {
    let kind = e.kind().unwrap_or("error");
    let hint = match kind {
        "permission_denied" => " This token lacks the capability; ask the person running the host (for accept_proposal: they can accept it in the editor).",
        "proposal_conflict" => " The scene changed since the proposal was made and one op no longer applies. Nothing was changed; reject it and propose again against the current scene.",
        "unknown_proposal" => " It may have been accepted, rejected or dropped; see list_proposals.",
        "sim_running" => " A play session is running; stop it first (sim_run action=stop).",
        "no_last_play" => " Play in the editor (or sim_run start/step/stop) first, or use inputs kind=bot or idle.",
        "verify_limit" | "step_limit" => " Use fewer ticks.",
        "unknown_entity" | "unknown_type" => " Look the name up with scene_overview or get_schema.",
        _ => "",
    };
    Fail(format!("{} [{kind}]{hint}", e.message))
}
