use core::marker::PhantomData;

use crate::component::{Component, ComponentId};
use crate::entity::Entity;
use crate::frame::Frame;
use crate::registry::ComponentRegistry;
use crate::store::{AnyStore, SparseSet};

/// One field of a query tuple: either `&'q T` (read) or `&'q mut T` (write).
///
/// # Safety
/// Implementations must only ever downcast `store` to the `SparseSet<T>`
/// matching [`component_id`](Self::component_id) for the *same* registry the
/// id was computed from, and `&mut T` must only be produced when the caller
/// (`Query::new`) has proven no other field in the same query aliases the
/// same [`ComponentId`] mutably.
pub unsafe trait QueryFetch<'q> {
    type Item: 'q;
    type Component: Component;
    fn component_id(reg: &ComponentRegistry) -> ComponentId;
    fn is_mut() -> bool;
    /// # Safety
    /// `store` must point to a live, valid `Box<dyn AnyStore>` whose
    /// concrete type is `SparseSet<Self::Component>`, and must not be
    /// aliased by a simultaneous mutable fetch (enforced by the caller).
    unsafe fn fetch(store: *mut Box<dyn AnyStore>, e: Entity) -> Option<Self::Item>;
}

// SAFETY: see trait docs; `fetch` only ever reads through a shared
// reference derived from `store`, and the id/type pairing matches the
// registry that produced `store`.
unsafe impl<'q, T: Component> QueryFetch<'q> for &'q T {
    type Item = &'q T;
    type Component = T;
    fn component_id(reg: &ComponentRegistry) -> ComponentId {
        reg.component_id::<T>().unwrap_or_else(|| panic!("orr_ecs: component {} not registered", core::any::type_name::<T>()))
    }
    fn is_mut() -> bool {
        false
    }
    unsafe fn fetch(store: *mut Box<dyn AnyStore>, e: Entity) -> Option<Self::Item> {
        // SAFETY: caller contract on `QueryFetch::fetch`.
        let set = unsafe { (*store).as_any().downcast_ref::<SparseSet<T>>() }.expect("orr_ecs: store type mismatch");
        set.get(e)
    }
}

// SAFETY: `Query::new` panics if any two fields (this one included) resolve
// to the same `ComponentId` while either is mutable, so the `&mut` produced
// here never aliases another live reference into the same store.
unsafe impl<'q, T: Component> QueryFetch<'q> for &'q mut T {
    type Item = &'q mut T;
    type Component = T;
    fn component_id(reg: &ComponentRegistry) -> ComponentId {
        reg.component_id::<T>().unwrap_or_else(|| panic!("orr_ecs: component {} not registered", core::any::type_name::<T>()))
    }
    fn is_mut() -> bool {
        true
    }
    unsafe fn fetch(store: *mut Box<dyn AnyStore>, e: Entity) -> Option<Self::Item> {
        // SAFETY: caller contract on `QueryFetch::fetch`.
        let set = unsafe { (*store).as_any_mut().downcast_mut::<SparseSet<T>>() }.expect("orr_ecs: store type mismatch");
        set.get_mut(e)
    }
}

/// A tuple of 1-4 [`QueryFetch`] fields, e.g. `(&A, &mut B, &C)`.
pub trait QueryTuple<'q> {
    type Item: 'q;
    const ARITY: usize;
    fn component_ids(reg: &ComponentRegistry) -> Vec<ComponentId>;
    fn mut_flags() -> Vec<bool>;
    /// # Safety
    /// `stores` must have length `Self::ARITY`, each pointer valid and
    /// pointing at the store matching the corresponding `component_ids`
    /// entry, with the no-mutable-aliasing invariant already established.
    unsafe fn fetch(stores: &[*mut Box<dyn AnyStore>], e: Entity) -> Option<Self::Item>;
}

macro_rules! impl_query_tuple {
    ($n:expr; $($idx:tt => $name:ident),+) => {
        impl<'q, $($name: QueryFetch<'q>),+> QueryTuple<'q> for ($($name,)+) {
            type Item = ($($name::Item,)+);
            const ARITY: usize = $n;
            fn component_ids(reg: &ComponentRegistry) -> Vec<ComponentId> {
                vec![$($name::component_id(reg)),+]
            }
            fn mut_flags() -> Vec<bool> {
                vec![$($name::is_mut()),+]
            }
            unsafe fn fetch(stores: &[*mut Box<dyn AnyStore>], e: Entity) -> Option<Self::Item> {
                // SAFETY: forwarded from this function's own contract.
                Some(($( unsafe { $name::fetch(stores[$idx], e) }?, )+))
            }
        }
    };
}

impl_query_tuple!(1; 0 => A);
impl_query_tuple!(2; 0 => A, 1 => B);
impl_query_tuple!(3; 0 => A, 1 => B, 2 => C);
impl_query_tuple!(4; 0 => A, 1 => B, 2 => C, 3 => D);

/// Iterator over entities matching a [`QueryTuple`], produced by
/// [`Frame::query`](crate::Frame::query). Iteration order follows the dense
/// order of the *first* tuple field's store, so put the rarest component
/// first for the tightest loop.
pub struct QueryIter<'f, Q: QueryTuple<'f>> {
    entities: &'f [Entity],
    pos: usize,
    stores: Vec<*mut Box<dyn AnyStore>>,
    without: Vec<*mut Box<dyn AnyStore>>,
    components_base: *mut Box<dyn AnyStore>,
    registry: &'f ComponentRegistry,
    _marker: PhantomData<Q>,
}

// SAFETY: a `QueryIter` only exposes references scoped to `'f`, matching the
// exclusive/shared borrow of the `Frame` it was built from; raw pointers
// themselves carry no thread-safety guarantee but are only ever dereferenced
// on the thread that owns this iterator.
unsafe impl<'f, Q: QueryTuple<'f>> Send for QueryIter<'f, Q> {}

impl<'f, Q: QueryTuple<'f>> QueryIter<'f, Q> {
    pub(crate) fn new(frame: &'f mut Frame) -> Self {
        let registry: &'f ComponentRegistry = unsafe {
            // SAFETY: the registry `Arc` outlives `frame` and is never
            // mutated; reborrowing it for `'f` alongside `frame`'s own `'f`
            // borrow is sound because `Frame::registry` is never touched by
            // any of the store-mutating methods used during iteration.
            &*(frame.registry().as_ref() as *const ComponentRegistry)
        };
        let ids = Q::component_ids(registry);
        let muts = Q::mut_flags();
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                if ids[i] == ids[j] && (muts[i] || muts[j]) {
                    panic!(
                        "orr_ecs: query aliases component `{}` mutably in the same tuple",
                        registry.component_name(ids[i])
                    );
                }
            }
        }
        let base = frame.components_mut().as_mut_ptr();
        let stores: Vec<*mut Box<dyn AnyStore>> = ids.iter().map(|id| unsafe { base.add(id.0 as usize) }).collect();
        // SAFETY: `stores[0]` points at a live store owned by `frame`, valid
        // for the `'f` borrow; `entities()` returns a slice borrowed from
        // it, which we tie to `'f`.
        let entities: &'f [Entity] = unsafe { (*stores[0]).entities() };
        Self { entities, pos: 0, stores, without: Vec::new(), components_base: base, registry, _marker: PhantomData }
    }

    /// Excludes entities that have component `W`.
    pub fn without<W: Component>(mut self) -> Self {
        let id = self
            .registry
            .component_id::<W>()
            .unwrap_or_else(|| panic!("orr_ecs: component {} not registered", core::any::type_name::<W>()));
        // SAFETY: `id` is a valid index into the same `components` vec
        // `components_base` was derived from, still alive for `'f`.
        let ptr = unsafe { self.components_base.add(id.0 as usize) };
        self.without.push(ptr);
        self
    }

    pub fn for_each(self, mut f: impl FnMut(Entity, Q::Item)) {
        for (e, item) in self {
            f(e, item);
        }
    }
}

impl<'f, Q: QueryTuple<'f>> Iterator for QueryIter<'f, Q> {
    type Item = (Entity, Q::Item);

    fn next(&mut self) -> Option<Self::Item> {
        while self.pos < self.entities.len() {
            let e = self.entities[self.pos];
            self.pos += 1;
            // SAFETY: pointers in `self.without` point at live stores owned
            // by the same frame, valid for `'f`.
            if self.without.iter().any(|&s| unsafe { (*s).contains(e) }) {
                continue;
            }
            // SAFETY: established by `QueryIter::new`: `self.stores` has
            // `Q::ARITY` valid, non-aliased-when-mutable entries matching
            // `Q`'s component ids.
            if let Some(item) = unsafe { Q::fetch(&self.stores, e) } {
                return Some((e, item));
            }
        }
        None
    }
}

impl Frame {
    /// Queries components in the dense iteration order of the first tuple
    /// field. Always takes `&mut self` (even for read-only tuples) since a
    /// single query may mix `&T` and `&mut T` fields; attempting to alias
    /// the same component type mutably within one query panics.
    pub fn query<'f, Q: QueryTuple<'f>>(&'f mut self) -> QueryIter<'f, Q> {
        QueryIter::new(self)
    }

    /// Alias for [`query`](Self::query).
    pub fn query_mut<'f, Q: QueryTuple<'f>>(&'f mut self) -> QueryIter<'f, Q> {
        QueryIter::new(self)
    }
}
