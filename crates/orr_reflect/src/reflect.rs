//! The [`Reflect`] trait and its implementations for the basic types.

use bytemuck::Pod;
use orr_ecs::Entity;
use orr_fp::{FPVec2, FPVec3, FP, FP32};

use crate::desc::{IntKind, TypeDesc};

/// A `Pod` type that can describe itself field by field.
///
/// Derive it with `#[derive(Reflect)]` (see the crate docs for the field
/// attributes) or write it by hand when the raw layout is not what an author
/// should see (for example `orr_physics::Shape`).
pub trait Reflect: Pod + Send + Sync + 'static {
    /// Describes the type. Called once at registration.
    fn describe() -> TypeDesc;

    /// The value a newly added component starts with. All bytes zero unless
    /// the type says otherwise.
    fn default_value() -> Self {
        <Self as bytemuck::Zeroable>::zeroed()
    }
}

macro_rules! reflect_int {
    ($($t:ty => $k:expr),* $(,)?) => {$(
        impl Reflect for $t {
            fn describe() -> TypeDesc {
                TypeDesc::int($k)
            }
        }
    )*};
}

reflect_int! {
    u8 => IntKind::U8,
    u16 => IntKind::U16,
    u32 => IntKind::U32,
    u64 => IntKind::U64,
    i8 => IntKind::I8,
    i16 => IntKind::I16,
    i32 => IntKind::I32,
    i64 => IntKind::I64,
}

impl Reflect for FP {
    fn describe() -> TypeDesc {
        TypeDesc::fixed()
    }
}
impl Reflect for FP32 {
    fn describe() -> TypeDesc {
        TypeDesc::fixed32()
    }
}
impl Reflect for FPVec2 {
    fn describe() -> TypeDesc {
        TypeDesc::vec2()
    }
}
impl Reflect for FPVec3 {
    fn describe() -> TypeDesc {
        TypeDesc::vec3()
    }
}
impl Reflect for Entity {
    fn describe() -> TypeDesc {
        TypeDesc::entity()
    }
    fn default_value() -> Self {
        Entity::NONE
    }
}

impl<T: Reflect, const N: usize> Reflect for [T; N]
where
    [T; N]: Pod,
{
    fn describe() -> TypeDesc {
        TypeDesc::array(T::describe(), N)
    }
    fn default_value() -> Self {
        [T::default_value(); N]
    }
}
