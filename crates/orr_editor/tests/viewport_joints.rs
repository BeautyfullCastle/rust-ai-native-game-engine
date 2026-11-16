//! The joint markers of the 2D viewport: lines on top of the bodies, bold when selected.
#![allow(clippy::float_arithmetic, clippy::disallowed_types)]

use orr_ecs::Entity;
use orr_editor::game::{JointView, Pose};
use orr_editor::viewport::{
    add_anchor_handles, add_joints, build_list, dragged_anchors, pick_anchor, AnchorGrab, HANDLE_COLOR, JOINT_AXIS_COLOR, JOINT_COLOR,
};
use orr_render::Camera;

fn joint(index: u32, axis: Option<[f32; 2]>) -> JointView {
    let pose_a = Pose { pos: [0.0, 2.0], angle: 0.0 };
    let pose_b = Pose { pos: [4.0, 2.0], angle: std::f32::consts::FRAC_PI_2 };
    JointView { entity: Entity { index, version: 1 }, a: [1.0, 2.0], b: [4.0, 2.0], axis, pose_a, pose_b }
}

fn scene() -> (Camera, (u32, u32)) {
    (Camera::new([0.0, 0.0], 10.0), (800, 600))
}

#[test]
fn a_pose_takes_a_world_point_to_the_body_and_back() {
    let pose = Pose { pos: [3.0, -1.0], angle: 0.7 };
    for local in [[0.0, 0.0], [1.5, 0.0], [-0.25, 2.0]] {
        let back = pose.to_local(pose.to_world(local));
        assert!((back[0] - local[0]).abs() < 1e-5 && (back[1] - local[1]).abs() < 1e-5, "{local:?} came back as {back:?}");
    }
    let turned = Pose { pos: [0.0, 0.0], angle: std::f32::consts::FRAC_PI_2 };
    let local = turned.to_local([0.0, 2.0]);
    assert!((local[0] - 2.0).abs() < 1e-5 && local[1].abs() < 1e-5, "a body turned a quarter sees +y as its +x");
}

#[test]
fn handles_exist_only_for_the_selected_joint() {
    let (camera, size) = scene();
    let joints = [joint(5, None), joint(6, None)];
    let on_a = [1.0, 2.0];
    assert!(pick_anchor(&joints, None, on_a, &camera, size).is_none());
    let hit = pick_anchor(&joints, Some(joints[1].entity), on_a, &camera, size).unwrap();
    assert_eq!((hit.joint, hit.grab, hit.center), (joints[1].entity, AnchorGrab::A, on_a));
    assert!(pick_anchor(&joints, Some(Entity { index: 9, version: 1 }), on_a, &camera, size).is_none());
}

#[test]
fn a_press_off_the_handles_grabs_nothing_and_the_nearer_anchor_wins() {
    let (camera, size) = scene();
    let selected = joint(5, None);
    let reach = 7.0 / camera.pixels_per_unit(size.0, size.1);
    assert!(pick_anchor(&[selected], Some(selected.entity), [1.0, 2.0 + 1.5 * reach], &camera, size).is_none());
    assert_eq!(pick_anchor(&[selected], Some(selected.entity), [1.0, 2.0 + 0.5 * reach], &camera, size).unwrap().grab, AnchorGrab::A);
    assert_eq!(pick_anchor(&[selected], Some(selected.entity), [4.0 + 0.5 * reach, 2.0], &camera, size).unwrap().grab, AnchorGrab::B);
    let mut close = selected;
    close.b = [1.0 + 0.6 * reach, 2.0];
    let near_b = [1.0 + 0.4 * reach, 2.0];
    assert_eq!(pick_anchor(&[close], Some(close.entity), near_b, &camera, size).unwrap().grab, AnchorGrab::B, "the nearer one");
    assert_eq!(pick_anchor(&[close], Some(close.entity), [1.0, 2.0], &camera, size).unwrap().grab, AnchorGrab::A);
}

#[test]
fn anchors_on_one_spot_move_together() {
    let (camera, size) = scene();
    let mut pin = joint(5, None);
    pin.b = pin.a;
    let hit = pick_anchor(&[pin], Some(pin.entity), pin.a, &camera, size).unwrap();
    assert_eq!(hit.grab, AnchorGrab::Both);
    let (a, b) = dragged_anchors(&pin, AnchorGrab::Both, [2.0, 2.0]);
    assert_eq!(a, Some([2.0, 0.0]), "first body at (0, 2), not turned");
    let b = b.unwrap();
    assert!(b[0].abs() < 1e-5 && (b[1] - 2.0).abs() < 1e-5, "second body at (4, 2), turned a quarter: {b:?}");
}

#[test]
fn a_drag_gives_the_local_anchor_of_the_grabbed_side_only() {
    let selected = joint(5, None);
    let (a, b) = dragged_anchors(&selected, AnchorGrab::A, [1.5, 3.0]);
    assert_eq!((a, b), (Some([1.5, 1.0]), None));
    let (a, b) = dragged_anchors(&selected, AnchorGrab::B, [4.0, 3.0]);
    assert_eq!(a, None);
    let b = b.unwrap();
    assert!((b[0] - 1.0).abs() < 1e-5 && b[1].abs() < 1e-5, "{b:?}");
}

#[test]
fn the_selected_joint_gets_a_disc_and_a_square_handle_on_top() {
    let (camera, size) = scene();
    let selected = joint(5, None);
    let mut list = build_list(&[], None, &camera, size);
    let (lines, shapes) = (list.lines.len(), list.shapes.len());
    add_anchor_handles(&mut list, &[selected], None, &camera, size);
    assert_eq!((list.lines.len(), list.shapes.len()), (lines, shapes), "no selection, no handles");
    add_anchor_handles(&mut list, &[selected], Some(selected.entity), &camera, size);
    assert_eq!(list.shapes.len(), shapes + 2, "a disc and a square");
    assert!(list.lines.len() > lines, "rings");
    assert!(list.shapes[shapes..].iter().all(|s| s.color == HANDLE_COLOR));
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

mod drag {
    use super::*;
    use orr_editor::game::EditorGame;
    use orr_editor::{Editor, Mode};
    use serde_json::json;
    use std::time::{Duration, Instant};

    const SCENE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes/joints_2d_demo.scene.yaml");
    const VP: (u32, u32) = (800, 600);

    fn editor_on_the_slider() -> Editor {
        let mut ed = Editor::open_game(std::path::Path::new(SCENE), EditorGame::PhysGame).expect("open the joint demo");
        ed.sync();
        assert!(ed.select_named("slider_prismatic"));
        ed.sync();
        ed
    }

    fn selected_joint(ed: &Editor) -> JointView {
        let entity = ed.entity_of(ed.selection().expect("a selection")).expect("the selected entity");
        *ed.joints().iter().find(|j| j.entity == entity).expect("the slider joint is drawn")
    }

    fn settle(ed: &mut Editor) {
        let until = Instant::now() + Duration::from_secs(8);
        while ed.in_gesture() {
            assert!(Instant::now() < until, "the drag did not finish");
            ed.pump();
            std::thread::sleep(Duration::from_millis(2));
        }
        ed.sync();
    }

    fn anchor(ed: &mut Editor, path: &str) -> [f64; 2] {
        let target = ed.selection().cloned().expect("a selection");
        let value = ed.host_call("world.get", json!({"entity": target.param(), "component": "orr_physics::Joint", "path": path})).expect("read the anchor");
        [value["value"][0].as_f64().unwrap(), value["value"][1].as_f64().unwrap()]
    }

    #[test]
    fn a_handle_drag_is_one_undo_step_and_stores_the_local_anchor() {
        let mut ed = editor_on_the_slider();
        let joint = selected_joint(&ed);
        let hit = ed.pick_anchor(joint.a, VP).expect("the handle is under the pointer");
        assert_eq!((hit.joint, hit.grab), (joint.entity, AnchorGrab::Both), "the demo slider has both anchors on one spot");
        let hit = orr_editor::viewport::AnchorHit { grab: AnchorGrab::A, ..hit };
        let (steps, start) = (ed.history().entries.len(), anchor(&mut ed, "anchor_a"));
        assert_eq!(start, [0.0, 0.0]);

        let target = [joint.a[0] + 1.0, joint.a[1] + 0.5];
        ed.begin_edit("move joint anchor");
        for i in 1..=10 {
            let f = i as f32 / 10.0;
            assert!(ed.drag_anchor(hit.joint, hit.grab, [joint.a[0] + (target[0] - joint.a[0]) * f, joint.a[1] + (target[1] - joint.a[1]) * f]));
        }
        ed.end_edit();
        settle(&mut ed);

        assert_eq!(ed.history().entries.len(), steps + 1, "ten mouse moves are one undo step");
        let expected = joint.pose_a.to_local(target);
        let stored = anchor(&mut ed, "anchor_a");
        assert!((stored[0] - f64::from(expected[0])).abs() < 1e-3 && (stored[1] - f64::from(expected[1])).abs() < 1e-3, "{stored:?} vs {expected:?}");
        assert_eq!(anchor(&mut ed, "anchor_b"), [0.0, 0.0], "the other anchor stays");
        let moved = selected_joint(&ed);
        assert!((moved.a[0] - target[0]).abs() < 1e-3 && (moved.a[1] - target[1]).abs() < 1e-3, "the marker follows: {:?}", moved.a);

        assert!(ed.undo());
        ed.sync();
        assert_eq!(anchor(&mut ed, "anchor_a"), [0.0, 0.0], "one undo puts the anchor back");
    }

    #[test]
    fn handles_are_not_there_in_play_or_for_another_selection() {
        let mut ed = editor_on_the_slider();
        let joint = selected_joint(&ed);
        let handles = |ed: &Editor| ed.viewport_list(VP, &[]).shapes.iter().filter(|s| s.color == HANDLE_COLOR).count();
        assert_eq!(handles(&ed), 2);
        assert!(ed.select_named("spring_hook"));
        ed.sync();
        assert!(ed.pick_anchor(joint.a, VP).is_none());
        assert_eq!(handles(&ed), 0);

        assert!(ed.select_named("slider_prismatic"));
        ed.sync();
        ed.play();
        ed.sync();
        assert_eq!(ed.mode(), Mode::Play);
        assert!(ed.pick_anchor(joint.a, VP).is_none());
        assert!(!ed.drag_anchor(joint.entity, AnchorGrab::A, [0.0, 0.0]));
        assert_eq!(handles(&ed), 0);
    }
}
