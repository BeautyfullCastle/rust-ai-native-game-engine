use orr_ecs::{Component, Entity, Frame};

/// A read-only window onto one sim [`Frame`].
///
/// The view layer may query components but has no way to change them: this
/// type wraps a shared reference and exposes only `&self` methods. The
/// underlying frame is also kept behind an `Arc` inside [`crate::Snapshot`],
/// so it cannot be reached mutably even by cloning the snapshot.
///
/// ```compile_fail
/// use orr_bridge::FrameView;
/// fn mutate(view: FrameView<'_>, e: orr_ecs::Entity) {
///     // There is no `get_mut`, `add`, `spawn` or `frame_mut` on `FrameView`.
///     let _ = view.get_mut::<u32>(e);
/// }
/// ```
#[derive(Clone, Copy)]
pub struct FrameView<'a> {
    frame: &'a Frame,
}

impl<'a> FrameView<'a> {
    pub(crate) fn new(frame: &'a Frame) -> Self {
        Self { frame }
    }

    /// A read-only window onto a frame that does not come from a snapshot:
    /// a host that holds a `Frame` itself and runs view-side code on it
    /// (the view stream of `orr_viewstream`, an editor viewport). It grants
    /// exactly what a snapshot's view does: reads only.
    pub fn of(frame: &'a Frame) -> Self {
        Self { frame }
    }

    /// The sim tick this frame is the state of.
    pub fn tick(&self) -> u64 {
        self.frame.tick()
    }

    /// Whether `e` is alive. A stale handle (same index, older version) is not.
    pub fn exists(&self, e: Entity) -> bool {
        self.frame.exists(e)
    }

    pub fn alive_count(&self) -> u32 {
        self.frame.alive_count()
    }

    /// `e`'s component `T`, if it has one. Panics if `T` is not registered.
    pub fn get<T: Component>(&self, e: Entity) -> Option<&'a T> {
        self.frame.get::<T>(e)
    }

    pub fn has<T: Component>(&self, e: Entity) -> bool {
        self.frame.has::<T>(e)
    }

    pub fn count<T: Component>(&self) -> u32 {
        self.frame.count::<T>()
    }

    /// Every `T`, in the deterministic dense order of the sim: the owning
    /// entities and the values, as parallel slices.
    pub fn dense<T: Component>(&self) -> (&'a [Entity], &'a [T]) {
        self.frame.dense::<T>()
    }

    /// Iterates `(entity, &T)` for every entity that has `T`.
    pub fn iter<T: Component>(&self) -> impl Iterator<Item = (Entity, &'a T)> {
        let (entities, data) = self.frame.dense::<T>();
        entities.iter().copied().zip(data.iter())
    }

    /// A singleton value. Panics if `T` is not a registered singleton.
    pub fn singleton<T: Component>(&self) -> &'a T {
        self.frame.singleton::<T>()
    }

    /// The frame checksum (xxh3), for tests and desync diagnostics.
    pub fn checksum(&self) -> u64 {
        self.frame.checksum()
    }
}
