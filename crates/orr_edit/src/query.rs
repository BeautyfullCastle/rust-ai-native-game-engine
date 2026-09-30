//! Read-only queries over a frame, the same in edit mode and play mode.

use orr_ecs::{Entity, Frame};
use orr_reflect::{Guid, SceneIndex, TypeKind, TypeRegistry, Value};

use crate::error::EditError;
use crate::refs::map_refs;

/// Names an entity: by scene GUID, or by frame handle (for entities that
/// have no GUID, such as ones spawned during play).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A scene entity.
    Guid(Guid),
    /// A frame entity handle (in edit mode, of the preview frame).
    Entity(Entity),
}

impl Target {
    pub(crate) fn describe(&self) -> String {
        match self {
            Target::Guid(g) => g.to_string(),
            Target::Entity(e) => format!("entity {}v{}", e.index, e.version),
        }
    }
}

impl From<Guid> for Target {
    fn from(g: Guid) -> Self {
        Target::Guid(g)
    }
}
impl From<Entity> for Target {
    fn from(e: Entity) -> Self {
        Target::Entity(e)
    }
}

/// One entity in a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityInfo {
    /// The scene GUID; `None` for an entity created during play.
    pub guid: Option<Guid>,
    /// The handle in the frame this view reads.
    pub entity: Entity,
    /// Display name (from the scene).
    pub name: Option<String>,
    /// Reflected component type names the entity has, sorted.
    pub components: Vec<String>,
}

/// A read-only window on a frame plus its GUID map. Get one from
/// [`EditorDoc::view`](crate::EditorDoc::view) or
/// [`PlayController::view`](crate::PlayController::view).
///
/// Values come back in scene form: an entity reference is
/// `Value::EntityGuid` when the entity has a GUID, `Value::EntityGuid(None)`
/// for no entity, and `Value::Entity` only for an entity without a GUID.
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub(crate) types: &'a TypeRegistry,
    pub(crate) frame: &'a Frame,
    pub(crate) index: &'a SceneIndex,
}

impl<'a> View<'a> {
    /// The frame being read (for rendering).
    pub fn frame(&self) -> &'a Frame {
        self.frame
    }
    /// The type registry.
    pub fn types(&self) -> &'a TypeRegistry {
        self.types
    }
    /// The GUID map of the scene this frame was baked from.
    pub fn index(&self) -> &'a SceneIndex {
        self.index
    }
    /// Frame tick (0 in edit mode).
    pub fn tick(&self) -> u64 {
        self.frame.tick()
    }
    /// Checksum of the frame.
    pub fn checksum(&self) -> u64 {
        self.frame.checksum()
    }

    /// The GUID of a frame entity, if it has one.
    pub fn guid_of(&self, e: Entity) -> Option<&'a Guid> {
        self.index.guid(e)
    }
    /// The live frame entity of a GUID.
    pub fn entity_of(&self, guid: &Guid) -> Option<Entity> {
        self.index.entity(guid).filter(|&e| self.frame.exists(e))
    }

    /// Finds the live frame entity for a target.
    pub fn resolve(&self, t: &Target) -> Result<Entity, EditError> {
        let e = match t {
            Target::Guid(g) => self.index.entity(g),
            Target::Entity(e) => Some(*e),
        };
        e.filter(|&e| self.frame.exists(e)).ok_or_else(|| EditError::UnknownEntity(t.describe()))
    }

    fn info_of(&self, e: Entity) -> EntityInfo {
        let guid = self.index.guid(e).cloned();
        let name = guid.as_ref().and_then(|g| self.index.name(g)).map(str::to_string);
        EntityInfo {
            guid,
            entity: e,
            name,
            components: self.types.component_names(self.frame, e).into_iter().map(str::to_string).collect(),
        }
    }

    /// Every live entity, in frame order.
    pub fn entities(&self) -> Vec<EntityInfo> {
        self.frame.entities().map(|e| self.info_of(e)).collect()
    }

    /// One entity.
    pub fn entity(&self, t: &Target) -> Result<EntityInfo, EditError> {
        Ok(self.info_of(self.resolve(t)?))
    }

    /// Frame form of a value to scene form (see the type docs).
    pub(crate) fn scene_form(&self, v: &Value) -> Value {
        map_refs(v, &mut |r| {
            Ok(match r {
                Value::Entity(e) if *e == Entity::NONE => Value::EntityGuid(None),
                Value::Entity(e) => match self.index.guid(*e) {
                    Some(g) => Value::EntityGuid(Some(g.to_string())),
                    None => r.clone(),
                },
                other => other.clone(),
            })
        })
        .expect("mapping to scene form cannot fail")
    }

    /// All reflected components of an entity as `(type name, value)`, sorted by name.
    pub fn components(&self, t: &Target) -> Result<Vec<(String, Value)>, EditError> {
        let e = self.resolve(t)?;
        let mut out = Vec::new();
        for name in self.types.component_names(self.frame, e) {
            out.push((name.to_string(), self.scene_form(&self.types.read_component(self.frame, e, name)?)));
        }
        Ok(out)
    }

    /// One component as a whole struct value.
    pub fn component(&self, t: &Target, name: &str) -> Result<Value, EditError> {
        self.field(t, name, "")
    }

    /// One field of a component (`path` like `"pos.x"`, `""` = whole).
    pub fn field(&self, t: &Target, name: &str, path: &str) -> Result<Value, EditError> {
        let e = self.resolve(t)?;
        if !self.types.has_component(self.frame, e, name)? {
            return Err(EditError::NoComponent { entity: t.describe(), component: name.to_string() });
        }
        Ok(self.scene_form(&self.types.get_field(self.frame, e, name, path)?))
    }

    /// Every reflected singleton the frame has, as `(type name, value)`.
    pub fn singletons(&self) -> Vec<(String, Value)> {
        self.types
            .singletons()
            .filter_map(|t| {
                let v = self.types.read_singleton(self.frame, t.name()).ok()?;
                Some((t.name().to_string(), self.scene_form(&v)))
            })
            .collect()
    }

    /// One singleton (or one field of it with `path`).
    pub fn singleton(&self, name: &str, path: &str) -> Result<Value, EditError> {
        if self.types.get(name).is_none_or(|t| t.kind() != TypeKind::Singleton) {
            return Err(EditError::UnknownType(name.to_string()));
        }
        Ok(self.scene_form(&self.types.get_singleton_field(self.frame, name, path)?))
    }

    /// The JSON Schema (draft 2020-12) of scene files for this registry.
    pub fn json_schema(&self) -> String {
        self.types.json_schema()
    }

    /// The JSON Schema of one component or singleton.
    pub fn type_schema(&self, name: &str) -> Option<String> {
        self.types.type_json_schema(name)
    }
}
