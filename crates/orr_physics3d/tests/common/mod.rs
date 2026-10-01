//! Scene builders shared by the physics3d tests.
#![allow(dead_code)]

use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FPQuat, FPVec3, FP};
use orr_physics3d::{init, register, spawn_body, step, Body, Collider, PhysicsConfig, Scratch, Shape};

/// `v3!(x, y, z)` builds an `FPVec3` from decimal literals (no floats).
#[macro_export]
macro_rules! v3 {
    ($x:expr, $y:expr, $z:expr) => {
        orr_fp::FPVec3::new(orr_fp::fp!($x), orr_fp::fp!($y), orr_fp::fp!($z))
    };
}

pub fn new_frame() -> Frame {
    new_frame_with(PhysicsConfig::default())
}

pub fn new_frame_with(cfg: PhysicsConfig) -> Frame {
    let mut b = ComponentRegistryBuilder::new();
    register(&mut b);
    let mut f = Frame::new(b.build());
    init(&mut f, cfg);
    f
}

/// A large static ground slab whose top face is the plane y = 0.
pub fn ground(f: &mut Frame) -> Entity {
    spawn_body(
        f,
        Body::new_static(v3!(0, -0.5, 0)),
        Collider::new(Shape::cuboid(fp!(200), FP::HALF, fp!(200))).with_friction(fp!(0.6)),
    )
}

pub fn spawn_box(f: &mut Frame, pos: FPVec3, half: FPVec3) -> Entity {
    let s = Shape::cuboid(half.x, half.y, half.z);
    spawn_body(f, Body::new_dynamic(pos, &s, FP::ONE), Collider::new(s).with_friction(fp!(0.6)))
}

pub fn spawn_sphere(f: &mut Frame, pos: FPVec3, r: FP) -> Entity {
    let s = Shape::sphere(r);
    spawn_body(f, Body::new_dynamic(pos, &s, FP::ONE), Collider::new(s).with_friction(fp!(0.6)))
}

pub fn spawn_capsule(f: &mut Frame, pos: FPVec3, half_len: FP, r: FP, rot: FPQuat) -> Entity {
    let s = Shape::capsule(half_len, r);
    spawn_body(f, Body::new_dynamic(pos, &s, FP::ONE).with_rotation(rot), Collider::new(s).with_friction(fp!(0.6)))
}

pub fn run(f: &mut Frame, sc: &mut Scratch, ticks: u32) {
    for _ in 0..ticks {
        step(f, sc);
    }
}

pub fn body(f: &Frame, e: Entity) -> Body {
    *f.get::<Body>(e).unwrap()
}
