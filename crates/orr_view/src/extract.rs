use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_fp::{FPVec2, FP};

use crate::math::{Transform2, Vec2};

/// The one place `FP` becomes `f32` (design doc 4.1). Lossy by design.
pub fn fp_to_f32(v: FP) -> f32 {
    v.to_f32()
}

pub fn fp_to_vec2(v: FPVec2) -> Vec2 {
    Vec2::new(v.x.to_f32(), v.y.to_f32())
}

/// How an entity is shown between ticks (design doc 4.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InterpMode {
    /// Interpolate the last two predicted ticks; smooth rollback corrections.
    /// For entities the local player controls.
    Prediction,
    /// Play the confirmed (verified) frames slightly late, so a wrong guess
    /// is never shown. For remote entities and spectators.
    Snapshot,
    /// Show the newest predicted tick as it is.
    None,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Circle,
    Quad,
}

/// How an entity looks. Not interpolated: the newest value is used.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Style {
    pub shape: Shape,
    /// Radius (circle) or half side length (quad), in world units.
    pub size: f32,
    pub color: [f32; 4],
}

/// One drawable entity as read from one sim frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Extracted {
    pub entity: Entity,
    pub transform: Transform2,
    pub mode: InterpMode,
    pub style: Style,
}

/// Game-specific code that reads drawable entities out of a sim frame.
/// Called on the view side with a read-only [`FrameView`], once per new
/// predicted frame (twice with the previous tick) and once per new verified
/// frame. Must append to `out` and must be a pure function of the frame.
pub trait Extractor {
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted>);
}
