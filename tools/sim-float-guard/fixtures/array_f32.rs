#![forbid(clippy::float_arithmetic)]

use bytemuck::{Pod, Zeroable};
use orr_ecs::Component;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FloatArrayState {
    pub values: [f32; 4],
}

pub fn component_bound<T: Component>() {}

pub fn accepts_float_array() {
    component_bound::<FloatArrayState>();
}
