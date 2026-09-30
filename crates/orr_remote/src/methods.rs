//! The method table: one place for names, capabilities and parameter docs.
//! `rpc.discover` prints it and the dispatcher enforces it.

use serde_json::{json, Value as J};

use crate::caps::{Cap, Caps};

/// One parameter of a method.
pub struct ParamDoc {
    /// Parameter name (a key of the `params` object).
    pub name: &'static str,
    /// Type in words.
    pub ty: &'static str,
    /// True if the call fails without it.
    pub required: bool,
    /// What it does.
    pub doc: &'static str,
}

/// One method.
pub struct MethodDoc {
    /// Namespaced name (`world.patch`).
    pub name: &'static str,
    /// Capability the caller's token needs; `None` = any authenticated client.
    pub cap: Option<Cap>,
    /// One line.
    pub summary: &'static str,
    /// Parameters (`params` is an object).
    pub params: &'static [ParamDoc],
    /// The `result` in words.
    pub result: &'static str,
}

const fn p(name: &'static str, ty: &'static str, required: bool, doc: &'static str) -> ParamDoc {
    ParamDoc { name, ty, required, doc }
}

const ENTITY: ParamDoc = p("entity", "string", true, "GUID `e_7f3a91c2`, or a frame handle `12v0` (for entities created during play)");
const COMPONENT: ParamDoc = p("component", "string", true, "registered component type name, e.g. `orr_physics::Body`");
const PATH: ParamDoc = p("path", "string", false, "field path like `pos.x` or `shape.half_extents`; empty or missing = the whole value");
const VALUE: ParamDoc = p("value", "json", true, "the value in the format of `registry.schema` (fixed-point numbers as exact decimals)");

const PROPOSAL: ParamDoc = p("id", "string", true, "proposal id `p1` (from proposal.begin)");
const OPS: ParamDoc = p(
    "ops",
    "object[]",
    true,
    "edits, each `{ op, ... }` with the params of the world.* method: `patch` {entity, component, path?, value}, `insert` {entity, component, value?}, `remove` {entity, component}, `spawn` {name?, guid?, components?}, `despawn` {entity}, `rename` {entity, name}, `singleton.patch` {name, path?, value}",
);
const INPUTS: ParamDoc = p(
    "inputs",
    "object",
    true,
    "what to run: `{kind:\"last_play\"}` (recording of the last stopped play session), `{kind:\"replay\", base64}` (a .orrp), `{kind:\"bot\", ticks, seed?, players?}` (scripted players), `{kind:\"idle\", ticks, players?}` (default inputs)",
);
const TICKS: ParamDoc = p("ticks", "integer", false, "run at most this many ticks (of a recording)");
const CHECKS: ParamDoc = p(
    "checks",
    "string[]",
    false,
    "rules judged on the report, e.g. `lost_bodies.max == 0`, `mean_height >= 2.5`, `base:dynamic_bodies.final == 40`, `kinetic_energy.delta <= 10`, `no_divergence`, `no_divergence_before 300`, `recording_matches` (metric.stat with stat = start|final|min|max|delta; no stat = final; `base:` = the base run)",
);
const SAMPLE_EVERY: ParamDoc = p("sample_every", "integer", false, "sample metrics and checksums every this many ticks (default 60; 0 = only start and end)");
const SERIES: ParamDoc = p("series", "bool", false, "also return every sampled value of each metric (default false)");

/// Every method, in the order `rpc.discover` lists them.
pub static METHODS: &[MethodDoc] = &[
    MethodDoc { name: "rpc.discover", cap: None, summary: "List the methods with their parameters and required capabilities, and the value format.", params: &[], result: "{ erp_version, methods, value_format, you }" },
    MethodDoc { name: "registry.schema", cap: Some(Cap::Read), summary: "JSON Schema (draft 2020-12) of the scene format: every component and singleton, whole or one type.", params: &[p("type", "string", false, "one registered type name; missing = the whole scene schema")], result: "{ schema }" },
    MethodDoc { name: "registry.types", cap: Some(Cap::Read), summary: "List the registered component and singleton types.", params: &[], result: "{ types: [{ name, kind, doc }] }" },
    MethodDoc { name: "world.query", cap: Some(Cap::Read), summary: "List entities (of the play frame while playing, else of the scene) with GUID, name and component names.", params: &[p("components", "string[]", false, "keep only entities that have all of these components"), p("name", "string", false, "keep only entities whose name contains this text"), p("values", "bool", false, "also return every component value (default false)"), p("limit", "integer", false, "at most this many entities (default 1000, max 20000)"), p("offset", "integer", false, "skip this many entities")], result: "{ entities: [{ id, guid, handle, name, components, values? }], total, truncated, tick, checksum }" },
    MethodDoc { name: "world.get", cap: Some(Cap::Read), summary: "Read the components of one entity, or one component or one field.", params: &[ENTITY, p("component", "string", false, "one component; missing = all components"), PATH], result: "{ entity, components } or { value }" },
    MethodDoc { name: "world.singleton.get", cap: Some(Cap::Read), summary: "Read one singleton (or a field of it), or all of them.", params: &[p("name", "string", false, "singleton type name; missing = all singletons"), PATH], result: "{ singletons } or { value }" },
    MethodDoc { name: "world.patch", cap: Some(Cap::SceneEdit), summary: "Set one field (or a whole component). While playing it is a recorded debug command and also needs sim_control.", params: &[ENTITY, COMPONENT, PATH, VALUE], result: "{ changed, checksum }" },
    MethodDoc { name: "world.insert", cap: Some(Cap::SceneEdit), summary: "Add a component (default value if `value` is missing).", params: &[ENTITY, COMPONENT, p("value", "json", false, "whole component value; struct fields may be left out (they take the type default)")], result: "{ changed, checksum }" },
    MethodDoc { name: "world.remove", cap: Some(Cap::SceneEdit), summary: "Remove a component.", params: &[ENTITY, COMPONENT], result: "{ changed, checksum }" },
    MethodDoc { name: "world.spawn", cap: Some(Cap::SceneEdit), summary: "Create an entity. In edit mode it gets a fresh GUID (or `guid`); while playing it has no GUID and is addressed by handle.", params: &[p("name", "string", false, "display name (edit mode only)"), p("guid", "string", false, "GUID to use (edit mode only)"), p("components", "object", false, "component type name to whole value")], result: "{ guid?, handle?, changed, checksum }" },
    MethodDoc { name: "world.despawn", cap: Some(Cap::SceneEdit), summary: "Delete an entity (refused while another entity points at it, in edit mode).", params: &[ENTITY], result: "{ changed, checksum }" },
    MethodDoc { name: "world.rename", cap: Some(Cap::SceneEdit), summary: "Set or clear the display name (edit mode only).", params: &[ENTITY, p("name", "string|null", true, "new name, or null to clear")], result: "{ changed, checksum }" },
    MethodDoc { name: "world.singleton.patch", cap: Some(Cap::SceneEdit), summary: "Set one field (or all) of a singleton. While playing it is a recorded debug command and also needs sim_control.", params: &[p("name", "string", true, "singleton type name"), PATH, VALUE], result: "{ changed, checksum }" },
    MethodDoc { name: "tx.begin", cap: Some(Cap::SceneEdit), summary: "Start a transaction: the edits until `tx.commit` are one undo step. Other clients' edits are refused meanwhile. Rolled back if the connection drops or after the timeout.", params: &[p("label", "string", false, "name of the history entry")], result: "{ ok }" },
    MethodDoc { name: "tx.commit", cap: Some(Cap::SceneEdit), summary: "End the transaction and record it in the history.", params: &[], result: "{ ok, history }" },
    MethodDoc { name: "tx.rollback", cap: Some(Cap::SceneEdit), summary: "End the transaction and take back everything it did.", params: &[], result: "{ ok, checksum }" },
    MethodDoc { name: "history.list", cap: Some(Cap::Read), summary: "The undo history, oldest first, with who made each entry.", params: &[], result: "{ entries: [{ id, label, origin, op_count, undone }], can_undo, can_redo, dirty, in_tx }" },
    MethodDoc { name: "history.undo", cap: Some(Cap::SceneEdit), summary: "Take back the last history entry (a whole transaction at once), whoever made it.", params: &[], result: "{ history, checksum }" },
    MethodDoc { name: "history.redo", cap: Some(Cap::SceneEdit), summary: "Repeat the last undone entry.", params: &[], result: "{ history, checksum }" },
    MethodDoc { name: "scene.save", cap: Some(Cap::Read), summary: "The scene as YAML text. With `write: true` (needs scene_edit) the server also writes its configured scene file and marks the document saved.", params: &[p("write", "bool", false, "write the server's scene file (only if it was started with one)"), p("source", "string", false, "`doc` (default) or `play`: the live play frame turned back into a scene"), p("path", "string", false, "with `write`: write this file instead and make it the scene file (only on a host started with scene paths allowed, like the editor's own)")], result: "{ text, checksum, dirty, written? }" },
    MethodDoc { name: "scene.load", cap: Some(Cap::SceneEdit), summary: "Replace the document with scene text. History is cleared. Refused while playing or in a transaction.", params: &[p("text", "string", true, "scene YAML (orr.scene/1)"), p("path", "string", false, "the file the text came from: becomes the scene file `scene.save` writes (only on a host started with scene paths allowed)")], result: "{ entities, checksum }" },
    MethodDoc { name: "sim.state", cap: Some(Cap::Read), summary: "Mode (edit or play), head and last tick, playing, checksum, branch count.", params: &[], result: "{ mode, playing, head_tick, last_tick, checksum, branches, ... }" },
    MethodDoc { name: "sim.checksum", cap: Some(Cap::Read), summary: "Checksum of a tick of the play session (recorded value), default the live head; the preview frame in edit mode.", params: &[p("tick", "integer", false, "a tick inside the recorded range")], result: "{ tick, checksum }" },
    MethodDoc { name: "sim.start", cap: Some(Cap::SimControl), summary: "Start a play session from the current document (paused unless `run`).", params: &[p("player_count", "integer", false, "players, 1..=16 (default: the server's)"), p("tick_rate", "integer", false, "ticks per second, 1..=1000 (default: the server's)"), p("run", "bool", false, "start playing at once")], result: "sim.state" },
    MethodDoc { name: "sim.stop", cap: Some(Cap::SimControl), summary: "End the play session; the document is untouched.", params: &[p("include_replay", "bool", false, "return the recording (.orrp) as base64")], result: "{ tick, checksum, replay_bytes, replay? }" },
    MethodDoc { name: "sim.play", cap: Some(Cap::SimControl), summary: "Run by the wall clock.", params: &[], result: "sim.state" },
    MethodDoc { name: "sim.pause", cap: Some(Cap::SimControl), summary: "Stop running by the wall clock.", params: &[], result: "sim.state" },
    MethodDoc { name: "sim.step", cap: Some(Cap::SimControl), summary: "Run n ticks now (default 1), playing or not. Limited per call by the server.", params: &[p("n", "integer", false, "ticks to run")], result: "sim.state" },
    MethodDoc { name: "sim.seek", cap: Some(Cap::SimControl), summary: "Go to the state of a recorded tick and pause.", params: &[p("tick", "integer", true, "a tick in first_tick..=last_tick")], result: "sim.state" },
    MethodDoc { name: "sim.branch", cap: Some(Cap::SimControl), summary: "Cut the recorded future after the head; recording goes on from there.", params: &[], result: "sim.state" },
    MethodDoc { name: "sim.speed", cap: Some(Cap::SimControl), summary: "Wall-clock speed in thousandths (1000 = 1x), clamped to 250..=4000.", params: &[p("permille", "integer", true, "speed x 1000")], result: "sim.state" },
    MethodDoc { name: "sim.debug", cap: Some(Cap::SimControl), summary: "A raw byte-level debug command (what the Remote bridge sends): applied at a tick boundary and recorded.", params: &[p("cmd", "string", true, "set_field | set_singleton_field | spawn | despawn | add_component | remove_component"), p("entity", "string", false, "handle `12v0`"), p("component", "integer", false, "component id"), p("singleton", "integer", false, "singleton id"), p("offset", "integer", false, "byte offset"), p("bytes", "string", false, "hex bytes"), p("components", "object[]", false, "for spawn: [{ component, bytes }]")], result: "{ ok }" },
    MethodDoc { name: "sim.input", cap: Some(Cap::SimControl), summary: "Set the held input of a player (raw bytes of the game's Input type).", params: &[p("player", "integer", true, "player slot"), p("input", "string", true, "hex bytes")], result: "{ ok }" },
    MethodDoc { name: "sim.command", cap: Some(Cap::SimControl), summary: "Queue a game command for the next tick (encoded bytes of the game's Command type).", params: &[p("player", "integer", false, "player slot (default 0)"), p("command", "string", true, "hex bytes")], result: "{ ok }" },
    MethodDoc { name: "proposal.begin", cap: Some(Cap::SceneEdit), summary: "Start a proposal: a named set of edits staged on a private copy of the scene, for you to build, look at and verify before anyone accepts it. The scene is untouched. Its history entry will carry your client name.", params: &[p("label", "string", false, "name of the proposal (and of the history entry it becomes)")], result: "{ id, label, origin }" },
    MethodDoc { name: "proposal.apply", cap: Some(Cap::SceneEdit), summary: "Stage edits on a proposal. All or nothing per call: if one op is invalid, none is staged.", params: &[PROPOSAL, OPS], result: "{ id, applied, changed, op_count, spawned: [{ index, guid }], checksum }" },
    MethodDoc { name: "proposal.list", cap: Some(Cap::Read), summary: "The open proposals, oldest first. `stale` = the scene changed since the proposal was begun.", params: &[], result: "{ proposals: [{ id, label, origin, op_count, stale }] }" },
    MethodDoc { name: "proposal.get", cap: Some(Cap::Read), summary: "One proposal: its ops, a unified text diff of the scene, a structured summary, and whether it would accept cleanly now.", params: &[PROPOSAL], result: "{ id, label, origin, stale, op_count, ops, diff, summary, accepts_cleanly, accept_error? }" },
    MethodDoc { name: "proposal.preview", cap: Some(Cap::Read), summary: "Read the staged scene as it would be after accept: one entity (`entity`) like world.get, or a list like world.query (with component values by default).", params: &[PROPOSAL, p("entity", "string", false, "GUID; missing = list entities"), p("components", "string[]", false, "list: keep entities with all of these components"), p("name", "string", false, "list: keep entities whose name contains this"), p("values", "bool", false, "list: include component values (default true)"), p("limit", "integer", false, "list: at most this many (default 1000)"), p("offset", "integer", false, "list: skip this many")], result: "{ entity, components } or { entities, total, truncated, checksum }" },
    MethodDoc { name: "proposal.verify", cap: Some(Cap::Read), summary: "Run the scene (base) and the scene with the proposal (candidate) headlessly on the same inputs and compare checksums and metrics; optionally judge with `checks`. Runs on the host thread: keep `ticks` moderate.", params: &[PROPOSAL, INPUTS, TICKS, CHECKS, SAMPLE_EVERY, SERIES], result: "verify report: { ticks, identical, first_divergence, checksums, metrics: [{ name, kind, base, candidate, delta }], recording?, checks?: { passed, results }, lines }" },
    MethodDoc { name: "verify.self", cap: Some(Cap::Read), summary: "Baseline run of the scene against itself on the same inputs: its metrics, and for a recording whether the scene reproduces it. Same result shape as proposal.verify.", params: &[INPUTS, TICKS, CHECKS, SAMPLE_EVERY, SERIES], result: "verify report" },
    MethodDoc { name: "proposal.accept", cap: Some(Cap::Approve), summary: "Apply the proposal to the scene as one history entry (origin = who began it; `history.undo` takes it back). Fails with a conflict, changing nothing, if the scene changed so that an op no longer applies. Edit mode only.", params: &[PROPOSAL], result: "{ history_id, applied, checksum }" },
    MethodDoc { name: "proposal.reject", cap: Some(Cap::SceneEdit), summary: "Discard a proposal.", params: &[PROPOSAL], result: "{ ok }" },
    MethodDoc { name: "activity.list", cap: Some(Cap::Read), summary: "The activity log: what every client's requests did, one line each (newest last), with the entities touched, old/new values of field writes, and verification outcomes. A bounded ring; this call itself is not recorded.", params: &[p("since", "integer", false, "only entries with a larger `seq` (default 0 = all still kept)"), p("limit", "integer", false, "at most this many, the newest (default 200, max 2000)"), p("include_reads", "bool", false, "also the read-only requests (default false)")], result: "{ entries: [{ seq, at_ms, client, kind, method, summary, ok, read, error?, entities?, proposal?, change?, diff?, verify? }], last_seq, truncated, clients: [{ client, capabilities, requests, connected_ms }] (connected now), now_ms }" },
    MethodDoc { name: "watch.subscribe", cap: Some(Cap::Read), summary: "Have the server push notifications: `watch.tick`, `watch.history`, `watch.events`, `watch.notes`, `watch.proposals`, `watch.activity` (new activity entries), binary frame messages (`frames`, WebSocket only) and the language-neutral view stream (`viewstream`, see docs/view-stream.md; the schema arrives first as `watch.viewstream.schema`, then the frames as binary messages, or as `watch.viewstream` notifications with hex on plain TCP).", params: &[p("topics", "string[]", true, "tick | history | events | notes | proposals | activity | frames | viewstream"), p("max_fps", "integer", false, "cap for `frames` and `viewstream` (default 60)"), p("source", "string", false, "`frames` and `viewstream`: `sim` (default: the play session's frames, none in edit mode), `view` (the play frame while playing, else the scene's preview frame, sent after every edit) or `proposal:p3` (the staged scene of that proposal, edit mode only; `frames` only)"), p("include_reads", "bool", false, "`activity`: also push read-only requests (default false)")], result: "{ topics }" },
    MethodDoc { name: "debug.panic", cap: Some(Cap::SimControl), summary: "Test hook: make the host thread panic (refused unless the host was started with debug hooks). Used to test that a view survives a crashed host.", params: &[], result: "{ ok }" },
    MethodDoc { name: "watch.unsubscribe", cap: Some(Cap::Read), summary: "Stop pushes (all topics, or the listed ones).", params: &[p("topics", "string[]", false, "topics to stop; missing = all")], result: "{ topics }" },
];

/// The method with this name.
pub fn find(name: &str) -> Option<&'static MethodDoc> {
    METHODS.iter().find(|m| m.name == name)
}

/// The value format, in words, for `rpc.discover`.
pub const VALUE_FORMAT: &str = "Values follow the scene file and `registry.schema`: bool = true/false; integer = JSON number; \
fixed-point = JSON number with its exact decimal text (never rounded through a float; the server accepts a number or a string \
with the same text, an exponent, and rounds extra digits to 1/65536); vec2/vec3 = [x, y] / [x, y, z]; entity reference = GUID \
string `e_7f3a91c2` or null (`12v0` handle text for entities created during play); enum = variant name; flags = array of names; \
struct = object with all fields (a whole-component write may leave fields out: they take the type default); tagged value = \
object with `kind`. u64 checksums are strings `0x` + 16 hex digits.";

/// The `rpc.discover` result for a client with `caps` (`you`).
pub fn discover(client: &str, caps: Caps) -> J {
    let methods: Vec<J> = METHODS
        .iter()
        .map(|m| {
            json!({
                "name": m.name,
                "capability": m.cap.map(Cap::name),
                "summary": m.summary,
                "params": m.params.iter().map(|q| json!({"name": q.name, "type": q.ty, "required": q.required, "doc": q.doc})).collect::<Vec<_>>(),
                "result": m.result,
                "allowed": m.cap.is_none_or(|c| caps.has(c)),
            })
        })
        .collect();
    json!({
        "erp_version": 1,
        "protocol": "JSON-RPC 2.0 over WebSocket (text messages) or newline-delimited JSON over TCP",
        "auth": "first message {\"method\":\"auth\",\"params\":{\"token\":...}}, or ?token= in the WebSocket URL; no token = only in dev mode",
        "capabilities": ["read", "scene_edit", "sim_control", "approve"],
        "methods": methods,
        "notifications": ["watch.tick", "watch.history", "watch.events", "watch.notes", "watch.proposals", "watch.activity", "watch.viewstream.schema", "watch.viewstream"],
        "value_format": VALUE_FORMAT,
        "you": { "client": client, "capabilities": caps.list().into_iter().map(Cap::name).collect::<Vec<_>>() },
    })
}
