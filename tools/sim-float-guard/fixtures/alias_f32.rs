#![forbid(clippy::float_arithmetic)]

use bytemuck::{Pod, Zeroable};
use orr_ecs::Component;

type Scalar = f32;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct AliasedState {
    pub value: Scalar,
}

pub fn component_bound<T: Component>() {}

pub fn accepts_aliased_state() {
    component_bound::<AliasedState>();
}
