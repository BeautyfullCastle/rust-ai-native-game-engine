//! Applying an [`Op`] to a [`Scene`] value, and working out its inverse.
//!
//! Every op is fully checked before the scene is touched, so an `Err`
//! leaves the scene as it was. Component values are checked by a round trip
//! through the component's bytes (`default + write + set + read`), the same
//! path a bake takes. That gives range and type checks, and one canonical
//! form for every stored value.

use std::collections::BTreeMap;

use orr_ecs::Entity;
use orr_reflect::{Guid, Scene, SceneEntity, TypeInfo, TypeKind, TypeRegistry, Value};

use crate::error::EditError;
use crate::op::Op;
use crate::refs::{map_refs, refers_to};

/// What an applied op changed, so the preview frame can be updated cheaply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Nothing changed.
    None,
    /// One component value changed (entity and component exist before and after).
    Field { guid: Guid, component: String },
    /// Only a display name changed.
    Rename { guid: Guid },
    /// Entities, components or singletons appeared or vanished.
    Structural,
}

/// The result of [`apply`].
pub(crate) struct Done {
    /// The op as stored (GUID chosen, value canonical), for redo.
    pub forward: Op,
    /// The op that restores the scene exactly.
    pub inverse: Op,
    pub effect: Effect,
    pub changed: bool,
    pub guid: Option<Guid>,
}

/// Maps GUIDs to stand-in entities (position in the sorted entity map) and back.
/// Only used to run values through component bytes; the handles never leave this module.
pub(crate) struct Refs<'a> {
    entities: &'a BTreeMap<Guid, SceneEntity>,
    extra: Option<&'a Guid>,
}

impl<'a> Refs<'a> {
    pub(crate) fn new(entities: &'a BTreeMap<Guid, SceneEntity>, extra: Option<&'a Guid>) -> Self {
        Self { entities, extra }
    }

    fn to_entity(&self, guid: &str) -> Option<Entity> {
        let pos = match self.entities.keys().position(|k| k.as_str() == guid) {
            Some(p) => p,
            None if self.extra.is_some_and(|g| g.as_str() == guid) => self.entities.len(),
            None => return None,
        };
        Some(Entity { index: pos as u32, version: 1 })
    }

    fn to_guid(&self, e: Entity) -> Option<String> {
        if e == Entity::NONE {
            return None;
        }
        let i = e.index as usize;
        match self.entities.keys().nth(i) {
            Some(g) => Some(g.as_str().to_string()),
            None => self.extra.map(|g| g.as_str().to_string()),
        }
    }

    /// GUID form to stand-in `Entity` form. Unknown GUIDs are an error.
    pub(crate) fn resolve(&self, v: &Value) -> Result<Value, EditError> {
        map_refs(v, &mut |r| match r {
            Value::EntityGuid(None) => Ok(Value::Entity(Entity::NONE)),
            Value::EntityGuid(Some(g)) => self
                .to_entity(g)
                .map(Value::Entity)
                .ok_or_else(|| EditError::Invalid(format!("reference to unknown entity '{g}'"))),
            Value::Entity(e) if *e == Entity::NONE => Ok(Value::Entity(Entity::NONE)),
            _ => Err(EditError::Invalid("entity references in scene edits must be GUIDs (Value::EntityGuid)".into())),
        })
    }

    /// The reverse of [`resolve`](Self::resolve).
    pub(crate) fn unresolve(&self, v: &Value) -> Value {
        map_refs(v, &mut |r| {
            Ok(match r {
                Value::Entity(e) => Value::EntityGuid(self.to_guid(*e)),
                other => other.clone(),
            })
        })
        .expect("mapping to GUIDs cannot fail")
    }
}

pub(crate) fn type_of<'a>(reg: &'a TypeRegistry, name: &str, kind: TypeKind) -> Result<&'a TypeInfo, EditError> {
    reg.get(name).filter(|t| t.kind() == kind).ok_or_else(|| EditError::UnknownType(name.to_string()))
}

/// `patch` laid over `base`: struct fields of `patch` replace the ones of
/// `base` (recursively); any other value replaces `base` whole.
fn overlay(base: &Value, patch: &Value) -> Value {
    match (base, patch) {
        (Value::Struct(b), Value::Struct(p)) => {
            let mut out: Vec<(String, Value)> = b
                .iter()
                .map(|(n, bv)| (n.clone(), p.iter().find(|(pn, _)| pn == n).map_or_else(|| bv.clone(), |(_, pv)| overlay(bv, pv))))
                .collect();
            // Unknown fields stay in, so the check names them.
            out.extend(p.iter().filter(|(pn, _)| !b.iter().any(|(n, _)| n == pn)).cloned());
            Value::Struct(out)
        }
        _ => patch.clone(),
    }
}

/// The value of a type's default bytes, in frame form.
pub(crate) fn default_value(t: &TypeInfo) -> Value {
    t.read(&t.default_bytes())
}

/// Fills fields a struct value leaves out with the type's defaults.
pub(crate) fn with_defaults(t: &TypeInfo, value: &Value) -> Value {
    overlay(&default_value(t), value)
}

/// Checks a whole value and returns its canonical form. A struct value may
/// list only some fields; the rest come from the type's default.
pub(crate) fn normalize(t: &TypeInfo, value: &Value, refs: &Refs<'_>) -> Result<Value, EditError> {
    let mut bytes = t.default_bytes();
    t.desc().write(&mut bytes, &refs.resolve(&with_defaults(t, value))?)?;
    Ok(refs.unresolve(&t.read(&bytes)))
}

/// The canonical value after setting `path` to `value` on `old` (or on the default).
fn set_path(t: &TypeInfo, old: Option<&Value>, path: &str, value: &Value, refs: &Refs<'_>) -> Result<Value, EditError> {
    let mut bytes = t.default_bytes();
    if let Some(old) = old {
        t.desc().write(&mut bytes, &refs.resolve(old)?)?;
    }
    t.set(&mut bytes, path, refs.resolve(value)?)?;
    Ok(refs.unresolve(&t.read(&bytes)))
}

fn insert_sorted(list: &mut Vec<(String, Value)>, name: &str, value: Value) {
    let at = list.partition_point(|(n, _)| n.as_str() < name);
    list.insert(at, (name.to_string(), value));
}

fn entity_mut<'a>(scene: &'a mut Scene, guid: &Guid) -> Result<&'a mut SceneEntity, EditError> {
    scene.entities.get_mut(guid).ok_or_else(|| EditError::UnknownEntity(guid.to_string()))
}

/// Applies `op` to `scene`. `op`'s `SpawnEntity` must already carry a GUID.
pub(crate) fn apply(scene: &mut Scene, reg: &TypeRegistry, op: &Op) -> Result<Done, EditError> {
    match op {
        Op::SetField { guid, component, path, value } => {
            let t = type_of(reg, component, TypeKind::Component)?;
            let ent = scene.entities.get(guid).ok_or_else(|| EditError::UnknownEntity(guid.to_string()))?;
            let old = ent
                .components
                .iter()
                .find(|(n, _)| n == component)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| EditError::NoComponent { entity: guid.to_string(), component: component.clone() })?;
            let new = set_path(t, Some(&old), path, value, &Refs::new(&scene.entities, None))?;
            if new == old {
                return Ok(unchanged(op));
            }
            let slot = entity_mut(scene, guid)?.components.iter_mut().find(|(n, _)| n == component).expect("checked above");
            slot.1 = new;
            Ok(Done {
                forward: op.clone(),
                inverse: Op::SetField { guid: guid.clone(), component: component.clone(), path: String::new(), value: old },
                effect: Effect::Field { guid: guid.clone(), component: component.clone() },
                changed: true,
                guid: None,
            })
        }
        Op::AddComponent { guid, component, value } => {
            let t = type_of(reg, component, TypeKind::Component)?;
            let ent = scene.entities.get(guid).ok_or_else(|| EditError::UnknownEntity(guid.to_string()))?;
            if ent.components.iter().any(|(n, _)| n == component) {
                return Err(EditError::HasComponent { entity: guid.to_string(), component: component.clone() });
            }
            let refs = Refs::new(&scene.entities, None);
            let new = match value {
                Some(v) => normalize(t, v, &refs)?,
                None => refs.unresolve(&t.read(&t.default_bytes())),
            };
            insert_sorted(&mut entity_mut(scene, guid)?.components, component, new.clone());
            Ok(Done {
                forward: Op::AddComponent { guid: guid.clone(), component: component.clone(), value: Some(new) },
                inverse: Op::RemoveComponent { guid: guid.clone(), component: component.clone() },
                effect: Effect::Structural,
                changed: true,
                guid: None,
            })
        }
        Op::RemoveComponent { guid, component } => {
            type_of(reg, component, TypeKind::Component)?;
            let ent = entity_mut(scene, guid)?;
            let at = ent
                .components
                .iter()
                .position(|(n, _)| n == component)
                .ok_or_else(|| EditError::NoComponent { entity: guid.to_string(), component: component.clone() })?;
            let (_, old) = ent.components.remove(at);
            Ok(Done {
                forward: op.clone(),
                inverse: Op::AddComponent { guid: guid.clone(), component: component.clone(), value: Some(old) },
                effect: Effect::Structural,
                changed: true,
                guid: None,
            })
        }
        Op::SpawnEntity { guid, name, components } => {
            let guid = guid.clone().ok_or_else(|| EditError::Invalid("SpawnEntity needs a GUID here".into()))?;
            if scene.entities.contains_key(&guid) {
                return Err(EditError::GuidExists(guid));
            }
            let refs = Refs::new(&scene.entities, Some(&guid));
            let mut list: Vec<(String, Value)> = Vec::new();
            for (cname, v) in components {
                let t = type_of(reg, cname, TypeKind::Component)?;
                if list.iter().any(|(n, _)| n == cname) {
                    return Err(EditError::HasComponent { entity: guid.to_string(), component: cname.clone() });
                }
                insert_sorted(&mut list, cname, normalize(t, v, &refs)?);
            }
            let forward = Op::SpawnEntity { guid: Some(guid.clone()), name: name.clone(), components: list.clone() };
            scene.entities.insert(guid.clone(), SceneEntity { name: name.clone(), components: list });
            Ok(Done {
                forward,
                inverse: Op::DespawnEntity { guid: guid.clone() },
                effect: Effect::Structural,
                changed: true,
                guid: Some(guid),
            })
        }
        Op::DespawnEntity { guid } => {
            let ent = scene.entities.get(guid).ok_or_else(|| EditError::UnknownEntity(guid.to_string()))?;
            for (other, e) in &scene.entities {
                if other != guid && e.components.iter().any(|(_, v)| refers_to(v, guid.as_str())) {
                    return Err(EditError::Referenced { guid: guid.clone(), by: other.clone() });
                }
            }
            if scene.singletons.iter().any(|(_, v)| refers_to(v, guid.as_str())) {
                return Err(EditError::Invalid(format!("entity '{guid}' is referenced by a singleton")));
            }
            let inverse = Op::SpawnEntity { guid: Some(guid.clone()), name: ent.name.clone(), components: ent.components.clone() };
            scene.entities.remove(guid);
            Ok(Done { forward: op.clone(), inverse, effect: Effect::Structural, changed: true, guid: None })
        }
        Op::Rename { guid, name } => {
            let ent = entity_mut(scene, guid)?;
            if &ent.name == name {
                return Ok(unchanged(op));
            }
            let old = std::mem::replace(&mut ent.name, name.clone());
            Ok(Done {
                forward: op.clone(),
                inverse: Op::Rename { guid: guid.clone(), name: old },
                effect: Effect::Rename { guid: guid.clone() },
                changed: true,
                guid: None,
            })
        }
        Op::SetSingletonField { singleton, path, value } => {
            let t = type_of(reg, singleton, TypeKind::Singleton)?;
            let at = scene.singletons.iter().position(|(n, _)| n == singleton);
            let old = at.map(|i| scene.singletons[i].1.clone());
            let new = set_path(t, old.as_ref(), path, value, &Refs::new(&scene.entities, None))?;
            if old.as_ref() == Some(&new) {
                return Ok(unchanged(op));
            }
            match at {
                Some(i) => scene.singletons[i].1 = new,
                None => insert_sorted(&mut scene.singletons, singleton, new),
            }
            let inverse = match old {
                Some(v) => Op::SetSingletonField { singleton: singleton.clone(), path: String::new(), value: v },
                None => Op::RemoveSingleton { singleton: singleton.clone() },
            };
            Ok(Done { forward: op.clone(), inverse, effect: Effect::Structural, changed: true, guid: None })
        }
        Op::RemoveSingleton { singleton } => {
            type_of(reg, singleton, TypeKind::Singleton)?;
            let at = scene
                .singletons
                .iter()
                .position(|(n, _)| n == singleton)
                .ok_or_else(|| EditError::Invalid(format!("the scene has no value for singleton '{singleton}'")))?;
            let (_, old) = scene.singletons.remove(at);
            Ok(Done {
                forward: op.clone(),
                inverse: Op::SetSingletonField { singleton: singleton.clone(), path: String::new(), value: old },
                effect: Effect::Structural,
                changed: true,
                guid: None,
            })
        }
    }
}

fn unchanged(op: &Op) -> Done {
    Done { forward: op.clone(), inverse: op.clone(), effect: Effect::None, changed: false, guid: None }
}
