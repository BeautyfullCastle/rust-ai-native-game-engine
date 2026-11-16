//! Joints saved to a scene file and loaded back step like the ones made with `add_joint`.
use orr_ecs::{ComponentRegistry, ComponentRegistryBuilder, Entity, Frame};
use std::sync::Arc;

use orr_fp::{fp, FPVec2, FP};
use orr_physics::{
    add_joint, init, register, register_joints, register_joints_reflect, register_reflect, spawn_body, step, Body, Collider, Joint, PhysicsConfig, Scratch, Shape,
    JOINT_PIN, JOINT_ROD, JOINT_ROPE,
};
use orr_reflect::{Scene, TypeRegistry};

fn registry() -> (Arc<ComponentRegistry>, TypeRegistry) {
    let mut builder = ComponentRegistryBuilder::new();
    register(&mut builder);
    register_joints(&mut builder);
    let mut types = TypeRegistry::new();
    register_reflect(&mut types);
    register_joints_reflect(&mut types);
    (builder.build(), types)
}

/// A static hub and a tilted dynamic ball at `offset` from it, joined by `make`.
fn scene_frame(registry: &Arc<ComponentRegistry>, offset: FPVec2, make: impl Fn(Entity, Entity) -> Joint) -> Frame {
    let mut f = Frame::new(registry.clone());
    init(&mut f, PhysicsConfig::default());
    let at = FPVec2::new(FP::ZERO, fp!(10));
    let hub = spawn_body(&mut f, Body::new_static(at, fp!(0.3)), Collider { mask: 0, ..Collider::new(Shape::circle(fp!(0.1))) });
    let shape = Shape::circle(fp!(0.2));
    let ball = spawn_body(&mut f, Body::new_dynamic(at + offset, &shape, FP::ONE), Collider::new(shape));
    f.get_mut::<Body>(ball).unwrap().angle = fp!(0.7);
    add_joint(&mut f, make(hub, ball));
    f
}

fn round_trip_steps_the_same(name: &str, offset: FPVec2, make: impl Fn(Entity, Entity) -> Joint) {
    let (frame_registry, types) = registry();
    let mut original = scene_frame(&frame_registry, offset, make);
    let yaml = Scene::unbake(&types, &original, None).unwrap().to_yaml();
    assert!(yaml.contains("orr_physics::Joint"), "{name}: the joint is in the scene:\n{yaml}");
    assert!(!yaml.contains("ref_angle"), "{name}: the recorded angle stays out of the file");
    let mut loaded = Frame::new(frame_registry);
    Scene::parse(&yaml, &types).unwrap().bake(&types, &mut loaded).unwrap();

    let mut sc = (Scratch::new(), Scratch::new());
    let mut events = Vec::new();
    for tick in 1..=90 {
        original.set_tick(tick);
        loaded.set_tick(tick);
        step(&mut original, &mut sc.0, &mut events);
        step(&mut loaded, &mut sc.1, &mut events);
    }
    assert_eq!(original.dense::<Body>().1, loaded.dense::<Body>().1, "{name}: the loaded joint steps like the original");
    assert_eq!(original.dense::<Joint>().1, loaded.dense::<Joint>().1, "{name}: and records the same angle");
}

#[test]
fn a_pin_with_motor_and_limits_round_trips() {
    round_trip_steps_the_same("pin", FPVec2::new(fp!(2), FP::ZERO), |a, b| {
        Joint::new(JOINT_PIN, a, b, FPVec2::ZERO, FPVec2::new(fp!(-2), FP::ZERO), FP::ZERO).with_motor(fp!(2), fp!(50)).with_limits(fp!(-1), fp!(1))
    });
}

#[test]
fn a_rod_round_trips() {
    round_trip_steps_the_same("rod", FPVec2::new(fp!(2), FP::ZERO), |a, b| Joint::new(JOINT_ROD, a, b, FPVec2::ZERO, FPVec2::ZERO, fp!(2)));
}

#[test]
fn a_rope_round_trips() {
    round_trip_steps_the_same("rope", FPVec2::new(fp!(1), FP::ZERO), |a, b| Joint::new(JOINT_ROPE, a, b, FPVec2::ZERO, FPVec2::ZERO, fp!(3)));
}

#[test]
fn a_spring_round_trips() {
    round_trip_steps_the_same("spring", FPVec2::new(fp!(3), FP::ZERO), |a, b| Joint::spring(a, b, FPVec2::ZERO, FPVec2::ZERO, fp!(2), fp!(1.5), fp!(0.2)));
}

#[test]
fn a_prismatic_joint_with_motor_and_limits_round_trips() {
    round_trip_steps_the_same("prismatic", FPVec2::new(fp!(0.5), fp!(0.5)), |a, b| {
        Joint::prismatic(a, b, FPVec2::ZERO, FPVec2::ZERO, FPVec2::new(FP::ONE, FP::ONE)).with_motor(fp!(1), fp!(20)).with_limits(fp!(-1), fp!(2))
    });
}

#[test]
fn a_joint_loaded_from_a_scene_records_the_angle_at_the_first_step() {
    let (frame_registry, types) = registry();
    let original = scene_frame(&frame_registry, FPVec2::new(fp!(2), FP::ZERO), |a, b| Joint::new(JOINT_PIN, a, b, FPVec2::ZERO, FPVec2::new(fp!(-2), FP::ZERO), FP::ZERO));
    let yaml = Scene::unbake(&types, &original, None).unwrap().to_yaml();
    let mut loaded = Frame::new(frame_registry);
    Scene::parse(&yaml, &types).unwrap().bake(&types, &mut loaded).unwrap();
    let expected = original.dense::<Joint>().1[0].ref_angle;
    assert_ne!(loaded.dense::<Joint>().1[0].ref_angle, expected, "a loaded joint waits for the first step");
    loaded.set_tick(1);
    step(&mut loaded, &mut Scratch::new(), &mut Vec::new());
    assert_eq!(loaded.dense::<Joint>().1[0].ref_angle, expected);
}
