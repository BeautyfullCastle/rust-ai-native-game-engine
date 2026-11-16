//! The world placement of 2D joints for editor markers.
#![allow(clippy::float_arithmetic)]
use orr_bridge::FrameView;
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{fp, FPVec2, FP};
use orr_physics::{add_joint, init, register, register_joints, spawn_body, Body, Collider, Joint, PhysicsConfig, Shape, JOINT_ROD};
use orr_sample::physics_view::joint_views;

fn close(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 1.0e-3 && (a[1] - b[1]).abs() < 1.0e-3
}

fn new_frame() -> Frame {
    let mut b = ComponentRegistryBuilder::new();
    register(&mut b);
    register_joints(&mut b);
    let mut f = Frame::new(b.build());
    init(&mut f, PhysicsConfig::default());
    f
}

fn body(frame: &mut Frame, x: i32, y: i32, angle: FP) -> orr_ecs::Entity {
    spawn_body(frame, Body::new_static(FPVec2::new(FP::from_int(x), FP::from_int(y)), angle), Collider::new(Shape::circle(fp!(0.5))))
}

#[test]
fn anchors_are_placed_by_the_body_poses() {
    let mut f = new_frame();
    // A quarter turn: the local anchor (1, 0) lands at (0, 1) above the body.
    let a = body(&mut f, 2, 3, fp!(1.5707963));
    let b = body(&mut f, 6, 3, FP::ZERO);
    let rod = add_joint(&mut f, Joint::new(JOINT_ROD, a, b, FPVec2::new(FP::ONE, FP::ZERO), FPVec2::new(fp!(-1), FP::ZERO), fp!(4)));
    // A joint to a missing body is not placed.
    let gone = f.spawn();
    add_joint(&mut f, Joint::new(JOINT_ROD, a, gone, FPVec2::ZERO, FPVec2::ZERO, FP::ONE));
    let views = joint_views(FrameView::of(&f));
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].entity, rod);
    assert!(close(views[0].a, [2.0, 4.0]), "{:?}", views[0].a);
    assert!(close(views[0].b, [5.0, 3.0]), "{:?}", views[0].b);
    assert_eq!(views[0].axis, None);
}

#[test]
fn a_prismatic_axis_turns_with_the_first_body() {
    let mut f = new_frame();
    let a = body(&mut f, 0, 0, fp!(1.5707963));
    let b = body(&mut f, 3, 0, FP::ZERO);
    // The axis is along local x of `a`, so after a quarter turn it points up.
    add_joint(&mut f, Joint::prismatic(a, b, FPVec2::ZERO, FPVec2::ZERO, FPVec2::new(FP::ONE, FP::ZERO)));
    let views = joint_views(FrameView::of(&f));
    assert!(close(views[0].axis.unwrap(), [0.0, 1.0]), "{:?}", views[0].axis);
}

#[test]
fn a_frame_without_joints_registered_has_no_views() {
    let mut b = ComponentRegistryBuilder::new();
    register(&mut b);
    let f = Frame::new(b.build());
    assert!(joint_views(FrameView::of(&f)).is_empty());
}
