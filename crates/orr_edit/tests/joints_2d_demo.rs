//! The 2D joint demo scene loads in the 2D physics game, steps, and keeps its joints.
mod common;

use common::*;
use orr_edit::{EditorDoc, PlayController};
use orr_physics::{Body, Joint, JOINT_PIN, JOINT_PRISMATIC, JOINT_SPRING};
use orr_sample::physics_game::PhysGame;
use orr_session::ControlOp;
use orr_sim::Simulation;

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes/joints_2d_demo.scene.yaml");

fn doc() -> EditorDoc {
    let text = std::fs::read_to_string(PATH).expect("scenes/joints_2d_demo.scene.yaml");
    EditorDoc::from_yaml(&text, types(), Simulation::<PhysGame>::build_registry(), SEED).expect("load the joint demo")
}

#[test]
fn the_joint_demo_loads_steps_and_moves_its_joints() {
    let doc = doc();
    let mut pc = PlayController::<PhysGame>::start_play(&doc, doc.play_config(1, 60)).expect("start play");
    let before: Vec<Body> = pc.session().frame().dense::<Body>().1.to_vec();
    pc.control(ControlOp::Step(120));
    let frame = pc.session().frame();
    let joints = frame.dense::<Joint>().1;
    assert_eq!(joints.len(), 4, "pin, motor pin, prismatic and spring");
    for kind in [JOINT_PIN, JOINT_PRISMATIC, JOINT_SPRING] {
        assert!(joints.iter().any(|j| j.kind == kind), "kind {kind} is in the demo");
    }
    assert!(joints.iter().all(|j| j.ref_angle.abs() < orr_fp::fp!(4)), "the first step recorded every joint");
    assert_ne!(before, frame.dense::<Body>().1, "the bodies moved");

    let mut again = PlayController::<PhysGame>::start_play(&doc, doc.play_config(1, 60)).expect("start play");
    again.control(ControlOp::Step(120));
    assert_eq!(frame.checksum(), again.session().frame().checksum(), "the demo is deterministic");
}

fn slider(frame: &orr_ecs::Frame) -> Joint {
    *frame.dense::<Joint>().1.iter().find(|j| j.kind == JOINT_PRISMATIC).expect("the demo has a slider")
}

#[test]
fn an_anchor_drag_is_one_undo_step_and_the_joint_records_its_reference_again() {
    use orr_edit::{Op, Origin};
    let mut doc = doc();
    let guid = guid_named(&doc, "slider_prismatic");
    let before = state(&doc);
    let steps_before = doc.history().len();

    // A drag is one transaction holding many writes of the same field.
    doc.begin_tx("move joint anchor", Origin::User).unwrap();
    for x in 1..=20 {
        let op = Op::SetField { guid: guid.clone(), component: "orr_physics::Joint".into(), path: "anchor_a".into(), value: vec2(x, 2) };
        doc.apply(op, Origin::User).unwrap();
    }
    doc.commit_tx().unwrap();
    assert_eq!(doc.history().len(), steps_before + 1, "one undo step per drag");
    let edited = slider(doc.frame());
    assert_eq!(edited.anchor_a, orr_fp::FPVec2::new(orr_fp::FP::from_int(20), orr_fp::FP::from_int(2)));
    assert_eq!(edited.ref_angle, orr_fp::fp!(100), "the edited joint has no recorded angle");

    let mut pc = PlayController::<PhysGame>::start_play(&doc, doc.play_config(1, 60)).expect("start play");
    pc.control(ControlOp::Step(1));
    assert!(slider(pc.session().frame()).ref_angle.abs() < orr_fp::fp!(4), "the first step recorded the angle again");

    doc.undo().unwrap();
    assert_eq!(state(&doc), before, "one undo restores the scene");
}
