//! Generic prefab instances (`generic-v1` links).
//!
//! A prefab file is a closed, entity-only `orr.scene/1` fragment with
//! fragment-local entity ids. An instance is a set of ordinary scene entities
//! plus one link that stores the source label, the fragment text the instance
//! was last synchronised with (the *baseline*) and the source-id to live-id map.
//!
//! Overrides are not stored. They are the leaf differences between the live
//! entities and the baseline (with the baseline's entity references mapped to
//! live ids). So every edit path (inspector, agents, undo) keeps overrides
//! correct without extra bookkeeping. Updating an instance from a changed
//! prefab is a three-way merge of baseline, new prefab and live entities.
//!
//! Nothing here opens a file. Callers pass the prefab text they already read.

use std::collections::{BTreeMap, BTreeSet};

use orr_reflect::{parse_path, Guid, LinkProfile, PathSeg, PrefabLink, Scene, SceneEntity, Value};

use crate::refs::{map_refs, refers_to};
use crate::{EditError, EditorDoc, Op, Origin, SceneFragment, FRAGMENT_MAX_BYTES};

/// Most generic instances one scene can link.
pub const PREFAB_MAX_INSTANCES: usize = 1024;

fn invalid(message: impl Into<String>) -> EditError {
    EditError::Invalid(message.into())
}

/// One way an instance differs from its prefab.
#[derive(Clone, Debug, PartialEq)]
pub enum OverrideKind {
    /// A reflected field (or the whole component if `path` is empty) has another value.
    Field {
        /// Component type name.
        component: String,
        /// Reflect path of the changed leaf.
        path: String,
        /// Value in the prefab, with references mapped to live ids.
        base: Value,
        /// Value in the instance.
        live: Value,
    },
    /// The instance has a component the prefab entity does not have.
    ComponentAdded {
        /// Component type name.
        component: String,
        /// Value in the instance.
        live: Value,
    },
    /// The instance lacks a component the prefab entity has.
    ComponentRemoved {
        /// Component type name.
        component: String,
        /// Value in the prefab, with references mapped to live ids.
        base: Value,
    },
    /// The entity has another name.
    Renamed {
        /// Name in the prefab.
        base: Option<String>,
        /// Name in the instance.
        live: Option<String>,
    },
    /// The instance deleted this member entity.
    EntityRemoved,
}

/// One override of one member entity.
#[derive(Clone, Debug, PartialEq)]
pub struct PrefabOverride {
    /// Live entity of the instance (it may no longer exist for `EntityRemoved`).
    pub entity: Guid,
    /// Fragment-local id of the member.
    pub source: Guid,
    /// What differs.
    pub kind: OverrideKind,
}

/// A source text prepared from a selection.
#[derive(Clone, Debug, PartialEq)]
pub struct PrefabSource {
    /// Canonical prefab text with fragment-local ids `e_00000001`, ...
    pub text: String,
    /// Fragment-local id to the selected live entity.
    pub guids: BTreeMap<Guid, Guid>,
}

/// Summary of one instance.
#[derive(Clone, Debug, PartialEq)]
pub struct PrefabInstanceInfo {
    /// Instance key (a live member).
    pub root: Guid,
    /// Source label of the prefab file.
    pub source: String,
    /// SHA-256 of the baseline.
    pub digest: String,
    /// Fragment-local id, live id and whether the live entity exists.
    pub members: Vec<(Guid, Guid, bool)>,
    /// All overrides.
    pub overrides: Vec<PrefabOverride>,
}

/// What an update from a changed prefab did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrefabUpdate {
    /// Instances that took the new prefab.
    pub updated: Vec<Guid>,
    /// Instances that already had this prefab text.
    pub unchanged: usize,
    /// Member entities created.
    pub spawned: usize,
    /// Member entities deleted.
    pub removed: usize,
    /// Places where the instance and the prefab both changed, or a prefab
    /// deletion could not be applied. The instance value was kept.
    pub conflicts: Vec<String>,
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn same_names(a: &[(String, Value)], b: &[(String, Value)]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.0 == y.0)
}

/// Leaf differences `(path, base, live)`. Structs, tagged values of one variant
/// and same-length arrays recurse; every other difference is one leaf.
fn leaf_diffs(base: &Value, live: &Value, path: &str, out: &mut Vec<(String, Value, Value)>) {
    if base == live {
        return;
    }
    match (base, live) {
        (Value::Struct(a), Value::Struct(b)) if same_names(a, b) => {
            for ((name, x), (_, y)) in a.iter().zip(b) {
                leaf_diffs(x, y, &join(path, name), out);
            }
        }
        (Value::Variant(n1, a), Value::Variant(n2, b)) if n1 == n2 && same_names(a, b) => {
            for ((name, x), (_, y)) in a.iter().zip(b) {
                leaf_diffs(x, y, &join(path, name), out);
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                leaf_diffs(x, y, &format!("{path}[{i}]"), out);
            }
        }
        _ => out.push((path.to_string(), base.clone(), live.clone())),
    }
}

fn set_at(value: &mut Value, path: &[PathSeg], new: Value) -> bool {
    let Some((head, tail)) = path.split_first() else {
        *value = new;
        return true;
    };
    let child = match (head, value) {
        (PathSeg::Field(name), Value::Struct(f) | Value::Variant(_, f)) => f.iter_mut().find(|(n, _)| n == name).map(|(_, v)| v),
        (PathSeg::Index(i), Value::Array(items)) => items.get_mut(*i),
        _ => None,
    };
    child.is_some_and(|child| set_at(child, tail, new))
}

fn under(path: &str, prefix: &str) -> bool {
    prefix.is_empty() || path == prefix || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('.') || rest.starts_with('['))
}

fn component<'a>(entity: &'a SceneEntity, name: &str) -> Option<&'a Value> {
    entity.components.iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

/// Maps every entity reference through `f`; a reference without a mapping is an error.
fn remap(value: &Value, f: &dyn Fn(&str) -> Option<String>) -> Result<Value, EditError> {
    map_refs(value, &mut |reference| match reference {
        Value::EntityGuid(None) => Ok(Value::EntityGuid(None)),
        Value::EntityGuid(Some(text)) => f(text)
            .map(|mapped| Value::EntityGuid(Some(mapped)))
            .ok_or_else(|| invalid(format!("reference '{text}' is outside the prefab instance"))),
        _ => Err(invalid("raw entity handles cannot be part of a prefab")),
    })
}

fn remap_entity(entity: &SceneEntity, map: &BTreeMap<Guid, Guid>) -> Result<SceneEntity, EditError> {
    let f = |text: &str| Guid::parse(text).ok().and_then(|g| map.get(&g)).map(ToString::to_string);
    Ok(SceneEntity {
        name: entity.name.clone(),
        components: entity.components.iter().map(|(n, v)| Ok((n.clone(), remap(v, &f)?))).collect::<Result<_, EditError>>()?,
    })
}

fn allocate(next: &mut u64, reserved: &mut BTreeSet<Guid>) -> Result<Guid, EditError> {
    loop {
        let n = u32::try_from(*next).map_err(|_| invalid("entity id allocation exhausted"))?;
        *next += 1;
        let guid = Guid::from_u32(n);
        if reserved.insert(guid.clone()) {
            return Ok(guid);
        }
    }
}

/// Three-way merge in live space. An unchanged instance value follows the
/// prefab; an overridden one stays. If both changed, the instance wins.
fn merge3(base: &Value, new: &Value, live: &Value, at: &str, conflicts: &mut Vec<String>) -> Value {
    if live == base {
        return new.clone();
    }
    if new == base || live == new {
        return live.clone();
    }
    match (base, new, live) {
        (Value::Struct(b), Value::Struct(n), Value::Struct(l)) if same_names(b, n) && same_names(b, l) => Value::Struct(
            b.iter()
                .zip(n)
                .zip(l)
                .map(|(((name, bv), (_, nv)), (_, lv))| (name.clone(), merge3(bv, nv, lv, &join(at, name), conflicts)))
                .collect(),
        ),
        (Value::Variant(b0, b), Value::Variant(n0, n), Value::Variant(l0, l)) if b0 == n0 && b0 == l0 && same_names(b, n) && same_names(b, l) => Value::Variant(
            b0.clone(),
            b.iter()
                .zip(n)
                .zip(l)
                .map(|(((name, bv), (_, nv)), (_, lv))| (name.clone(), merge3(bv, nv, lv, &join(at, name), conflicts)))
                .collect(),
        ),
        (Value::Array(b), Value::Array(n), Value::Array(l)) if b.len() == n.len() && b.len() == l.len() => Value::Array(
            b.iter()
                .zip(n)
                .zip(l)
                .enumerate()
                .map(|(i, ((bv, nv), lv))| merge3(bv, nv, lv, &format!("{at}[{i}]"), conflicts))
                .collect(),
        ),
        _ => {
            conflicts.push(format!("{at}: the prefab and the instance both changed it; the instance value was kept"));
            live.clone()
        }
    }
}

impl EditorDoc {
    fn generic_link(&self, root: &Guid) -> Result<&PrefabLink, EditError> {
        let link = self.scene.prefab_links.get(root).ok_or_else(|| invalid("unknown prefab instance"))?;
        if link.profile != LinkProfile::GenericV1 {
            return Err(invalid("this link is not a generic prefab instance"));
        }
        Ok(link)
    }

    /// The instance key and the fragment-local id of a live entity, if it is a
    /// member of a generic prefab instance.
    pub fn prefab_member(&self, live: &Guid) -> Option<(Guid, Guid)> {
        self.scene.prefab_links.iter().find_map(|(root, link)| {
            (link.profile == LinkProfile::GenericV1)
                .then(|| link.guids.iter().find(|(_, target)| *target == live).map(|(source, _)| (root.clone(), source.clone())))
                .flatten()
        })
    }

    /// Resolves an instance key or any live member to the instance key.
    pub fn prefab_instance_key(&self, any_member: &Guid) -> Result<Guid, EditError> {
        if self.scene.prefab_links.get(any_member).is_some_and(|l| l.profile == LinkProfile::GenericV1) {
            return Ok(any_member.clone());
        }
        self.prefab_member(any_member).map(|(root, _)| root).ok_or_else(|| invalid(format!("'{any_member}' is not part of a prefab instance")))
    }

    fn baseline_entities(&self, link: &PrefabLink) -> Result<BTreeMap<Guid, SceneEntity>, EditError> {
        Ok(Scene::parse(&link.baseline, &self.types)?.entities)
    }

    /// Prefab text parsed and canonicalised: one level, closed, no singletons.
    fn parse_prefab_text(&self, text: &str) -> Result<(String, BTreeMap<Guid, SceneEntity>), EditError> {
        if text.len() > FRAGMENT_MAX_BYTES {
            return Err(invalid("prefab file is too large"));
        }
        let parsed = Scene::parse(text, &self.types)?;
        if parsed.has_prefab_links() {
            return Err(invalid("a prefab file cannot contain prefab instances (nested prefabs are not supported); detach the inner instance first"));
        }
        if !parsed.singletons.is_empty() {
            return Err(invalid("a prefab file cannot contain singleton state"));
        }
        let fragment = SceneFragment::capture(&parsed, &parsed.entities.keys().cloned().collect::<Vec<_>>())?;
        Ok((fragment.to_yaml(), fragment.entities().clone()))
    }

    /// Turns a closed selection into prefab text with fragment-local ids.
    ///
    /// An entity that refers to an entity outside the selection is refused, with
    /// a message that names the reference. The selection must not contain
    /// members of another prefab instance (no nesting). The scene is unchanged.
    pub fn create_prefab_source(&self, selected: &[Guid]) -> Result<PrefabSource, EditError> {
        if self.scene.prefab_links.values().any(|l| l.profile != LinkProfile::GenericV1) {
            return Err(invalid("this scene uses linked Collect groups; generic prefabs are not available"));
        }
        if let Some(member) = selected.iter().find(|g| self.prefab_member(g).is_some()) {
            return Err(invalid(format!(
                "'{member}' already belongs to a prefab instance; nested prefabs are not supported. Detach that instance first"
            )));
        }
        let fragment = SceneFragment::capture(&self.scene, selected).map_err(|e| match e {
            EditError::Invalid(why) if why.contains("outside the fragment selection") => invalid(format!(
                "cannot create a prefab: {why}. Select the referenced entity too, or clear the reference first"
            )),
            other => other,
        })?;
        let guids: BTreeMap<Guid, Guid> = fragment
            .entities()
            .keys()
            .enumerate()
            .map(|(i, live)| (Guid::from_u32(u32::try_from(i + 1).expect("at most 256 entities")), live.clone()))
            .collect();
        let to_local: BTreeMap<Guid, Guid> = guids.iter().map(|(local, live)| (live.clone(), local.clone())).collect();
        let mut local = Scene::default();
        for (live, entity) in fragment.entities() {
            local.entities.insert(to_local[live].clone(), remap_entity(entity, &to_local)?);
        }
        let (text, _) = self.parse_prefab_text(&local.to_yaml())?;
        Ok(PrefabSource { text, guids })
    }

    /// Makes `guids` (fragment-local id to live entity) an instance of the prefab
    /// `text`. Use it for the entities a prefab was just created from. A link
    /// change is one undo step; no entity changes.
    pub fn link_prefab_instance(&mut self, source: &str, text: &str, guids: BTreeMap<Guid, Guid>, origin: Origin) -> Result<Guid, EditError> {
        if self.scene.prefab_links.values().any(|l| l.profile != LinkProfile::GenericV1) {
            return Err(invalid("this scene uses linked Collect groups; generic prefabs are not available"));
        }
        if self.scene.prefab_links.len() >= PREFAB_MAX_INSTANCES {
            return Err(invalid("at most 1024 prefab instances are supported"));
        }
        let (baseline, entities) = self.parse_prefab_text(text)?;
        if !entities.keys().eq(guids.keys()) {
            return Err(invalid("the entity map must cover exactly the prefab entities"));
        }
        for live in guids.values() {
            if !self.scene.entities.contains_key(live) {
                return Err(EditError::UnknownEntity(live.to_string()));
            }
            if self.prefab_member(live).is_some() {
                return Err(invalid(format!("'{live}' already belongs to a prefab instance")));
            }
        }
        let root = guids.values().next().expect("nonempty prefab").clone();
        let mut links = self.scene.prefab_links.clone();
        links.insert(root.clone(), PrefabLink { profile: LinkProfile::GenericV1, source: source.into(), digest: PrefabLink::canonical_digest(&baseline), baseline, guids, ordinals: BTreeMap::new(), position_overrides: BTreeSet::new() });
        self.apply_linked_batch("link prefab instance", Vec::new(), links, origin)?;
        Ok(root)
    }

    /// Places one new instance: fresh entity ids, internal references remapped,
    /// one undo step. `source` is an inert relative label of the prefab file.
    pub fn instantiate_prefab(&mut self, source: &str, text: &str, origin: Origin) -> Result<(Guid, BTreeMap<Guid, Guid>), EditError> {
        if self.scene.prefab_links.values().any(|l| l.profile != LinkProfile::GenericV1) {
            return Err(invalid("this scene uses linked Collect groups; generic prefabs are not available"));
        }
        if self.scene.prefab_links.len() >= PREFAB_MAX_INSTANCES {
            return Err(invalid("at most 1024 prefab instances are supported"));
        }
        let (baseline, _) = self.parse_prefab_text(text)?;
        let fragment = SceneFragment::from_yaml(&baseline, &self.types)?;
        let (instance, ops) = self.plan_fragment(&fragment, None)?;
        let root = instance.guids.values().next().expect("nonempty prefab").clone();
        let mut links = self.scene.prefab_links.clone();
        links.insert(
            root.clone(),
            PrefabLink {
                profile: LinkProfile::GenericV1,
                source: source.into(),
                digest: PrefabLink::canonical_digest(&baseline),
                baseline,
                guids: instance.guids.clone(),
                ordinals: BTreeMap::new(),
                position_overrides: BTreeSet::new(),
            },
        );
        self.apply_linked_batch("instantiate prefab", ops, links, origin)?;
        Ok((root, instance.guids))
    }

    /// Removes the link of one instance. All entities and values stay.
    pub fn detach_prefab_instance(&mut self, any_member: &Guid, origin: Origin) -> Result<Guid, EditError> {
        let root = self.prefab_instance_key(any_member)?;
        let mut links = self.scene.prefab_links.clone();
        links.remove(&root);
        self.apply_linked_batch("detach prefab instance", Vec::new(), links, origin)?;
        Ok(root)
    }

    fn overrides_of(&self, link: &PrefabLink) -> Result<Vec<PrefabOverride>, EditError> {
        let base = self.baseline_entities(link)?;
        let mut out = Vec::new();
        for (source, base_entity) in &base {
            let live_id = &link.guids[source];
            let expected = remap_entity(base_entity, &link.guids)?;
            let Some(live) = self.scene.entities.get(live_id) else {
                out.push(PrefabOverride { entity: live_id.clone(), source: source.clone(), kind: OverrideKind::EntityRemoved });
                continue;
            };
            let at = |kind| PrefabOverride { entity: live_id.clone(), source: source.clone(), kind };
            if live.name != expected.name {
                out.push(at(OverrideKind::Renamed { base: expected.name.clone(), live: live.name.clone() }));
            }
            for (name, base_value) in &expected.components {
                match component(live, name) {
                    None => out.push(at(OverrideKind::ComponentRemoved { component: name.clone(), base: base_value.clone() })),
                    Some(live_value) => {
                        let mut leaves = Vec::new();
                        leaf_diffs(base_value, live_value, "", &mut leaves);
                        out.extend(leaves.into_iter().map(|(path, base, live)| at(OverrideKind::Field { component: name.clone(), path, base, live })));
                    }
                }
            }
            for (name, live_value) in &live.components {
                if component(&expected, name).is_none() {
                    out.push(at(OverrideKind::ComponentAdded { component: name.clone(), live: live_value.clone() }));
                }
            }
        }
        Ok(out)
    }

    /// Every override of one instance.
    pub fn prefab_overrides(&self, any_member: &Guid) -> Result<Vec<PrefabOverride>, EditError> {
        let root = self.prefab_instance_key(any_member)?;
        self.overrides_of(self.generic_link(&root)?)
    }

    /// All generic instances, with their overrides.
    pub fn prefab_instances(&self) -> Result<Vec<PrefabInstanceInfo>, EditError> {
        self.scene
            .prefab_links
            .iter()
            .filter(|(_, link)| link.profile == LinkProfile::GenericV1)
            .map(|(root, link)| {
                Ok(PrefabInstanceInfo {
                    root: root.clone(),
                    source: link.source.clone(),
                    digest: link.digest.clone(),
                    members: link.guids.iter().map(|(s, l)| (s.clone(), l.clone(), self.scene.entities.contains_key(l))).collect(),
                    overrides: self.overrides_of(link)?,
                })
            })
            .collect()
    }

    fn revert_ops(&self, link: &PrefabLink, picked: &[PrefabOverride]) -> Result<Vec<Op>, EditError> {
        let base = self.baseline_entities(link)?;
        let mut spawns = Vec::new();
        let mut rest = Vec::new();
        for o in picked {
            match &o.kind {
                OverrideKind::Field { component, path, base, .. } => rest.push(Op::SetField { guid: o.entity.clone(), component: component.clone(), path: path.clone(), value: base.clone() }),
                OverrideKind::ComponentAdded { component, .. } => rest.push(Op::RemoveComponent { guid: o.entity.clone(), component: component.clone() }),
                OverrideKind::ComponentRemoved { component, base } => rest.push(Op::AddComponent { guid: o.entity.clone(), component: component.clone(), value: Some(base.clone()) }),
                OverrideKind::Renamed { base, .. } => rest.push(Op::Rename { guid: o.entity.clone(), name: base.clone() }),
                OverrideKind::EntityRemoved => {
                    let expected = remap_entity(&base[&o.source], &link.guids)?;
                    spawns.push(Op::SpawnEntity { guid: Some(o.entity.clone()), name: expected.name.clone(), components: Vec::new() });
                    rest.extend(expected.components.into_iter().map(|(component, value)| Op::AddComponent { guid: o.entity.clone(), component, value: Some(value) }));
                }
            }
        }
        spawns.extend(rest);
        Ok(spawns)
    }

    /// Restores prefab values for one field, one component (`path` empty) of a
    /// member entity. A struct path restores every overridden field below it.
    /// Returns how many overrides were reverted; one undo step.
    pub fn revert_prefab_field(&mut self, entity: &Guid, component: &str, path: &str, origin: Origin) -> Result<usize, EditError> {
        let (root, _) = self.prefab_member(entity).ok_or_else(|| invalid(format!("'{entity}' is not part of a prefab instance")))?;
        let link = self.generic_link(&root)?;
        let picked: Vec<PrefabOverride> = self
            .overrides_of(link)?
            .into_iter()
            .filter(|o| {
                o.entity == *entity
                    && match &o.kind {
                        OverrideKind::Field { component: c, path: p, .. } => c == component && under(p, path),
                        OverrideKind::ComponentAdded { component: c, .. } | OverrideKind::ComponentRemoved { component: c, .. } => c == component && path.is_empty(),
                        _ => false,
                    }
            })
            .collect();
        if picked.is_empty() {
            return Err(invalid(format!("{component}.{path} is not overridden")));
        }
        let ops = self.revert_ops(link, &picked)?;
        self.apply_atomic_batch("revert prefab field", ops, origin)?;
        Ok(picked.len())
    }

    /// Restores every prefab value of one instance (fields, components, names and
    /// deleted members). Returns how many overrides were reverted; one undo step.
    pub fn revert_prefab_instance(&mut self, any_member: &Guid, origin: Origin) -> Result<usize, EditError> {
        let root = self.prefab_instance_key(any_member)?;
        let link = self.generic_link(&root)?;
        let picked = self.overrides_of(link)?;
        if picked.is_empty() {
            return Ok(0);
        }
        let ops = self.revert_ops(link, &picked)?;
        self.apply_atomic_batch("revert prefab instance", ops, origin)?;
        Ok(picked.len())
    }

    /// The prefab text that results from writing this instance's overrides into
    /// its baseline. Nothing changes in the document; the caller writes the file
    /// and then calls [`update_prefab_instances`](Self::update_prefab_instances).
    /// An override that refers to an entity outside the instance is refused.
    pub fn prefab_text_with_overrides(&self, any_member: &Guid) -> Result<(String, String), EditError> {
        let root = self.prefab_instance_key(any_member)?;
        let link = self.generic_link(&root)?;
        let mut base = self.baseline_entities(link)?;
        let to_source: BTreeMap<Guid, Guid> = link.guids.iter().map(|(s, l)| (l.clone(), s.clone())).collect();
        let back = |text: &str| Guid::parse(text).ok().and_then(|g| to_source.get(&g)).map(ToString::to_string);
        for o in self.overrides_of(link)? {
            let entity = base.get_mut(&o.source).ok_or_else(|| invalid("override of an unknown member"))?;
            match o.kind {
                OverrideKind::Field { component: name, path, live, .. } => {
                    let value = remap(&live, &back).map_err(|_| invalid(format!("{}.{path} refers to an entity outside the prefab instance; clear it or revert it before applying", name)))?;
                    let slot = entity.components.iter_mut().find(|(n, _)| *n == name).map(|(_, v)| v).ok_or_else(|| invalid("override of a missing component"))?;
                    if !set_at(slot, &parse_path(&path)?, value) {
                        return Err(invalid(format!("cannot write {name}.{path} into the prefab")));
                    }
                }
                OverrideKind::ComponentAdded { component: name, live } => {
                    let value = remap(&live, &back).map_err(|_| invalid(format!("{name} refers to an entity outside the prefab instance; clear it or revert it before applying")))?;
                    entity.components.push((name, value));
                    entity.components.sort_by(|a, b| a.0.cmp(&b.0));
                }
                OverrideKind::ComponentRemoved { component: name, .. } => entity.components.retain(|(n, _)| *n != name),
                OverrideKind::Renamed { live, .. } => entity.name = live,
                OverrideKind::EntityRemoved => {
                    base.remove(&o.source);
                }
            }
        }
        if base.is_empty() {
            return Err(invalid("the instance has no members left; a prefab needs at least one entity"));
        }
        let scene = Scene { entities: base, ..Scene::default() };
        let (text, _) = self.parse_prefab_text(&scene.to_yaml()).map_err(|e| invalid(format!("the changed prefab is not valid: {e}")))?;
        Ok((link.source.clone(), text))
    }

    /// Updates every instance of the prefab `source` to the prefab text `text`.
    /// Non-overridden values follow the prefab; overrides stay. Added and removed
    /// components and entities are applied. All instances change in one undo
    /// step. Conflicts are reported in the result, not raised.
    pub fn update_prefab_instances(&mut self, source: &str, text: &str, origin: Origin) -> Result<PrefabUpdate, EditError> {
        let (new_text, new_entities) = self.parse_prefab_text(text)?;
        let mut links = self.scene.prefab_links.clone();
        let mut report = PrefabUpdate::default();
        // An instance whose members are all deleted is dropped by the next change.
        let before = links.len();
        links.retain(|_, l| l.profile != LinkProfile::GenericV1 || l.guids.values().any(|g| self.scene.entities.contains_key(g)));
        let pruned = before != links.len();
        let mut reserved: BTreeSet<Guid> = self.scene.entities.keys().cloned().collect();
        reserved.extend(links.values().flat_map(|l| l.guids.values().cloned()));
        let mut next = u64::from(self.next_guid);
        let (mut spawns, mut edits, mut removals) = (Vec::new(), Vec::new(), Vec::new());
        for (root, link) in links.iter_mut().filter(|(_, l)| l.profile == LinkProfile::GenericV1 && l.source == source) {
            if link.baseline == new_text {
                report.unchanged += 1;
                continue;
            }
            let old = self.baseline_entities(link)?;
            let mut map = link.guids.clone();
            for src in new_entities.keys().filter(|s| !old.contains_key(*s)) {
                map.insert(src.clone(), allocate(&mut next, &mut reserved)?);
            }
            let targets: BTreeSet<&Guid> = link.guids.values().collect();
            let mut merged: BTreeMap<Guid, Option<SceneEntity>> = BTreeMap::new(); // live id -> final entity
            let mut dropped: Vec<Guid> = Vec::new(); // source ids that leave the instance
            let mut doomed: Vec<Guid> = Vec::new(); // live ids to delete
            for src in old.keys().chain(new_entities.keys().filter(|s| !old.contains_key(*s))) {
                let live_id = map[src].clone();
                let live = self.scene.entities.get(&live_id);
                let at = |what: &str| format!("{}: {what}", live.and_then(|e| e.name.clone()).unwrap_or_else(|| live_id.to_string()));
                match (old.get(src), new_entities.get(src), live) {
                    (None, Some(n), _) => {
                        // New in the prefab.
                        merged.insert(live_id.clone(), Some(remap_entity(n, &map)?));
                        report.spawned += 1;
                    }
                    (Some(_), None, None) => dropped.push(src.clone()),
                    (Some(b), None, Some(l)) => {
                        if *l == remap_entity(b, &map)? {
                            doomed.push(live_id.clone());
                            dropped.push(src.clone());
                            merged.insert(live_id.clone(), None);
                        } else {
                            report.conflicts.push(at("the prefab removed this entity, but the instance changed it; it stays as a normal entity"));
                            dropped.push(src.clone());
                        }
                    }
                    (Some(_), Some(_), None) => {} // the instance deleted it; it stays deleted
                    (Some(b), Some(n), Some(l)) => {
                        let (b, n) = (remap_entity(b, &map)?, remap_entity(n, &map)?);
                        let mut conflicts = Vec::new();
                        let name = if l.name == b.name { n.name.clone() } else { l.name.clone() };
                        let mut components: Vec<(String, Value)> = Vec::new();
                        for (c, nv) in &n.components {
                            match (component(&b, c), component(l, c)) {
                                (Some(bv), Some(lv)) => components.push((c.clone(), merge3(bv, nv, lv, c, &mut conflicts))),
                                (Some(_), None) => {}
                                (None, Some(lv)) => {
                                    if lv != nv {
                                        conflicts.push(format!("{c}: the prefab added it, but the instance already has another one; the instance value was kept"));
                                    }
                                    components.push((c.clone(), lv.clone()));
                                }
                                (None, None) => components.push((c.clone(), nv.clone())),
                            }
                        }
                        for (c, bv) in &b.components {
                            if component(&n, c).is_some() {
                                continue;
                            }
                            if let Some(lv) = component(l, c) {
                                if lv == bv {
                                    continue; // removed by the prefab, not overridden
                                }
                                conflicts.push(format!("{c}: the prefab removed it, but the instance changed it; the component was kept"));
                                components.push((c.clone(), lv.clone()));
                            }
                        }
                        for (c, lv) in &l.components {
                            if component(&b, c).is_none() && component(&n, c).is_none() {
                                components.push((c.clone(), lv.clone()));
                            }
                        }
                        components.sort_by(|x, y| x.0.cmp(&y.0));
                        report.conflicts.extend(conflicts.into_iter().map(|c| at(&c)));
                        merged.insert(live_id.clone(), Some(SceneEntity { name, components }));
                    }
                    (None, None, _) => unreachable!("a source id comes from one of the two prefabs"),
                }
            }
            // A deletion that something else still refers to cannot be applied.
            for gone in doomed.clone() {
                let text = gone.as_str();
                let by_outside = self.scene.entities.iter().any(|(id, e)| !targets.contains(id) && e.components.iter().any(|(_, v)| refers_to(v, text)))
                    || self.scene.singletons.iter().any(|(_, v)| refers_to(v, text));
                let by_member = merged.values().flatten().any(|e| e.components.iter().any(|(_, v)| refers_to(v, text)));
                if by_outside || by_member {
                    let src = map.iter().find(|(_, l)| **l == gone).map(|(s, _)| s.clone()).expect("mapped");
                    report.conflicts.push(format!("{gone}: the prefab removed this entity, but something still refers to it; it stays as a normal entity"));
                    merged.remove(&gone);
                    doomed.retain(|g| *g != gone);
                    dropped.push(src);
                }
            }
            // Ops: compare each final entity with the live one.
            for (live_id, entity) in &merged {
                match (entity, self.scene.entities.get(live_id)) {
                    (Some(e), None) => {
                        spawns.push(Op::SpawnEntity { guid: Some(live_id.clone()), name: e.name.clone(), components: Vec::new() });
                        edits.extend(e.components.iter().map(|(c, v)| Op::AddComponent { guid: live_id.clone(), component: c.clone(), value: Some(v.clone()) }));
                    }
                    (Some(e), Some(l)) => {
                        if e.name != l.name {
                            edits.push(Op::Rename { guid: live_id.clone(), name: e.name.clone() });
                        }
                        for (c, v) in &e.components {
                            match component(l, c) {
                                None => edits.push(Op::AddComponent { guid: live_id.clone(), component: c.clone(), value: Some(v.clone()) }),
                                Some(old_value) => {
                                    let mut leaves = Vec::new();
                                    leaf_diffs(old_value, v, "", &mut leaves);
                                    edits.extend(leaves.into_iter().map(|(path, _, value)| Op::SetField { guid: live_id.clone(), component: c.clone(), path, value }));
                                }
                            }
                        }
                        for (c, _) in &l.components {
                            if component(e, c).is_none() {
                                edits.push(Op::RemoveComponent { guid: live_id.clone(), component: c.clone() });
                            }
                        }
                    }
                    (None, Some(_)) => {} // despawned below, referrers first
                    (None, None) => {}
                }
            }
            // A despawn is refused while another entity refers to the target, so
            // members that refer to other doomed members go first.
            let mut pending: Vec<&Guid> = merged.iter().filter(|(id, e)| e.is_none() && self.scene.entities.contains_key(*id)).map(|(id, _)| id).collect();
            while !pending.is_empty() {
                let referred = |g: &Guid| pending.iter().any(|o| *o != g && self.scene.entities[*o].components.iter().any(|(_, v)| refers_to(v, g.as_str())));
                let at = pending.iter().position(|g| !referred(g)).unwrap_or(0);
                removals.push(Op::DespawnEntity { guid: pending.remove(at).clone() });
            }
            report.removed += doomed.len();
            for src in dropped {
                map.remove(&src);
            }
            if map.is_empty() {
                return Err(invalid("the updated prefab would leave an instance without members"));
            }
            link.baseline = new_text.clone();
            link.digest = PrefabLink::canonical_digest(&new_text);
            link.guids = map;
            report.updated.push(root.clone());
        }
        // Re-key instances whose root member left.
        let rekeyed: Vec<(Guid, Guid)> = links.iter().filter(|(root, l)| !l.guids.values().any(|g| g == *root)).map(|(root, l)| (root.clone(), l.guids.values().next().expect("nonempty").clone())).collect();
        for (old_key, new_key) in rekeyed {
            let link = links.remove(&old_key).expect("present");
            links.insert(new_key.clone(), link);
            for key in report.updated.iter_mut().filter(|k| **k == old_key) {
                *key = new_key.clone();
            }
        }
        if report.updated.is_empty() && !pruned {
            return Ok(report);
        }
        spawns.extend(edits);
        spawns.extend(removals);
        self.apply_linked_batch("update prefab instances", spawns, links, origin)?;
        Ok(report)
    }
}
