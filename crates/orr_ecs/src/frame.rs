use std::sync::Arc;

use xxhash_rust::xxh3::Xxh3;

use crate::codec::{put_len, put_u32, put_u64, FrameDecodeError, Reader, FORMAT_VERSION, MAGIC};
use crate::component::{Component, ComponentId, ListId, SingletonId};
use crate::entity::{Entity, EntityAllocator};
use crate::list::{FrameList, ListPool};
use crate::registry::ComponentRegistry;
use crate::singleton::SingletonSlot;
use crate::store::SparseSet;

/// The entire deterministic simulation state at one tick.
///
/// A `Frame` owns: the entity allocator, one sparse-set store per registered
/// component type, one slot per registered singleton type, and one list pool
/// per registered [`FrameList`] element type. It is built from a shared
/// [`ComponentRegistry`] and is cheap to snapshot (`Clone` / `copy_from`
/// reuse allocations) and to checksum (`checksum`), which is what rollback
/// netcode needs to resimulate ticks and compare results across peers.
pub struct Frame {
    registry: Arc<ComponentRegistry>,
    tick: u64,
    allocator: EntityAllocator,
    components: Vec<Box<dyn crate::store::AnyStore>>,
    singletons: Vec<Box<dyn crate::singleton::AnySingleton>>,
    lists: Vec<Box<dyn crate::list::AnyListPool>>,
}

impl Frame {
    pub fn new(registry: Arc<ComponentRegistry>) -> Self {
        let components = registry.make_components();
        let singletons = registry.make_singletons();
        let lists = registry.make_lists();
        Self { registry, tick: 0, allocator: EntityAllocator::new(), components, singletons, lists }
    }

    pub fn registry(&self) -> &Arc<ComponentRegistry> {
        &self.registry
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn set_tick(&mut self, tick: u64) {
        self.tick = tick;
    }

    // ---- entities ----

    pub fn spawn(&mut self) -> Entity {
        self.allocator.spawn()
    }

    /// Despawns `e`, removing it from every registered component store.
    /// Does *not* free any [`FrameList`] allocations the entity's components
    /// may have referenced — those are a separate resource and must be
    /// freed explicitly (e.g. by the code that removes/replaces the
    /// component holding the handle), so a dangling handle in a despawned
    /// component's leftover bytes is never silently reinterpreted.
    pub fn despawn(&mut self, e: Entity) -> bool {
        if !self.allocator.despawn(e) {
            return false;
        }
        for store in &mut self.components {
            store.remove(e);
        }
        true
    }

    pub fn exists(&self, e: Entity) -> bool {
        self.allocator.exists(e)
    }

    pub fn alive_count(&self) -> u32 {
        self.allocator.alive_count()
    }

    /// Every live entity, including ones without components, in ascending
    /// index order (tools such as the scene saver use this).
    pub fn entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.allocator.iter_alive()
    }


    // ---- components ----

    fn component_id_of<T: Component>(&self) -> crate::component::ComponentId {
        self.registry.component_id::<T>().unwrap_or_else(|| {
            panic!("orr_ecs: component type {} is not registered", core::any::type_name::<T>())
        })
    }

    fn store<T: Component>(&self) -> &SparseSet<T> {
        let id = self.component_id_of::<T>();
        self.components[id.0 as usize].as_any().downcast_ref().expect("orr_ecs: store type mismatch")
    }

    fn store_mut<T: Component>(&mut self) -> &mut SparseSet<T> {
        let id = self.component_id_of::<T>();
        self.components[id.0 as usize].as_any_mut().downcast_mut().expect("orr_ecs: store type mismatch")
    }

    pub fn add<T: Component>(&mut self, e: Entity, v: T) -> Option<T> {
        self.store_mut::<T>().insert(e, v)
    }

    pub fn remove<T: Component>(&mut self, e: Entity) -> Option<T> {
        self.store_mut::<T>().remove(e)
    }

    pub fn get<T: Component>(&self, e: Entity) -> Option<&T> {
        self.store::<T>().get(e)
    }

    pub fn get_mut<T: Component>(&mut self, e: Entity) -> Option<&mut T> {
        self.store_mut::<T>().get_mut(e)
    }

    pub fn has<T: Component>(&self, e: Entity) -> bool {
        self.store::<T>().contains(e)
    }

    pub fn count<T: Component>(&self) -> u32 {
        self.store::<T>().len()
    }

    /// Read-only view of every `T` in dense (deterministic) order: the
    /// owning entities and the component values, as parallel slices. Takes
    /// `&self`, so read-only consumers (the view layer, through
    /// `orr_bridge::FrameView`) can iterate without mutable access.
    pub fn dense<T: Component>(&self) -> (&[Entity], &[T]) {
        let s = self.store::<T>();
        (s.dense_entities(), s.dense_data())
    }

    pub(crate) fn components_mut(&mut self) -> &mut Vec<Box<dyn crate::store::AnyStore>> {
        &mut self.components
    }

    // ---- singletons ----

    fn singleton_id_of<T: Component>(&self) -> crate::component::SingletonId {
        self.registry.singleton_id::<T>().unwrap_or_else(|| {
            panic!("orr_ecs: singleton type {} is not registered", core::any::type_name::<T>())
        })
    }

    pub fn singleton<T: Component>(&self) -> &T {
        let id = self.singleton_id_of::<T>();
        &self.singletons[id.0 as usize]
            .as_any()
            .downcast_ref::<SingletonSlot<T>>()
            .expect("orr_ecs: singleton type mismatch")
            .0
    }

    pub fn singleton_mut<T: Component>(&mut self) -> &mut T {
        let id = self.singleton_id_of::<T>();
        &mut self.singletons[id.0 as usize]
            .as_any_mut()
            .downcast_mut::<SingletonSlot<T>>()
            .expect("orr_ecs: singleton type mismatch")
            .0
    }

    pub fn set_singleton<T: Component>(&mut self, v: T) {
        *self.singleton_mut::<T>() = v;
    }

    // ---- lists ----

    fn list_id_of<T: Component>(&self) -> crate::component::ListId {
        self.registry
            .list_id::<T>()
            .unwrap_or_else(|| panic!("orr_ecs: list element type {} is not registered", core::any::type_name::<T>()))
    }

    fn list_pool<T: Component>(&self) -> &ListPool<T> {
        let id = self.list_id_of::<T>();
        self.lists[id.0 as usize].as_any().downcast_ref().expect("orr_ecs: list pool type mismatch")
    }
    fn list_pool_mut<T: Component>(&mut self) -> &mut ListPool<T> {
        let id = self.list_id_of::<T>();
        self.lists[id.0 as usize].as_any_mut().downcast_mut().expect("orr_ecs: list pool type mismatch")
    }

    pub fn alloc_list<T: Component>(&mut self) -> FrameList<T> {
        self.list_pool_mut::<T>().alloc()
    }
    pub fn list<T: Component>(&self, h: FrameList<T>) -> &[T] {
        self.list_pool::<T>().get(h)
    }
    pub fn list_mut<T: Component>(&mut self, h: FrameList<T>) -> &mut [T] {
        self.list_pool_mut::<T>().get_mut(h)
    }
    pub fn list_push<T: Component>(&mut self, h: FrameList<T>, v: T) {
        self.list_pool_mut::<T>().push(h, v)
    }
    pub fn list_clear<T: Component>(&mut self, h: FrameList<T>) {
        self.list_pool_mut::<T>().clear(h)
    }
    pub fn list_free<T: Component>(&mut self, h: FrameList<T>) {
        self.list_pool_mut::<T>().free(h)
    }

    // ---- snapshot / checksum ----

    /// Overwrites `self` with `other`'s entire state, reusing existing
    /// allocations wherever possible. Both frames must have been built from
    /// the *same* `Arc<ComponentRegistry>` (checked by pointer identity).
    pub fn copy_from(&mut self, other: &Frame) {
        assert!(
            Arc::ptr_eq(&self.registry, &other.registry),
            "orr_ecs: Frame::copy_from requires both frames to share the same ComponentRegistry"
        );
        self.tick = other.tick;
        self.allocator.copy_from(&other.allocator);
        for (dst, src) in self.components.iter_mut().zip(other.components.iter()) {
            dst.copy_from(src.as_ref());
        }
        for (dst, src) in self.singletons.iter_mut().zip(other.singletons.iter()) {
            dst.copy_from(src.as_ref());
        }
        for (dst, src) in self.lists.iter_mut().zip(other.lists.iter()) {
            dst.copy_from(src.as_ref());
        }
    }

    /// Deterministic checksum of the entire frame: tick, entity allocator
    /// state, every component store (in registration order: dense entities
    /// then dense data), every singleton, and every list pool. Two frames
    /// built by replaying the same operation sequence from the same initial
    /// state always produce the same checksum, on any platform.
    pub fn checksum(&self) -> u64 {
        let mut h = Xxh3::new();
        h.update(&self.tick.to_le_bytes());
        self.allocator.hash_into(&mut h);
        for store in &self.components {
            store.hash_into(&mut h);
        }
        for s in &self.singletons {
            h.update(s.bytes());
        }
        for l in &self.lists {
            l.hash_into(&mut h);
        }
        h.digest()
    }

    // ---- byte serialization ----

    /// Serializes the entire frame (everything [`checksum`](Self::checksum)
    /// covers) into a new byte vector. See [`write_bytes`](Self::write_bytes).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write_bytes(&mut out);
        out
    }

    /// Appends the frame's byte form to `out`.
    ///
    /// Format v1, all integers little-endian, all lengths `u32`:
    /// `"ORRF"`, `version u32`, `tick u64`; a schema block (for components,
    /// singletons, then lists: `count u32`, and per type `name_len u32`,
    /// name bytes, `elem_size u32`); the entity allocator; every component
    /// store, singleton and list pool in registration order (raw `Pod`
    /// bytes); finally the frame's `checksum u64`. Types are identified by
    /// registration order, name and size, never by `TypeId`, so the bytes
    /// are the same on every platform and process.
    pub fn write_bytes(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&MAGIC);
        put_u32(out, FORMAT_VERSION);
        put_u64(out, self.tick);

        put_len(out, self.components.len());
        for i in 0..self.components.len() {
            let id = ComponentId(i as u16);
            put_schema_entry(out, self.registry.component_name(id), self.registry.component_size(id));
        }
        put_len(out, self.singletons.len());
        for i in 0..self.singletons.len() {
            let id = SingletonId(i as u16);
            put_schema_entry(out, self.registry.singleton_name(id), self.registry.singleton_size(id));
        }
        put_len(out, self.lists.len());
        for i in 0..self.lists.len() {
            let id = ListId(i as u16);
            put_schema_entry(out, self.registry.list_name(id), self.registry.list_size(id));
        }

        self.allocator.write_bytes(out);
        for store in &self.components {
            store.write_bytes(out);
        }
        for s in &self.singletons {
            out.extend_from_slice(s.bytes());
        }
        for l in &self.lists {
            l.write_bytes(out);
        }
        put_u64(out, self.checksum());
    }

    /// Rebuilds a frame from [`to_bytes`](Self::to_bytes) output.
    ///
    /// `registry` must describe the same types in the same order as the
    /// registry the bytes were written with; otherwise this fails with a
    /// schema error. Malformed input of any kind returns an error and never
    /// panics: lengths are checked against the input before allocating,
    /// entity/free-list invariants are re-validated, and the stored checksum
    /// must match the rebuilt frame.
    pub fn from_bytes(registry: Arc<ComponentRegistry>, bytes: &[u8]) -> Result<Frame, FrameDecodeError> {
        let mut r = Reader::new(bytes);
        if r.take(4)? != MAGIC {
            return Err(FrameDecodeError::BadMagic);
        }
        let version = r.u32()?;
        if version != FORMAT_VERSION {
            return Err(FrameDecodeError::UnsupportedVersion(version));
        }
        let tick = r.u64()?;

        check_schema(&mut r, "component", registry.component_count() as u32, |i| {
            let id = ComponentId(i as u16);
            (registry.component_name(id), registry.component_size(id))
        })?;
        check_schema(&mut r, "singleton", registry.singleton_count() as u32, |i| {
            let id = SingletonId(i as u16);
            (registry.singleton_name(id), registry.singleton_size(id))
        })?;
        check_schema(&mut r, "list", registry.list_count() as u32, |i| {
            let id = ListId(i as u16);
            (registry.list_name(id), registry.list_size(id))
        })?;

        let mut f = Frame::new(registry);
        f.tick = tick;
        f.allocator = EntityAllocator::read_bytes(&mut r)?;
        for store in &mut f.components {
            store.read_bytes(&mut r, &f.allocator)?;
        }
        for s in &mut f.singletons {
            s.read_bytes(&mut r)?;
        }
        for l in &mut f.lists {
            l.read_bytes(&mut r)?;
        }

        let expected = r.u64()?;
        if r.remaining() != 0 {
            return Err(FrameDecodeError::TrailingBytes);
        }
        let actual = f.checksum();
        if actual != expected {
            return Err(FrameDecodeError::ChecksumMismatch { expected, actual });
        }
        Ok(f)
    }
}

fn put_schema_entry(out: &mut Vec<u8>, name: &str, elem_size: u32) {
    put_len(out, name.len());
    out.extend_from_slice(name.as_bytes());
    put_u32(out, elem_size);
}

/// Checks one schema section against the registry's `(name, size)` per index.
fn check_schema(
    r: &mut Reader,
    kind: &'static str,
    expected: u32,
    entry: impl Fn(u32) -> (&'static str, u32),
) -> Result<(), FrameDecodeError> {
    let found = r.u32()?;
    if found != expected {
        return Err(FrameDecodeError::SchemaCount { kind, expected, found });
    }
    for index in 0..expected {
        let (name, size) = entry(index);
        let name_len = r.u32()?;
        let same_name = r.take(name_len as usize)? == name.as_bytes();
        let same_size = r.u32()? == size;
        if !same_name || !same_size {
            return Err(FrameDecodeError::SchemaEntry { kind, index });
        }
    }
    Ok(())
}

impl Clone for Frame {
    fn clone(&self) -> Self {
        let mut f = Frame::new(self.registry.clone());
        f.copy_from(self);
        f
    }
}
