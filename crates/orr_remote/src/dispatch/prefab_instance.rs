//! `prefab_instance.*`: generic prefab instances of any registered game.
//! Callers read and write prefab files; these methods never touch the file system.

use std::collections::BTreeMap;

use orr_edit::{OverrideKind, PrefabOverride};
use orr_reflect::Guid;
use orr_sim::Game;
use serde_json::{json, Map, Value as J};

use super::{require_edit_mode, ErpTarget, P};
use crate::error::RpcError;
use crate::json::value_to_json;

fn guid(p: &P<'_>, name: &str) -> Result<Guid, RpcError> {
    Guid::parse(p.str(name)?).map_err(RpcError::params)
}

fn override_json(o: &PrefabOverride) -> J {
    let mut j = json!({"entity": o.entity.to_string(), "source_guid": o.source.to_string()});
    let (kind, extra) = match &o.kind {
        OverrideKind::Field { component, path, base, live } => ("field", json!({"component": component, "path": path, "base": value_to_json(base), "value": value_to_json(live)})),
        OverrideKind::ComponentAdded { component, live } => ("component_added", json!({"component": component, "value": value_to_json(live)})),
        OverrideKind::ComponentRemoved { component, base } => ("component_removed", json!({"component": component, "base": value_to_json(base)})),
        OverrideKind::Renamed { base, live } => ("renamed", json!({"base": base, "value": live})),
        OverrideKind::EntityRemoved => ("entity_removed", json!({})),
    };
    j["kind"] = json!(kind);
    if let (Some(into), J::Object(from)) = (j.as_object_mut(), extra) {
        into.extend(from);
    }
    j
}

fn list<G: Game>(t: &ErpTarget<'_, G>) -> Result<J, RpcError> {
    let instances: Vec<J> = t
        .doc
        .prefab_instances()?
        .into_iter()
        .map(|i| {
            json!({
                "instance": i.root.to_string(), "source": i.source, "digest": i.digest,
                "members": i.members.iter().map(|(s, l, present)| json!({"source_guid": s.to_string(), "entity": l.to_string(), "present": present})).collect::<Vec<_>>(),
                "overrides": i.overrides.iter().map(override_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({"revision": t.doc.revision(), "instances": instances}))
}

fn texts(p: &P<'_>) -> Result<BTreeMap<Guid, Guid>, RpcError> {
    let map = p.req_raw("guids")?.as_object().ok_or_else(|| RpcError::params("guids must map prefab ids to entity GUIDs"))?;
    map.iter()
        .map(|(k, v)| Ok((Guid::parse(k).map_err(RpcError::params)?, Guid::parse(v.as_str().ok_or_else(|| RpcError::params("guids values must be strings"))?).map_err(RpcError::params)?)))
        .collect()
}

pub(super) fn call<G: Game>(t: &mut ErpTarget<'_, G>, p: &P<'_>, method: &str, origin: orr_edit::Origin) -> Result<J, RpcError> {
    let allowed: &[&str] = match method {
        "prefab_instance.list" => &[],
        "prefab_instance.entity" => &["entity"],
        "prefab_instance.create" => &["selected"],
        "prefab_instance.apply_text" => &["entity"],
        "prefab_instance.link" => &["source", "text", "guids", "expected_revision"],
        "prefab_instance.instantiate" | "prefab_instance.update" => &["source", "text", "expected_revision"],
        "prefab_instance.revert" => &["entity", "component", "path", "expected_revision"],
        "prefab_instance.detach" => &["entity", "expected_revision"],
        _ => return Err(RpcError::params("unknown prefab_instance method")),
    };
    if p.0.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(RpcError::params("unknown prefab_instance parameter"));
    }
    match method {
        "prefab_instance.list" => return list(t),
        "prefab_instance.entity" => {
            let entity = guid(p, "entity")?;
            let Some((root, source)) = t.doc.prefab_member(&entity) else {
                return Ok(json!({"revision": t.doc.revision(), "member": false}));
            };
            let all = t.doc.prefab_overrides(&entity)?;
            let mine: Vec<J> = all.iter().filter(|o| o.entity == entity).map(override_json).collect();
            let link_source = t.doc.prefab_instances()?.into_iter().find(|i| i.root == root).map(|i| i.source).unwrap_or_default();
            return Ok(json!({
                "revision": t.doc.revision(), "member": true, "instance": root.to_string(), "source": link_source,
                "source_guid": source.to_string(), "overrides": mine, "instance_override_count": all.len(),
            }));
        }
        "prefab_instance.create" => {
            let selected = p.req_raw("selected")?.as_array().ok_or_else(|| RpcError::params("selected must be a GUID array"))?;
            let guids = selected
                .iter()
                .map(|v| Guid::parse(v.as_str().ok_or_else(|| RpcError::params("selected GUID must be a string"))?).map_err(RpcError::params))
                .collect::<Result<Vec<_>, _>>()?;
            let source = t.doc.create_prefab_source(&guids)?;
            let map: Map<String, J> = source.guids.iter().map(|(local, live)| (local.to_string(), json!(live.to_string()))).collect();
            return Ok(json!({"text": source.text, "guids": map, "revision": t.doc.revision()}));
        }
        "prefab_instance.apply_text" => {
            let (source, text) = t.doc.prefab_text_with_overrides(&guid(p, "entity")?)?;
            return Ok(json!({"source": source, "text": text, "revision": t.doc.revision()}));
        }
        _ => {}
    }
    require_edit_mode(t, method)?;
    if t.doc.in_tx() {
        return Err(RpcError::state("tx_open", "prefab operations require a closed transaction"));
    }
    if p.req_u64("expected_revision")? != t.doc.revision() {
        return Err(RpcError::state("stale_revision", "the document changed; list the prefab instances again"));
    }
    let mut extra = json!({});
    match method {
        "prefab_instance.link" => {
            let root = t.doc.link_prefab_instance(p.str("source")?, p.str("text")?, texts(p)?, origin)?;
            extra["instance"] = json!(root.to_string());
        }
        "prefab_instance.instantiate" => {
            let (root, map) = t.doc.instantiate_prefab(p.str("source")?, p.str("text")?, origin)?;
            extra["instance"] = json!(root.to_string());
            extra["guids"] = J::Object(map.iter().map(|(s, l)| (s.to_string(), json!(l.to_string()))).collect());
        }
        "prefab_instance.update" => {
            let report = t.doc.update_prefab_instances(p.str("source")?, p.str("text")?, origin)?;
            extra["report"] = json!({
                "updated": report.updated.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "unchanged": report.unchanged, "spawned": report.spawned, "removed": report.removed, "conflicts": report.conflicts,
            });
        }
        "prefab_instance.revert" => {
            let entity = guid(p, "entity")?;
            let reverted = match p.opt_str("component")? {
                Some(component) => t.doc.revert_prefab_field(&entity, component, p.opt_str("path")?.unwrap_or(""), origin)?,
                None => t.doc.revert_prefab_instance(&entity, origin)?,
            };
            extra["reverted"] = json!(reverted);
        }
        "prefab_instance.detach" => {
            let root = t.doc.detach_prefab_instance(&guid(p, "entity")?, origin)?;
            extra["instance"] = json!(root.to_string());
        }
        _ => unreachable!("checked above"),
    }
    let mut out = list(t)?;
    if let (Some(into), J::Object(from)) = (out.as_object_mut(), extra) {
        into.extend(from);
    }
    Ok(out)
}
