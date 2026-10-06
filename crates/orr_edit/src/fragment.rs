//! Closed, entity-only scene fragments. Instances are independent copies.

use std::collections::{BTreeMap, BTreeSet};

use orr_fp::FPVec2;
use orr_reflect::{parse_path, Guid, PathSeg, Scene, TypeRegistry, Value};

use crate::{refs::map_refs, EditError, EditorDoc, Op, Origin};

/// Maximum encoded fragment size, checked before YAML parsing.
pub const FRAGMENT_MAX_BYTES: usize = 1_048_576;
/// Maximum number of entities in one fragment.
pub const FRAGMENT_MAX_ENTITIES: usize = 256;
/// Maximum total number of components in one fragment.
pub const FRAGMENT_MAX_COMPONENTS: usize = 2048;
/// Maximum total number of recursive value nodes in one fragment.
pub const FRAGMENT_MAX_VALUES: usize = 65_536;
/// Maximum recursive value depth (root is zero).
pub const FRAGMENT_MAX_DEPTH: usize = 32;
/// Maximum bytes in a name, field path, or other individual string.
pub const FRAGMENT_MAX_STRING_BYTES: usize = 4096;

/// A closed set of entities encoded with the existing `orr.scene/1` format.
/// No singleton state, runtime handles or source-link metadata is retained.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneFragment {
    scene: Scene,
}

/// Translate this explicitly selected Vec2 field on each matching component.
/// Entities without that component stay unchanged; at least one must match.
#[derive(Clone, Debug, PartialEq)]
pub struct FragmentTranslation2D {
    /// Fully qualified reflected component name.
    pub component: String,
    /// Reflection field path, for example `pos`.
    pub path: String,
    /// Offset applied with checked fixed-point arithmetic.
    pub delta: FPVec2,
}

/// Stable source-to-instance identity mapping, in source GUID order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentInstance {
    /// Every destination differs from all live and original fragment IDs.
    pub guids: BTreeMap<Guid, Guid>,
}

fn invalid(message: impl Into<String>) -> EditError {
    EditError::Invalid(message.into())
}

fn check_string(text: &str) -> Result<(), EditError> {
    if text.len() > FRAGMENT_MAX_STRING_BYTES {
        return Err(invalid("fragment string exceeds 4096 bytes"));
    }
    Ok(())
}

impl SceneFragment {
    /// Capture exactly this nonempty selection. Outbound references, unknown
    /// or repeated IDs and raw entity handles are errors. Inbound references
    /// do not enlarge the selection. Source singletons are not captured.
    pub fn capture(scene: &Scene, selected: &[Guid]) -> Result<Self, EditError> {
        if selected.is_empty() || selected.len() > FRAGMENT_MAX_ENTITIES {
            return Err(invalid("fragment must contain 1..=256 entities"));
        }
        let mut ids = BTreeSet::new();
        for guid in selected {
            if !ids.insert(guid.clone()) {
                return Err(invalid(format!("duplicate fragment selection '{guid}'")));
            }
            if !scene.entities.contains_key(guid) {
                return Err(EditError::UnknownEntity(guid.to_string()));
            }
        }
        let mut components = 0;
        let mut budget = Budget::default();
        for guid in &ids {
            let entity = &scene.entities[guid];
            if let Some(name) = &entity.name {
                budget.string(name)?;
                if name.is_empty() {
                    return Err(invalid("fragment entity name must not be empty"));
                }
            }
            components += entity.components.len();
            if components > FRAGMENT_MAX_COMPONENTS {
                return Err(invalid("fragment exceeds 2048 components"));
            }
            let mut names = BTreeSet::new();
            for (name, value) in &entity.components {
                budget.string(name)?;
                if !names.insert(name) {
                    return Err(invalid(format!("duplicate component '{name}' on '{guid}'")));
                }
                check_value(value, &ids, 0, &mut budget)
                    .map_err(|e| invalid(format!("fragment {guid}/{name}: {e}")))?;
            }
        }
        let mut fragment = Self {
            scene: Scene::default(),
        };
        for guid in ids {
            let mut entity = scene.entities[&guid].clone();
            entity.components.sort_by(|a, b| a.0.cmp(&b.0));
            fragment.scene.entities.insert(guid, entity);
        }
        if fragment.to_yaml().len() > FRAGMENT_MAX_BYTES {
            return Err(invalid("fragment exceeds 1048576 encoded bytes"));
        }
        Ok(fragment)
    }

    /// Load a bounded strict scene file. Singleton payloads are rejected.
    pub fn from_yaml(text: &str, types: &TypeRegistry) -> Result<Self, EditError> {
        if text.len() > FRAGMENT_MAX_BYTES {
            return Err(invalid("fragment exceeds 1048576 encoded bytes"));
        }
        let scene = Scene::parse(text, types)?;
        if !scene.singletons.is_empty() {
            return Err(invalid("scene fragments cannot contain singleton payloads"));
        }
        Self::capture(&scene, &scene.entities.keys().cloned().collect::<Vec<_>>())
    }

    /// Save as deterministic entity-only `orr.scene/1` YAML. Comments are
    /// intentionally omitted; saving a fragment does not mark its source saved.
    pub fn to_yaml(&self) -> String {
        self.scene.to_yaml()
    }

    /// Read the captured entities and their original, fragment-local IDs.
    pub fn entities(&self) -> &BTreeMap<Guid, orr_reflect::SceneEntity> {
        &self.scene.entities
    }
}

#[derive(Default)]
struct Budget {
    nodes: usize,
    string_bytes: usize,
}

impl Budget {
    fn string(&mut self, text: &str) -> Result<(), EditError> {
        check_string(text)?;
        self.string_bytes += text.len();
        if self.string_bytes > FRAGMENT_MAX_BYTES {
            return Err(invalid(
                "fragment aggregate string bytes exceed encoded size limit",
            ));
        }
        Ok(())
    }
}

fn check_value(
    value: &Value,
    ids: &BTreeSet<Guid>,
    depth: usize,
    budget: &mut Budget,
) -> Result<(), EditError> {
    budget.nodes += 1;
    if depth > FRAGMENT_MAX_DEPTH || budget.nodes > FRAGMENT_MAX_VALUES {
        return Err(invalid("fragment value depth or node limit exceeded"));
    }
    match value {
        Value::Entity(_) => {
            return Err(invalid(
                "fragment references must be GUIDs, never runtime entity handles",
            ))
        }
        Value::EntityGuid(Some(text)) => {
            budget.string(text)?;
            let guid = Guid::parse(text).map_err(invalid)?;
            if !ids.contains(&guid) {
                return Err(invalid(format!(
                    "reference '{guid}' is outside the fragment selection"
                )));
            }
        }
        Value::Array(values) => {
            for value in values {
                check_value(value, ids, depth + 1, budget)?;
            }
        }
        Value::Struct(fields) | Value::Variant(_, fields) => {
            if let Value::Variant(name, _) = value {
                budget.string(name)?;
            }
            let mut names = BTreeSet::new();
            for (name, value) in fields {
                budget.string(name)?;
                if !names.insert(name) {
                    return Err(invalid(format!("duplicate fragment value field '{name}'")));
                }
                check_value(value, ids, depth + 1, budget)?;
            }
        }
        Value::Enum(text) => budget.string(text)?,
        Value::Flags(flags) => {
            if flags.len() > FRAGMENT_MAX_VALUES {
                return Err(invalid("fragment flag count exceeds value limit"));
            }
            for flag in flags {
                budget.string(flag)?;
                budget.nodes += 1;
                if budget.nodes > FRAGMENT_MAX_VALUES {
                    return Err(invalid("fragment value node limit exceeded"));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

impl EditorDoc {
    /// Instantiate one independent copy as one atomic undo entry. Every empty
    /// entity is spawned before any components are added, preserving forward,
    /// self and mutual references on insertion, undo and redo. Rejection leaves
    /// the complete live document (including allocation and redo) unchanged.
    pub fn instantiate_fragment(
        &mut self,
        fragment: &SceneFragment,
        placement: Option<&FragmentTranslation2D>,
        origin: Origin,
    ) -> Result<FragmentInstance, EditError> {
        let path = placement
            .map(|p| {
                check_string(&p.component)?;
                check_string(&p.path)?;
                Ok::<_, EditError>(parse_path(&p.path)?)
            })
            .transpose()?;
        let mut reserved: BTreeSet<Guid> = self
            .scene
            .entities
            .keys()
            .chain(fragment.scene.entities.keys())
            .cloned()
            .collect();
        let mut guids = BTreeMap::new();
        let mut candidate = u64::from(self.next_guid);
        for source in fragment.scene.entities.keys() {
            // At most reserved.len()+1 probes can be needed; no wrapping loop.
            let mut chosen = None;
            for _ in 0..=reserved.len() {
                let n = u32::try_from(candidate)
                    .map_err(|_| invalid("fragment GUID allocation exhausted"))?;
                candidate += 1;
                let guid = Guid::from_u32(n);
                if reserved.insert(guid.clone()) {
                    chosen = Some(guid);
                    break;
                }
            }
            guids.insert(
                source.clone(),
                chosen.ok_or_else(|| invalid("fragment GUID allocation exhausted"))?,
            );
        }
        let mut ops = Vec::new();
        for (source, entity) in &fragment.scene.entities {
            ops.push(Op::SpawnEntity {
                guid: Some(guids[source].clone()),
                name: entity.name.clone(),
                components: Vec::new(),
            });
        }
        let mut matches = 0;
        for (source, entity) in &fragment.scene.entities {
            for (component, value) in &entity.components {
                let mut value = map_refs(value, &mut |reference| match reference {
                    Value::EntityGuid(None) => Ok(Value::EntityGuid(None)),
                    Value::EntityGuid(Some(text)) => {
                        let source = Guid::parse(text).map_err(invalid)?;
                        let target = guids.get(&source).ok_or_else(|| {
                            invalid(format!("external fragment reference '{text}'"))
                        })?;
                        Ok(Value::EntityGuid(Some(target.to_string())))
                    }
                    _ => Err(invalid("raw fragment entity reference")),
                })?;
                if let Some(p) = placement.filter(|p| p.component == *component) {
                    translate(
                        &mut value,
                        path.as_deref().expect("placement path parsed"),
                        p.delta,
                    )?;
                    matches += 1;
                }
                ops.push(Op::AddComponent {
                    guid: guids[source].clone(),
                    component: component.clone(),
                    value: Some(value),
                });
            }
        }
        if placement.is_some() && matches == 0 {
            return Err(invalid("fragment translation matched no component"));
        }
        self.apply_atomic_batch("instantiate scene fragment", ops, origin)?;
        Ok(FragmentInstance { guids })
    }
}

fn translate(value: &mut Value, path: &[PathSeg], delta: FPVec2) -> Result<(), EditError> {
    if let Some((head, tail)) = path.split_first() {
        let child = match (head, value) {
            (PathSeg::Field(name), Value::Struct(fields) | Value::Variant(_, fields)) => fields
                .iter_mut()
                .find(|(field, _)| field == name)
                .map(|(_, value)| value),
            (PathSeg::Index(index), Value::Array(values)) => values.get_mut(*index),
            _ => None,
        }
        .ok_or_else(|| invalid("fragment translation field does not exist"))?;
        return translate(child, tail, delta);
    }
    let Value::Vec2(position) = value else {
        return Err(invalid("fragment translation field must be Vec2"));
    };
    let x = position
        .x
        .checked_add(delta.x)
        .ok_or_else(|| invalid("fragment translation overflows FP x"))?;
    let y = position
        .y
        .checked_add(delta.y)
        .ok_or_else(|| invalid("fragment translation overflows FP y"))?;
    *position = FPVec2::new(x, y);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recursive_values_remap_only_typed_references() {
        let original = Guid::from_u32(1);
        let replacement = Guid::from_u32(2);
        let value = Value::Struct(vec![(
            "nested".into(),
            Value::Array(vec![Value::Variant(
                "tag".into(),
                vec![
                    ("self".into(), Value::EntityGuid(Some(original.to_string()))),
                    ("null".into(), Value::EntityGuid(None)),
                    ("asset".into(), Value::Int(42)),
                    ("ordinary enum".into(), Value::Enum(original.to_string())),
                ],
            )]),
        )]);
        let mut scene = Scene::default();
        scene.entities.insert(
            original.clone(),
            orr_reflect::SceneEntity {
                components: vec![("Nested".into(), value.clone())],
                ..Default::default()
            },
        );
        let fragment = SceneFragment::capture(&scene, std::slice::from_ref(&original)).unwrap();
        let remapped = map_refs(&fragment.entities()[&original].components[0].1, &mut |v| {
            Ok(match v {
                Value::EntityGuid(Some(_)) => Value::EntityGuid(Some(replacement.to_string())),
                _ => v.clone(),
            })
        })
        .unwrap();
        let Value::Struct(fields) = remapped else {
            panic!()
        };
        let Value::Array(items) = &fields[0].1 else {
            panic!()
        };
        let Value::Variant(name, fields) = &items[0] else {
            panic!()
        };
        assert_eq!(name, "tag");
        assert_eq!(
            fields[0].1,
            Value::EntityGuid(Some(replacement.to_string()))
        );
        assert_eq!(fields[1].1, Value::EntityGuid(None));
        assert_eq!(fields[2].1, Value::Int(42));
        assert_eq!(fields[3].1, Value::Enum(original.to_string()));
    }

    #[test]
    fn programmatic_capture_checks_depth_nodes_components_and_string_budgets() {
        let guid = Guid::from_u32(1);
        let capture = |value: Value| {
            let mut scene = Scene::default();
            scene.entities.insert(
                guid.clone(),
                orr_reflect::SceneEntity {
                    components: vec![("Test".into(), value)],
                    ..Default::default()
                },
            );
            SceneFragment::capture(&scene, std::slice::from_ref(&guid))
        };
        let mut deep = Value::Int(0);
        for _ in 0..=FRAGMENT_MAX_DEPTH {
            deep = Value::Array(vec![deep]);
        }
        assert!(capture(deep).is_err());
        assert!(capture(Value::Array(vec![Value::Int(0); FRAGMENT_MAX_VALUES])).is_err());
        assert!(capture(Value::Enum("a".repeat(FRAGMENT_MAX_STRING_BYTES + 1))).is_err());
        assert!(capture(Value::Array(vec![
            Value::Enum(
                "a".repeat(FRAGMENT_MAX_STRING_BYTES)
            );
            257
        ]))
        .is_err());
        let mut scene = Scene::default();
        scene.entities.insert(
            guid.clone(),
            orr_reflect::SceneEntity {
                components: (0..=FRAGMENT_MAX_COMPONENTS)
                    .map(|i| (format!("Type{i}"), Value::Int(0)))
                    .collect(),
                ..Default::default()
            },
        );
        assert!(SceneFragment::capture(&scene, &[guid]).is_err());
    }

    #[test]
    fn allocator_exhaustion_is_bounded_and_preserves_live_state() {
        let types = TypeRegistry::new();
        let registry = orr_ecs::ComponentRegistryBuilder::new().build();
        let mut doc = EditorDoc::new(types, registry, 7).unwrap();
        let mut scene = Scene::default();
        scene
            .entities
            .insert(Guid::from_u32(1), orr_reflect::SceneEntity::default());
        scene
            .entities
            .insert(Guid::from_u32(2), orr_reflect::SceneEntity::default());
        let fragment =
            SceneFragment::capture(&scene, &[Guid::from_u32(1), Guid::from_u32(2)]).unwrap();
        doc.instantiate_fragment(&fragment, None, Origin::User)
            .unwrap();
        doc.undo().unwrap();
        doc.next_guid = u32::MAX;
        let before = (
            doc.to_yaml(),
            doc.frame().to_bytes(),
            doc.index().clone(),
            doc.history(),
            doc.revision(),
        );
        assert!(doc
            .instantiate_fragment(&fragment, None, Origin::User)
            .is_err());
        assert_eq!(doc.next_guid, u32::MAX);
        assert_eq!(
            (
                doc.to_yaml(),
                doc.frame().to_bytes(),
                doc.index().clone(),
                doc.history(),
                doc.revision()
            ),
            before
        );
        assert!(doc.can_redo());
        doc.redo().unwrap();
    }
}
