use std::sync::Arc;

use xxhash_rust::xxh3::Xxh3;

use crate::component::Component;
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
}

impl Clone for Frame {
    fn clone(&self) -> Self {
        let mut f = Frame::new(self.registry.clone());
        f.copy_from(self);
        f
    }
}
