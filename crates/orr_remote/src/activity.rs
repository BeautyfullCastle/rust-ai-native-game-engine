//! The activity log: what every ERP request did, for people watching an agent.
//!
//! The server keeps the newest [`ActivityEntry`]s in a bounded ring
//! ([`crate::ErpServer::activity_since`], the `activity.list` method and the
//! `watch.activity` topic read it). An entry has a one-line summary made from
//! the request and its result, the entities it touched, and for the
//! requests where it matters the detail a UI wants to expand: the old and
//! new value of a `world.patch` (the old value is read *before* the request
//! runs), the diff of a proposal that is accepted or rejected (captured
//! before it is gone), and the whole report of a verification.
//!
//! `at_ms` is a monotonic time since the server started, for display only.

use std::sync::Arc;

use orr_edit::{format_value, CheckOutcome, ProposalDiff, VerifyReport};
use orr_reflect::Value;
use orr_sim::Game;
use serde_json::{json, Map, Value as J};

use crate::caps::Caps;
use crate::dispatch::{component_type, view, ErpTarget, P};
use crate::error::RpcError;
use crate::json::{desc_at_path, json_to_value, value_to_json};
use crate::proposals::{proposal_id, report_json};

/// Entries the ring keeps by default.
pub const DEFAULT_ACTIVITY_CAPACITY: usize = 2000;

/// What kind of request an entry is (the UI's filters and colors).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityKind {
    /// A change of the scene or the history: `world.*`, `tx.*`, `history.undo`, `scene.load`.
    Edit,
    /// `proposal.begin` / `apply` / `accept` / `reject`.
    Proposal,
    /// `proposal.verify` and `verify.self`.
    Verify,
    /// `sim.*` that steers the play session.
    Sim,
    /// A read-only request (queries, lists, schema, state).
    Read,
    /// A client connected, disconnected or failed to authenticate.
    Session,
}

impl ActivityKind {
    /// The kind of a JSON name (see [`name`](Self::name)).
    pub fn from_name(name: &str) -> Option<ActivityKind> {
        Some(match name {
            "edit" => ActivityKind::Edit,
            "proposal" => ActivityKind::Proposal,
            "verify" => ActivityKind::Verify,
            "sim" => ActivityKind::Sim,
            "read" => ActivityKind::Read,
            "session" => ActivityKind::Session,
            _ => return None,
        })
    }

    /// The name used in JSON.
    pub fn name(self) -> &'static str {
        match self {
            ActivityKind::Edit => "edit",
            ActivityKind::Proposal => "proposal",
            ActivityKind::Verify => "verify",
            ActivityKind::Sim => "sim",
            ActivityKind::Read => "read",
            ActivityKind::Session => "session",
        }
    }
}

/// One field write: the entity (None = a singleton), the component or
/// singleton type, the field path, and the value before and after.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueChange {
    /// The entity as the client named it (GUID or handle); `None` for a singleton.
    pub entity: Option<String>,
    /// Component or singleton type name.
    pub component: String,
    /// Field path (empty = the whole value).
    pub path: String,
    /// The value before the request, if it could be read.
    pub old: Option<Value>,
    /// The value after the request, if it could be read.
    pub new: Option<Value>,
}

impl ValueChange {
    /// `old -> new` as text, for a one-line display.
    pub fn describe(&self) -> String {
        let f = |v: &Option<Value>| v.as_ref().map_or_else(|| "?".to_string(), format_value);
        format!("{} -> {}", f(&self.old), f(&self.new))
    }
}

/// The result of a verification request, typed.
#[derive(Clone, Debug)]
pub struct VerifyDetail {
    /// The proposal that was verified (`None` for `verify.self`).
    pub proposal: Option<String>,
    /// The `inputs` the run used, as the server described them.
    pub inputs: J,
    /// The report.
    pub report: VerifyReport,
    /// The verdict on the request's `checks` (None if there were none).
    pub outcome: Option<CheckOutcome>,
}

/// One executed request (or a session event).
#[derive(Clone, Debug)]
pub struct ActivityEntry {
    /// 1, 2, 3, ... in the order recorded.
    pub seq: u64,
    /// Milliseconds since the server started (monotonic; display only, never in sim state).
    pub at_ms: u64,
    /// The client name (`Origin::Agent(name)` in the history).
    pub client: String,
    /// The kind.
    pub kind: ActivityKind,
    /// The method (`world.patch`), or `session.connect` / `session.disconnect` / `session.auth_failed`.
    pub method: String,
    /// One human-readable line: `world.patch e_0000000e orr_physics::Body.pos = [6, 18]`.
    pub summary: String,
    /// False if the request ended in an error.
    pub ok: bool,
    /// The error message, if not ok.
    pub error: Option<String>,
    /// True for a read-only request (a UI can collapse these).
    pub read: bool,
    /// A successful `scene.save` changed the host's scene path. Omitted on the
    /// wire otherwise; derived from the committed effect, never request fields.
    pub scene_path_changed: bool,
    /// GUIDs (or handles) of the entities the request touched.
    pub entities: Vec<String>,
    /// The proposal the request was about (`p2`).
    pub proposal: Option<String>,
    /// The field write, for `world.patch` and `world.singleton.patch`.
    pub change: Option<ValueChange>,
    /// The proposal's diff at the time of `proposal.accept` / `proposal.reject`.
    pub diff: Option<Arc<ProposalDiff>>,
    /// The verification, for `proposal.verify` and `verify.self`.
    pub verify: Option<Arc<VerifyDetail>>,
}

impl ActivityEntry {
    /// The entry as JSON (`activity.list`, `watch.activity`).
    pub fn to_json(&self) -> J {
        let mut o = Map::new();
        o.insert("seq".into(), json!(self.seq));
        o.insert("at_ms".into(), json!(self.at_ms));
        o.insert("client".into(), json!(self.client));
        o.insert("kind".into(), json!(self.kind.name()));
        o.insert("method".into(), json!(self.method));
        o.insert("summary".into(), json!(self.summary));
        o.insert("ok".into(), json!(self.ok));
        o.insert("read".into(), json!(self.read));
        if self.scene_path_changed {
            o.insert("scene_path_changed".into(), json!(true));
        }
        if let Some(e) = &self.error {
            o.insert("error".into(), json!(e));
        }
        if !self.entities.is_empty() {
            o.insert("entities".into(), json!(self.entities));
        }
        if let Some(p) = &self.proposal {
            o.insert("proposal".into(), json!(p));
        }
        if let Some(c) = &self.change {
            o.insert(
                "change".into(),
                json!({
                    "entity": c.entity,
                    "component": c.component,
                    "path": c.path,
                    "old": c.old.as_ref().map(value_to_json),
                    "new": c.new.as_ref().map(value_to_json),
                }),
            );
        }
        if let Some(d) = &self.diff {
            o.insert("diff".into(), json!({"text": d.text, "lines": d.summary.lines(), "summary": crate::proposals::summary_json(&d.summary)}));
        }
        if let Some(v) = &self.verify {
            let mut r = report_json(&v.report, v.outcome.as_ref(), false);
            r["inputs"] = v.inputs.clone();
            o.insert("verify".into(), r);
        }
        J::Object(o)
    }
}

/// A connected client, for [`crate::ErpServer::clients`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientInfo {
    /// Connection number.
    pub id: u64,
    /// Client name.
    pub name: String,
    /// What it may do.
    pub caps: Caps,
    /// Server time (ms since start) it connected.
    pub connected_ms: u64,
    /// Requests it made.
    pub requests: u64,
}

// ---- classification ----

/// The kind of a method, and whether it is read-only.
pub(crate) fn classify(method: &str, params: &J) -> (ActivityKind, bool) {
    let kind = match method {
        "world.query" | "world.get" | "world.singleton.get" => ActivityKind::Read,
        m if m.starts_with("world.") => ActivityKind::Edit,
        "tx.begin" | "tx.commit" | "tx.rollback" | "history.undo" | "history.redo" | "scene.load" => ActivityKind::Edit,
        "scene.save" if params.get("write").and_then(J::as_bool) == Some(true) => ActivityKind::Edit,
        "proposal.begin" | "proposal.apply" | "proposal.accept" | "proposal.accept_verified" | "proposal.reject" => ActivityKind::Proposal,
        "proposal.verify" | "verify.self" => ActivityKind::Verify,
        "sim.state" | "sim.checksum" => ActivityKind::Read,
        m if m.starts_with("sim.") => ActivityKind::Sim,
        _ => ActivityKind::Read,
    };
    (kind, kind == ActivityKind::Read)
}

// ---- before / after ----

/// What is read before a request runs.
#[derive(Default)]
pub(crate) struct Pre {
    old: Option<Value>,
    diff: Option<Arc<ProposalDiff>>,
    entities: Vec<String>,
}

fn params_obj(params: &J) -> Map<String, J> {
    params.as_object().cloned().unwrap_or_default()
}

/// Reads what would be gone after the request: the old value of a field
/// write, the diff of a proposal about to be accepted or rejected.
pub(crate) fn before<G: Game>(t: &ErpTarget<'_, G>, method: &str, params: &J) -> Pre {
    let mut pre = Pre::default();
    let obj = params_obj(params);
    let p = P(&obj);
    match method {
        "world.patch" => {
            if let (Ok(target), Ok(comp)) = (p.target(), p.str("component")) {
                pre.old = view(t).field(&target, comp, p.opt_str("path").ok().flatten().unwrap_or("")).ok();
            }
        }
        "world.singleton.patch" => {
            if let Ok(name) = p.str("name") {
                pre.old = view(t).singleton(name, p.opt_str("path").ok().flatten().unwrap_or("")).ok();
            }
        }
        "proposal.accept" | "proposal.accept_verified" | "proposal.reject" => {
            if let Ok(id) = proposal_id(&p) {
                if let Ok(d) = t.doc.proposal_diff(id) {
                    pre.entities = guids_of(&d);
                    pre.diff = Some(Arc::new(d));
                }
            }
        }
        _ => {}
    }
    pre
}

/// The GUIDs a proposal's diff touches.
fn guids_of(d: &ProposalDiff) -> Vec<String> {
    let s = &d.summary;
    let mut v: Vec<String> = s.fields_changed.iter().filter_map(|f| f.entity.as_ref().map(|g| g.to_string())).collect();
    v.extend(s.entities_added.iter().map(|e| e.guid.to_string()));
    v.extend(s.entities_removed.iter().map(|e| e.guid.to_string()));
    v.extend(s.entities_renamed.iter().map(|r| r.guid.to_string()));
    v.extend(s.components_added.iter().map(|(g, _)| g.to_string()));
    v.extend(s.components_removed.iter().map(|(g, _)| g.to_string()));
    v.sort();
    v.dedup();
    v
}

/// Builds the entry of a finished request (`seq` and `at_ms` are set by the ring).
pub(crate) fn build<G: Game>(
    t: &ErpTarget<'_, G>,
    client: &str,
    method: &str,
    params: &J,
    result: &Result<J, RpcError>,
    pre: Pre,
    verify: Option<Arc<VerifyDetail>>,
) -> ActivityEntry {
    let (kind, read) = classify(method, params);
    let ok = result.is_ok();
    let res = result.as_ref().ok();
    let mut entities = pre.entities;
    if let Some(e) = params.get("entity").and_then(J::as_str) {
        entities.push(e.to_string());
    }
    if let Some(g) = res.and_then(|r| r.get("guid")).and_then(J::as_str) {
        entities.push(g.to_string());
    }
    let proposal = params.get("id").and_then(J::as_str).filter(|_| method.starts_with("proposal.")).map(str::to_string).or_else(|| {
        res.and_then(|r| r.get("id")).and_then(J::as_str).filter(|_| method == "proposal.begin").map(str::to_string)
    });
    let change = match (method, ok) {
        ("world.patch", true) => change_of(t, params, false, pre.old),
        ("world.singleton.patch", true) => change_of(t, params, true, pre.old),
        _ => None,
    };
    ActivityEntry {
        seq: 0,
        at_ms: 0,
        client: client.to_string(),
        kind,
        method: method.to_string(),
        summary: summarize(method, params, res, verify.as_deref()),
        ok,
        error: result.as_ref().err().map(|e| e.message.clone()),
        read,
        scene_path_changed: false,
        entities,
        proposal,
        change,
        diff: if ok { pre.diff } else { None },
        verify: if ok { verify } else { None },
    }
}

fn change_of<G: Game>(t: &ErpTarget<'_, G>, params: &J, singleton: bool, old: Option<Value>) -> Option<ValueChange> {
    let obj = params_obj(params);
    let p = P(&obj);
    let path = p.opt_str("path").ok().flatten().unwrap_or("").to_string();
    let v = view(t);
    let (entity, comp, mut new) = if singleton {
        let name = p.str("name").ok()?.to_string();
        let new = v.singleton(&name, &path).ok();
        (None, name, new)
    } else {
        let comp = p.str("component").ok()?.to_string();
        let target = p.target().ok()?;
        let new = v.field(&target, &comp, &path).ok();
        (Some(p.str("entity").ok()?.to_string()), comp, new)
    };
    if new == old {
        // A debug edit in play mode lands at the next tick boundary: show what was asked for.
        if let (Some(raw), Ok(ti)) = (p.raw("value"), component_type(&v, &comp).or_else(|_| crate::dispatch::singleton_type(&v, &comp))) {
            if let Ok(desc) = desc_at_path(ti.desc(), &path) {
                if let Ok(asked) = json_to_value(&desc, raw, path.is_empty()) {
                    new = Some(asked);
                }
            }
        }
    }
    Some(ValueChange { entity, component: comp, path, old, new })
}

// ---- summaries ----

/// A JSON value on one line (`[6, 18]`, strings unquoted), at most `max` characters.
fn compact(v: &J, max: usize) -> String {
    fn go(v: &J, out: &mut String) {
        match v {
            J::String(s) => out.push_str(s),
            J::Array(a) => {
                out.push('[');
                for (i, x) in a.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    go(x, out);
                }
                out.push(']');
            }
            J::Object(m) => {
                out.push('{');
                for (i, (k, x)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(k);
                    out.push_str(": ");
                    go(x, out);
                }
                out.push('}');
            }
            other => out.push_str(&other.to_string()),
        }
    }
    let mut s = String::new();
    go(v, &mut s);
    if s.chars().count() > max {
        s = s.chars().take(max.saturating_sub(1)).collect::<String>() + "\u{2026}";
    }
    s
}

fn sv<'a>(p: &'a J, k: &str) -> &'a str {
    p.get(k).and_then(J::as_str).unwrap_or("?")
}

fn dotted(component: &str, path: &str) -> String {
    if path.is_empty() {
        component.to_string()
    } else {
        format!("{component}.{path}")
    }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The one-line summary of a request. `res` is the result if it succeeded.
pub(crate) fn summarize(method: &str, params: &J, res: Option<&J>, verify: Option<&VerifyDetail>) -> String {
    let arg = |k: &str| res.and_then(|r| r.get(k));
    let path = params.get("path").and_then(J::as_str).unwrap_or("");
    match method {
        "world.patch" => {
            let value = params.get("value").map_or_else(String::new, |v| compact(v, 60));
            format!("world.patch {} {} = {value}", sv(params, "entity"), dotted(sv(params, "component"), path))
        }
        "world.singleton.patch" => {
            let value = params.get("value").map_or_else(String::new, |v| compact(v, 60));
            format!("world.singleton.patch {} = {value}", dotted(sv(params, "name"), path))
        }
        "world.insert" | "world.remove" => format!("{method} {} {}", sv(params, "entity"), sv(params, "component")),
        "world.despawn" => format!("world.despawn {}", sv(params, "entity")),
        "world.rename" => format!("world.rename {} {}", sv(params, "entity"), params.get("name").map_or_else(|| "?".to_string(), |v| compact(v, 40))),
        "world.spawn" => {
            let name = params.get("name").and_then(J::as_str).map(|n| format!(" \"{n}\"")).unwrap_or_default();
            match arg("guid").and_then(J::as_str).or_else(|| arg("handle").and_then(J::as_str)) {
                Some(g) => format!("world.spawn{name} \u{2192} {g}"),
                None => format!("world.spawn{name}"),
            }
        }
        "world.query" => match arg("total").and_then(J::as_u64) {
            Some(n) => format!("world.query ({})", plural(n, "entity", "entities")),
            None => "world.query".to_string(),
        },
        "world.get" => format!("world.get {}", sv(params, "entity")),
        "tx.begin" => format!("tx.begin \"{}\"", params.get("label").and_then(J::as_str).unwrap_or("agent edit")),
        "proposal.begin" => {
            let id = arg("id").and_then(J::as_str).unwrap_or("?");
            format!("proposal.begin {id} \"{}\"", arg("label").and_then(J::as_str).or_else(|| params.get("label").and_then(J::as_str)).unwrap_or(""))
        }
        "proposal.apply" => {
            let n = params.get("ops").and_then(J::as_array).map_or(0, Vec::len) as u64;
            match arg("op_count").and_then(J::as_u64) {
                Some(total) => format!("proposal.apply {} +{}, {total} staged", sv(params, "id"), plural(n, "op", "ops")),
                None => format!("proposal.apply {} +{}", sv(params, "id"), plural(n, "op", "ops")),
            }
        }
        "proposal.accept" | "proposal.accept_verified" => match arg("history_id").and_then(J::as_u64) {
            Some(h) => format!("{method} {} \u{2192} history #{h}", sv(params, "id")),
            None => format!("{method} {}", sv(params, "id")),
        },
        "proposal.reject" | "proposal.get" | "proposal.preview" => format!("{method} {}", sv(params, "id")),
        "proposal.verify" | "verify.self" => {
            let head = if method == "verify.self" { "verify.self".to_string() } else { format!("proposal.verify {}", sv(params, "id")) };
            match verify {
                Some(v) => {
                    let ticks = v.report.ticks;
                    match &v.outcome {
                        Some(o) if !o.results.is_empty() => {
                            let passed = o.results.iter().filter(|r| r.passed).count();
                            let total = o.results.len();
                            if o.passed {
                                format!("{head}: {passed}/{total} checks passed ({ticks} ticks)")
                            } else {
                                format!("{head}: {passed}/{total} checks passed, {} FAILED ({ticks} ticks)", total - passed)
                            }
                        }
                        _ => match v.report.first_divergence {
                            None => format!("{head}: no checks, identical ({ticks} ticks)"),
                            Some(t) => format!("{head}: no checks, diverges at tick {t} ({ticks} ticks)"),
                        },
                    }
                }
                None => head,
            }
        }
        "sim.step" => {
            let n = params.get("n").and_then(J::as_u64).unwrap_or(1);
            match arg("head_tick").and_then(J::as_u64) {
                Some(t) => format!("sim.step {n} \u{2192} tick {t}"),
                None => format!("sim.step {n}"),
            }
        }
        "sim.seek" => format!("sim.seek {}", params.get("tick").and_then(J::as_u64).map_or_else(|| "?".to_string(), |t| t.to_string())),
        "sim.start" | "sim.play" | "sim.pause" | "sim.branch" | "sim.speed" => match arg("head_tick").and_then(J::as_u64) {
            Some(t) => format!("{method} \u{2192} tick {t}"),
            None => method.to_string(),
        },
        "sim.stop" => match arg("tick").and_then(J::as_u64) {
            Some(t) => format!("sim.stop at tick {t}"),
            None => "sim.stop".to_string(),
        },
        "history.undo" | "history.redo" => method.to_string(),
        "scene.load" => match arg("entities").and_then(J::as_u64) {
            Some(n) => format!("scene.load ({})", plural(n, "entity", "entities")),
            None => "scene.load".to_string(),
        },
        _ => method.to_string(),
    }
}

pub(crate) fn session_summary(what: &str, caps: Option<Caps>) -> String {
    match caps {
        Some(c) => format!("{what} ({c})"),
        None => what.to_string(),
    }
}

/// The parameters of `activity.list`, parsed.
pub(crate) struct ListParams {
    pub since: u64,
    pub limit: usize,
    pub include_reads: bool,
}

pub(crate) fn list_params(params: &J) -> Result<ListParams, RpcError> {
    let obj = params_obj(params);
    let p = P(&obj);
    Ok(ListParams {
        since: p.opt_u64("since")?.unwrap_or(0),
        limit: p.opt_u64("limit")?.unwrap_or(200).clamp(1, 2000) as usize,
        include_reads: p.opt_bool("include_reads")?.unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_read_like_a_log() {
        let p = json!({"entity": "e_0000000e", "component": "orr_physics::Body", "path": "pos", "value": [6, 18]});
        assert_eq!(summarize("world.patch", &p, None, None), "world.patch e_0000000e orr_physics::Body.pos = [6, 18]");
        assert_eq!(summarize("proposal.begin", &json!({"label": "lift hero"}), Some(&json!({"id": "p2", "label": "lift hero"})), None), "proposal.begin p2 \"lift hero\"");
        assert_eq!(summarize("proposal.accept", &json!({"id": "p2"}), Some(&json!({"history_id": 14})), None), "proposal.accept p2 \u{2192} history #14");
        assert_eq!(summarize("sim.step", &json!({"n": 60}), Some(&json!({"head_tick": 180})), None), "sim.step 60 \u{2192} tick 180");
        assert_eq!(summarize("world.query", &json!({}), Some(&json!({"total": 49})), None), "world.query (49 entities)");
    }

    #[test]
    fn classification() {
        assert_eq!(classify("world.patch", &J::Null), (ActivityKind::Edit, false));
        assert_eq!(classify("world.query", &J::Null), (ActivityKind::Read, true));
        assert_eq!(classify("proposal.verify", &J::Null), (ActivityKind::Verify, false));
        assert_eq!(classify("sim.state", &J::Null), (ActivityKind::Read, true));
        assert_eq!(classify("sim.step", &J::Null), (ActivityKind::Sim, false));
        assert_eq!(classify("scene.save", &json!({"write": true})).0, ActivityKind::Edit);
        assert_eq!(classify("scene.save", &json!({})).0, ActivityKind::Read);
    }
}
