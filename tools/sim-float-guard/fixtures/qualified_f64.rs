#![forbid(clippy::float_arithmetic)]

use bytemuck::{Pod, Zeroable};
use orr_ecs::Component;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct QualifiedState {
    pub value: core::primitive::f64,
}

pub fn component_bound<T: Component>() {}

pub fn accepts_qualified_state() {
    component_bound::<QualifiedState>();
}
