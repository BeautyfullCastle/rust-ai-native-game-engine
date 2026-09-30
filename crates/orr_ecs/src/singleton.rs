use core::any::Any;

use crate::codec::{FrameDecodeError, Reader};
use crate::component::Component;

/// Type-erased holder for one registered singleton value.
pub trait AnySingleton: Send + Sync {
    fn bytes(&self) -> &[u8];
    fn bytes_mut(&mut self) -> &mut [u8];
    fn copy_from(&mut self, other: &dyn AnySingleton);
    /// Overwrites the value from exactly `bytes().len()` bytes of `r`.
    fn read_bytes(&mut self, r: &mut Reader) -> Result<(), FrameDecodeError>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Concrete storage for singleton `T`. Defaults to `T::zeroed()` (all
/// `Component`/`Pod` types are `Zeroable`), so a freshly-built `Frame` always
/// has a well-defined value even before `set_singleton` is called.
pub struct SingletonSlot<T: Component>(pub T);

impl<T: Component> Default for SingletonSlot<T> {
    fn default() -> Self {
        Self(bytemuck::Zeroable::zeroed())
    }
}

impl<T: Component> AnySingleton for SingletonSlot<T> {
    fn bytes(&self) -> &[u8] {
        bytemuck::bytes_of(&self.0)
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        bytemuck::bytes_of_mut(&mut self.0)
    }

    fn copy_from(&mut self, other: &dyn AnySingleton) {
        let other = other
            .as_any()
            .downcast_ref::<SingletonSlot<T>>()
            .expect("orr_ecs: copy_from between mismatched singleton types");
        self.0 = other.0;
    }

    fn read_bytes(&mut self, r: &mut Reader) -> Result<(), FrameDecodeError> {
        let raw = r.take(core::mem::size_of::<T>())?;
        bytemuck::bytes_of_mut(&mut self.0).copy_from_slice(raw);
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
