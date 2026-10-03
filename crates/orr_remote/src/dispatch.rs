//! Executes ERP methods against the host's model.
//!
//! Direct calls here are synchronous. [`crate::ErpServer`] intercepts verification
//! to capture immutable inputs and execute it on a bounded worker; other methods
//! run on the host thread (a [`crate::LocalHost`] or the headless loop).

use std::path::PathBuf;

use orr_ecs::Entity;
use orr_edit::{EditorDoc, EntityInfo, Op, Origin, PlayController, StoppedPlay, Target, View};
use orr_reflect::{Guid, TypeInfo, TypeKind, Value};
use orr_session::{ControlOp, PlayMode, Speed};
use orr_sim::{EventKey, Game, PlayerSlot, SimCommand};
use serde_json::{json, Map, Value as J};

use crate::caps::{Cap, Caps};
use crate::codec::{b64_encode, hex_decode};
use crate::error::*;
use crate::json::{desc_at_path, handle_text, json_to_value, map_entity_refs, parse_handle, value_to_json};
use crate::methods::{self, MethodDoc};
use crate::proposals::{self, GameHooks};
use crate::viewstream::ViewStreamHook;
use crate::wire::{checksum_text, debug_error_name, debug_from_json};

/// The host's model an ERP request runs against. It borrows the host's
/// state, so any host loop (`Host`, or a test) can embed the server. A
/// person's edits (client `user`) made on the same `doc` share its undo
/// stack with agent edits.
pub struct ErpTarget<'a, G: Game> {
    /// The scene document (edit mode, undo history).
    pub doc: &'a mut EditorDoc,
    /// The running play session, if any.
    pub play: &'a mut Option<PlayController<G>>,
}

/// Host-side settings of the methods.
#[derive(Clone, Debug)]
pub struct HostLimits {
    /// Players of a play session started by `sim.start` without `player_count`.
    pub player_count: u8,
    /// Tick rate of a play session started without `tick_rate`.
    pub tick_rate: u32,
    /// Most ticks one `sim.step` call may run (the host thread is busy meanwhile).
    pub max_step_per_call: u32,
    /// The scene file `scene.save` with `write: true` writes; `None` = never writes a file.
    pub scene_path: Option<PathBuf>,
    /// Most simulation ticks one `proposal.verify` / `verify.self` call may run.
    /// Default 6000. The server runs one verification worker at a time; this
    /// execution cap does not bound replay decoding memory or wall-clock time.
    pub max_verify_ticks: u32,
    /// Build id of the host's simulation (shown in `rpc.discover`, given to
    /// the verification runs). Default 0 = not tracked.
    pub build_id: u64,
    /// The game-specific parts of verification (metrics, scripted players).
    pub game: GameHooks,
    /// `scene.save` / `scene.load` may name a file (`path`): true for an
    /// embedding editor, whose person owns the machine; false (default) for
    /// a headless host, where only the configured scene file is written.
    pub allow_scene_paths: bool,
    /// Enables the `debug.panic` test hook (default false).
    pub debug_hooks: bool,
    /// The game's view stream producer (the `viewstream` topic of
    /// `watch.subscribe`); `None` = the host has no view stream.
    pub view_stream: Option<ViewStreamHook>,
    /// Client mode: the host is a relay client (`orr_remote_host --join`) and plays on a server; its
    /// view stream, `sim.state`/`session.status`, `sim.input` and `sim.command` come from that
    /// session and every method that needs a document or a local simulation is refused.
    pub client_session: Option<crate::client_mode::ClientSessionHook>,
}

impl Default for HostLimits {
    fn default() -> Self {
        Self {
            player_count: 2,
            tick_rate: 60,
            max_step_per_call: 600,
            scene_path: None,
            max_verify_ticks: 6000,
            build_id: 0,
            game: GameHooks::default(),
            allow_scene_paths: false,
            debug_hooks: false,
            view_stream: None,
            client_session: None,
        }
    }
}

/// Who is calling.
pub(crate) struct CallCtx<'a> {
    pub client: &'a str,
    pub caps: Caps,
    /// The recording of the last stopped play session of the host, if known.
    pub last_play: Option<&'a StoppedPlay>,
    /// The connection making the call, and the one that opened the current
    /// transaction; `None` skips the ownership check (in-process callers).
    pub tx_check: Option<(u64, Option<u64>)>,
}

/// What a call did besides its result.
#[derive(Default)]
pub(crate) struct Effects {
    /// Sim events of ticks the call ran (`sim.step`), as `(key, payload bytes)`.
    pub events: Vec<(EventKey, Vec<u8>)>,
    pub tx: TxChange,
    /// The recording of a play session this call stopped.
    pub stopped: Option<StoppedPlay>,
    /// The typed report of a verification this call ran.
    pub verify: Option<std::sync::Arc<crate::activity::VerifyDetail>>,
    /// The scene file the call set (`scene.save` / `scene.load` with `path`).
    pub scene_path: Option<PathBuf>,
    /// The call asked the host thread to panic (`debug.panic`).
    pub crash: bool,
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub(crate) enum TxChange {
    #[default]
    None,
    Opened,
    Closed,
}

/// Runs one method for an in-process caller (tests, embedding without a
/// socket). `client` becomes `Origin::Agent(client)`.
pub fn call_local<G: Game>(
    target: &mut ErpTarget<'_, G>,
    limits: &HostLimits,
    client: &str,
    caps: Caps,
    method: &str,
    params: &J,
) -> Result<J, RpcError> {
    let mut fx = Effects::default();
    call(target, limits, None, &CallCtx { client, caps, last_play: None, tx_check: None }, &mut fx, method, params)
}

/// Only the connection that opened a transaction may end it.
fn own_tx<G: Game>(t: &ErpTarget<'_, G>, ctx: &CallCtx<'_>) -> Result<(), RpcError> {
    if let Some((conn, owner)) = ctx.tx_check {
        if t.doc.in_tx() && owner != Some(conn) {
            return Err(RpcError::state("tx_busy", "the open transaction belongs to someone else"));
        }
    }
    Ok(())
}

fn is_world_mutation(method: &str) -> bool {
    matches!(
        method,
        "world.patch" | "world.insert" | "world.remove" | "world.spawn" | "world.despawn" | "world.singleton.patch"
    )
}

fn denied(method: &str, need: Cap, have: Caps) -> RpcError {
    RpcError::new(
        PERMISSION_DENIED,
        "permission_denied",
        format!("method '{method}' needs the '{need}' capability (this client has: {have})"),
    )
    .with("required", json!(need.name()))
}

/// Checks the capability of a method for a client. `playing`: a world edit
/// while a play session runs is a change of the sim and also needs `sim_control`.
pub(crate) fn authorize(method: &str, caps: Caps, playing: bool, params: &J) -> Result<&'static MethodDoc, RpcError> {
    let doc = methods::find(method)
        .ok_or_else(|| RpcError::new(METHOD_NOT_FOUND, "method_not_found", format!("unknown method '{method}' (see rpc.discover)")))?;
    if let Some(need) = doc.cap {
        if !caps.has(need) {
            return Err(denied(method, need, caps));
        }
    }
    if playing && is_world_mutation(method) && !caps.has(Cap::SimControl) {
        return Err(denied(method, Cap::SimControl, caps));
    }
    if method == "scene.save" && params.get("write").and_then(J::as_bool) == Some(true) && !caps.has(Cap::SceneEdit) {
        return Err(denied(method, Cap::SceneEdit, caps));
    }
    Ok(doc)
}

pub(crate) fn call<G: Game>(
    t: &mut ErpTarget<'_, G>,
    lim: &HostLimits,
    input: Option<&crate::input::StructuredInput<G>>,
    ctx: &CallCtx<'_>,
    fx: &mut Effects,
    method: &str,
    params: &J,
) -> Result<J, RpcError> {
    authorize(method, ctx.caps, t.play.is_some(), params)?;
    let empty = Map::new();
    let obj: &Map<String, J> = match params {
        J::Null => &empty,
        J::Object(m) => m,
        _ => return Err(RpcError::params("params must be an object")),
    };
    let p = P(obj);
    let origin = crate::caps::origin_of_client(ctx.client);
    match method {
        "rpc.discover" => {
            let mut result = proposals::discover_with_engine(t, lim, ctx);
            if let Some(input) = input {
                result["engine"]["input"] = input.descriptor.clone();
            }
            Ok(result)
        },
        "proposal.begin" => proposals::begin(t, &p, origin),
        "proposal.apply" => proposals::apply(t, &p),
        "proposal.list" => Ok(proposals::list(t)),
        "proposal.get" => proposals::get(t, &p),
        "proposal.preview" => proposals::preview(t, &p),
        "proposal.verify" => proposals::verify(t, lim, ctx, fx, &p, true),
        "verify.self" => proposals::verify(t, lim, ctx, fx, &p, false),
        "proposal.accept" | "proposal.accept_verified" => {
            require_edit_mode(t, method)?;
            proposals::accept(t, &p, method == "proposal.accept_verified")
        }
        "proposal.reject" => proposals::reject(t, &p),
        "registry.schema" => registry_schema(t, &p),
        "registry.input" => input.map(|i| i.descriptor.clone()).ok_or_else(input_unavailable),
        "registry.types" => Ok(registry_types(t)),
        "world.query" => world_query(t, &p),
        "world.get" => world_get(t, &p),
        "world.singleton.get" => singleton_get(t, &p),
        "world.patch" => world_patch(t, &p, origin),
        "world.insert" => world_insert(t, &p, origin),
        "world.remove" => world_remove(t, &p, origin),
        "world.spawn" => world_spawn(t, &p, origin),
        "world.despawn" => world_despawn(t, &p, origin),
        "world.rename" => world_rename(t, &p, origin),
        "world.singleton.patch" => singleton_patch(t, &p, origin),
        "tx.begin" => {
            require_edit_mode(t, "tx.begin")?;
            t.doc.begin_tx(p.opt_str("label")?.unwrap_or("agent edit"), origin)?;
            fx.tx = TxChange::Opened;
            Ok(json!({"ok": true}))
        }
        "tx.commit" => {
            require_edit_mode(t, "tx.commit")?;
            own_tx(t, ctx)?;
            t.doc.commit_tx()?;
            fx.tx = TxChange::Closed;
            Ok(json!({"ok": true, "history": history_json(t.doc)}))
        }
        "tx.rollback" => {
            require_edit_mode(t, "tx.rollback")?;
            own_tx(t, ctx)?;
            t.doc.rollback_tx()?;
            fx.tx = TxChange::Closed;
            Ok(json!({"ok": true, "checksum": checksum_text(t.doc.checksum())}))
        }
        "history.list" => Ok(history_json(t.doc)),
        "history.undo" => {
            require_edit_mode(t, "history.undo")?;
            t.doc.undo()?;
            Ok(json!({"history": history_json(t.doc), "checksum": checksum_text(t.doc.checksum())}))
        }
        "history.redo" => {
            require_edit_mode(t, "history.redo")?;
            t.doc.redo()?;
            Ok(json!({"history": history_json(t.doc), "checksum": checksum_text(t.doc.checksum())}))
        }
        "scene.save" => scene_save(t, lim, &p, fx),
        "debug.panic" => {
            if !lim.debug_hooks {
                return Err(RpcError::state("disabled", "debug hooks are off on this host"));
            }
            fx.crash = true;
            Ok(json!({"ok": true}))
        }
        "scene.load" => {
            require_edit_mode(t, "scene.load")?;
            let text = p.str("text")?;
            let path = scene_path_param(lim, &p)?;
            t.doc.load_yaml(text)?;
            if path.is_some() {
                fx.scene_path = path;
            }
            Ok(json!({"entities": t.doc.scene().entities.len(), "checksum": checksum_text(t.doc.checksum())}))
        }
        "sim.state" => Ok(state_json(t, lim)),
        "sim.checksum" => sim_checksum(t, &p),
        "sim.start" => sim_start(t, lim, input, &p),
        "sim.stop" => sim_stop(t, &p, fx),
        "sim.play" => control(t, lim, ControlOp::Play),
        "sim.pause" => control(t, lim, ControlOp::Pause),
        "sim.branch" => control(t, lim, ControlOp::Branch),
        "sim.speed" => {
            let permille = p.req_u64("permille")?;
            let permille = u32::try_from(permille).map_err(|_| RpcError::params("'permille' is too large"))?;
            control(t, lim, ControlOp::SetSpeed(Speed::from_permille(permille)))
        }
        "sim.step" => sim_step(t, lim, &p, fx),
        "sim.seek" => sim_seek(t, lim, &p),
        "sim.debug" => {
            let cmd = debug_from_json(obj).map_err(RpcError::params)?;
            let pc = play_mut(t)?;
            match pc.session_mut().debug(cmd) {
                Ok(()) => Ok(json!({"ok": true})),
                Err(e) => Err(RpcError::new(DEBUG_REFUSED, "debug_refused", e.to_string()).with("error", json!(debug_error_name(e)))),
            }
        }
        "session.status" => Err(RpcError::state("not_a_client", "this host is not a relay client (session.status is for hosts started with --join); see sim.state")),
        "sim.input" => sim_input(t, &p),
        "sim.input_value" => sim_input_value(t, input, &p),
        "sim.command" => sim_command(t, &p),
        // `watch.*` is handled by the server, which owns the subscriptions.
        other => Err(RpcError::new(METHOD_NOT_FOUND, "method_not_found", format!("method '{other}' cannot be called here"))),
    }
}

// ---- params ----

pub(crate) struct P<'a>(pub(crate) &'a Map<String, J>);

impl<'a> P<'a> {
    pub(crate) fn raw(&self, name: &str) -> Option<&'a J> {
        self.0.get(name)
    }
    pub(crate) fn req_raw(&self, name: &str) -> Result<&'a J, RpcError> {
        self.raw(name).ok_or_else(|| RpcError::params(format!("missing parameter '{name}'")))
    }
    pub(crate) fn opt_str(&self, name: &str) -> Result<Option<&'a str>, RpcError> {
        match self.raw(name) {
            None | Some(J::Null) => Ok(None),
            Some(J::String(s)) => Ok(Some(s)),
            Some(_) => Err(RpcError::params(format!("'{name}' must be a string"))),
        }
    }
    pub(crate) fn str(&self, name: &str) -> Result<&'a str, RpcError> {
        self.opt_str(name)?.ok_or_else(|| RpcError::params(format!("missing parameter '{name}' (string)")))
    }
    pub(crate) fn opt_u64(&self, name: &str) -> Result<Option<u64>, RpcError> {
        match self.raw(name) {
            None | Some(J::Null) => Ok(None),
            Some(J::Number(n)) => n.as_u64().map(Some).ok_or_else(|| RpcError::params(format!("'{name}' must be a non-negative integer"))),
            Some(_) => Err(RpcError::params(format!("'{name}' must be a non-negative integer"))),
        }
    }
    pub(crate) fn req_u64(&self, name: &str) -> Result<u64, RpcError> {
        self.opt_u64(name)?.ok_or_else(|| RpcError::params(format!("missing parameter '{name}' (integer)")))
    }
    pub(crate) fn opt_bool(&self, name: &str) -> Result<Option<bool>, RpcError> {
        match self.raw(name) {
            None | Some(J::Null) => Ok(None),
            Some(J::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(RpcError::params(format!("'{name}' must be true or false"))),
        }
    }
    pub(crate) fn target(&self) -> Result<Target, RpcError> {
        parse_target(self.str("entity")?)
    }
}

pub(crate) fn parse_target(s: &str) -> Result<Target, RpcError> {
    if Guid::parse(s).is_ok() {
        Ok(Target::Guid(Guid::parse(s).expect("checked")))
    } else if let Some(e) = parse_handle(s) {
        Ok(Target::Entity(e))
    } else {
        Err(RpcError::params(format!("'{s}' is not an entity: use a GUID like e_7f3a91c2 or a handle like 12v0")))
    }
}

// ---- helpers ----

pub(crate) fn view<'a, G: Game>(t: &'a ErpTarget<'_, G>) -> View<'a> {
    match t.play.as_ref() {
        Some(pc) => pc.view(),
        None => t.doc.view(),
    }
}

fn live_checksum<G: Game>(t: &ErpTarget<'_, G>) -> u64 {
    match t.play.as_ref() {
        Some(pc) => pc.session().frame().checksum(),
        None => t.doc.checksum(),
    }
}

pub(crate) fn require_edit_mode<G: Game>(t: &ErpTarget<'_, G>, what: &str) -> Result<(), RpcError> {
    if t.play.is_some() {
        return Err(RpcError::state("sim_running", format!("'{what}' changes the scene document; stop the play session first (sim.stop)")));
    }
    Ok(())
}

fn play_mut<'a, 'b, G: Game>(t: &'a mut ErpTarget<'b, G>) -> Result<&'a mut PlayController<G>, RpcError> {
    t.play.as_mut().ok_or_else(|| RpcError::state("no_play", "no play session (call sim.start)"))
}

pub(crate) fn component_type<'a>(v: &View<'a>, name: &str) -> Result<&'a TypeInfo, RpcError> {
    v.types().get(name).filter(|ti| ti.kind() == TypeKind::Component).ok_or_else(|| {
        RpcError::new(NOT_FOUND, "unknown_type", format!("unknown component type '{name}' (see registry.types)"))
    })
}

pub(crate) fn singleton_type<'a>(v: &View<'a>, name: &str) -> Result<&'a TypeInfo, RpcError> {
    v.types().get(name).filter(|ti| ti.kind() == TypeKind::Singleton).ok_or_else(|| {
        RpcError::new(NOT_FOUND, "unknown_type", format!("unknown singleton type '{name}' (see registry.types)"))
    })
}

pub(crate) fn invalid_value(msg: String) -> RpcError {
    RpcError::new(INVALID_VALUE, "invalid_value", msg)
}

/// Decodes the JSON `value` for `path` of type `ti`. Entity handles become
/// GUIDs in edit mode (`edit = true`); during play they stay handles.
fn decode_value<G: Game>(t: &ErpTarget<'_, G>, ti: &TypeInfo, path: &str, j: &J) -> Result<Value, RpcError> {
    let desc = desc_at_path(ti.desc(), path).map_err(|e| invalid_value(format!("{}: {e}", ti.name())))?;
    let v = json_to_value(&desc, j, path.is_empty())
        .map_err(|e| invalid_value(if path.is_empty() { format!("{}: {e}", ti.name()) } else { format!("{}.{path}: {e}", ti.name()) }))?;
    if t.play.is_some() {
        return Ok(v);
    }
    scene_form(&t.doc.view(), &v)
}

/// The scene form of a decoded value: entity handles become GUIDs (looked up in `view`).
pub(crate) fn scene_form(view: &View<'_>, v: &Value) -> Result<Value, RpcError> {
    map_entity_refs(v, &mut |r| match r {
        Value::Entity(e) if *e == Entity::NONE => Ok(Value::EntityGuid(None)),
        Value::Entity(e) => view
            .guid_of(*e)
            .map(|g| Value::EntityGuid(Some(g.to_string())))
            .ok_or_else(|| format!("entity {} has no GUID in the scene", handle_text(*e))),
        other => Ok(other.clone()),
    })
    .map_err(invalid_value)
}

/// The GUID of a target (edit mode edits address entities by GUID).
fn edit_guid<G: Game>(t: &ErpTarget<'_, G>, target: &Target) -> Result<Guid, RpcError> {
    match target {
        Target::Guid(g) => Ok(g.clone()),
        Target::Entity(e) => t
            .doc
            .view()
            .guid_of(*e)
            .cloned()
            .ok_or_else(|| RpcError::new(NOT_FOUND, "unknown_entity", format!("entity {} has no GUID in the scene", handle_text(*e)))),
    }
}

fn edited<G: Game>(t: &ErpTarget<'_, G>, changed: bool) -> J {
    json!({"changed": changed, "checksum": checksum_text(live_checksum(t))})
}

fn history_json(doc: &EditorDoc) -> J {
    json!({
        "entries": doc.history().into_iter().map(|e| json!({
            "id": e.id, "label": e.label, "origin": e.origin.to_string(), "op_count": e.op_count, "undone": e.undone,
        })).collect::<Vec<_>>(),
        "can_undo": doc.can_undo(),
        "can_redo": doc.can_redo(),
        "dirty": doc.is_dirty(),
        "in_tx": doc.in_tx(),
    })
}

// ---- registry ----

fn registry_schema<G: Game>(t: &ErpTarget<'_, G>, p: &P<'_>) -> Result<J, RpcError> {
    let v = view(t);
    let text = match p.opt_str("type")? {
        None => v.json_schema(),
        Some(name) => v
            .type_schema(name)
            .ok_or_else(|| RpcError::new(NOT_FOUND, "unknown_type", format!("unknown type '{name}' (see registry.types)")))?,
    };
    let schema: J = serde_json::from_str(&text).map_err(|e| RpcError::new(INTERNAL_ERROR, "schema", format!("schema is not JSON: {e}")))?;
    Ok(json!({"schema": schema, "value_format": methods::VALUE_FORMAT}))
}

fn registry_types<G: Game>(t: &ErpTarget<'_, G>) -> J {
    let v = view(t);
    let types: Vec<J> = v
        .types()
        .types()
        .map(|ti| {
            json!({
                "name": ti.name(),
                "kind": if ti.kind() == TypeKind::Component { "component" } else { "singleton" },
                "doc": ti.doc(),
            })
        })
        .collect();
    json!({"types": types})
}

// ---- world reads ----

pub(crate) fn entity_json(v: &View<'_>, info: &EntityInfo, values: bool) -> Result<J, RpcError> {
    let handle = handle_text(info.entity);
    let mut o = Map::new();
    o.insert("id".into(), json!(info.guid.as_ref().map_or_else(|| handle.clone(), |g| g.to_string())));
    o.insert("guid".into(), info.guid.as_ref().map_or(J::Null, |g| json!(g.to_string())));
    o.insert("handle".into(), json!(handle));
    o.insert("name".into(), info.name.as_ref().map_or(J::Null, |n| json!(n)));
    o.insert("components".into(), json!(info.components));
    if values {
        let mut vals = Map::new();
        for (name, val) in v.components(&Target::Entity(info.entity))? {
            vals.insert(name, value_to_json(&val));
        }
        o.insert("values".into(), J::Object(vals));
    }
    Ok(J::Object(o))
}

fn world_query<G: Game>(t: &ErpTarget<'_, G>, p: &P<'_>) -> Result<J, RpcError> {
    query_view(&view(t), p, false)
}

/// `world.query` over any view (the scene, the play frame, a proposal
/// preview). `values_default`: whether component values are included when
/// `values` is not given.
pub(crate) fn query_view(v: &View<'_>, p: &P<'_>, values_default: bool) -> Result<J, RpcError> {
    let filter: Vec<String> = match p.raw("components") {
        None | Some(J::Null) => Vec::new(),
        Some(J::Array(items)) => items
            .iter()
            .map(|i| i.as_str().map(str::to_string).ok_or_else(|| RpcError::params("'components' must be a list of strings")))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(RpcError::params("'components' must be a list of strings")),
    };
    for c in &filter {
        component_type(v, c)?;
    }
    let name_filter = p.opt_str("name")?;
    let values = p.opt_bool("values")?.unwrap_or(values_default);
    let limit = p.opt_u64("limit")?.unwrap_or(1000).min(20_000) as usize;
    let offset = p.opt_u64("offset")?.unwrap_or(0) as usize;
    let mut total = 0usize;
    let mut out = Vec::new();
    for info in v.entities() {
        if !filter.iter().all(|c| info.components.iter().any(|x| x == c)) {
            continue;
        }
        if let Some(n) = name_filter {
            if !info.name.as_deref().is_some_and(|x| x.contains(n)) {
                continue;
            }
        }
        total += 1;
        if total > offset && out.len() < limit {
            out.push(entity_json(v, &info, values)?);
        }
    }
    Ok(json!({
        "entities": out,
        "total": total,
        "truncated": total.saturating_sub(offset) > limit,
        "tick": v.tick(),
        "checksum": checksum_text(v.checksum()),
    }))
}

fn world_get<G: Game>(t: &ErpTarget<'_, G>, p: &P<'_>) -> Result<J, RpcError> {
    get_view(&view(t), p)
}

/// `world.get` over any view.
pub(crate) fn get_view(v: &View<'_>, p: &P<'_>) -> Result<J, RpcError> {
    let target = p.target()?;
    match p.opt_str("component")? {
        Some(c) => {
            component_type(v, c)?;
            let value = v.field(&target, c, p.opt_str("path")?.unwrap_or(""))?;
            Ok(json!({"value": value_to_json(&value)}))
        }
        None => {
            if p.opt_str("path")?.is_some_and(|s| !s.is_empty()) {
                return Err(RpcError::params("'path' needs 'component'"));
            }
            let info = v.entity(&target)?;
            let mut vals = Map::new();
            for (name, val) in v.components(&target)? {
                vals.insert(name, value_to_json(&val));
            }
            Ok(json!({"entity": entity_json(v, &info, false)?, "components": vals}))
        }
    }
}

fn singleton_get<G: Game>(t: &ErpTarget<'_, G>, p: &P<'_>) -> Result<J, RpcError> {
    let v = view(t);
    match p.opt_str("name")? {
        Some(n) => {
            singleton_type(&v, n)?;
            Ok(json!({"value": value_to_json(&v.singleton(n, p.opt_str("path")?.unwrap_or(""))?)}))
        }
        None => {
            let mut o = Map::new();
            for (name, val) in v.singletons() {
                o.insert(name, value_to_json(&val));
            }
            Ok(json!({"singletons": o}))
        }
    }
}

// ---- world edits ----

fn world_patch<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>, origin: Origin) -> Result<J, RpcError> {
    let target = p.target()?;
    let component = p.str("component")?;
    let path = p.opt_str("path")?.unwrap_or("");
    let raw = p.req_raw("value")?;
    let ti = component_type(&view(t), component)?;
    let mut value = decode_value(t, ti, path, raw)?;
    if path.is_empty() {
        // A whole-component write may list only some fields: the rest stay as they are.
        value = overlay(&view(t).component(&target, component)?, &value);
    }
    let changed = match t.play.as_mut() {
        Some(pc) => pc.set_field(&target, component, path, value)?,
        None => {
            let guid = edit_guid(t, &target)?;
            t.doc.apply(Op::SetField { guid, component: component.to_string(), path: path.to_string(), value }, origin)?.changed
        }
    };
    Ok(edited(t, changed))
}

/// `patch` laid over `base`: struct fields of `patch` replace the ones of
/// `base` (recursively); any other value replaces `base` whole.
pub(crate) fn overlay(base: &Value, patch: &Value) -> Value {
    match (base, patch) {
        (Value::Struct(b), Value::Struct(p)) => {
            let mut out: Vec<(String, Value)> = b
                .iter()
                .map(|(n, bv)| (n.clone(), p.iter().find(|(pn, _)| pn == n).map_or_else(|| bv.clone(), |(_, pv)| overlay(bv, pv))))
                .collect();
            out.extend(p.iter().filter(|(pn, _)| !b.iter().any(|(n, _)| n == pn)).cloned());
            Value::Struct(out)
        }
        _ => patch.clone(),
    }
}

fn world_insert<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>, origin: Origin) -> Result<J, RpcError> {
    let target = p.target()?;
    let component = p.str("component")?;
    let ti = component_type(&view(t), component)?;
    let value = match p.raw("value") {
        None | Some(J::Null) => None,
        Some(j) => Some(decode_value(t, ti, "", j)?),
    };
    match t.play.as_mut() {
        Some(pc) => pc.add_component(&target, component, value)?,
        None => {
            let guid = edit_guid(t, &target)?;
            t.doc.apply(Op::AddComponent { guid, component: component.to_string(), value }, origin)?;
        }
    }
    Ok(edited(t, true))
}

fn world_remove<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>, origin: Origin) -> Result<J, RpcError> {
    let target = p.target()?;
    let component = p.str("component")?;
    component_type(&view(t), component)?;
    match t.play.as_mut() {
        Some(pc) => pc.remove_component(&target, component)?,
        None => {
            let guid = edit_guid(t, &target)?;
            t.doc.apply(Op::RemoveComponent { guid, component: component.to_string() }, origin)?;
        }
    }
    Ok(edited(t, true))
}

fn world_spawn<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>, origin: Origin) -> Result<J, RpcError> {
    let mut comps: Vec<(String, Value)> = Vec::new();
    match p.raw("components") {
        None | Some(J::Null) => {}
        Some(J::Object(m)) => {
            for (name, j) in m {
                let ti = component_type(&view(t), name)?;
                comps.push((name.clone(), decode_value(t, ti, "", j)?));
            }
        }
        Some(_) => return Err(RpcError::params("'components' must be an object: component name to value")),
    }
    let name = p.opt_str("name")?.map(str::to_string);
    let guid = match p.opt_str("guid")? {
        Some(g) => Some(Guid::parse(g).map_err(RpcError::params)?),
        None => None,
    };
    if let Some(pc) = t.play.as_mut() {
        if name.is_some() || guid.is_some() {
            return Err(RpcError::state("sim_running", "entities spawned during play have no name or GUID"));
        }
        let e = pc.spawn(&comps)?;
        let mut o = json!({"handle": handle_text(e), "changed": true});
        o["checksum"] = json!(checksum_text(pc.session().frame().checksum()));
        return Ok(o);
    }
    let applied = t.doc.apply(Op::SpawnEntity { guid, name, components: comps }, origin)?;
    let mut o = edited(t, applied.changed);
    o["guid"] = applied.guid.map_or(J::Null, |g| json!(g.to_string()));
    Ok(o)
}

fn world_despawn<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>, origin: Origin) -> Result<J, RpcError> {
    let target = p.target()?;
    match t.play.as_mut() {
        Some(pc) => pc.despawn(&target)?,
        None => {
            let guid = edit_guid(t, &target)?;
            t.doc.apply(Op::DespawnEntity { guid }, origin)?;
        }
    }
    Ok(edited(t, true))
}

fn world_rename<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>, origin: Origin) -> Result<J, RpcError> {
    require_edit_mode(t, "world.rename")?;
    let target = p.target()?;
    let guid = edit_guid(t, &target)?;
    let name = match p.req_raw("name")? {
        J::Null => None,
        J::String(s) => Some(s.clone()),
        _ => return Err(RpcError::params("'name' must be a string or null")),
    };
    let changed = t.doc.apply(Op::Rename { guid, name }, origin)?.changed;
    Ok(edited(t, changed))
}

fn singleton_patch<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>, origin: Origin) -> Result<J, RpcError> {
    let name = p.str("name")?;
    let path = p.opt_str("path")?.unwrap_or("");
    let raw = p.req_raw("value")?;
    let ti = singleton_type(&view(t), name)?;
    let value = decode_value(t, ti, path, raw)?;
    let changed = match t.play.as_mut() {
        Some(pc) => pc.set_singleton_field(name, path, value)?,
        None => t.doc.apply(Op::SetSingletonField { singleton: name.to_string(), path: path.to_string(), value }, origin)?.changed,
    };
    Ok(edited(t, changed))
}

// ---- scene ----

/// The `path` parameter of `scene.save` / `scene.load`, if the host allows one.
fn scene_path_param(lim: &HostLimits, p: &P<'_>) -> Result<Option<PathBuf>, RpcError> {
    match p.opt_str("path")? {
        Some(path) if lim.allow_scene_paths => Ok(Some(PathBuf::from(path))),
        // A host that does not let clients pick files ignores the parameter (the configured file is used).
        _ => Ok(None),
    }
}

fn scene_save<G: Game>(t: &mut ErpTarget<'_, G>, lim: &HostLimits, p: &P<'_>, fx: &mut Effects) -> Result<J, RpcError> {
    let write = p.opt_bool("write")?.unwrap_or(false);
    match p.opt_str("source")?.unwrap_or("doc") {
        "doc" => {}
        "play" => {
            if write {
                return Err(RpcError::params("'write' saves the document, not the play frame"));
            }
            let pc = t.play.as_ref().ok_or_else(|| RpcError::state("no_play", "no play session (call sim.start)"))?;
            let text = pc.capture_scene()?.to_yaml();
            return Ok(json!({"text": text, "checksum": checksum_text(pc.session().frame().checksum()), "dirty": t.doc.is_dirty()}));
        }
        other => return Err(RpcError::params(format!("'source' must be doc or play, not '{other}'"))),
    }
    if !write {
        return Ok(json!({"text": t.doc.to_yaml(), "checksum": checksum_text(t.doc.checksum()), "dirty": t.doc.is_dirty()}));
    }
    let named = scene_path_param(lim, p)?;
    let path = named
        .as_ref()
        .or(lim.scene_path.as_ref())
        .ok_or_else(|| RpcError::state("no_scene_path", "the server was started without a scene file, so it cannot write one"))?;
    if t.doc.in_tx() {
        return Err(RpcError::state("tx_open", "a transaction is open; commit or roll it back before saving"));
    }
    let text = t.doc.save_yaml();
    let tmp = path.with_extension("scene.yaml.tmp");
    std::fs::write(&tmp, &text)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| RpcError::new(INTERNAL_ERROR, "io", format!("cannot write {}: {e}", path.display())))?;
    if named.is_some() {
        fx.scene_path.clone_from(&named);
    }
    Ok(json!({
        "text": text,
        "checksum": checksum_text(t.doc.checksum()),
        "dirty": t.doc.is_dirty(),
        "written": path.display().to_string(),
    }))
}

// ---- sim ----

/// The `sim.state` result.
pub(crate) fn state_json<G: Game>(t: &ErpTarget<'_, G>, lim: &HostLimits) -> J {
    let base = |mode: &str| {
        json!({
            "mode": mode,
            "dirty": t.doc.is_dirty(),
            "in_tx": t.doc.in_tx(),
            "doc_checksum": checksum_text(t.doc.checksum()),
        })
    };
    let mut o = match t.play.as_ref() {
        Some(pc) => {
            let s = pc.session();
            let mut o = base("play");
            let extra = json!({
                "playing": s.is_playing(),
                "session_mode": match s.mode() { orr_session::PlayMode::Record => "record", orr_session::PlayMode::Viewer => "viewer" },
                "head_tick": s.head_tick(),
                "first_tick": s.first_tick(),
                "last_tick": s.last_tick(),
                "checksum": checksum_text(s.frame().checksum()),
                "speed_permille": s.speed().permille(),
                "branches": s.branch_count(),
                "epoch": s.epoch(),
                "tick_rate": s.tick_rate(),
                "player_count": s.player_count(),
            });
            merge(&mut o, extra);
            o
        }
        None => {
            let mut o = base("edit");
            merge(
                &mut o,
                json!({
                    "playing": false,
                    "head_tick": 0,
                    "first_tick": 0,
                    "last_tick": 0,
                    "checksum": checksum_text(t.doc.checksum()),
                    "speed_permille": 1000,
                    "branches": 0,
                    "epoch": 0,
                    "tick_rate": lim.tick_rate,
                    "player_count": lim.player_count,
                }),
            );
            o
        }
    };
    if let J::Object(m) = &mut o {
        m.insert("entities".into(), json!(view(t).frame().alive_count()));
        m.insert("scene_path".into(), lim.scene_path.as_ref().map_or(J::Null, |p| json!(p.display().to_string())));
    }
    o
}

fn merge(into: &mut J, from: J) {
    if let (J::Object(a), J::Object(b)) = (into, from) {
        a.extend(b);
    }
}

fn sim_checksum<G: Game>(t: &ErpTarget<'_, G>, p: &P<'_>) -> Result<J, RpcError> {
    let tick = p.opt_u64("tick")?;
    match t.play.as_ref() {
        Some(pc) => {
            let s = pc.session();
            match tick {
                Some(k) if k != s.head_tick() => match s.checksum_at(k) {
                    Some(c) => Ok(json!({"tick": k, "checksum": checksum_text(c)})),
                    None => Err(RpcError::params(format!(
                        "no recorded checksum for tick {k} (recorded range {}..={})",
                        s.first_tick(),
                        s.last_tick()
                    ))),
                },
                _ => Ok(json!({"tick": s.head_tick(), "checksum": checksum_text(s.frame().checksum())})),
            }
        }
        None => match tick {
            None | Some(0) => Ok(json!({"tick": 0, "checksum": checksum_text(t.doc.checksum())})),
            Some(k) => Err(RpcError::params(format!("no play session: only tick 0 (the scene) has a checksum, not {k}"))),
        },
    }
}

fn sim_start<G: Game>(
    t: &mut ErpTarget<'_, G>, lim: &HostLimits, input: Option<&crate::input::StructuredInput<G>>, p: &P<'_>,
) -> Result<J, RpcError> {
    if t.play.is_some() {
        return Err(RpcError::state("sim_running", "a play session is already running (sim.stop first)"));
    }
    let players = p.opt_u64("player_count")?.unwrap_or(u64::from(lim.player_count));
    let rate = p.opt_u64("tick_rate")?.unwrap_or(u64::from(lim.tick_rate));
    if !(1..=16).contains(&players) {
        return Err(RpcError::params("'player_count' must be 1..=16"));
    }
    if let Some(input) = input {
        if players > u64::from(input.max_players) {
            return Err(RpcError::params(format!("this input adapter supports at most {} players", input.max_players)));
        }
    }
    if !(1..=1000).contains(&rate) {
        return Err(RpcError::params("'tick_rate' must be 1..=1000"));
    }
    let mut cfg = t.doc.play_config(players as u8, rate as u32);
    if input.is_some() {
        cfg.game_id = lim.game.name.clone();
        cfg.build_id = lim.build_id;
    }
    let mut pc = PlayController::<G>::start_play(t.doc, cfg)?;
    if let Some(input) = input {
        let commands = input.commands.clone();
        pc.session_mut().set_commands_from_input(move |slot, held| commands(slot, held));
    }
    if p.opt_bool("run")?.unwrap_or(false) {
        pc.control(ControlOp::Play);
    }
    *t.play = Some(pc);
    Ok(state_json(t, lim))
}

fn sim_stop<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>, fx: &mut Effects) -> Result<J, RpcError> {
    let include = p.opt_bool("include_replay")?.unwrap_or(false);
    let pc = t.play.take().ok_or_else(|| RpcError::state("no_play", "no play session (call sim.start)"))?;
    let stopped = pc.stop_play();
    let mut o = json!({
        "tick": stopped.tick,
        "checksum": checksum_text(stopped.checksum),
        "replay_bytes": stopped.replay.len(),
    });
    if include {
        o["replay"] = json!(b64_encode(&stopped.replay));
    }
    fx.stopped = Some(stopped);
    Ok(o)
}

fn control<G: Game>(t: &mut ErpTarget<'_, G>, lim: &HostLimits, op: ControlOp) -> Result<J, RpcError> {
    play_mut(t)?.control(op);
    Ok(state_json(t, lim))
}

fn sim_step<G: Game>(t: &mut ErpTarget<'_, G>, lim: &HostLimits, p: &P<'_>, fx: &mut Effects) -> Result<J, RpcError> {
    let n = p.opt_u64("n")?.unwrap_or(1);
    if n == 0 {
        return Err(RpcError::params("'n' must be at least 1"));
    }
    if n > u64::from(lim.max_step_per_call) {
        return Err(RpcError::new(
            LIMIT_EXCEEDED,
            "step_limit",
            format!("'n' is {n}; this server runs at most {} ticks per call (call again)", lim.max_step_per_call),
        )
        .with("max", json!(lim.max_step_per_call)));
    }
    let events = play_mut(t)?.control(ControlOp::Step(n as u32));
    for e in events {
        fx.events.push((e.key, bytemuck::bytes_of(&e.payload).to_vec()));
    }
    Ok(state_json(t, lim))
}

fn sim_seek<G: Game>(t: &mut ErpTarget<'_, G>, lim: &HostLimits, p: &P<'_>) -> Result<J, RpcError> {
    let tick = p.req_u64("tick")?;
    let pc = play_mut(t)?;
    let (first, last) = (pc.session().first_tick(), pc.session().last_tick());
    if tick < first || tick > last {
        return Err(RpcError::params(format!("tick {tick} is outside the recorded range {first}..={last}")));
    }
    pc.control(ControlOp::Seek(tick));
    if pc.session().head_tick() != tick {
        return Err(RpcError::state("seek_failed", format!("could not seek to tick {tick} (the recording has a gap)")));
    }
    Ok(state_json(t, lim))
}

pub(crate) fn slot_of<G: Game>(t: &ErpTarget<'_, G>, p: &P<'_>, required: bool) -> Result<PlayerSlot, RpcError> {
    let n = if required { p.req_u64("player")? } else { p.opt_u64("player")?.unwrap_or(0) };
    let players = t.play.as_ref().map_or(0, |pc| pc.session().player_count());
    if n >= u64::from(players) {
        return Err(RpcError::params(format!("player {n} does not exist (the session has {players} players)")));
    }
    Ok(PlayerSlot(n as u8))
}

fn input_unavailable() -> RpcError {
    RpcError::state("input_unavailable", "this host has no structured input adapter (raw sim.input is unchanged)")
}

fn sim_input_value<G: Game>(
    t: &mut ErpTarget<'_, G>, input: Option<&crate::input::StructuredInput<G>>, p: &P<'_>,
) -> Result<J, RpcError> {
    reject_unhandled_grant(p)?;
    let adapter = input.ok_or_else(input_unavailable)?;
    require_writable_play(t)?;
    let slot = slot_of(t, p, true)?;
    let value = p.raw("value").ok_or_else(|| RpcError::params("missing parameter 'value' (the complete reflected input object)"))?;
    let value = adapter.decode(value)?;
    play_mut(t)?.session_mut().set_input(slot, value);
    Ok(json!({"ok": true}))
}

fn reject_unhandled_grant(p: &P<'_>) -> Result<(), RpcError> {
    if ["grant", "generation", "sequence"].iter().any(|key| p.raw(key).is_some()) {
        return Err(RpcError::state("input_stale", "managed input requires an active server connection and grant"));
    }
    Ok(())
}

fn sim_input<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>) -> Result<J, RpcError> {
    reject_unhandled_grant(p)?;
    require_writable_play(t)?;
    let slot = slot_of(t, p, true)?;
    let bytes = hex_decode(p.str("input")?).ok_or_else(|| RpcError::params("'input' is not valid hex"))?;
    let input = bytemuck::try_pod_read_unaligned::<G::Input>(&bytes)
        .map_err(|_| RpcError::params(format!("'input' must be {} bytes", core::mem::size_of::<G::Input>())))?;
    play_mut(t)?.session_mut().set_input_without_commands(slot, input);
    Ok(json!({"ok": true}))
}

fn sim_command<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>) -> Result<J, RpcError> {
    require_writable_play(t)?;
    let slot = slot_of(t, p, false)?;
    let bytes = hex_decode(p.str("command")?).ok_or_else(|| RpcError::params("'command' is not valid hex"))?;
    let cmd = <G::Command as SimCommand>::decode(&bytes).ok_or_else(|| RpcError::params("'command' does not decode as this game's Command"))?;
    play_mut(t)?.session_mut().push_command(slot, cmd).map_err(|e| RpcError::state("read_only", e.to_string()))?;
    Ok(json!({"ok": true}))
}

pub(crate) fn require_writable_play<G: Game>(t: &mut ErpTarget<'_, G>) -> Result<(), RpcError> {
    if play_mut(t)?.session().mode() == PlayMode::Viewer {
        return Err(RpcError::state("read_only", "replay viewer is read-only; branch before sending inputs or commands"));
    }
    Ok(())
}
