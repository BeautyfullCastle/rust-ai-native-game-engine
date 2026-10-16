#![forbid(clippy::float_arithmetic)]

use bytemuck::{Pod, Zeroable};
use orr_ecs::Component;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FloatState {
    pub value: f64,
}

pub fn component_bound<T: Component>() {}

pub fn accepts_float_state() {
    component_bound::<FloatState>();
}
