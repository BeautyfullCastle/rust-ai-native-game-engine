//! Proposal and verification methods (`proposal.*`, `verify.self`), the
//! game hooks they need, and the `watch.proposals` bookkeeping.
//!
//! A proposal is a set of edits staged on a private copy of the document
//! ([`orr_edit::EditorDoc::propose`]); this module turns the ERP JSON of the
//! edits into [`Op`]s and the results (diff, summary, verification report)
//! back into JSON. Everything runs on the host thread like the other methods.

use std::collections::BTreeMap;
use std::sync::Arc;

use orr_ecs::Frame;
use orr_edit::{
    Check, CheckOutcome, EditorDoc, MetricValue, Metrics, Op, Origin, ProposalId, ProposalSummary, ReflectMetrics, Target, VerifyInputs,
    VerifyOptions, VerifyReport, View,
};
use orr_reflect::{Guid, Value};
use orr_sim::{Game, PlayerSlot};
use serde_json::{json, Map, Value as J};

use crate::activity::VerifyDetail;
use crate::codec::b64_decode;
use crate::dispatch::{
    component_type, get_view, overlay, query_view, scene_form, singleton_type, CallCtx, Effects, ErpTarget, HostLimits, P,
};
use crate::error::*;
use crate::json::{desc_at_path, json_to_value, value_to_json};
use crate::methods;
use crate::wire::checksum_text;

/// Bytes of one player's input at `(seed, tick, slot)`, see [`GameHooks::with_bot`].
pub type BotFn = dyn Fn(u64, u64, u8) -> Vec<u8> + Send + Sync;

/// The game-specific parts of verification, so this crate stays game-agnostic:
/// the game's metrics (besides the generic `ReflectMetrics`) and a scripted
/// player for `{"kind":"bot"}` inputs.
#[derive(Clone, Default)]
pub struct GameHooks {
    /// Name of the game, shown in `rpc.discover`.
    pub name: String,
    /// The game's metrics.
    pub metrics: Option<Arc<dyn Metrics + Send + Sync>>,
    /// The scripted player.
    pub bot: Option<Arc<BotFn>>,
}

impl GameHooks {
    /// Hooks for a game called `name`, with no metrics and no bot.
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), metrics: None, bot: None }
    }

    /// Sets the game's metrics.
    pub fn with_metrics(mut self, metrics: impl Metrics + Send + 'static) -> Self {
        self.metrics = Some(Arc::new(metrics));
        self
    }

    /// Sets the scripted player: `(seed, tick, slot) -> Input` of the game
    /// (a deterministic pure function).
    pub fn with_bot<I: bytemuck::Pod>(mut self, f: impl Fn(u64, u64, u8) -> I + Send + Sync + 'static) -> Self {
        self.bot = Some(Arc::new(move |seed, tick, slot| bytemuck::bytes_of(&f(seed, tick, slot)).to_vec()));
        self
    }
}

impl core::fmt::Debug for GameHooks {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GameHooks").field("name", &self.name).field("metrics", &self.metrics.is_some()).field("bot", &self.bot.is_some()).finish()
    }
}

/// A stable build id for a host binary of this version running `game`
/// (FNV-1a 64 of `orr_remote_host/<version>/<game>`), for hosts that have no
/// build system id of their own.
pub fn default_build_id(game: &str) -> u64 {
    let text = format!("orr_remote_host/{}/{game}", env!("CARGO_PKG_VERSION"));
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// `ReflectMetrics` plus the game's metrics.
struct Combined<'a> {
    reflect: ReflectMetrics<'a>,
    game: Option<&'a (dyn Metrics + Send + Sync)>,
}

impl<'a> Combined<'a> {
    fn new(doc: &'a EditorDoc, lim: &'a HostLimits) -> Self {
        Self { reflect: ReflectMetrics::new(doc.types()), game: lim.game.metrics.as_deref() }
    }
}

impl Metrics for Combined<'_> {
    fn sample(&self, frame: &Frame) -> Vec<(String, MetricValue)> {
        let mut v = self.reflect.sample(frame);
        if let Some(g) = self.game {
            v.extend(g.sample(frame));
        }
        v
    }
}

fn kind_of(v: MetricValue) -> &'static str {
    match v {
        MetricValue::Int(_) => "int",
        MetricValue::Fixed(_) => "fixed",
    }
}

fn metric_json(v: MetricValue) -> J {
    match v {
        MetricValue::Int(i) => json!(i),
        MetricValue::Fixed(f) => value_to_json(&Value::Fixed(f)),
    }
}

/// `rpc.discover` plus the `engine` block (version, build id, metrics, verify limits).
pub(crate) fn discover_with_engine<G: Game>(t: &ErpTarget<'_, G>, lim: &HostLimits, ctx: &CallCtx<'_>) -> J {
    let mut d = methods::discover(ctx.client, ctx.caps);
    let metrics = Combined::new(t.doc, lim).sample(t.doc.frame());
    let names: Vec<J> = metrics.into_iter().map(|(n, v)| json!({"name": n, "kind": kind_of(v)})).collect();
    d["engine"] = json!({
        "name": "orrery",
        "version": env!("CARGO_PKG_VERSION"),
        "erp_version": 1,
        "build_id": checksum_text(lim.build_id),
        "game": lim.game.name,
        "metrics": names,
        "verify": {
            "input_kinds": ["last_play", "replay", "bot", "idle"],
            "bot_available": lim.game.bot.is_some(),
            "max_ticks": lim.max_verify_ticks,
            "default_players": lim.player_count,
            "tick_rate": lim.tick_rate,
            "check_grammar": "<metric>[.start|final|min|max|delta] <|<=|==|!=|>=|> <number> | base:<metric>... | no_divergence | no_divergence_before <tick> | recording_matches",
        },
    });
    d
}

// ---- ids, ops ----

pub(crate) fn proposal_id(p: &P<'_>) -> Result<ProposalId, RpcError> {
    let bad = || RpcError::params("'id' must be a proposal id like `p1` (from proposal.begin)");
    match p.raw("id") {
        Some(J::String(s)) => s.strip_prefix('p').unwrap_or(s).parse::<u64>().map(ProposalId).map_err(|_| bad()),
        Some(J::Number(n)) => n.as_u64().map(ProposalId).ok_or_else(bad),
        Some(_) => Err(bad()),
        None => Err(RpcError::params("missing parameter 'id' (proposal id like `p1`)")),
    }
}

fn guid_of_target(v: &View<'_>, target: &Target) -> Result<Guid, RpcError> {
    match target {
        Target::Guid(g) => Ok(g.clone()),
        Target::Entity(e) => v
            .guid_of(*e)
            .cloned()
            .ok_or_else(|| RpcError::new(NOT_FOUND, "unknown_entity", format!("entity {} has no GUID in the scene", crate::json::handle_text(*e)))),
    }
}

fn decode(v: &View<'_>, ti: &orr_reflect::TypeInfo, path: &str, j: &J) -> Result<Value, RpcError> {
    let wrap = |e: String| {
        RpcError::new(INVALID_VALUE, "invalid_value", if path.is_empty() { format!("{}: {e}", ti.name()) } else { format!("{}.{path}: {e}", ti.name()) })
    };
    let desc = desc_at_path(ti.desc(), path).map_err(wrap)?;
    let value = json_to_value(&desc, j, path.is_empty()).map_err(wrap)?;
    scene_form(v, &value)
}

/// One op of `proposal.apply`, checked against the proposal's staged view.
fn parse_op(v: &View<'_>, j: &J) -> Result<Op, RpcError> {
    let obj = j.as_object().ok_or_else(|| RpcError::params("each op must be an object like {\"op\":\"patch\", ...}"))?;
    let p = P(obj);
    let kind = p.str("op")?;
    let kind = kind.strip_prefix("world.").unwrap_or(kind);
    match kind {
        "patch" => {
            let guid = guid_of_target(v, &p.target()?)?;
            let component = p.str("component")?.to_string();
            let path = p.opt_str("path")?.unwrap_or("").to_string();
            let ti = component_type(v, &component)?;
            let mut value = decode(v, ti, &path, p.req_raw("value")?)?;
            if path.is_empty() {
                // A whole-component write may list only some fields: the rest stay as they are.
                if let Ok(cur) = v.component(&Target::Guid(guid.clone()), &component) {
                    value = overlay(&cur, &value);
                }
            }
            Ok(Op::SetField { guid, component, path, value })
        }
        "insert" => {
            let guid = guid_of_target(v, &p.target()?)?;
            let component = p.str("component")?.to_string();
            let ti = component_type(v, &component)?;
            let value = match p.raw("value") {
                None | Some(J::Null) => None,
                Some(j) => Some(decode(v, ti, "", j)?),
            };
            Ok(Op::AddComponent { guid, component, value })
        }
        "remove" => {
            let guid = guid_of_target(v, &p.target()?)?;
            let component = p.str("component")?.to_string();
            component_type(v, &component)?;
            Ok(Op::RemoveComponent { guid, component })
        }
        "spawn" => {
            let mut components: Vec<(String, Value)> = Vec::new();
            match p.raw("components") {
                None | Some(J::Null) => {}
                Some(J::Object(m)) => {
                    for (name, j) in m {
                        let ti = component_type(v, name)?;
                        components.push((name.clone(), decode(v, ti, "", j)?));
                    }
                }
                Some(_) => return Err(RpcError::params("'components' must be an object: component name to value")),
            }
            let guid = match p.opt_str("guid")? {
                Some(g) => Some(Guid::parse(g).map_err(RpcError::params)?),
                None => None,
            };
            Ok(Op::SpawnEntity { guid, name: p.opt_str("name")?.map(str::to_string), components })
        }
        "despawn" => Ok(Op::DespawnEntity { guid: guid_of_target(v, &p.target()?)? }),
        "rename" => {
            let guid = guid_of_target(v, &p.target()?)?;
            let name = match p.req_raw("name")? {
                J::Null => None,
                J::String(s) => Some(s.clone()),
                _ => return Err(RpcError::params("'name' must be a string or null")),
            };
            Ok(Op::Rename { guid, name })
        }
        "singleton.patch" => {
            let name = p.str("name")?.to_string();
            let path = p.opt_str("path")?.unwrap_or("").to_string();
            let ti = singleton_type(v, &name)?;
            let value = decode(v, ti, &path, p.req_raw("value")?)?;
            Ok(Op::SetSingletonField { singleton: name, path, value })
        }
        other => Err(RpcError::params(format!(
            "unknown op '{other}' (patch, insert, remove, spawn, despawn, rename, singleton.patch)"
        ))),
    }
}

fn op_json(op: &Op) -> J {
    let mut o = Map::new();
    let mut put = |k: &str, v: J| {
        o.insert(k.to_string(), v);
    };
    match op {
        Op::SetField { guid, component, path, value } => {
            put("op", json!("patch"));
            put("entity", json!(guid.to_string()));
            put("component", json!(component));
            if !path.is_empty() {
                put("path", json!(path));
            }
            put("value", value_to_json(value));
        }
        Op::AddComponent { guid, component, value } => {
            put("op", json!("insert"));
            put("entity", json!(guid.to_string()));
            put("component", json!(component));
            if let Some(v) = value {
                put("value", value_to_json(v));
            }
        }
        Op::RemoveComponent { guid, component } => {
            put("op", json!("remove"));
            put("entity", json!(guid.to_string()));
            put("component", json!(component));
        }
        Op::SpawnEntity { guid, name, components } => {
            put("op", json!("spawn"));
            if let Some(g) = guid {
                put("guid", json!(g.to_string()));
            }
            if let Some(n) = name {
                put("name", json!(n));
            }
            put("components", J::Object(components.iter().map(|(n, v)| (n.clone(), value_to_json(v))).collect()));
        }
        Op::DespawnEntity { guid } => {
            put("op", json!("despawn"));
            put("entity", json!(guid.to_string()));
        }
        Op::Rename { guid, name } => {
            put("op", json!("rename"));
            put("entity", json!(guid.to_string()));
            put("name", json!(name));
        }
        Op::SetSingletonField { singleton, path, value } => {
            put("op", json!("singleton.patch"));
            put("name", json!(singleton));
            if !path.is_empty() {
                put("path", json!(path));
            }
            put("value", value_to_json(value));
        }
        Op::RemoveSingleton { singleton } => {
            put("op", json!("singleton.remove"));
            put("name", json!(singleton));
        }
    }
    J::Object(o)
}

fn summary_json(s: &ProposalSummary) -> J {
    let entity = |e: &orr_edit::EntityRef| json!({"guid": e.guid.to_string(), "name": e.name});
    json!({
        "entities_added": s.entities_added.iter().map(entity).collect::<Vec<_>>(),
        "entities_removed": s.entities_removed.iter().map(entity).collect::<Vec<_>>(),
        "entities_renamed": s.entities_renamed.iter().map(|r| json!({"guid": r.guid.to_string(), "old": r.old, "new": r.new})).collect::<Vec<_>>(),
        "components_added": s.components_added.iter().map(|(g, c)| json!({"entity": g.to_string(), "component": c})).collect::<Vec<_>>(),
        "components_removed": s.components_removed.iter().map(|(g, c)| json!({"entity": g.to_string(), "component": c})).collect::<Vec<_>>(),
        "fields_changed": s.fields_changed.iter().map(|c| json!({
            "entity": c.entity.as_ref().map(|g| g.to_string()),
            "component": c.component,
            "path": c.path,
            "old": value_to_json(&c.old),
            "new": value_to_json(&c.new),
            "text": c.describe(),
        })).collect::<Vec<_>>(),
        "singletons_added": s.singletons_added,
        "singletons_removed": s.singletons_removed,
        "lines": s.lines(),
    })
}

// ---- methods ----

pub(crate) fn begin<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>, origin: Origin) -> Result<J, RpcError> {
    let label = p.opt_str("label")?.unwrap_or("agent proposal").to_string();
    let id = t.doc.propose(&label, origin.clone())?;
    Ok(json!({"id": id.to_string(), "label": label, "origin": origin.to_string()}))
}

/// Most ops one `proposal.apply` call may carry.
const MAX_OPS_PER_CALL: usize = 5000;

pub(crate) fn apply<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>) -> Result<J, RpcError> {
    let id = proposal_id(p)?;
    let list = match p.raw("ops") {
        Some(J::Array(a)) => a,
        _ => return Err(RpcError::params("'ops' must be a list of ops: [{\"op\":\"patch\", ...}, ...]")),
    };
    if list.len() > MAX_OPS_PER_CALL {
        return Err(RpcError::new(LIMIT_EXCEEDED, "too_many_ops", format!("{} ops in one call; at most {MAX_OPS_PER_CALL}", list.len())));
    }
    let mut ops = Vec::with_capacity(list.len());
    {
        let v = t.doc.proposal_preview(id)?;
        for (i, j) in list.iter().enumerate() {
            let op = parse_op(&v, j).map_err(|mut e| {
                e.message = format!("ops[{i}]: {}", e.message);
                e
            })?;
            ops.push(op);
        }
    }
    let n = ops.len();
    let applied = t.doc.proposal_apply_all(id, ops)?;
    let spawned: Vec<J> = applied
        .iter()
        .enumerate()
        .filter_map(|(i, a)| a.guid.as_ref().map(|g| json!({"index": i, "guid": g.to_string()})))
        .collect();
    Ok(json!({
        "id": id.to_string(),
        "applied": n,
        "changed": applied.iter().filter(|a| a.changed).count(),
        "op_count": t.doc.proposal_info(id)?.op_count,
        "spawned": spawned,
        "checksum": checksum_text(t.doc.proposal_preview(id)?.checksum()),
    }))
}

fn info_json(i: &orr_edit::ProposalInfo) -> J {
    json!({"id": i.id.to_string(), "label": i.label, "origin": i.origin.to_string(), "op_count": i.op_count, "stale": i.stale})
}

pub(crate) fn list<G: Game>(t: &ErpTarget<'_, G>) -> J {
    json!({"proposals": t.doc.list_proposals().iter().map(info_json).collect::<Vec<_>>()})
}

pub(crate) fn get<G: Game>(t: &ErpTarget<'_, G>, p: &P<'_>) -> Result<J, RpcError> {
    let id = proposal_id(p)?;
    let info = t.doc.proposal_info(id)?;
    let diff = t.doc.proposal_diff(id)?;
    let mut o = match info_json(&info) {
        J::Object(m) => m,
        _ => Map::new(),
    };
    o.insert("ops".into(), J::Array(t.doc.proposal_ops(id)?.iter().map(op_json).collect()));
    o.insert("diff".into(), json!(diff.text));
    o.insert("summary".into(), summary_json(&diff.summary));
    match t.doc.proposal_check(id) {
        Ok(()) => {
            o.insert("accepts_cleanly".into(), json!(true));
        }
        Err(e) => {
            o.insert("accepts_cleanly".into(), json!(false));
            o.insert("accept_error".into(), json!(e.to_string()));
        }
    }
    Ok(J::Object(o))
}

pub(crate) fn preview<G: Game>(t: &ErpTarget<'_, G>, p: &P<'_>) -> Result<J, RpcError> {
    let id = proposal_id(p)?;
    let v = t.doc.proposal_preview(id)?;
    let mut out = if p.raw("entity").is_some_and(|e| !e.is_null()) { get_view(&v, p)? } else { query_view(&v, p, true)? };
    if let J::Object(m) = &mut out {
        m.insert("proposal".into(), json!(id.to_string()));
    }
    Ok(out)
}

pub(crate) fn accept<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>) -> Result<J, RpcError> {
    let id = proposal_id(p)?;
    let a = t.doc.accept(id)?;
    Ok(json!({"history_id": a.history_id, "applied": a.applied.len(), "checksum": checksum_text(t.doc.checksum())}))
}

pub(crate) fn reject<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>) -> Result<J, RpcError> {
    let id = proposal_id(p)?;
    t.doc.reject(id)?;
    Ok(json!({"ok": true}))
}

// ---- verification ----

fn build_inputs<G: Game>(lim: &HostLimits, ctx: &CallCtx<'_>, spec: &J, top_ticks: Option<u64>) -> Result<(VerifyInputs<'static, G>, J), RpcError> {
    let obj = spec
        .as_object()
        .ok_or_else(|| RpcError::params("'inputs' must be an object like {\"kind\":\"bot\",\"ticks\":300} (kinds: last_play, replay, bot, idle)"))?;
    let q = P(obj);
    let kind = q.str("kind")?;
    let limit = u64::from(lim.max_verify_ticks);
    let too_long = |n: u64| {
        RpcError::new(LIMIT_EXCEEDED, "verify_limit", format!("{n} ticks requested; this server verifies at most {limit} ticks per call")).with("max", json!(limit))
    };
    let scripted_ticks = || -> Result<u32, RpcError> {
        let n = q.req_u64("ticks")?;
        if n == 0 {
            return Err(RpcError::params("'inputs.ticks' must be at least 1"));
        }
        if n > limit {
            return Err(too_long(n));
        }
        Ok(n as u32)
    };
    let players = |q: &P<'_>| -> Result<u8, RpcError> {
        let n = q.opt_u64("players")?.unwrap_or(u64::from(lim.player_count));
        if !(1..=16).contains(&n) {
            return Err(RpcError::params("'inputs.players' must be 1..=16"));
        }
        Ok(n as u8)
    };
    match kind {
        "last_play" | "replay" => {
            let bytes: Vec<u8> = if kind == "last_play" {
                ctx.last_play
                    .map(|s| s.replay.clone())
                    .ok_or_else(|| RpcError::state("no_last_play", "no play session has been stopped in this host yet (sim.start, sim.step, sim.stop first)"))?
            } else {
                b64_decode(q.str("base64")?).ok_or_else(|| RpcError::params("'inputs.base64' is not valid base64"))?
            };
            let inputs = VerifyInputs::<G>::from_replay(&bytes)?;
            if let Some(n) = top_ticks {
                if n > limit {
                    return Err(too_long(n));
                }
            }
            Ok((inputs, json!({"kind": kind, "replay_bytes": bytes.len()})))
        }
        "idle" => {
            let ticks = scripted_ticks()?;
            let players = players(&q)?;
            let inputs = VerifyInputs::<G>::scripted(ticks, players, |_, _| bytemuck::Zeroable::zeroed());
            Ok((inputs, json!({"kind": "idle", "ticks": ticks, "players": players})))
        }
        "bot" => {
            let bot = lim.game.bot.clone().ok_or_else(|| RpcError::state("no_bot", "this host has no scripted player; use `idle`, `last_play` or `replay` inputs"))?;
            let ticks = scripted_ticks()?;
            let players = players(&q)?;
            let seed = q.opt_u64("seed")?.unwrap_or(1);
            let inputs = VerifyInputs::<G>::scripted(ticks, players, move |tick, slot: PlayerSlot| {
                let bytes = bot(seed, tick, slot.0);
                bytemuck::try_pod_read_unaligned::<G::Input>(&bytes).unwrap_or_else(|_| bytemuck::Zeroable::zeroed())
            });
            Ok((inputs, json!({"kind": "bot", "ticks": ticks, "players": players, "seed": seed})))
        }
        other => Err(RpcError::params(format!("unknown inputs kind '{other}' (last_play, replay, bot, idle)"))),
    }
}

fn stats_json(s: &orr_edit::MetricStats, series: bool) -> J {
    let mut o = json!({"start": metric_json(s.start), "end": metric_json(s.end), "min": metric_json(s.min), "max": metric_json(s.max)});
    if series {
        o["series"] = J::Array(s.series.iter().map(|(t, v)| json!([t, metric_json(*v)])).collect());
    }
    o
}

pub(crate) fn report_json(r: &VerifyReport, outcome: Option<&CheckOutcome>, series: bool) -> J {
    let metrics: Vec<J> = r
        .metrics
        .iter()
        .map(|m| {
            json!({
                "name": m.name,
                "kind": kind_of(m.base.end),
                "base": stats_json(&m.base, series),
                "candidate": stats_json(&m.candidate, series),
                "delta": metric_json(m.delta),
            })
        })
        .collect();
    let mut o = json!({
        "start_tick": r.start_tick,
        "end_tick": r.end_tick,
        "ticks": r.ticks,
        "identical": r.identical(),
        "first_divergence": r.first_divergence,
        "first_metric_difference": r.first_metric_difference,
        "checksums": {
            "base_start": checksum_text(r.base_start_checksum),
            "candidate_start": checksum_text(r.candidate_start_checksum),
            "base_final": checksum_text(r.base_final_checksum),
            "candidate_final": checksum_text(r.candidate_final_checksum),
        },
        "samples": r.samples.iter().map(|s| json!({"tick": s.tick, "base": checksum_text(s.base), "candidate": checksum_text(s.candidate)})).collect::<Vec<_>>(),
        "metrics": metrics,
        "debug_commands_replayed": r.debug_commands_replayed,
        "recording": r.recording.map(|c| json!({"checked": c.checked, "mismatches": c.mismatches, "first_mismatch": c.first_mismatch})),
        "lines": r.lines(),
    });
    o["passed"] = outcome.map_or(J::Null, |c| json!(c.passed));
    o["checks"] = outcome.map_or(J::Null, |c| {
        json!({"passed": c.passed, "results": c.results.iter().map(|x| json!({"check": x.check, "passed": x.passed, "reason": x.reason})).collect::<Vec<_>>()})
    });
    o
}

pub(crate) fn verify<G: Game>(t: &ErpTarget<'_, G>, lim: &HostLimits, ctx: &CallCtx<'_>, fx: &mut Effects, p: &P<'_>, with_proposal: bool) -> Result<J, RpcError> {
    let id = if with_proposal { Some(proposal_id(p)?) } else { None };
    if let Some(id) = id {
        t.doc.proposal_info(id)?;
    }
    let top_ticks = p.opt_u64("ticks")?;
    let (inputs, described) = build_inputs::<G>(lim, ctx, p.req_raw("inputs")?, top_ticks)?;
    let checks = match p.raw("checks") {
        None | Some(J::Null) => Vec::new(),
        Some(J::Array(a)) => {
            let texts: Vec<&str> = a.iter().map(|c| c.as_str().ok_or_else(|| RpcError::params("'checks' must be a list of strings"))).collect::<Result<_, _>>()?;
            Check::parse_all(&texts)?
        }
        Some(_) => return Err(RpcError::params("'checks' must be a list of strings")),
    };
    let series = p.opt_bool("series")?.unwrap_or(false);
    let mut opts = VerifyOptions { tick_rate: lim.tick_rate, build_id: lim.build_id, max_ticks: Some(lim.max_verify_ticks), ..VerifyOptions::default() };
    if let Some(n) = top_ticks {
        opts.max_ticks = Some(n.clamp(1, u64::from(lim.max_verify_ticks)) as u32);
    }
    if let Some(n) = p.opt_u64("sample_every")? {
        opts.sample_every = u32::try_from(n).map_err(|_| RpcError::params("'sample_every' is too large"))?;
    }
    let metrics = Combined::new(t.doc, lim);
    let report = match id {
        Some(id) => t.doc.verify_proposal::<G>(id, &inputs, &metrics, &opts)?,
        None => t.doc.verify_self::<G>(&inputs, &metrics, &opts)?,
    };
    let outcome = if checks.is_empty() { None } else { Some(report.check(&checks)) };
    let mut out = report_json(&report, outcome.as_ref(), series);
    out["inputs"] = described.clone();
    if let Some(id) = id {
        out["proposal"] = json!(id.to_string());
    }
    fx.verify = Some(std::sync::Arc::new(VerifyDetail { proposal: id.map(|i| i.to_string()), inputs: described, report, outcome }));
    Ok(out)
}

// ---- watch.proposals ----

#[derive(Clone, PartialEq, Eq)]
struct Snap {
    label: String,
    origin: String,
    op_count: usize,
    stale: bool,
}

/// What `watch.proposals` subscribers have been told: the open proposals and
/// the newest history entry seen. Turns changes of the document's proposal
/// list into `new` / `changed` / `accepted` / `rejected` events, whoever made them.
pub(crate) struct ProposalWatch {
    known: BTreeMap<u64, Snap>,
    hist_max: u64,
}

fn snapshot(doc: &EditorDoc) -> BTreeMap<u64, Snap> {
    doc.list_proposals()
        .into_iter()
        .map(|i| (i.id.0, Snap { label: i.label, origin: i.origin.to_string(), op_count: i.op_count, stale: i.stale }))
        .collect()
}

fn snap_json(event: &str, id: u64, s: &Snap) -> J {
    json!({"event": event, "id": format!("p{id}"), "label": s.label, "origin": s.origin, "op_count": s.op_count, "stale": s.stale})
}

impl ProposalWatch {
    pub(crate) fn capture(doc: &EditorDoc) -> Self {
        Self { known: snapshot(doc), hist_max: doc.history().iter().map(|h| h.id).max().unwrap_or(0) }
    }

    /// The `watch.proposals` params with no events (the state on subscribe).
    pub(crate) fn state_params(doc: &EditorDoc) -> J {
        json!({"events": [], "open": doc.list_proposals().iter().map(info_json).collect::<Vec<_>>()})
    }

    /// Events since the last call (or capture), and the new state.
    pub(crate) fn advance(&mut self, doc: &EditorDoc) -> Option<J> {
        let now = snapshot(doc);
        let history = doc.history();
        let mut events = Vec::new();
        for (id, s) in &now {
            match self.known.get(id) {
                None => events.push(snap_json("new", *id, s)),
                Some(old) if old != s => events.push(snap_json("changed", *id, s)),
                Some(_) => {}
            }
        }
        for (id, s) in &self.known {
            if now.contains_key(id) {
                continue;
            }
            let entry = history.iter().find(|h| h.id > self.hist_max && !h.undone && h.label == s.label && h.origin.to_string() == s.origin);
            match entry {
                Some(h) => {
                    let mut e = snap_json("accepted", *id, s);
                    e["history_id"] = json!(h.id);
                    events.push(e);
                }
                None => events.push(snap_json("rejected", *id, s)),
            }
        }
        self.hist_max = history.iter().map(|h| h.id).max().unwrap_or(0).max(self.hist_max);
        self.known = now;
        if events.is_empty() {
            None
        } else {
            Some(json!({"events": events, "open": doc.list_proposals().iter().map(info_json).collect::<Vec<_>>()}))
        }
    }
}
