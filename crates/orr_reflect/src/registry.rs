//! The type registry: reflection descriptors plus type-erased access to a `Frame`.

use std::collections::BTreeMap;

use orr_ecs::{ComponentRegistry, Entity, Frame};

use crate::desc::{FieldDesc, Kind, TypeDesc};
use crate::reflect::Reflect;
use crate::value::{parse_path, ReflectError, Value};

/// Whether a type is a per-entity component or a singleton.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TypeKind {
    /// One value per entity that has it.
    Component,
    /// One value per `Frame`.
    Singleton,
}

/// Runs after a scene is baked into a `Frame`, once per registered singleton.
/// The flag says whether the scene contained a value for the singleton. Use
/// it to allocate `FrameList`s or to fill in defaults.
pub type SingletonInit = fn(&mut Frame, bool);

#[derive(Clone, Copy)]
struct Ops {
    /// Is the type registered in the frame's `ComponentRegistry`?
    in_frame: fn(&Frame) -> bool,
    /// Name the frame's `ComponentRegistry` uses for the type, if registered.
    ecs_name: fn(&ComponentRegistry) -> Option<&'static str>,
    has: fn(&Frame, Entity) -> bool,
    bytes: for<'a> fn(&'a Frame, Entity) -> Option<&'a [u8]>,
    bytes_mut: for<'a> fn(&'a mut Frame, Entity) -> Option<&'a mut [u8]>,
    insert: fn(&mut Frame, Entity, &[u8]),
    remove: fn(&mut Frame, Entity) -> bool,
    entities: fn(&Frame) -> Vec<Entity>,
    singleton_bytes: for<'a> fn(&'a Frame) -> &'a [u8],
    singleton_bytes_mut: for<'a> fn(&'a mut Frame) -> &'a mut [u8],
    default_bytes: fn() -> Vec<u8>,
}

/// One registered type.
pub struct TypeInfo {
    name: &'static str,
    kind: TypeKind,
    desc: TypeDesc,
    size: usize,
    init: Option<SingletonInit>,
    ops: Ops,
}

impl TypeInfo {
    /// The stable name (`"Transform"`, `"orr_physics::Body"`).
    pub fn name(&self) -> &'static str {
        self.name
    }
    /// Component or singleton.
    pub fn kind(&self) -> TypeKind {
        self.kind
    }
    /// The descriptor.
    pub fn desc(&self) -> &TypeDesc {
        &self.desc
    }
    /// Documentation of the type.
    pub fn doc(&self) -> &str {
        &self.desc.doc
    }
    /// Size of the Rust type in bytes.
    pub fn size(&self) -> usize {
        self.size
    }
    /// The visible fields, if the type is a struct layout. A tagged view has
    /// none (its fields depend on the variant).
    pub fn fields(&self) -> &[FieldDesc] {
        match &self.desc.kind {
            Kind::Struct { fields, .. } => fields,
            _ => &[],
        }
    }
    /// The bytes a new value starts with.
    pub fn default_bytes(&self) -> Vec<u8> {
        (self.ops.default_bytes)()
    }
    /// Reads the whole value from the bytes of a component.
    pub fn read(&self, bytes: &[u8]) -> Value {
        self.desc.read(bytes)
    }
    /// Reads the field at `path` (`"pos.x"`, `"kills[3]"`, `""` for all).
    pub fn get(&self, bytes: &[u8], path: &str) -> Result<Value, ReflectError> {
        self.desc.get(bytes, &parse_path(path)?)
    }
    /// Writes the field at `path` after checking type and range.
    pub fn set(&self, bytes: &mut [u8], path: &str, value: Value) -> Result<(), ReflectError> {
        self.desc.set(bytes, &parse_path(path)?, value)
    }
    /// The init hook of a singleton.
    pub fn init_hook(&self) -> Option<SingletonInit> {
        self.init
    }
}

/// All types an editor and a scene file can name.
///
/// Types are kept sorted by name, so every listing, schema and scene file
/// has the same order on every machine.
#[derive(Default)]
pub struct TypeRegistry {
    types: BTreeMap<&'static str, TypeInfo>,
}

/// Names that a component or singleton cannot use: they are keys of the scene format.
const RESERVED: [&str; 3] = ["name", "schema", "entities"];

impl TypeRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    fn add<T: Reflect>(&mut self, name: &'static str, kind: TypeKind, init: Option<SingletonInit>, ops: Ops) {
        assert!(
            !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b':'),
            "orr_reflect: type name {name:?} must use letters, digits, '_' and ':' only"
        );
        assert!(!RESERVED.contains(&name), "orr_reflect: '{name}' is a reserved scene key");
        assert!(!self.types.contains_key(name), "orr_reflect: type name '{name}' registered twice");
        let desc = T::describe();
        if let Err(e) = desc.check_layout(core::mem::size_of::<T>()) {
            panic!("orr_reflect: bad descriptor for '{name}': {e}");
        }
        self.types.insert(name, TypeInfo { name, kind, desc, size: core::mem::size_of::<T>(), init, ops });
    }

    /// Registers `T` as a component under a stable name. The type must also
    /// be registered in the `ComponentRegistry` of any `Frame` you use it with.
    pub fn register_component<T: Reflect>(&mut self, name: &'static str) -> &mut Self {
        let ops = Ops {
            in_frame: |f| f.registry().component_id::<T>().is_some(),
            ecs_name: |r| r.component_id::<T>().map(|id| r.component_name(id)),
            has: |f, e| f.has::<T>(e),
            bytes: |f, e| f.get::<T>(e).map(bytemuck::bytes_of),
            bytes_mut: |f, e| f.get_mut::<T>(e).map(bytemuck::bytes_of_mut),
            insert: |f, e, b| {
                f.add::<T>(e, bytemuck::pod_read_unaligned(b));
            },
            remove: |f, e| f.remove::<T>(e).is_some(),
            entities: |f| f.dense::<T>().0.to_vec(),
            singleton_bytes: |_| &[],
            singleton_bytes_mut: |_| &mut [],
            default_bytes: || bytemuck::bytes_of(&T::default_value()).to_vec(),
        };
        self.add::<T>(name, TypeKind::Component, None, ops);
        self
    }

    /// Registers `T` as a singleton under a stable name.
    pub fn register_singleton<T: Reflect>(&mut self, name: &'static str) -> &mut Self {
        self.register_singleton_inner::<T>(name, None)
    }

    /// Registers a singleton with a hook that runs after a scene is baked
    /// (see [`SingletonInit`]).
    pub fn register_singleton_with_init<T: Reflect>(&mut self, name: &'static str, init: SingletonInit) -> &mut Self {
        self.register_singleton_inner::<T>(name, Some(init))
    }

    fn register_singleton_inner<T: Reflect>(&mut self, name: &'static str, init: Option<SingletonInit>) -> &mut Self {
        let ops = Ops {
            in_frame: |f| f.registry().singleton_id::<T>().is_some(),
            ecs_name: |r| r.singleton_id::<T>().map(|id| r.singleton_name(id)),
            has: |_, _| false,
            bytes: |_, _| None,
            bytes_mut: |_, _| None,
            insert: |_, _, _| {},
            remove: |_, _| false,
            entities: |_| Vec::new(),
            singleton_bytes: |f| bytemuck::bytes_of(f.singleton::<T>()),
            singleton_bytes_mut: |f| bytemuck::bytes_of_mut(f.singleton_mut::<T>()),
            default_bytes: || bytemuck::bytes_of(&T::default_value()).to_vec(),
        };
        self.add::<T>(name, TypeKind::Singleton, init, ops);
        self
    }

    // ---- listing ----

    /// Every type, sorted by name.
    pub fn types(&self) -> impl Iterator<Item = &TypeInfo> {
        self.types.values()
    }
    /// Every component type, sorted by name.
    pub fn components(&self) -> impl Iterator<Item = &TypeInfo> {
        self.types.values().filter(|t| t.kind == TypeKind::Component)
    }
    /// Every singleton type, sorted by name.
    pub fn singletons(&self) -> impl Iterator<Item = &TypeInfo> {
        self.types.values().filter(|t| t.kind == TypeKind::Singleton)
    }
    /// A type by name.
    pub fn get(&self, name: &str) -> Option<&TypeInfo> {
        self.types.get(name)
    }

    fn info(&self, name: &str, kind: TypeKind) -> Result<&TypeInfo, ReflectError> {
        let t = self.types.get(name).ok_or_else(|| ReflectError::UnknownType(name.to_string()))?;
        if t.kind != kind {
            return Err(ReflectError::WrongTypeKind(name.to_string()));
        }
        Ok(t)
    }

    fn comp<'a>(&'a self, frame: &Frame, name: &str) -> Result<&'a TypeInfo, ReflectError> {
        let t = self.info(name, TypeKind::Component)?;
        if !(t.ops.in_frame)(frame) {
            return Err(ReflectError::NotInFrame(name.to_string()));
        }
        Ok(t)
    }

    fn single<'a>(&'a self, frame: &Frame, name: &str) -> Result<&'a TypeInfo, ReflectError> {
        let t = self.info(name, TypeKind::Singleton)?;
        if !(t.ops.in_frame)(frame) {
            return Err(ReflectError::NotInFrame(name.to_string()));
        }
        Ok(t)
    }

    /// Lists mismatches between this registry and the `ComponentRegistry` of
    /// a frame: types missing from the frame, or registered there under a
    /// different name. Empty means they agree.
    pub fn check_against(&self, registry: &ComponentRegistry) -> Vec<String> {
        let mut out = Vec::new();
        for t in self.types.values() {
            match (t.ops.ecs_name)(registry) {
                None => out.push(format!("'{}' is not registered in the frame", t.name)),
                Some(n) if n != t.name => out.push(format!("'{}' is registered in the frame as '{n}'", t.name)),
                Some(_) => {}
            }
        }
        out
    }

    // ---- components on a frame ----

    /// Names of the reflected components that `e` has, sorted.
    pub fn component_names(&self, frame: &Frame, e: Entity) -> Vec<&'static str> {
        self.components().filter(|t| (t.ops.in_frame)(frame) && (t.ops.has)(frame, e)).map(|t| t.name).collect()
    }

    /// True if `e` has the component.
    pub fn has_component(&self, frame: &Frame, e: Entity, name: &str) -> Result<bool, ReflectError> {
        let t = self.comp(frame, name)?;
        Ok(frame.exists(e) && (t.ops.has)(frame, e))
    }

    /// Entities that have the component, in the frame's dense order.
    pub fn entities_with(&self, frame: &Frame, name: &str) -> Result<Vec<Entity>, ReflectError> {
        let t = self.comp(frame, name)?;
        Ok((t.ops.entities)(frame))
    }

    /// Adds the component with its default value. Fails if `e` already has it.
    pub fn add_component(&self, frame: &mut Frame, e: Entity, name: &str) -> Result<(), ReflectError> {
        let t = self.comp(frame, name)?;
        if !frame.exists(e) {
            return Err(ReflectError::NoEntity);
        }
        if (t.ops.has)(frame, e) {
            return Err(ReflectError::Invalid(format!("the entity already has '{name}'")));
        }
        let bytes = t.default_bytes();
        (t.ops.insert)(frame, e, &bytes);
        Ok(())
    }

    /// Adds the component from a whole struct value (all visible fields).
    /// Fields hidden from reflection get their default. Replaces an existing component.
    pub fn add_component_value(&self, frame: &mut Frame, e: Entity, name: &str, value: &Value) -> Result<(), ReflectError> {
        let t = self.comp(frame, name)?;
        if !frame.exists(e) {
            return Err(ReflectError::NoEntity);
        }
        let mut bytes = t.default_bytes();
        t.desc.write(&mut bytes, value)?;
        (t.ops.insert)(frame, e, &bytes);
        Ok(())
    }

    /// Removes the component. Returns whether `e` had it.
    pub fn remove_component(&self, frame: &mut Frame, e: Entity, name: &str) -> Result<bool, ReflectError> {
        let t = self.comp(frame, name)?;
        if !frame.exists(e) {
            return Err(ReflectError::NoEntity);
        }
        Ok((t.ops.remove)(frame, e))
    }

    /// Reads the whole component as a struct value.
    pub fn read_component(&self, frame: &Frame, e: Entity, name: &str) -> Result<Value, ReflectError> {
        self.get_field(frame, e, name, "")
    }

    /// Reads one field of a component (`path` like `"pos.x"`; `""` = all).
    pub fn get_field(&self, frame: &Frame, e: Entity, name: &str, path: &str) -> Result<Value, ReflectError> {
        let t = self.comp(frame, name)?;
        if !frame.exists(e) {
            return Err(ReflectError::NoEntity);
        }
        let bytes = (t.ops.bytes)(frame, e).ok_or_else(|| ReflectError::NoComponent(name.to_string()))?;
        t.get(bytes, path)
    }

    /// Writes one field of a component after checking type and range.
    ///
    /// This changes the `Frame` directly. In a running simulation the editor
    /// must send a debug command instead, or the change is not part of the
    /// input log and the replay diverges.
    pub fn set_field(&self, frame: &mut Frame, e: Entity, name: &str, path: &str, value: Value) -> Result<(), ReflectError> {
        let t = self.comp(frame, name)?;
        if !frame.exists(e) {
            return Err(ReflectError::NoEntity);
        }
        let bytes = (t.ops.bytes_mut)(frame, e).ok_or_else(|| ReflectError::NoComponent(name.to_string()))?;
        t.set(bytes, path, value)
    }

    // ---- singletons on a frame ----

    /// Reads a whole singleton.
    pub fn read_singleton(&self, frame: &Frame, name: &str) -> Result<Value, ReflectError> {
        self.get_singleton_field(frame, name, "")
    }

    /// Reads one field of a singleton.
    pub fn get_singleton_field(&self, frame: &Frame, name: &str, path: &str) -> Result<Value, ReflectError> {
        let t = self.single(frame, name)?;
        t.get((t.ops.singleton_bytes)(frame), path)
    }

    /// Writes one field of a singleton (same caveat as [`set_field`](Self::set_field)).
    pub fn set_singleton_field(&self, frame: &mut Frame, name: &str, path: &str, value: Value) -> Result<(), ReflectError> {
        let t = self.single(frame, name)?;
        t.set((t.ops.singleton_bytes_mut)(frame), path, value)
    }

    // ---- crate-internal helpers for the scene code ----

    #[cfg(feature = "scene")]
    pub(crate) fn bytes_of<'a>(&self, t: &TypeInfo, frame: &'a Frame, e: Entity) -> Option<&'a [u8]> {
        if !(t.ops.in_frame)(frame) {
            return None;
        }
        (t.ops.bytes)(frame, e)
    }

    #[cfg(feature = "scene")]
    pub(crate) fn singleton_bytes_of<'a>(&self, t: &TypeInfo, frame: &'a Frame) -> Option<&'a [u8]> {
        if !(t.ops.in_frame)(frame) {
            return None;
        }
        Some((t.ops.singleton_bytes)(frame))
    }

    #[cfg(feature = "scene")]
    pub(crate) fn insert_bytes(&self, t: &TypeInfo, frame: &mut Frame, e: Entity, bytes: &[u8]) {
        (t.ops.insert)(frame, e, bytes);
    }

    #[cfg(feature = "scene")]
    pub(crate) fn singleton_bytes_mut_of<'a>(&self, t: &TypeInfo, frame: &'a mut Frame) -> &'a mut [u8] {
        (t.ops.singleton_bytes_mut)(frame)
    }

    #[cfg(feature = "scene")]
    pub(crate) fn in_frame(&self, t: &TypeInfo, frame: &Frame) -> bool {
        (t.ops.in_frame)(frame)
    }
}
