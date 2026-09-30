//! Scene to `Frame` ("bake") and back ("unbake").
//!
//! # Bake
//!
//! 1. Entities are created in ascending GUID order (byte order of the text),
//!    so the same scene always gives the same entity indices and versions.
//! 2. For each entity, components are added in ascending name order. Every
//!    component starts from the default bytes of its type. The scene fields
//!    then overwrite the visible fields, and fields hidden from reflection
//!    keep their defaults. Component stores therefore fill in entity order.
//! 3. Entity references (GUID text) become `Entity` handles.
//! 4. Singletons are written over the current singleton value of the frame,
//!    in ascending name order.
//! 5. Then the init hook of every registered singleton runs, whether the
//!    scene had a value for it or not (for example to allocate `FrameList`s).
//!
//! Bake into a fresh frame. Entities that already exist are not touched.
//!
//! # The GUID map
//!
//! The link between GUIDs and entities is the returned [`SceneIndex`]. It is a
//! side table that the loader (or the editor) owns. It is **not** part of the
//! `Frame`: the `Frame` stays small and identical on every peer, and games
//! that never load scene files need nothing. The index uses `BTreeMap`s, so
//! iterating it is deterministic. To save a play-mode state back to a
//! scene, pass the index to [`unbake`](super::Scene::unbake).
//!
//! # Unbake
//!
//! Every live entity is written, including ones without components. Entities
//! that the index does not know (spawned while playing) get a GUID made from
//! their index and version, moved to the next free value on a clash. That
//! makes a saved scene the same for the same frame. A reference to an entity
//! that no longer exists is written as `null`. Fields hidden from reflection
//! (sleep state of a body, `FrameList` handles) are not saved, so a baked scene
//! starts them from their defaults.

use std::collections::{BTreeMap, BTreeSet};

use orr_ecs::{Entity, Frame};

use super::decode::map_entities;
use super::{Guid, Scene, SceneEntity};
use crate::registry::{TypeKind, TypeRegistry};
use crate::value::{ReflectError, Value};

/// GUID to entity and back, plus entity names. See the module docs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneIndex {
    by_guid: BTreeMap<Guid, Entity>,
    by_entity: BTreeMap<Entity, Guid>,
    names: BTreeMap<Guid, String>,
}

impl SceneIndex {
    /// The entity of a GUID.
    pub fn entity(&self, guid: &Guid) -> Option<Entity> {
        self.by_guid.get(guid).copied()
    }
    /// The GUID of an entity.
    pub fn guid(&self, e: Entity) -> Option<&Guid> {
        self.by_entity.get(&e)
    }
    /// The display name of an entity.
    pub fn name(&self, guid: &Guid) -> Option<&str> {
        self.names.get(guid).map(String::as_str)
    }
    /// Sets or clears the display name of an entity.
    pub fn set_name(&mut self, guid: &Guid, name: Option<String>) {
        match name {
            Some(n) => self.names.insert(guid.clone(), n),
            None => self.names.remove(guid),
        };
    }
    /// Adds a link (for an entity created after the bake).
    pub fn insert(&mut self, guid: Guid, e: Entity) {
        self.by_entity.insert(e, guid.clone());
        self.by_guid.insert(guid, e);
    }
    /// All links in GUID order.
    pub fn iter(&self) -> impl Iterator<Item = (&Guid, Entity)> {
        self.by_guid.iter().map(|(g, e)| (g, *e))
    }
    /// Number of linked entities.
    pub fn len(&self) -> usize {
        self.by_guid.len()
    }
    /// True if nothing is linked.
    pub fn is_empty(&self) -> bool {
        self.by_guid.is_empty()
    }
}

/// Why baking or unbaking failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BakeError {
    /// The scene or registry names a type the `Frame` does not have.
    NotInFrame(String),
    /// The scene names a type the registry does not know.
    UnknownType(String),
    /// A value could not be written (the scene was not validated by `parse`).
    Value {
        /// Where.
        at: String,
        /// Why.
        error: ReflectError,
    },
    /// An entity reference names a GUID that is not in the scene.
    MissingEntity(String),
    /// A value in the frame cannot be written to a file.
    Unsaveable {
        /// Where.
        at: String,
        /// Why.
        error: ReflectError,
    },
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BakeError::NotInFrame(n) => write!(f, "type '{n}' is not registered in the frame"),
            BakeError::UnknownType(n) => write!(f, "unknown type '{n}'"),
            BakeError::Value { at, error } => write!(f, "{at}: {error}"),
            BakeError::MissingEntity(g) => write!(f, "reference to missing entity '{g}'"),
            BakeError::Unsaveable { at, error } => write!(f, "cannot save {at}: {error}"),
        }
    }
}

impl std::error::Error for BakeError {}

pub(super) fn bake(scene: &Scene, reg: &TypeRegistry, frame: &mut Frame) -> Result<SceneIndex, BakeError> {
    // Check every type first, so a failure leaves the frame untouched.
    for (name, _) in &scene.singletons {
        let t = reg.get(name).filter(|t| t.kind() == TypeKind::Singleton).ok_or_else(|| BakeError::UnknownType(name.clone()))?;
        if !reg.in_frame(t, frame) {
            return Err(BakeError::NotInFrame(name.clone()));
        }
    }
    for ent in scene.entities.values() {
        for (name, _) in &ent.components {
            let t = reg.get(name).filter(|t| t.kind() == TypeKind::Component).ok_or_else(|| BakeError::UnknownType(name.clone()))?;
            if !reg.in_frame(t, frame) {
                return Err(BakeError::NotInFrame(name.clone()));
            }
        }
    }
    for g in scene.entities.values().flat_map(|e| e.components.iter()).flat_map(|(_, v)| entity_refs(v)) {
        if !scene.entities.contains_key(&Guid(g.clone())) {
            return Err(BakeError::MissingEntity(g));
        }
    }

    let mut index = SceneIndex::default();
    for (guid, ent) in &scene.entities {
        let e = frame.spawn();
        index.insert(guid.clone(), e);
        if let Some(n) = &ent.name {
            index.set_name(guid, Some(n.clone()));
        }
    }

    for (guid, ent) in &scene.entities {
        let e = index.by_guid[guid];
        for (name, value) in &ent.components {
            let t = reg.get(name).expect("checked above");
            let resolved = resolve(value, &index);
            let mut bytes = t.default_bytes();
            t.desc().write(&mut bytes, &resolved).map_err(|error| BakeError::Value { at: format!("{guid}.{name}"), error })?;
            reg.insert_bytes(t, frame, e, &bytes);
        }
    }

    let mut present: BTreeSet<&str> = BTreeSet::new();
    for (name, value) in &scene.singletons {
        let t = reg.get(name).expect("checked above");
        let resolved = resolve(value, &index);
        let bytes = reg.singleton_bytes_mut_of(t, frame);
        t.desc().write(bytes, &resolved).map_err(|error| BakeError::Value { at: format!("singleton {name}"), error })?;
        present.insert(name.as_str());
    }
    for t in reg.singletons() {
        if !reg.in_frame(t, frame) {
            continue;
        }
        if let Some(hook) = t.init_hook() {
            hook(frame, present.contains(t.name()));
        }
    }
    Ok(index)
}

fn resolve(v: &Value, index: &SceneIndex) -> Value {
    map_entities(v, &mut |g| match g {
        None => Entity::NONE,
        Some(g) => index.by_guid.get(&Guid(g.to_string())).copied().unwrap_or(Entity::NONE),
    })
}

fn entity_refs(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::EntityGuid(Some(g)) => out.push(g.clone()),
            Value::Array(items) => items.iter().for_each(|x| walk(x, out)),
            Value::Struct(f) | Value::Variant(_, f) => f.iter().for_each(|(_, x)| walk(x, out)),
            _ => {}
        }
    }
    walk(v, &mut out);
    out
}

/// A GUID for an entity the index does not know: a mix of index and version.
fn generated_guid(e: Entity, used: &BTreeSet<Guid>) -> Guid {
    let mut x = (u64::from(e.index) << 32) | u64::from(e.version);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    let mut n = (x & 0xffff_ffff) as u32;
    loop {
        let g = Guid::from_u32(n);
        if !used.contains(&g) {
            return g;
        }
        n = n.wrapping_add(1);
    }
}

pub(super) fn unbake(reg: &TypeRegistry, frame: &Frame, index: Option<&SceneIndex>) -> Result<Scene, BakeError> {
    let live: Vec<Entity> = frame.entities().collect();
    let mut used: BTreeSet<Guid> = BTreeSet::new();
    let mut map: BTreeMap<Entity, Guid> = BTreeMap::new();
    if let Some(ix) = index {
        for &e in &live {
            if let Some(g) = ix.guid(e) {
                used.insert(g.clone());
                map.insert(e, g.clone());
            }
        }
    }
    for &e in &live {
        if !map.contains_key(&e) {
            let g = generated_guid(e, &used);
            used.insert(g.clone());
            map.insert(e, g);
        }
    }

    let to_guid = |v: &Value| -> Value { to_guid_refs(v, &map) };
    let mut scene = Scene::default();
    for t in reg.singletons() {
        if !reg.in_frame(t, frame) {
            continue;
        }
        let Some(bytes) = reg.singleton_bytes_of(t, frame) else { continue };
        let v = t.desc().read(bytes);
        t.desc().check(&v, false).map_err(|error| BakeError::Unsaveable { at: format!("singleton {}", t.name()), error })?;
        scene.singletons.push((t.name().to_string(), to_guid(&v)));
    }
    for &e in &live {
        let guid = map[&e].clone();
        let mut ent = SceneEntity { name: index.and_then(|ix| ix.name(&guid)).map(str::to_string), components: Vec::new() };
        for t in reg.components() {
            let Some(bytes) = reg.bytes_of(t, frame, e) else { continue };
            let v = t.desc().read(bytes);
            t.desc().check(&v, false).map_err(|error| BakeError::Unsaveable { at: format!("{guid}.{}", t.name()), error })?;
            ent.components.push((t.name().to_string(), to_guid(&v)));
        }
        scene.entities.insert(guid, ent);
    }
    Ok(scene)
}

fn to_guid_refs(v: &Value, map: &BTreeMap<Entity, Guid>) -> Value {
    match v {
        Value::Entity(e) => Value::EntityGuid(map.get(e).map(|g| g.as_str().to_string())),
        Value::Array(items) => Value::Array(items.iter().map(|x| to_guid_refs(x, map)).collect()),
        Value::Struct(f) => Value::Struct(f.iter().map(|(n, x)| (n.clone(), to_guid_refs(x, map))).collect()),
        Value::Variant(name, f) => Value::Variant(name.clone(), f.iter().map(|(n, x)| (n.clone(), to_guid_refs(x, map))).collect()),
        other => other.clone(),
    }
}
