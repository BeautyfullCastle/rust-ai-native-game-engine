use core::any::Any;

use crate::component::Component;

/// Type-erased holder for one registered singleton value.
pub trait AnySingleton: Send + Sync {
    fn bytes(&self) -> &[u8];
    fn copy_from(&mut self, other: &dyn AnySingleton);
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

    fn copy_from(&mut self, other: &dyn AnySingleton) {
        let other = other
            .as_any()
            .downcast_ref::<SingletonSlot<T>>()
            .expect("orr_ecs: copy_from between mismatched singleton types");
        self.0 = other.0;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
