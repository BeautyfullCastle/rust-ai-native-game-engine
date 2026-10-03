#![forbid(clippy::float_arithmetic)]

use bytemuck::{Pod, Zeroable};
use orr_ecs::Component;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Nested {
    pub value: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct NestedState {
    pub nested: Nested,
}

pub fn component_bound<T: Component>() {}

pub fn accepts_nested_state() {
    component_bound::<NestedState>();
}
