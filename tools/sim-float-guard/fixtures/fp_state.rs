#![forbid(clippy::float_arithmetic)]

use bytemuck::{Pod, Zeroable};
use orr_ecs::Component;
use orr_fp::{FPVec2, FP};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Position {
    pub x: FP,
    pub velocity: FPVec2,
}

pub fn component_bound<T: Component>() {}

pub fn accepts_position() {
    component_bound::<Position>();
}
