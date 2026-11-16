//! The joint markers of the 2D viewport: lines on top of the bodies, bold when selected.
#![allow(clippy::float_arithmetic)]

use orr_ecs::Entity;
use orr_editor::game::JointView;
use orr_editor::viewport::{add_joints, build_list, JOINT_AXIS_COLOR, JOINT_COLOR};
use orr_render::Camera;

fn joint(index: u32, axis: Option<[f32; 2]>) -> JointView {
    JointView { entity: Entity { index, version: 1 }, a: [1.0, 2.0], b: [4.0, 2.0], axis }
}

#[test]
fn a_joint_is_a_line_between_the_anchors_with_a_cross_on_the_first() {
    let camera = Camera::new([0.0, 0.0], 10.0);
    let size = (800, 600);
    let mut list = build_list(&[], None, &camera, size);
    let before = list.lines.len();
    add_joints(&mut list, &[joint(5, None)], None, &camera, size);
    let added = &list.lines[before..];
    assert_eq!(added.len(), 3, "the line and two lines of the cross");
    assert!(added.iter().all(|l| l.color == JOINT_COLOR && l.width == 1.5));
    assert_eq!((added[0].a, added[0].b), ([1.0, 2.0], [4.0, 2.0]));
    assert!(added[1..].iter().all(|l| l.a != l.b && (l.a[0] + l.b[0]) / 2.0 == 1.0 && (l.a[1] + l.b[1]) / 2.0 == 2.0));
    assert_eq!(list.shapes.len(), build_list(&[], None, &camera, size).shapes.len(), "no shapes are added");
}

#[test]
fn a_prismatic_joint_adds_an_axis_line_through_the_first_anchor() {
    let camera = Camera::new([0.0, 0.0], 10.0);
    let size = (800, 600);
    let mut list = build_list(&[], None, &camera, size);
    let before = list.lines.len();
    add_joints(&mut list, &[joint(5, Some([0.0, 1.0]))], None, &camera, size);
    let axis: Vec<_> = list.lines[before..].iter().filter(|l| l.color == JOINT_AXIS_COLOR).collect();
    assert_eq!(axis.len(), 1);
    assert_eq!((axis[0].a, axis[0].b), ([1.0, 1.4], [1.0, 2.6]));
}

#[test]
fn only_the_selected_joint_is_bold() {
    let camera = Camera::new([0.0, 0.0], 10.0);
    let size = (800, 600);
    let (first, second) = (joint(5, Some([1.0, 0.0])), joint(6, None));
    let mut list = build_list(&[], None, &camera, size);
    let before = list.lines.len();
    add_joints(&mut list, &[first, second], Some(first.entity), &camera, size);
    let added = &list.lines[before..];
    // First joint: line, two cross lines and the axis. Second: line and two cross lines.
    assert_eq!(added.len(), 7);
    assert!(added[..4].iter().all(|l| l.width == 3.0));
    assert!(added[4..].iter().all(|l| l.width == 1.5));
}
