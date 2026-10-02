//! The MCP tools: definitions (name, description, JSON Schema) and the code
//! that runs them against the ERP endpoint.
//!
//! Tools are in groups (`scene`, `propose`, `verify`, `sim`, `history`) that
//! `--tools` switches on and off, to keep the prompt small.

use serde_json::{json, Map, Value as J};

use crate::bridge::{Bridge, Fail};
use crate::report;

/// The tool groups and what each is for.
pub const GROUPS: &[(&str, &str)] = &[
    ("scene", "read the scene: entities, one entity, the type schema"),
    ("propose", "stage changes as proposals, list, accept, reject"),
    ("verify", "check a proposal (or the scene) by deterministic replay"),
    ("sim", "start, step, seek and stop a play session"),
    ("history", "the undo history and undo"),
];

/// Which tool groups are on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolGroups(u8);

impl ToolGroups {
    /// Every group.
    pub const ALL: ToolGroups = ToolGroups((1 << GROUPS.len() as u8) - 1);

    /// Parses `scene,propose,verify` (or `all`).
    pub fn parse(text: &str) -> Result<ToolGroups, String> {
        let mut bits = 0u8;
        for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if part == "all" {
                bits = Self::ALL.0;
            } else if let Some(i) = GROUPS.iter().position(|(n, _)| *n == part) {
                bits |= 1 << i;
            } else {
                let names: Vec<&str> = GROUPS.iter().map(|(n, _)| *n).collect();
                return Err(format!("unknown tool group '{part}' (groups: {}, or all)", names.join(", ")));
            }
        }
        if bits == 0 {
            return Err("no tool group selected".into());
        }
        Ok(ToolGroups(bits))
    }

    /// True if `group` is on.
    pub fn has(self, group: &str) -> bool {
        GROUPS.iter().position(|(n, _)| *n == group).is_some_and(|i| self.0 & (1 << i) != 0)
    }

    /// The names of the groups that are on, in a fixed order.
    pub fn names(self) -> Vec<&'static str> {
        GROUPS.iter().filter(|(n, _)| self.has(n)).map(|(n, _)| *n).collect()
    }
}

/// What a tool returned: text for the model, and the same facts as JSON.
pub struct Out {
    /// Human-readable text (the `content` of the result).
    pub text: String,
    /// Structured form (an object; the `structuredContent` of the result).
    pub structured: J,
}

/// One tool.
pub struct ToolDef {
    /// Name.
    pub name: &'static str,
    /// Group.
    pub group: &'static str,
    /// Short display title.
    pub title: &'static str,
    /// For the model.
    pub description: &'static str,
    /// True if it changes nothing.
    pub read_only: bool,
    /// The ERP capability it needs (`read`, `scene_edit`, `sim_control`, `approve`).
    pub needs: &'static str,
    schema: fn() -> J,
}

impl ToolDef {
    /// The JSON Schema of the arguments.
    pub fn input_schema(&self) -> J {
        (self.schema)()
    }

    /// The `tools/list` entry.
    pub fn to_json(&self) -> J {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input_schema(),
            "annotations": {"title": self.title, "readOnlyHint": self.read_only, "openWorldHint": false},
        })
    }
}

fn obj(props: J, required: &[&str]) -> J {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

macro_rules! ops_help {
    () => {
        "Op forms (values in the schema's value format: fixed-point numbers as plain decimals, vec2 as [x, y], entity references as GUID strings):\n\
 - {\"op\":\"patch\",\"entity\":\"e_00000005\",\"component\":\"orr_physics::Body\",\"path\":\"pos\",\"value\":[0, 12.5]}  (path optional: without it `value` is the whole component, listed fields only)\n\
 - {\"op\":\"insert\",\"entity\":\"e_00000005\",\"component\":\"PaddleTag\"}  (`value` optional: the type default)\n\
 - {\"op\":\"remove\",\"entity\":\"e_00000005\",\"component\":\"PaddleTag\"}\n\
 - {\"op\":\"spawn\",\"name\":\"crate\",\"components\":{\"orr_physics::Body\":{...}}}  (a GUID is assigned and returned)\n\
 - {\"op\":\"despawn\",\"entity\":\"e_00000005\"}\n\
 - {\"op\":\"rename\",\"entity\":\"e_00000005\",\"name\":\"hero\"}\n\
 - {\"op\":\"singleton.patch\",\"name\":\"Scene\",\"path\":\"spawn_batch\",\"value\":3}"
    };
}

/// Every tool of the given groups, in a fixed order.
pub fn defs(groups: ToolGroups) -> Vec<&'static ToolDef> {
    TOOLS.iter().filter(|t| groups.has(t.group)).collect()
}

/// The tool with this name (of any group).
pub fn find(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}

static TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "scene_overview",
        group: "scene",
        title: "Scene overview",
        description: "List the entities of the scene (GUID, display name, component types) and the singletons, with counts. Start here to find the GUID of what you want to change. \
Component values are not included: use get_entity. Filter with `name` (substring of the display name) or `components` (entities that have all of them). \
Examples: {} ; {\"name\":\"body_0\",\"limit\":20} ; {\"components\":[\"PaddleTag\"]}.",
        read_only: true,
        needs: "read",
        schema: || {
            obj(
                json!({
                    "name": {"type": "string", "description": "keep entities whose display name contains this text"},
                    "components": {"type": "array", "items": {"type": "string"}, "description": "keep entities that have all of these component types, e.g. [\"orr_physics::Body\"]"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 2000, "description": "at most this many entities (default 200)"},
                    "offset": {"type": "integer", "minimum": 0, "description": "skip this many entities (paging)"},
                }),
                &[],
            )
        },
    },
    ToolDef {
        name: "get_entity",
        group: "scene",
        title: "Get entity",
        description: "Read the components of one entity: all of them, one component, or one field. `entity` is a GUID like e_00000005 (from scene_overview). \
Fixed-point numbers come back as exact decimal numbers, vectors as [x, y]. Pass `proposal_id` to read the entity as it WOULD be after that proposal is accepted. \
Examples: {\"entity\":\"e_00000005\"} ; {\"entity\":\"e_00000005\",\"component\":\"orr_physics::Body\",\"path\":\"pos\"}.",
        read_only: true,
        needs: "read",
        schema: || {
            obj(
                json!({
                    "entity": {"type": "string", "description": "GUID, e.g. e_00000005"},
                    "component": {"type": "string", "description": "one component type name, e.g. orr_physics::Body; omit for all"},
                    "path": {"type": "string", "description": "field path inside the component, e.g. pos or shape.half_extents; needs `component`"},
                    "proposal_id": {"type": "string", "description": "read the staged version in this proposal, e.g. p1"},
                }),
                &["entity"],
            )
        },
    },
    ToolDef {
        name: "get_schema",
        group: "scene",
        title: "Get schema",
        description: "The JSON Schema (draft 2020-12) of the scene format: every component and singleton type with its fields, ranges and docs. \
Use it to learn which fields exist and what values they take before you write a patch. With `type` you get one type (small); without it the whole schema (large, ask once). \
With `list_types: true` you get just the type names and one-line docs. With `input: true` get the game's structured player-input schema and value format (when supported); do not combine it with `type` or `list_types`. Examples: {\"type\":\"orr_physics::Body\"} ; {\"list_types\":true} ; {\"input\":true}.",
        read_only: true,
        needs: "read",
        schema: || {
            obj(
                json!({
                    "type": {"type": "string", "description": "one registered type name, e.g. orr_physics::Collider"},
                    "list_types": {"type": "boolean", "description": "only list type names and docs"},
                    "input": {"type": "boolean", "description": "the structured player-input schema and value format; exclusive with type/list_types"},
                }),
                &[],
            )
        },
    },
    ToolDef {
        name: "propose_changes",
        group: "propose",
        title: "Propose changes",
        description: concat!(
            "Stage a set of edits as a PROPOSAL. The scene is NOT changed: the engine keeps the edits on a private copy and returns the proposal id, a unified diff and a summary of what would change. \
All ops of one call apply together or not at all. Give `proposal_id` to add ops to an existing proposal, otherwise a new one is made. \
Then verify_proposal (replay check), then accept_proposal (a person may hold that step back). Use GUIDs from scene_overview and field names from get_schema.\n\n",
            ops_help!()
        ),
        read_only: false,
        needs: "scene_edit",
        schema: || {
            obj(
                json!({
                    "label": {"type": "string", "description": "short name of the change; becomes the undo-history entry, e.g. \"lift body_05\""},
                    "ops": {"type": "array", "minItems": 1, "description": "the edits, in order", "items": {
                        "type": "object",
                        "properties": {
                            "op": {"type": "string", "enum": ["patch", "insert", "remove", "spawn", "despawn", "rename", "singleton.patch"]},
                            "entity": {"type": "string", "description": "GUID, e.g. e_00000005"},
                            "component": {"type": "string", "description": "component type name, e.g. orr_physics::Body"},
                            "path": {"type": "string", "description": "field path, e.g. pos or shape.radius"},
                            "value": {"description": "the new value in the schema's format (any JSON)"},
                            "name": {"type": ["string", "null"], "description": "rename: the new display name (null clears); spawn: the display name; singleton.patch: the singleton type name"},
                            "guid": {"type": "string", "description": "spawn: use this GUID (optional)"},
                            "components": {"type": "object", "description": "spawn: component type name to whole value"},
                        },
                        "required": ["op"],
                        "additionalProperties": false,
                    }},
                    "proposal_id": {"type": "string", "description": "add to this open proposal (e.g. p1) instead of starting a new one"},
                }),
                &["ops"],
            )
        },
    },
    ToolDef {
        name: "list_proposals",
        group: "propose",
        title: "List proposals",
        description: "List the open proposals (id, label, who made it, number of ops, whether the scene changed since). With `proposal_id` it shows that proposal in full: its ops, the diff and whether it would accept cleanly now. \
People at the editor make proposals too; you see each other's.",
        read_only: true,
        needs: "read",
        schema: || obj(json!({"proposal_id": {"type": "string", "description": "show this proposal in full, e.g. p1"}}), &[]),
    },
    ToolDef {
        name: "accept_proposal",
        group: "propose",
        title: "Accept proposal",
        description: "Apply a proposal to the scene as ONE undoable history entry (recorded as made by whoever proposed it). Only after verify_proposal passed. Pass its `verified_state` to refuse any intervening scene or proposal changes. Omitting it is manual acceptance of the current proposal. \
If the scene changed meanwhile so that an op no longer applies you get a conflict and nothing changes. This needs the `approve` capability; if the host withheld it, ask the person to accept in the editor. \
Not possible while a play session runs. Example: {\"proposal_id\":\"p1\"}.",
        read_only: false,
        needs: "approve",
        schema: || obj(json!({
            "proposal_id": {"type": "string", "description": "e.g. p1"},
            "verified_state": {"type": "object", "description": "copy unchanged from verify_proposal to reject intervening edits", "properties": {
                "document_id": {"type": "string", "pattern": "^[0-9a-fA-F]{32}$"},
                "id": {"type": "string"},
                "document_revision": {"type": "integer", "minimum": 0},
                "proposal_revision": {"type": "integer", "minimum": 0}
            }, "required": ["document_id", "id", "document_revision", "proposal_revision"], "additionalProperties": false}
        }), &["proposal_id"]),
    },
    ToolDef {
        name: "reject_proposal",
        group: "propose",
        title: "Reject proposal",
        description: "Discard a proposal (for example after verify_proposal failed). The scene is untouched. Example: {\"proposal_id\":\"p1\"}.",
        read_only: false,
        needs: "scene_edit",
        schema: || obj(json!({"proposal_id": {"type": "string", "description": "e.g. p1"}}), &["proposal_id"]),
    },
    ToolDef {
        name: "verify_proposal",
        group: "verify",
        title: "Verify proposal",
        description: "Check a proposal by REPLAY: the engine runs the scene without and with the proposal, headlessly and deterministically, on the same inputs, and compares checksums and metrics. \
Omit `proposal_id` to run the scene alone (baseline metrics). Give `checks` for pass/fail, e.g. [\"lost_bodies.max == 0\", \"mean_height >= 2.5\", \"kinetic_energy.delta <= 10\"]. \
Check grammar: `<metric>[.start|final|min|max|delta] <|<=|==|!=|>=|> <number>` (no stat = final; delta = candidate final minus base final; prefix `base:` reads the run without the proposal), or `no_divergence`, `no_divergence_before <tick>`, `recording_matches`. \
Inputs: {\"kind\":\"bot\",\"ticks\":300,\"seed\":1} scripted players (the default), {\"kind\":\"idle\",\"ticks\":300} no input, {\"kind\":\"last_play\"} the last play session stopped in this host, {\"kind\":\"replay\",\"base64\":\"...\"} a .orrp file. \
Any scene edit changes the checksums from tick 0, so judge behaviour by metrics and checks, not by divergence. The host stays responsive while verification runs on captured scene/proposal state. Only one verification may run at a time; verify_busy means retry after it finishes.",
        read_only: true,
        needs: "read",
        schema: || {
            obj(
                json!({
                    "proposal_id": {"type": "string", "description": "the proposal to check, e.g. p1; omit for a baseline run of the scene"},
                    "inputs": {"type": "object", "description": "what to run; default {\"kind\":\"bot\",\"ticks\":300}", "properties": {
                        "kind": {"type": "string", "enum": ["bot", "idle", "last_play", "replay"]},
                        "ticks": {"type": "integer", "minimum": 1, "description": "bot/idle: ticks to run"},
                        "seed": {"type": "integer", "minimum": 0, "description": "bot: seed of the scripted players"},
                        "players": {"type": "integer", "minimum": 1, "maximum": 16},
                        "base64": {"type": "string", "description": "replay: the .orrp bytes"},
                    }, "required": ["kind"]},
                    "checks": {"type": "array", "items": {"type": "string"}, "description": "pass/fail rules, e.g. [\"lost_bodies.max == 0\"]"},
                    "ticks": {"type": "integer", "minimum": 1, "description": "run at most this many ticks (of a recording)"},
                    "sample_every": {"type": "integer", "minimum": 0, "description": "sample metrics every this many ticks (default 60)"},
                    "series": {"type": "boolean", "description": "also return every sampled value of each metric (structured result only)"},
                }),
                &[],
            )
        },
    },
    ToolDef {
        name: "sim_run",
        group: "sim",
        title: "Play session",
        description: "Drive a play session of the scene (a copy: the scene document is untouched). `action`: `state` (mode, tick, checksum), `start` (paused), `step` (run n ticks now), `seek` (go to a recorded tick), `play`/`pause` (run by the wall clock or not), `stop` (end; the recording stays available to verify_proposal as inputs kind=last_play). \
Typical: start, step n=120, state, stop. Examples: {\"action\":\"step\",\"n\":60} ; {\"action\":\"seek\",\"tick\":30}.",
        read_only: false,
        needs: "sim_control",
        schema: || {
            obj(
                json!({
                    "action": {"type": "string", "enum": ["state", "start", "step", "seek", "play", "pause", "stop"]},
                    "n": {"type": "integer", "minimum": 1, "description": "step: ticks to run (default 1)"},
                    "tick": {"type": "integer", "minimum": 0, "description": "seek: the tick to go to"},
                    "player_count": {"type": "integer", "minimum": 1, "maximum": 16, "description": "start: number of players"},
                }),
                &["action"],
            )
        },
    },
    ToolDef {
        name: "sim_input",
        group: "sim",
        title: "Set player input",
        description: "Set one player's structured input for subsequent play-session ticks. Discover the game's fields, ranges and value format first with get_schema input=true; input support depends on the host. Pass the complete input object, with exact decimal JSON numbers for fixed-point values. Start a session with sim_run action=start first, then sim_input and sim_run action=step. Input stays held until replaced; send the schema's neutral value to release it. This does not edit the scene document or advance a tick.",
        read_only: false,
        needs: "sim_control",
        schema: || {
            obj(
                json!({
                    "player": {"type": "integer", "minimum": 0, "description": "zero-based player slot in the active session"},
                    "value": {"type": "object", "description": "complete structured input using the schema returned by get_schema input=true"},
                }),
                &["player", "value"],
            )
        },
    },
    ToolDef {
        name: "history",
        group: "history",
        title: "Change history",
        description: "The undo history of the scene, oldest first: id, label, who made it (`user` or `agent:<name>`), number of ops, whether undone. Your accepted proposals show as `agent:<your client name>`.",
        read_only: true,
        needs: "read",
        schema: || obj(json!({}), &[]),
    },
    ToolDef {
        name: "undo",
        group: "history",
        title: "Undo",
        description: "Take back the last history entry (whoever made it; a whole accepted proposal at once). The scene returns exactly to its state before. Not possible while a play session runs.",
        read_only: false,
        needs: "scene_edit",
        schema: || obj(json!({}), &[]),
    },
];

// ---- running a tool ----

struct Args<'a>(&'a Map<String, J>);

impl<'a> Args<'a> {
    fn raw(&self, name: &str) -> Option<&'a J> {
        self.0.get(name).filter(|v| !v.is_null())
    }
    fn opt_str(&self, name: &str) -> Result<Option<&'a str>, Fail> {
        match self.raw(name) {
            None => Ok(None),
            Some(J::String(s)) => Ok(Some(s)),
            Some(_) => Err(Fail(format!("argument '{name}' must be a string"))),
        }
    }
    fn str(&self, name: &str) -> Result<&'a str, Fail> {
        self.opt_str(name)?.ok_or_else(|| Fail(format!("missing argument '{name}'")))
    }
    fn opt_u64(&self, name: &str) -> Result<Option<u64>, Fail> {
        match self.raw(name) {
            None => Ok(None),
            Some(v) => v.as_u64().map(Some).ok_or_else(|| Fail(format!("argument '{name}' must be a non-negative integer"))),
        }
    }
    fn opt_bool(&self, name: &str) -> Result<Option<bool>, Fail> {
        match self.raw(name) {
            None => Ok(None),
            Some(J::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(Fail(format!("argument '{name}' must be true or false"))),
        }
    }
}

/// Runs the tool `name` with `arguments` (a JSON object).
pub fn run(bridge: &mut Bridge, name: &str, arguments: &Map<String, J>) -> Result<Out, Fail> {
    let a = Args(arguments);
    match name {
        "scene_overview" => scene_overview(bridge, &a),
        "get_entity" => get_entity(bridge, &a),
        "get_schema" => get_schema(bridge, &a),
        "propose_changes" => propose_changes(bridge, &a),
        "list_proposals" => list_proposals(bridge, &a),
        "accept_proposal" => accept_proposal(bridge, &a),
        "reject_proposal" => reject_proposal(bridge, &a),
        "verify_proposal" => verify_proposal(bridge, &a),
        "sim_run" => sim_run(bridge, &a),
        "sim_input" => sim_input(bridge, &a),
        "history" => history(bridge),
        "undo" => undo(bridge),
        other => Err(Fail(format!("unknown tool '{other}'"))),
    }
}

fn text_out(text: String, structured: J) -> Result<Out, Fail> {
    Ok(Out { text, structured })
}

fn s(v: &J) -> &str {
    v.as_str().unwrap_or("")
}

fn scene_overview(b: &mut Bridge, a: &Args<'_>) -> Result<Out, Fail> {
    let mut p = Map::new();
    p.insert("limit".into(), json!(a.opt_u64("limit")?.unwrap_or(200)));
    if let Some(o) = a.opt_u64("offset")? {
        p.insert("offset".into(), json!(o));
    }
    if let Some(n) = a.opt_str("name")? {
        p.insert("name".into(), json!(n));
    }
    if let Some(c) = a.raw("components") {
        p.insert("components".into(), c.clone());
    }
    let offset = a.opt_u64("offset")?.unwrap_or(0);
    let q = b.call("world.query", J::Object(p))?;
    let singles = b.call("world.singleton.get", json!({}))?;
    let state = b.call("sim.state", J::Null)?;
    let entities: Vec<J> = q["entities"]
        .as_array()
        .map(|list| list.iter().map(|e| json!({"id": e["id"], "name": e["name"], "components": e["components"]})).collect())
        .unwrap_or_default();
    let total = q["total"].as_u64().unwrap_or(0);
    let mut text = format!(
        "Scene ({} mode): {} entities, checksum {}.\n",
        s(&state["mode"]),
        state["entities"],
        s(&q["checksum"])
    );
    let names: Vec<&str> = singles["singletons"].as_object().map(|m| m.keys().map(String::as_str).collect()).unwrap_or_default();
    text.push_str(&format!("Singletons: {}\n", if names.is_empty() { "none".to_string() } else { names.join(", ") }));
    if let Some(m) = singles["singletons"].as_object() {
        for (k, v) in m {
            text.push_str(&format!("  {k} = {v}\n"));
        }
    }
    let shown = entities.len() as u64;
    text.push_str(&format!("Entities {}-{} of {} matching:\n", if shown == 0 { 0 } else { offset + 1 }, offset + shown, total));
    for e in &entities {
        let comps: Vec<&str> = e["components"].as_array().map(|c| c.iter().map(s).collect()).unwrap_or_default();
        text.push_str(&format!("  {}  {}  [{}]\n", s(&e["id"]), e["name"].as_str().unwrap_or("(unnamed)"), comps.join(", ")));
    }
    if q["truncated"] == true {
        text.push_str("(more entities: use offset/limit, or filter with name/components)\n");
    }
    text_out(
        text,
        json!({"mode": state["mode"], "checksum": q["checksum"], "total": total, "truncated": q["truncated"], "entities": entities, "singletons": singles["singletons"]}),
    )
}

fn get_entity(b: &mut Bridge, a: &Args<'_>) -> Result<Out, Fail> {
    let mut p = Map::new();
    p.insert("entity".into(), json!(a.str("entity")?));
    for k in ["component", "path"] {
        if let Some(v) = a.opt_str(k)? {
            p.insert(k.into(), json!(v));
        }
    }
    let r = match a.opt_str("proposal_id")? {
        Some(id) => {
            p.insert("id".into(), json!(id));
            b.call("proposal.preview", J::Object(p))?
        }
        None => b.call("world.get", J::Object(p))?,
    };
    let text = serde_json::to_string_pretty(&r).unwrap_or_default();
    text_out(text, if r.is_object() { r } else { json!({"value": r}) })
}

fn get_schema(b: &mut Bridge, a: &Args<'_>) -> Result<Out, Fail> {
    if a.opt_bool("input")? == Some(true) {
        if a.raw("type").is_some() || a.opt_bool("list_types")? == Some(true) {
            return Err(Fail::new("argument 'input' cannot be combined with 'type' or 'list_types'"));
        }
        let r = b.call("registry.input", J::Null)?;
        return text_out(serde_json::to_string_pretty(&r).unwrap_or_default(), r);
    }
    if a.opt_bool("list_types")? == Some(true) {
        let r = b.call("registry.types", J::Null)?;
        let mut text = String::new();
        for t in r["types"].as_array().map(Vec::as_slice).unwrap_or_default() {
            text.push_str(&format!("{} ({}): {}\n", s(&t["name"]), s(&t["kind"]), s(&t["doc"])));
        }
        return text_out(text, r);
    }
    let mut p = Map::new();
    if let Some(t) = a.opt_str("type")? {
        p.insert("type".into(), json!(t));
    }
    let r = b.call("registry.schema", J::Object(p))?;
    let schema = strip_fixed_docs(&r["schema"]);
    let text = serde_json::to_string(&schema).unwrap_or_default();
    text_out(text, json!({"schema": schema, "value_format": r["value_format"]}))
}

/// The schema without the long, identical "Fixed-point number, Q48.16 ..."
/// description the engine repeats on every fixed-point field (the value
/// format says it once), to keep the schema small for a model.
fn strip_fixed_docs(schema: &J) -> J {
    match schema {
        J::Object(m) => J::Object(
            m.iter()
                .filter(|(k, v)| !(k.as_str() == "description" && v.as_str().is_some_and(|d| d.starts_with("Fixed-point number"))))
                .map(|(k, v)| (k.clone(), strip_fixed_docs(v)))
                .collect(),
        ),
        J::Array(a) => J::Array(a.iter().map(strip_fixed_docs).collect()),
        other => other.clone(),
    }
}

/// A proposal staged by [`stage_proposal`].
pub struct Staged {
    /// The proposal id (`p1`).
    pub id: String,
    /// `proposal.get` of it: ops, diff, summary.
    pub got: J,
    /// `proposal.apply`'s `spawned` list (op index and GUID of each new entity).
    pub spawned: J,
}

/// Stages `ops` on a proposal: a new one named `label`, or the open one `existing`.
/// All ops apply or none: on failure a proposal made here is discarded again
/// and the error says so.
pub fn stage_proposal(b: &mut Bridge, label: &str, ops: J, existing: Option<&str>) -> Result<Staged, Fail> {
    let (id, created) = match existing {
        Some(id) => (id.to_string(), false),
        None => {
            let r = b.call("proposal.begin", json!({"label": label}))?;
            (s(&r["id"]).to_string(), true)
        }
    };
    let applied = match b.call("proposal.apply", json!({"id": id, "ops": ops})) {
        Ok(v) => v,
        Err(mut f) => {
            if created {
                let _ = b.call("proposal.reject", json!({"id": id}));
                f.0.push_str(" (Nothing was staged; the proposal was discarded.)");
            } else {
                f.0.push_str(" (None of the ops of this call was staged.)");
            }
            return Err(f);
        }
    };
    let got = b.call("proposal.get", json!({"id": id}))?;
    Ok(Staged { id, got, spawned: applied["spawned"].clone() })
}

fn propose_changes(b: &mut Bridge, a: &Args<'_>) -> Result<Out, Fail> {
    let ops = match a.raw("ops") {
        Some(J::Array(list)) => J::Array(list.clone()),
        Some(_) => return Err(Fail::new("argument 'ops' must be a list of op objects")),
        None => return Err(Fail::new("missing argument 'ops'")),
    };
    let label = a.opt_str("label")?.unwrap_or("agent changes");
    let Staged { id, got, spawned } = stage_proposal(b, label, ops, a.opt_str("proposal_id")?)?;
    let mut text = report::describe_proposal(&id, &got);
    text.push_str(&report::spawned_text(&spawned));
    text.push_str(&format!(
        "\nThe scene is unchanged. Next: verify_proposal {{\"proposal_id\":\"{id}\",\"checks\":[...]}}, then accept_proposal, or reject_proposal.\n"
    ));
    let mut structured = got;
    structured["spawned"] = spawned;
    text_out(text, structured)
}

fn list_proposals(b: &mut Bridge, a: &Args<'_>) -> Result<Out, Fail> {
    if let Some(id) = a.opt_str("proposal_id")? {
        let got = b.call("proposal.get", json!({"id": id}))?;
        let mut text = report::describe_proposal(id, &got);
        if got["stale"] == true {
            text.push_str("The scene changed since this proposal was made.\n");
        }
        return text_out(text, got);
    }
    let r = b.call("proposal.list", J::Null)?;
    let list = r["proposals"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut text = if list.is_empty() { "No open proposals.\n".to_string() } else { format!("{} open proposal(s):\n", list.len()) };
    for p in list {
        text.push_str(&format!(
            "  {}  \"{}\"  by {}  {} op(s){}\n",
            s(&p["id"]),
            s(&p["label"]),
            s(&p["origin"]),
            p["op_count"],
            if p["stale"] == true { "  (scene changed since)" } else { "" }
        ));
    }
    text_out(text, r)
}

fn accept_proposal(b: &mut Bridge, a: &Args<'_>) -> Result<Out, Fail> {
    let id = a.str("proposal_id")?;
    let before = b.call("proposal.list", J::Null).ok();
    // Args::raw treats null as absent for ordinary optional arguments.
    // A supplied but invalid guard must never select manual acceptance.
    let r = match a.0.get("verified_state") {
        Some(state) => b.call("proposal.accept_verified", json!({"id": id, "verified_state": state}))?,
        None => b.call("proposal.accept", json!({"id": id}))?,
    };
    let info = before.as_ref().and_then(|l| l["proposals"].as_array()).and_then(|l| l.iter().find(|p| s(&p["id"]) == id));
    let who = info.map(|p| format!(" \"{}\" by {}", s(&p["label"]), s(&p["origin"]))).unwrap_or_default();
    let text = match r["history_id"].as_u64() {
        Some(h) => format!("Accepted {id}{who}: applied {} op(s) as history entry #{h}. Scene checksum {}. Use `undo` to take it back.\n", r["applied"], s(&r["checksum"])),
        None => format!("Accepted {id}{who}, but its ops changed nothing (no history entry). Scene checksum {}.\n", s(&r["checksum"])),
    };
    text_out(text, r)
}

fn reject_proposal(b: &mut Bridge, a: &Args<'_>) -> Result<Out, Fail> {
    let id = a.str("proposal_id")?;
    let r = b.call("proposal.reject", json!({"id": id}))?;
    text_out(format!("Rejected {id}. The scene was not changed.\n"), r)
}

fn verify_proposal(b: &mut Bridge, a: &Args<'_>) -> Result<Out, Fail> {
    let mut p = Map::new();
    let id = a.opt_str("proposal_id")?;
    if let Some(id) = id {
        p.insert("id".into(), json!(id));
    }
    p.insert("inputs".into(), a.raw("inputs").cloned().unwrap_or_else(|| json!({"kind": "bot", "ticks": 300})));
    match a.raw("checks") {
        None => {}
        Some(J::String(one)) => {
            p.insert("checks".into(), json!([one]));
        }
        Some(list) => {
            p.insert("checks".into(), list.clone());
        }
    }
    for k in ["ticks", "sample_every"] {
        if let Some(v) = a.opt_u64(k)? {
            p.insert(k.into(), json!(v));
        }
    }
    if let Some(v) = a.opt_bool("series")? {
        p.insert("series".into(), json!(v));
    }
    let r = b.call(if id.is_some() { "proposal.verify" } else { "verify.self" }, J::Object(p))?;
    text_out(report::verify_text(&r), r)
}

fn sim_run(b: &mut Bridge, a: &Args<'_>) -> Result<Out, Fail> {
    let action = a.str("action")?;
    let r = match action {
        "state" => b.call("sim.state", J::Null)?,
        "start" => {
            let mut p = Map::new();
            if let Some(n) = a.opt_u64("player_count")? {
                p.insert("player_count".into(), json!(n));
            }
            b.call("sim.start", J::Object(p))?
        }
        "step" => b.call("sim.step", json!({"n": a.opt_u64("n")?.unwrap_or(1)}))?,
        "seek" => match a.opt_u64("tick")? {
            Some(t) => b.call("sim.seek", json!({"tick": t}))?,
            None => return Err(Fail::new("action 'seek' needs argument 'tick'")),
        },
        "play" => b.call("sim.play", J::Null)?,
        "pause" => b.call("sim.pause", J::Null)?,
        "stop" => {
            let r = b.call("sim.stop", json!({}))?;
            let text = format!(
                "Play stopped at tick {}, checksum {}. The recording ({} bytes) is kept: verify_proposal with inputs {{\"kind\":\"last_play\"}} replays it.\n",
                r["tick"],
                s(&r["checksum"]),
                r["replay_bytes"]
            );
            return text_out(text, r);
        }
        other => return Err(Fail(format!("unknown action '{other}' (state, start, step, seek, play, pause, stop)"))),
    };
    text_out(report::state_text(&r), r)
}

fn sim_input(b: &mut Bridge, a: &Args<'_>) -> Result<Out, Fail> {
    let player = a.opt_u64("player")?.ok_or_else(|| Fail::new("missing argument 'player'"))?;
    let value = a.raw("value").ok_or_else(|| Fail::new("missing argument 'value'"))?;
    if !value.is_object() {
        return Err(Fail::new("argument 'value' must be an object"));
    }
    let r = b.call("sim.input_value", json!({"player": player, "value": value}))?;
    text_out(format!("Player {player} input set for subsequent ticks.\n"), r)
}

fn history(b: &mut Bridge) -> Result<Out, Fail> {
    let r = b.call("history.list", J::Null)?;
    text_out(report::history_text(&r), r)
}

fn undo(b: &mut Bridge) -> Result<Out, Fail> {
    let before = b.call("history.list", J::Null)?;
    let top = before["entries"].as_array().and_then(|l| l.iter().rev().find(|e| e["undone"] == false)).cloned();
    let r = b.call("history.undo", J::Null)?;
    let text = match top {
        Some(e) => format!("Undid entry #{} \"{}\" by {}. Scene checksum {}.\n", e["id"], s(&e["label"]), s(&e["origin"]), s(&r["checksum"])),
        None => format!("Undid the last entry. Scene checksum {}.\n", s(&r["checksum"])),
    };
    text_out(text, r)
}
