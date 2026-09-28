use bytemuck::Pod;

/// Marker trait for types that can live in a [`Frame`](crate::Frame): as a
/// per-entity component, a singleton, or the element type of a
/// [`FrameList`](crate::FrameList).
///
/// It is blanket-implemented for every `T: Pod + Send + Sync + 'static`, so
/// there is nothing to implement by hand — derive `Pod`/`Zeroable` (and
/// `Clone`/`Copy`, `#[repr(C)]`) on a plain-old-data struct and it is a
/// `Component`:
///
/// ```
/// use bytemuck::{Pod, Zeroable};
///
/// #[repr(C)]
/// #[derive(Clone, Copy, Pod, Zeroable)]
/// struct Position { x: i64, y: i64 }
/// ```
///
/// `Pod` is the load-bearing bound: it guarantees no padding bytes and no
/// heap pointers, which is what makes byte-for-byte checksums deterministic
/// and `memcpy`-style snapshot/restore sound.
pub trait Component: Pod + Send + Sync + 'static {}
impl<T: Pod + Send + Sync + 'static> Component for T {}

/// Dense index of a registered component type within a
/// [`ComponentRegistry`](crate::ComponentRegistry). Assigned in registration
/// order; that order defines checksum iteration order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ComponentId(pub u16);

/// Dense index of a registered singleton type.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SingletonId(pub u16);

/// Dense index of a registered [`FrameList`](crate::FrameList) element type.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ListId(pub u16);
