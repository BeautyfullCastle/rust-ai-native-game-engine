use std::any::TypeId;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::component::{Component, ComponentId, ListId, SingletonId};
use crate::list::{AnyListPool, ListPool};
use crate::singleton::{AnySingleton, SingletonSlot};
use crate::store::{AnyStore, SparseSet};

struct ComponentDescriptor {
    name: &'static str,
    make: fn() -> Box<dyn AnyStore>,
}
struct SingletonDescriptor {
    name: &'static str,
    make: fn() -> Box<dyn AnySingleton>,
}
struct ListDescriptor {
    name: &'static str,
    make: fn() -> Box<dyn AnyListPool>,
}

/// Immutable, shared description of every component, singleton and
/// [`FrameList`](crate::FrameList) element type a simulation uses.
///
/// Built once via [`ComponentRegistryBuilder`] and shared as `Arc` across
/// every [`Frame`](crate::Frame) (they must all share the *same* `Arc` —
/// `copy_from` checks this by pointer identity). Registration order fixes
/// each type's dense id, which in turn fixes checksum iteration order: two
/// processes that register types in the same order and run the same op
/// sequence will produce identical checksums.
pub struct ComponentRegistry {
    components: Vec<ComponentDescriptor>,
    singletons: Vec<SingletonDescriptor>,
    lists: Vec<ListDescriptor>,
    component_lookup: BTreeMap<TypeId, ComponentId>,
    singleton_lookup: BTreeMap<TypeId, SingletonId>,
    list_lookup: BTreeMap<TypeId, ListId>,
}

impl ComponentRegistry {
    pub fn component_id<T: Component>(&self) -> Option<ComponentId> {
        self.component_lookup.get(&TypeId::of::<T>()).copied()
    }
    pub fn singleton_id<T: Component>(&self) -> Option<SingletonId> {
        self.singleton_lookup.get(&TypeId::of::<T>()).copied()
    }
    pub fn list_id<T: Component>(&self) -> Option<ListId> {
        self.list_lookup.get(&TypeId::of::<T>()).copied()
    }

    pub fn component_count(&self) -> u16 {
        self.components.len() as u16
    }
    pub fn singleton_count(&self) -> u16 {
        self.singletons.len() as u16
    }
    pub fn list_count(&self) -> u16 {
        self.lists.len() as u16
    }

    pub fn component_name(&self, id: ComponentId) -> &'static str {
        self.components[id.0 as usize].name
    }
    pub fn singleton_name(&self, id: SingletonId) -> &'static str {
        self.singletons[id.0 as usize].name
    }
    pub fn list_name(&self, id: ListId) -> &'static str {
        self.lists[id.0 as usize].name
    }

    pub(crate) fn make_components(&self) -> Vec<Box<dyn AnyStore>> {
        self.components.iter().map(|d| (d.make)()).collect()
    }
    pub(crate) fn make_singletons(&self) -> Vec<Box<dyn AnySingleton>> {
        self.singletons.iter().map(|d| (d.make)()).collect()
    }
    pub(crate) fn make_lists(&self) -> Vec<Box<dyn AnyListPool>> {
        self.lists.iter().map(|d| (d.make)()).collect()
    }
}

/// Builder for a [`ComponentRegistry`]. Register every component, singleton
/// and list-element type up front, in a fixed order, then [`build`](Self::build)
/// once; the resulting registry is immutable for the lifetime of the process.
#[derive(Default)]
pub struct ComponentRegistryBuilder {
    components: Vec<ComponentDescriptor>,
    singletons: Vec<SingletonDescriptor>,
    lists: Vec<ListDescriptor>,
    component_lookup: BTreeMap<TypeId, ComponentId>,
    singleton_lookup: BTreeMap<TypeId, SingletonId>,
    list_lookup: BTreeMap<TypeId, ListId>,
}

impl ComponentRegistryBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `T` as a per-entity component under a stable `name` (used
    /// for diagnostics and future schema/serialization work). Panics if `T`
    /// is already registered as a component.
    pub fn register_component<T: Component>(&mut self, name: &'static str) -> ComponentId {
        assert!(
            self.component_lookup.insert(TypeId::of::<T>(), ComponentId(self.components.len() as u16)).is_none(),
            "orr_ecs: component type already registered"
        );
        let id = ComponentId(self.components.len() as u16);
        self.components.push(ComponentDescriptor { name, make: || Box::new(SparseSet::<T>::default()) });
        id
    }

    /// Registers `T` as a singleton slot.
    pub fn register_singleton<T: Component>(&mut self, name: &'static str) -> SingletonId {
        assert!(
            self.singleton_lookup.insert(TypeId::of::<T>(), SingletonId(self.singletons.len() as u16)).is_none(),
            "orr_ecs: singleton type already registered"
        );
        let id = SingletonId(self.singletons.len() as u16);
        self.singletons.push(SingletonDescriptor { name, make: || Box::new(SingletonSlot::<T>::default()) });
        id
    }

    /// Registers `T` as a [`FrameList`](crate::FrameList) element type.
    pub fn register_list<T: Component>(&mut self, name: &'static str) -> ListId {
        assert!(
            self.list_lookup.insert(TypeId::of::<T>(), ListId(self.lists.len() as u16)).is_none(),
            "orr_ecs: list element type already registered"
        );
        let id = ListId(self.lists.len() as u16);
        self.lists.push(ListDescriptor { name, make: || Box::new(ListPool::<T>::default()) });
        id
    }

    pub fn build(self) -> Arc<ComponentRegistry> {
        Arc::new(ComponentRegistry {
            components: self.components,
            singletons: self.singletons,
            lists: self.lists,
            component_lookup: self.component_lookup,
            singleton_lookup: self.singleton_lookup,
            list_lookup: self.list_lookup,
        })
    }
}
