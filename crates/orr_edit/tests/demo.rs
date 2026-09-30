#![allow(clippy::disallowed_types)] // timing note only (Instant); this is a tool crate

mod common;

use std::time::Instant;

use common::*;
use orr_edit::{Op, Origin, PlayController};
use orr_sample::physics_game::PhysGame;
use orr_session::ControlOp;

#[test]
fn demo_scene_round_trips_byte_identically() {
    let text = demo_text();
    assert!(!text.contains('\r'), "scene files use LF");
    let doc = demo_doc();
    assert_eq!(doc.to_yaml(), text, "unedited load returns the file text");
    // The regenerated text (not the remembered one) is identical too.
    assert_eq!(doc.scene().to_yaml(), text);
    assert_eq!(doc.scene().entities.len(), 49);
    let bodies = doc.view().entities().iter().filter(|e| e.name.as_deref().is_some_and(|n| n.starts_with("body_"))).count();
    assert_eq!(bodies, 40);
    let named = |n: &str| doc.view().entities().iter().any(|e| e.name.as_deref() == Some(n));
    assert!(named("floor") && named("wall_left") && named("wall_right") && named("paddle_0") && named("paddle_1"));
    assert_synced(&doc);
}

#[test]
fn demo_scene_plays_deterministically() {
    const N: u32 = 300;
    let run = || {
        let doc = demo_doc();
        let mut pc = PlayController::<PhysGame>::start_play(&doc, doc.play_config(2, 60)).unwrap();
        pc.control(ControlOp::Step(N));
        assert_eq!(pc.session().head_tick(), u64::from(N));
        pc.session().frame().checksum()
    };
    let (a, b) = (run(), run());
    assert_eq!(a, b, "same scene, same ticks, same checksum");
    assert_ne!(a, demo_doc().checksum());
}

/// Timing note: one set-field edit on the demo scene, with the in-place preview
/// patch and with a full rebake. Run with `--nocapture` to see the numbers.
#[test]
fn timing_note_set_field_and_rebake() {
    let mut doc = demo_doc();
    let body = guid_named(&doc, "body_10");
    let iters = 2000;

    let t = Instant::now();
    for i in 0..iters {
        let op = Op::SetField {
            guid: body.clone(),
            component: "orr_physics::Body".into(),
            path: "pos.x".into(),
            value: fixed(1 + (i % 7)),
        };
        doc.apply(op, Origin::User).unwrap();
    }
    let per_edit = t.elapsed() / iters as u32;

    let t = Instant::now();
    for _ in 0..iters {
        doc.rebake().unwrap();
    }
    let per_rebake = t.elapsed() / iters as u32;

    let t = Instant::now();
    for i in 0..iters {
        doc.apply(Op::Rename { guid: body.clone(), name: Some(format!("n{i}")) }, Origin::User).unwrap();
        doc.apply(Op::AddComponent { guid: body.clone(), component: "PaddleTag".into(), value: None }, Origin::User).unwrap();
        doc.apply(Op::RemoveComponent { guid: body.clone(), component: "PaddleTag".into() }, Origin::User).unwrap();
    }
    let per_structural = t.elapsed() / (iters as u32 * 3);

    println!(
        "orr_edit timing on the demo scene (49 entities): set-field edit {per_edit:?} (in-place patch), \
         full rebake {per_rebake:?}, structural edit (rename/add/remove averaged) {per_structural:?}"
    );
    assert_synced(&doc);
}
