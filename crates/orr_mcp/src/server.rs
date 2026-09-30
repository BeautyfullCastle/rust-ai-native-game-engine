//! The MCP protocol: newline-delimited JSON-RPC 2.0 over a reader and a
//! writer (stdin and stdout in the binary).
//!
//! Implemented by hand: `initialize`, `notifications/initialized`, `ping`,
//! `tools/list`, `tools/call`, `resources/list`, `resources/templates/list`
//! and `resources/read`. Nothing but JSON-RPC messages, one per line, is
//! ever written to the output; logs go to stderr.

use std::io::{self, BufRead, Write};

use serde_json::{json, Map, Value as J};

use crate::agents_md;
use crate::bridge::Bridge;
use crate::tools::{self, ToolGroups};

/// The MCP revision this server speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Revisions it also accepts from a client (the tool result shape is compatible).
const ALSO_SUPPORTED: &[&str] = &["2025-03-26", "2024-11-05"];

/// JSON-RPC: not valid JSON.
pub const PARSE_ERROR: i64 = -32700;
/// JSON-RPC: not a valid request object.
pub const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC: no such method.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC: bad parameters (also an unknown tool).
pub const INVALID_PARAMS: i64 = -32602;
/// JSON-RPC: server-side failure.
pub const INTERNAL_ERROR: i64 = -32603;
/// A request other than `ping` before `initialize`, or an unknown resource.
pub const NOT_READY_OR_NOT_FOUND: i64 = -32002;

/// Short instructions a client may show the model.
const INSTRUCTIONS: &str = "Orrery is a deterministic game engine. You change its scene through PROPOSALS: \
propose_changes stages edits (nothing changes yet, you get a diff), verify_proposal replays the scene with and without them and \
judges metrics with `checks`, accept_proposal applies them as one undoable step. Look first: scene_overview, get_entity, get_schema. \
Read the resource orrery://agents for the full guide (value format, check grammar, metric names).";

type RpcResult = Result<J, (i64, String)>;

/// The MCP server: one client, one ERP endpoint.
pub struct McpServer {
    bridge: Bridge,
    groups: ToolGroups,
    initialized: bool,
    /// Log each request (method and outcome) to stderr.
    pub verbose: bool,
}

fn error_response(id: J, code: i64, message: &str) -> J {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

impl McpServer {
    /// A server that offers the tools of `groups` and reaches the engine through `bridge`.
    pub fn new(bridge: Bridge, groups: ToolGroups) -> McpServer {
        McpServer { bridge, groups, initialized: false, verbose: false }
    }

    /// The tool groups that are on.
    pub fn groups(&self) -> ToolGroups {
        self.groups
    }

    /// Handles one line of input; returns the line to send back, if any
    /// (notifications get none).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let msg: J = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(e) => return Some(error_response(J::Null, PARSE_ERROR, &format!("parse error: {e}")).to_string()),
        };
        self.handle(&msg).map(|r| r.to_string())
    }

    /// Handles one parsed message.
    pub fn handle(&mut self, msg: &J) -> Option<J> {
        let Some(obj) = msg.as_object() else {
            let what = if msg.is_array() { "batches are not supported (MCP 2025-06-18): send one message per line" } else { "a request must be a JSON object" };
            return Some(error_response(J::Null, INVALID_REQUEST, what));
        };
        let id = obj.get("id").cloned();
        if let Some(i) = &id {
            if !(i.is_string() || i.is_number()) {
                return Some(error_response(J::Null, INVALID_REQUEST, "'id' must be a string or a number"));
            }
        }
        let Some(method) = obj.get("method").and_then(J::as_str) else {
            // A response to a request of ours (we send none), or junk.
            return match id {
                Some(_) if obj.contains_key("result") || obj.contains_key("error") => None,
                other => Some(error_response(other.unwrap_or(J::Null), INVALID_REQUEST, "missing 'method'")),
            };
        };
        if obj.get("jsonrpc").is_some_and(|v| v != "2.0") {
            return Some(error_response(id.unwrap_or(J::Null), INVALID_REQUEST, "'jsonrpc' must be \"2.0\""));
        }
        let params = match obj.get("params") {
            None | Some(J::Null) => Map::new(),
            Some(J::Object(m)) => m.clone(),
            Some(_) => {
                return id.map(|id| error_response(id, INVALID_PARAMS, "'params' must be an object"));
            }
        };
        let result = self.dispatch(method, &params, id.is_some());
        if self.verbose {
            match &result {
                Ok(_) => eprintln!("orr_mcp: {method} ok"),
                Err((c, m)) => eprintln!("orr_mcp: {method} error {c}: {m}"),
            }
        }
        let id = id?; // a notification is never answered
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, message)) => error_response(id, code, &message),
        })
    }

    fn dispatch(&mut self, method: &str, params: &Map<String, J>, is_request: bool) -> RpcResult {
        match method {
            "initialize" => return self.initialize(params),
            "ping" => return Ok(json!({})),
            _ => {}
        }
        if method.starts_with("notifications/") {
            // `initialized`, `cancelled`, ...: nothing to do. (Sent with an id it is not a notification.)
            return if is_request { Err((METHOD_NOT_FOUND, format!("'{method}' is a notification and takes no id"))) } else { Ok(J::Null) };
        }
        if !self.initialized {
            return Err((NOT_READY_OR_NOT_FOUND, format!("server not initialized: send `initialize` before '{method}'")));
        }
        match method {
            "tools/list" => Ok(json!({"tools": tools::defs(self.groups).iter().map(|t| t.to_json()).collect::<Vec<_>>()})),
            "tools/call" => self.tools_call(params),
            "resources/list" => Ok(json!({"resources": resources()})),
            "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
            "resources/read" => self.resources_read(params),
            other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
        }
    }

    fn initialize(&mut self, params: &Map<String, J>) -> RpcResult {
        let requested = params
            .get("protocolVersion")
            .and_then(J::as_str)
            .ok_or_else(|| (INVALID_PARAMS, "initialize needs 'protocolVersion' (a string like \"2025-06-18\")".to_string()))?;
        if !params.get("capabilities").is_none_or(J::is_object) || !params.get("clientInfo").is_none_or(J::is_object) {
            return Err((INVALID_PARAMS, "'capabilities' and 'clientInfo' must be objects".into()));
        }
        // Answer with the client's revision if we speak it, else with ours (the client may then disconnect).
        let version = if requested == PROTOCOL_VERSION || ALSO_SUPPORTED.contains(&requested) { requested } else { PROTOCOL_VERSION };
        self.initialized = true;
        Ok(json!({
            "protocolVersion": version,
            "capabilities": {
                "tools": {"listChanged": false},
                "resources": {"subscribe": false, "listChanged": false},
            },
            "serverInfo": {"name": "orr_mcp", "title": "Orrery", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS,
        }))
    }

    fn tools_call(&mut self, params: &Map<String, J>) -> RpcResult {
        let name = params.get("name").and_then(J::as_str).ok_or_else(|| (INVALID_PARAMS, "tools/call needs 'name' (a string)".to_string()))?;
        let empty = Map::new();
        let args = match params.get("arguments") {
            None | Some(J::Null) => &empty,
            Some(J::Object(m)) => m,
            Some(_) => return Err((INVALID_PARAMS, "'arguments' must be an object".into())),
        };
        let def = tools::find(name).filter(|t| self.groups.has(t.group)).ok_or_else(|| (INVALID_PARAMS, format!("unknown tool: {name}")))?;
        Ok(match tools::run(&mut self.bridge, def.name, args) {
            Ok(out) => {
                let structured = if out.structured.is_object() { out.structured } else { json!({"value": out.structured}) };
                json!({"content": [{"type": "text", "text": out.text}], "structuredContent": structured, "isError": false})
            }
            Err(fail) => json!({"content": [{"type": "text", "text": fail.0}], "isError": true}),
        })
    }

    fn resources_read(&mut self, params: &Map<String, J>) -> RpcResult {
        let uri = params.get("uri").and_then(J::as_str).ok_or_else(|| (INVALID_PARAMS, "resources/read needs 'uri' (a string)".to_string()))?;
        let fail = |f: crate::bridge::Fail| (INTERNAL_ERROR, f.0);
        let (mime, text) = match uri {
            "orrery://scene" => {
                let r = self.bridge.call("scene.save", json!({})).map_err(fail)?;
                ("application/yaml", r["text"].as_str().unwrap_or("").to_string())
            }
            "orrery://schema" => {
                let r = self.bridge.call("registry.schema", json!({})).map_err(fail)?;
                ("application/schema+json", serde_json::to_string_pretty(&r["schema"]).unwrap_or_default())
            }
            "orrery://agents" => ("text/markdown", agents_md::generate(&mut self.bridge, self.groups).map_err(fail)?),
            other => return Err((NOT_READY_OR_NOT_FOUND, format!("resource not found: {other}"))),
        };
        Ok(json!({"contents": [{"uri": uri, "mimeType": mime, "text": text}]}))
    }
}

fn resources() -> J {
    json!([
        {"uri": "orrery://scene", "name": "scene", "title": "Current scene (YAML)", "description": "The scene document as YAML text (orr.scene/1): every entity with its components, and the singletons.", "mimeType": "application/yaml"},
        {"uri": "orrery://schema", "name": "schema", "title": "Scene JSON Schema", "description": "JSON Schema (draft 2020-12) of every component and singleton type: fields, ranges, docs.", "mimeType": "application/schema+json"},
        {"uri": "orrery://agents", "name": "agents", "title": "Agent guide (AGENTS.md)", "description": "How to work with this engine: the propose, verify, accept workflow, the value format, check grammar and metric names. Pinned to the engine version.", "mimeType": "text/markdown"},
    ])
}

/// Reads lines from `input` and answers on `output` until the input ends.
/// Every line written is one JSON-RPC message.
pub fn serve<R: BufRead, W: Write>(server: &mut McpServer, mut input: R, mut output: W) -> io::Result<()> {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if input.read_until(b'\n', &mut buf)? == 0 {
            return Ok(());
        }
        let line = String::from_utf8_lossy(&buf);
        if let Some(reply) = server.handle_line(&line) {
            output.write_all(reply.as_bytes())?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}
