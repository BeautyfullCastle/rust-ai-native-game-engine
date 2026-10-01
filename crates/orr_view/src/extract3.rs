//! The 3D counterpart of `extract`: what a 3D game's extractor produces.

use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_fp::{FPQuat, FPVec3};

use crate::extract::InterpMode;
use crate::math3::{Quat, Transform3, Vec3};

/// `FP` vector to `f32`: the one place a 3D position becomes a float.
pub fn fp_to_vec3(v: FPVec3) -> Vec3 {
    Vec3::new(v.x.to_f32(), v.y.to_f32(), v.z.to_f32())
}

/// `FP` quaternion to a normalized `f32` quaternion.
pub fn fp_to_quat(q: FPQuat) -> Quat {
    Quat::new(q.x.to_f32(), q.y.to_f32(), q.z.to_f32(), q.w.to_f32()).normalize()
}

/// A body's pose from its fixed point position and orientation.
pub fn fp_to_transform3(pos: FPVec3, rot: FPQuat) -> Transform3 {
    Transform3::new(fp_to_vec3(pos), fp_to_quat(rot))
}

/// The drawable shape of a 3D entity, in world units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shape3 {
    Sphere { radius: f32 },
    /// Half extents along the local axes.
    Box { half: [f32; 3] },
    /// A segment along local y (`half_length` to each side) grown by `radius`.
    Capsule { half_length: f32, radius: f32 },
    /// A horizontal rectangle at the entity's position (local xz, normal local y).
    Plane { half_x: f32, half_z: f32 },
}

/// How a 3D entity looks. Not interpolated: the newest value is used.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Style3 {
    pub shape: Shape3,
    /// Linear RGBA.
    pub color: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    /// Ground checker pattern.
    pub checker: bool,
}

impl Style3 {
    /// A plain dielectric of `color`.
    pub fn new(shape: Shape3, color: [f32; 3]) -> Self {
        Self { shape, color: [color[0], color[1], color[2], 1.0], roughness: 0.6, metallic: 0.0, checker: false }
    }
}

/// One drawable 3D entity as read from one sim frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Extracted3 {
    pub entity: Entity,
    pub transform: Transform3,
    pub mode: InterpMode,
    pub style: Style3,
}

/// Game specific code that reads drawable 3D entities out of a sim frame.
/// Same contract as [`crate::Extractor`]: a pure function of the frame that
/// appends to `out`.
pub trait Extractor3 {
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted3>);
}
