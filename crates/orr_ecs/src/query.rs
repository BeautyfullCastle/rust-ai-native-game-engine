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
    /// Downcasts the type-erased store to a concrete `*mut SparseSet<Self::Component>`
    /// once per query construction, so per-entity fetches never pay for a
    /// vtable call or `downcast` again.
    ///
    /// # Safety
    /// `store` must point to a live, valid `Box<dyn AnyStore>` whose
    /// concrete type is `SparseSet<Self::Component>`.
    unsafe fn resolve(store: *mut Box<dyn AnyStore>) -> *mut SparseSet<Self::Component>;
    /// Sparse lookup by entity.
    ///
    /// # Safety
    /// `store` must be a live pointer produced by [`resolve`](Self::resolve)
    /// for the same query, and must not be aliased by a simultaneous mutable
    /// fetch (enforced by the caller).
    unsafe fn fetch(store: *mut SparseSet<Self::Component>, e: Entity) -> Option<Self::Item>;
    /// Direct dense-slot access, used only for a query tuple's first field:
    /// `pos` is the position of `e` in *that store's own* dense arrays (it
    /// came from iterating `entities()` of this very store), so no sparse
    /// lookup or presence check is needed.
    ///
    /// # Safety
    /// `store` must be a live pointer produced by [`resolve`](Self::resolve)
    /// for the same query, not aliased by a simultaneous mutable fetch, and
    /// `pos` must be `< ` that store's dense length.
    unsafe fn fetch_dense(store: *mut SparseSet<Self::Component>, pos: usize) -> Self::Item;
}

// SAFETY: see trait docs; `fetch`/`fetch_dense` only ever read through a
// shared reference derived from `store`, and the id/type pairing matches the
// registry that produced `store`.
unsafe impl<'q, T: Component> QueryFetch<'q> for &'q T {
    type Item = &'q T;
    type Component = T;
    #[inline]
    fn component_id(reg: &ComponentRegistry) -> ComponentId {
        reg.component_id::<T>().unwrap_or_else(|| panic!("orr_ecs: component {} not registered", core::any::type_name::<T>()))
    }
    #[inline]
    fn is_mut() -> bool {
        false
    }
    #[inline]
    unsafe fn resolve(store: *mut Box<dyn AnyStore>) -> *mut SparseSet<T> {
        // SAFETY: caller contract on `QueryFetch::resolve`.
        let set = unsafe { (*store).as_any_mut() }.downcast_mut::<SparseSet<T>>().expect("orr_ecs: store type mismatch");
        set as *mut SparseSet<T>
    }
    #[inline]
    unsafe fn fetch(store: *mut SparseSet<T>, e: Entity) -> Option<Self::Item> {
        // SAFETY: caller contract on `QueryFetch::fetch`.
        unsafe { (*store).get(e) }
    }
    #[inline]
    unsafe fn fetch_dense(store: *mut SparseSet<T>, pos: usize) -> Self::Item {
        // SAFETY: caller contract on `QueryFetch::fetch_dense`: `pos` is a
        // valid dense index into this store.
        unsafe { (*store).dense_data().get_unchecked(pos) }
    }
}

// SAFETY: `Query::new` panics if any two fields (this one included) resolve
// to the same `ComponentId` while either is mutable, so the `&mut` produced
// here never aliases another live reference into the same store.
unsafe impl<'q, T: Component> QueryFetch<'q> for &'q mut T {
    type Item = &'q mut T;
    type Component = T;
    #[inline]
    fn component_id(reg: &ComponentRegistry) -> ComponentId {
        reg.component_id::<T>().unwrap_or_else(|| panic!("orr_ecs: component {} not registered", core::any::type_name::<T>()))
    }
    #[inline]
    fn is_mut() -> bool {
        true
    }
    #[inline]
    unsafe fn resolve(store: *mut Box<dyn AnyStore>) -> *mut SparseSet<T> {
        // SAFETY: caller contract on `QueryFetch::resolve`.
        let set = unsafe { (*store).as_any_mut() }.downcast_mut::<SparseSet<T>>().expect("orr_ecs: store type mismatch");
        set as *mut SparseSet<T>
    }
    #[inline]
    unsafe fn fetch(store: *mut SparseSet<T>, e: Entity) -> Option<Self::Item> {
        // SAFETY: caller contract on `QueryFetch::fetch`.
        unsafe { (*store).get_mut(e) }
    }
    #[inline]
    unsafe fn fetch_dense(store: *mut SparseSet<T>, pos: usize) -> Self::Item {
        // SAFETY: caller contract on `QueryFetch::fetch_dense`: `pos` is a
        // valid dense index into this store.
        unsafe { (*store).dense_data_mut().get_unchecked_mut(pos) }
    }
}

/// A tuple of 1-4 [`QueryFetch`] fields, e.g. `(&A, &mut B, &C)`.
pub trait QueryTuple<'q> {
    type Item: 'q;
    /// Each field's store, downcast once per query construction (see
    /// [`QueryFetch::resolve`]).
    type Resolved: Copy;
    const ARITY: usize;
    fn component_ids(reg: &ComponentRegistry) -> Vec<ComponentId>;
    fn mut_flags() -> Vec<bool>;
    /// # Safety
    /// `stores` must have length `Self::ARITY`, each pointer valid and
    /// pointing at the store matching the corresponding `component_ids`
    /// entry.
    unsafe fn resolve(stores: &[*mut Box<dyn AnyStore>]) -> Self::Resolved;
    /// # Safety
    /// `resolved` must come from [`resolve`](Self::resolve) for this same
    /// query, with the no-mutable-aliasing invariant already established.
    unsafe fn fetch(resolved: &Self::Resolved, e: Entity) -> Option<Self::Item>;
    /// Like [`fetch`](Self::fetch), but the first field is read directly at
    /// dense slot `pos` instead of via sparse lookup.
    ///
    /// # Safety
    /// Same as `fetch`, plus `pos` must be a valid dense index into the
    /// first field's store and `e` must be the entity at that dense slot.
    unsafe fn fetch_lead(resolved: &Self::Resolved, pos: usize, e: Entity) -> Option<Self::Item>;
    /// Like [`fetch_lead`](Self::fetch_lead), but *every* field is read
    /// directly at dense slot `pos`. Only valid when every field's dense
    /// entity order has already been checked to equal the first field's
    /// (see `QueryIter::new`'s `aligned` check), so no per-field presence
    /// check is needed.
    ///
    /// # Safety
    /// Same as `fetch_lead`, plus every field's dense entity order must
    /// equal the first field's, so `pos` also names the right dense slot in
    /// every other field's store.
    unsafe fn fetch_all_dense(resolved: &Self::Resolved, pos: usize) -> Self::Item;
}

macro_rules! impl_query_tuple {
    ($n:expr; $head:ident) => {
        impl<'q, $head: QueryFetch<'q>> QueryTuple<'q> for ($head,) {
            type Item = ($head::Item,);
            type Resolved = *mut SparseSet<$head::Component>;
            const ARITY: usize = $n;
            fn component_ids(reg: &ComponentRegistry) -> Vec<ComponentId> {
                vec![$head::component_id(reg)]
            }
            fn mut_flags() -> Vec<bool> {
                vec![$head::is_mut()]
            }
            unsafe fn resolve(stores: &[*mut Box<dyn AnyStore>]) -> Self::Resolved {
                // SAFETY: forwarded from this function's own contract.
                unsafe { $head::resolve(stores[0]) }
            }
            unsafe fn fetch(resolved: &Self::Resolved, e: Entity) -> Option<Self::Item> {
                // SAFETY: forwarded from this function's own contract.
                Some((unsafe { $head::fetch(*resolved, e) }?,))
            }
            unsafe fn fetch_lead(resolved: &Self::Resolved, pos: usize, _e: Entity) -> Option<Self::Item> {
                // SAFETY: forwarded from this function's own contract.
                Some((unsafe { $head::fetch_dense(*resolved, pos) },))
            }
            unsafe fn fetch_all_dense(resolved: &Self::Resolved, pos: usize) -> Self::Item {
                // SAFETY: forwarded from this function's own contract.
                (unsafe { $head::fetch_dense(*resolved, pos) },)
            }
        }
    };
    ($n:expr; $head:ident, $($idx:tt => $name:ident),+) => {
        impl<'q, $head: QueryFetch<'q>, $($name: QueryFetch<'q>),+> QueryTuple<'q> for ($head, $($name,)+) {
            type Item = ($head::Item, $($name::Item,)+);
            type Resolved = (*mut SparseSet<$head::Component>, $(*mut SparseSet<$name::Component>,)+);
            const ARITY: usize = $n;
            fn component_ids(reg: &ComponentRegistry) -> Vec<ComponentId> {
                vec![$head::component_id(reg), $($name::component_id(reg)),+]
            }
            fn mut_flags() -> Vec<bool> {
                vec![$head::is_mut(), $($name::is_mut()),+]
            }
            unsafe fn resolve(stores: &[*mut Box<dyn AnyStore>]) -> Self::Resolved {
                // SAFETY: forwarded from this function's own contract.
                unsafe { ($head::resolve(stores[0]), $($name::resolve(stores[$idx]),)+) }
            }
            unsafe fn fetch(resolved: &Self::Resolved, e: Entity) -> Option<Self::Item> {
                // SAFETY: forwarded from this function's own contract.
                Some(( unsafe { $head::fetch(resolved.0, e) }?, $( unsafe { $name::fetch(resolved.$idx, e) }?, )+ ))
            }
            unsafe fn fetch_lead(resolved: &Self::Resolved, pos: usize, e: Entity) -> Option<Self::Item> {
                // SAFETY: forwarded from this function's own contract.
                Some(( unsafe { $head::fetch_dense(resolved.0, pos) }, $( unsafe { $name::fetch(resolved.$idx, e) }?, )+ ))
            }
            unsafe fn fetch_all_dense(resolved: &Self::Resolved, pos: usize) -> Self::Item {
                // SAFETY: forwarded from this function's own contract.
                ( unsafe { $head::fetch_dense(resolved.0, pos) }, $( unsafe { $name::fetch_dense(resolved.$idx, pos) }, )+ )
            }
        }
    };
}

impl_query_tuple!(1; A);
impl_query_tuple!(2; A, 1 => B);
impl_query_tuple!(3; A, 1 => B, 2 => C);
impl_query_tuple!(4; A, 1 => B, 2 => C, 3 => D);

/// Iterator over entities matching a [`QueryTuple`], produced by
/// [`Frame::query`](crate::Frame::query). Iteration order follows the dense
/// order of the *first* tuple field's store, so put the rarest component
/// first for the tightest loop.
pub struct QueryIter<'f, Q: QueryTuple<'f>> {
    entities: &'f [Entity],
    pos: usize,
    resolved: Q::Resolved,
    /// True when every field's store has the exact same dense entity order
    /// as the first field's (checked once, cheaply, in `new`). When true,
    /// `next` can read every field by dense position directly, skipping the
    /// sparse lookup entirely.
    aligned: bool,
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
        // A common shape (e.g. every entity gets its components added in
        // the same order, as with a bulk spawn loop) leaves every field's
        // dense arrays in identical entity order. Detect that once here, as
        // a single flat byte comparison per field, so `next` can skip the
        // sparse lookup entirely for the whole iteration when it holds.
        let mut aligned = true;
        for &s in &stores[1..] {
            // SAFETY: `s` points at a live store owned by `frame`, valid for
            // the `'f` borrow, per the same reasoning as `stores[0]` above.
            let other: &'f [Entity] = unsafe { (*s).entities() };
            if bytemuck::cast_slice::<Entity, u8>(other) != bytemuck::cast_slice::<Entity, u8>(entities) {
                aligned = false;
                break;
            }
        }
        // SAFETY: each `stores[i]` points at a live `Box<dyn AnyStore>`
        // whose concrete type matches `ids[i]` (built from the same
        // registry via `Q::component_ids`), and the no-mutable-aliasing
        // invariant was just checked above.
        let resolved = unsafe { Q::resolve(&stores) };
        Self { entities, pos: 0, resolved, aligned, without: Vec::new(), components_base: base, registry, _marker: PhantomData }
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

    #[inline]
    pub fn for_each(self, mut f: impl FnMut(Entity, Q::Item)) {
        if self.aligned && self.without.is_empty() {
            // Fast path: every field's dense entity order matches the first
            // field's (checked once in `new`), so the whole run can be
            // driven by a plain position loop with direct dense access,
            // instead of going through `Iterator::next`'s per-item
            // `Option` wrapping and the (here, always-false) `without`
            // check.
            let remaining = &self.entities[self.pos..];
            for (i, &e) in remaining.iter().enumerate() {
                // SAFETY: `self.aligned` means every field's dense entity
                // order equals the first field's, so dense position
                // `self.pos + i` (this entity's own slot in the first
                // field's store) is also the right dense slot in every
                // other field's store.
                let item = unsafe { Q::fetch_all_dense(&self.resolved, self.pos + i) };
                f(e, item);
            }
            return;
        }
        for (e, item) in self {
            f(e, item);
        }
    }
}

impl<'f, Q: QueryTuple<'f>> Iterator for QueryIter<'f, Q> {
    type Item = (Entity, Q::Item);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.pos < self.entities.len() {
            let idx = self.pos;
            let e = self.entities[idx];
            self.pos += 1;
            // SAFETY: pointers in `self.without` point at live stores owned
            // by the same frame, valid for `'f`.
            if !self.without.is_empty() && self.without.iter().any(|&s| unsafe { (*s).contains(e) }) {
                continue;
            }
            if self.aligned {
                // SAFETY: established by `QueryIter::new`: `self.resolved`
                // holds `Q::ARITY` valid, non-aliased-when-mutable pointers
                // matching `Q`'s component ids; `self.aligned` means every
                // field's dense entity order equals the first field's, so
                // `idx` (e's own dense position in the first field's store)
                // is also the right dense slot in every other field.
                let item = unsafe { Q::fetch_all_dense(&self.resolved, idx) };
                return Some((e, item));
            }
            // SAFETY: established by `QueryIter::new`: `self.resolved` holds
            // `Q::ARITY` valid, non-aliased-when-mutable pointers matching
            // `Q`'s component ids; `idx` is `e`'s own dense position in the
            // first field's store, since `e` was read from `self.entities`
            // which is that very store's dense entity list.
            if let Some(item) = unsafe { Q::fetch_lead(&self.resolved, idx, e) } {
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
