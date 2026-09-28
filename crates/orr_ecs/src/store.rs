use core::any::Any;

use xxhash_rust::xxh3::Xxh3;

use crate::component::Component;
use crate::entity::Entity;

const NONE: u32 = u32::MAX;

/// A sparse-set store for one component type.
///
/// `sparse[entity.index]` gives the dense slot for that entity (or
/// `u32::MAX` if absent); `dense_entities`/`data` are parallel, tightly
/// packed vectors iterated in insertion/swap-remove order. Removal is
/// `swap_remove`, so iteration order is deterministic given the same
/// sequence of structural operations, but is *not* stable across removals
/// (the last dense element moves into the removed slot).
pub struct SparseSet<T: Component> {
    sparse: Vec<u32>,
    dense_entities: Vec<Entity>,
    data: Vec<T>,
}

impl<T: Component> Default for SparseSet<T> {
    fn default() -> Self {
        Self { sparse: Vec::new(), dense_entities: Vec::new(), data: Vec::new() }
    }
}

impl<T: Component> SparseSet<T> {
    fn ensure_sparse(&mut self, idx: u32) {
        let idx = idx as usize;
        if idx >= self.sparse.len() {
            self.sparse.resize(idx + 1, NONE);
        }
    }

    /// Inserts or replaces the component for `e`. Returns the previous value
    /// if `e` already had one (its dense slot is reused; version in the
    /// stored `Entity` is updated to `e`).
    pub fn insert(&mut self, e: Entity, v: T) -> Option<T> {
        self.ensure_sparse(e.index);
        let slot = self.sparse[e.index as usize];
        if slot != NONE {
            let old = core::mem::replace(&mut self.data[slot as usize], v);
            self.dense_entities[slot as usize] = e;
            Some(old)
        } else {
            let slot = self.dense_entities.len() as u32;
            self.dense_entities.push(e);
            self.data.push(v);
            self.sparse[e.index as usize] = slot;
            None
        }
    }

    /// Removes the component for `e` (exact index+version match required),
    /// returning it if present.
    pub fn remove(&mut self, e: Entity) -> Option<T> {
        let idx = e.index as usize;
        if idx >= self.sparse.len() {
            return None;
        }
        let slot = self.sparse[idx];
        if slot == NONE || self.dense_entities[slot as usize] != e {
            return None;
        }
        let last = self.dense_entities.len() - 1;
        self.dense_entities.swap_remove(slot as usize);
        let val = self.data.swap_remove(slot as usize);
        self.sparse[idx] = NONE;
        if (slot as usize) != last {
            let moved = self.dense_entities[slot as usize];
            self.sparse[moved.index as usize] = slot;
        }
        Some(val)
    }

    pub fn get(&self, e: Entity) -> Option<&T> {
        let idx = e.index as usize;
        if idx >= self.sparse.len() {
            return None;
        }
        let slot = self.sparse[idx];
        if slot == NONE || self.dense_entities[slot as usize] != e {
            return None;
        }
        Some(&self.data[slot as usize])
    }

    pub fn get_mut(&mut self, e: Entity) -> Option<&mut T> {
        let idx = e.index as usize;
        if idx >= self.sparse.len() {
            return None;
        }
        let slot = self.sparse[idx];
        if slot == NONE || self.dense_entities[slot as usize] != e {
            return None;
        }
        Some(&mut self.data[slot as usize])
    }

    pub fn contains(&self, e: Entity) -> bool {
        self.get(e).is_some()
    }

    pub fn len(&self) -> u32 {
        self.data.len() as u32
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn dense_entities(&self) -> &[Entity] {
        &self.dense_entities
    }

    #[inline]
    pub fn dense_data(&self) -> &[T] {
        &self.data
    }

    #[inline]
    pub fn dense_data_mut(&mut self) -> &mut [T] {
        &mut self.data
    }
}

/// Type-erased handle to a [`SparseSet<T>`], stored behind `Box<dyn AnyStore>`
/// inside a [`Frame`](crate::Frame) so heterogeneous component stores can
/// live in one `Vec` indexed by [`ComponentId`](crate::ComponentId).
pub trait AnyStore: Send + Sync {
    fn remove(&mut self, e: Entity) -> bool;
    fn contains(&self, e: Entity) -> bool;
    fn entities(&self) -> &[Entity];
    fn len(&self) -> u32;
    fn clear(&mut self);
    /// Overwrites `self` with `other`'s contents, reusing `self`'s existing
    /// allocations where possible. Panics if `other` is not the same
    /// concrete component type.
    fn copy_from(&mut self, other: &dyn AnyStore);
    /// Feeds this store's entire deterministic byte representation (dense
    /// entities, then dense data) into `h`.
    fn hash_into(&self, h: &mut Xxh3);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Component> AnyStore for SparseSet<T> {
    fn remove(&mut self, e: Entity) -> bool {
        SparseSet::remove(self, e).is_some()
    }

    fn contains(&self, e: Entity) -> bool {
        SparseSet::contains(self, e)
    }

    fn entities(&self) -> &[Entity] {
        self.dense_entities()
    }

    fn len(&self) -> u32 {
        SparseSet::len(self)
    }

    fn clear(&mut self) {
        self.sparse.clear();
        self.dense_entities.clear();
        self.data.clear();
    }

    fn copy_from(&mut self, other: &dyn AnyStore) {
        let other = other
            .as_any()
            .downcast_ref::<SparseSet<T>>()
            .expect("orr_ecs: copy_from between mismatched component store types");
        self.sparse.clone_from(&other.sparse);
        self.dense_entities.clone_from(&other.dense_entities);
        self.data.clone_from(&other.data);
    }

    fn hash_into(&self, h: &mut Xxh3) {
        h.update(bytemuck::cast_slice(&self.dense_entities));
        h.update(bytemuck::cast_slice(&self.data));
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
