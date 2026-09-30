mod common;

use common::*;
use orr_edit::{EditError, Op, Origin, PlayController, Target};
use orr_reflect::Value;
use orr_sample::physics_game::{PhysConfig, PhysGame, PhysInput, SceneMode};
use orr_session::{ControlOp, PlaySession};
use orr_sim::PlayerSlot;

const TICKS: u32 = 90;

fn start(doc: &orr_edit::EditorDoc) -> PlayController<PhysGame> {
    PlayController::<PhysGame>::start_play(doc, doc.play_config(2, 60)).expect("start play")
}

fn run(pc: &mut PlayController<PhysGame>, n: u32) {
    pc.control(ControlOp::Step(n));
}

fn cfg() -> PhysConfig {
    PhysConfig::new(40, SceneMode::Rain)
}

/// Opens a recording as a viewer, seeks to its end and returns the frame checksum there.
fn replay_end_checksum(bytes: &[u8]) -> (u64, u64) {
    let mut viewer = PlaySession::<PhysGame>::open_replay(bytes, cfg(), 0).expect("open replay");
    let last = viewer.last_tick();
    viewer.control(ControlOp::Seek(last));
    assert_eq!(viewer.head_tick(), last);
    (last, viewer.frame().checksum())
}

#[test]
fn play_starts_from_the_edit_frame_and_is_deterministic() {
    let doc = demo_doc();
    let mut a = start(&doc);
    let mut b = start(&doc);
    assert_eq!(a.session().frame().checksum(), doc.checksum(), "play starts from the preview frame");
    a.session_mut().set_input(PlayerSlot(0), PhysInput::new(1, 0, 0, false));
    b.session_mut().set_input(PlayerSlot(0), PhysInput::new(1, 0, 0, false));
    run(&mut a, TICKS);
    run(&mut b, TICKS);
    assert_eq!(a.session().head_tick(), u64::from(TICKS));
    assert_eq!(a.session().frame().checksum(), b.session().frame().checksum());
    assert_ne!(a.session().frame().checksum(), doc.checksum(), "the world moved");
}

#[test]
fn play_edit_is_recorded_and_replays_to_the_same_checksum() {
    let doc = demo_doc();
    let mut pc = start(&doc);
    run(&mut pc, 20);
    let body = target_named(&doc, "body_05");
    let before_edit = pc.session().frame().checksum();
    assert!(pc.set_field(&body, "orr_physics::Body", "vel", vec2(7, 3)).unwrap());
    assert_ne!(pc.session().frame().checksum(), before_edit, "the edit shows at once, while paused");
    let vel = pc.view().field(&body, "orr_physics::Body", "vel").unwrap();
    assert_eq!(vel, vec2(7, 3));
    assert!(pc.timeline().pending_edits >= 1);
    // A second, single-axis edit gives a minimal span.
    assert!(pc.set_field(&body, "orr_physics::Body", "pos.x", fixed(2)).unwrap());
    // The same value again records nothing.
    assert!(!pc.set_field(&body, "orr_physics::Body", "pos.x", fixed(2)).unwrap());
    assert!(pc.set_singleton_field("Scene", "max_entities", Value::Int(12345)).unwrap());
    run(&mut pc, 40);
    let head = pc.session().head_tick();
    let live = pc.session().frame().checksum();
    let stopped = pc.stop_play();
    assert_eq!((stopped.tick, stopped.checksum), (head, live));
    let (last, replayed) = replay_end_checksum(&stopped.replay);
    assert_eq!(last, head);
    assert_eq!(replayed, live, "the replay applies the debug edits and lands on the same frame");
}

#[test]
fn play_edit_changes_the_outcome() {
    let doc = demo_doc();
    let mut plain = start(&doc);
    run(&mut plain, 50);
    let mut edited = start(&doc);
    edited.set_field(&target_named(&doc, "body_05"), "orr_physics::Body", "vel", vec2(30, 30)).unwrap();
    run(&mut edited, 50);
    assert_ne!(plain.session().frame().checksum(), edited.session().frame().checksum());
}

#[test]
fn spawn_add_remove_despawn_in_play_replay_identically() {
    let doc = demo_doc();
    let mut pc = start(&doc);
    run(&mut pc, 10);
    let n = pc.view().entities().len();

    // Spawn a dynamic ball made from the defaults of Body and Collider.
    let ball = pc
        .spawn(&[
            ("orr_physics::Body".into(), Value::Struct(vec![("pos".into(), vec2(0, 30))])),
            ("orr_physics::Collider".into(), Value::Struct(vec![])),
        ])
        .unwrap();
    let info = pc.view().entity(&Target::Entity(ball)).unwrap();
    assert!(info.guid.is_none(), "an entity made in play has no GUID");
    assert_eq!(pc.view().entities().len(), n + 1);
    assert!(pc.set_field(&Target::Entity(ball), "orr_physics::Body", "vel", vec2(1, 1)).unwrap());

    let paddle = target_named(&doc, "paddle_1");
    pc.remove_component(&paddle, "PaddleTag").unwrap();
    assert!(pc.remove_component(&paddle, "PaddleTag").is_err());
    pc.add_component(&paddle, "PaddleTag", Some(Value::Struct(vec![("slot".into(), Value::Int(1))]))).unwrap();
    assert!(matches!(pc.add_component(&paddle, "PaddleTag", None), Err(EditError::HasComponent { .. })));
    pc.despawn(&target_named(&doc, "body_07")).unwrap();
    assert!(pc.despawn(&target_named(&doc, "body_07")).is_err(), "already gone");
    assert_eq!(pc.view().entities().len(), n);
    run(&mut pc, 30);

    let live = pc.session().frame().checksum();
    let head = pc.session().head_tick();
    let stopped = pc.stop_play();
    let (last, replayed) = replay_end_checksum(&stopped.replay);
    assert_eq!((last, replayed), (head, live));
}

#[test]
fn refused_play_edits_change_nothing() {
    let doc = demo_doc();
    let mut pc = start(&doc);
    run(&mut pc, 5);
    let body = target_named(&doc, "body_01");
    let sum = pc.session().frame().checksum();
    let pending = pc.timeline().pending_edits;
    assert!(pc.set_field(&body, "orr_physics::Body", "pos.x", Value::Bool(true)).is_err());
    assert!(pc.set_field(&body, "orr_physics::Body", "nope", fixed(1)).is_err());
    assert!(pc.set_field(&body, "orr_physics::Collider", "friction", fixed(-1)).is_err());
    assert!(pc.set_field(&body, "PaddleTag", "slot", Value::Int(1)).is_err());
    assert!(pc.set_field(&Target::Guid(orr_reflect::Guid::parse("e_0badf00d").unwrap()), "PaddleTag", "slot", Value::Int(1)).is_err());
    assert!(pc.set_singleton_field("orr_physics::Body", "pos", vec2(0, 0)).is_err());
    assert_eq!(pc.session().frame().checksum(), sum);
    assert_eq!(pc.timeline().pending_edits, pending);
}

#[test]
fn stop_play_leaves_the_document_untouched() {
    let mut doc = demo_doc();
    let body = guid_named(&doc, "body_03");
    doc.apply(Op::Rename { guid: body, name: Some("keep".into()) }, Origin::User).unwrap();
    let before = state(&doc);
    let hist = doc.history();
    let dirty = doc.is_dirty();
    let mut pc = start(&doc);
    run(&mut pc, 30);
    pc.set_field(&target_named(&doc, "body_01"), "orr_physics::Body", "pos.y", fixed(35)).unwrap();
    pc.despawn(&target_named(&doc, "body_02")).unwrap();
    run(&mut pc, 30);
    let _ = pc.stop_play();
    assert_eq!(state(&doc), before);
    assert_eq!(doc.history(), hist);
    assert_eq!(doc.is_dirty(), dirty);
    assert_synced(&doc);
    // And play can start again from the same state.
    assert_eq!(start(&doc).session().frame().checksum(), doc.checksum());
}

#[test]
fn seek_back_after_a_play_edit_gives_identical_frames() {
    let doc = demo_doc();
    let mut pc = start(&doc);
    let body = target_named(&doc, "body_04");
    let mut frames: Vec<Vec<u8>> = vec![pc.session().frame().to_bytes()];
    let step = |pc: &mut PlayController<PhysGame>, frames: &mut Vec<Vec<u8>>, n: u32| {
        for _ in 0..n {
            run(pc, 1);
            frames.push(pc.session().frame().to_bytes());
        }
    };
    step(&mut pc, &mut frames, 15);
    pc.set_field(&body, "orr_physics::Body", "vel", vec2(-9, 20)).unwrap();
    // The edit sits at the boundary before tick 16: the frame of tick 15 is still the
    // unedited one, tick 16 and later carry the edit.
    step(&mut pc, &mut frames, 40);
    let end = pc.session().head_tick();
    assert_eq!(end, 55);
    for &t in &[3u64, 15, 16, 40, 55, 1, 30, 55, 0] {
        pc.control(ControlOp::Seek(t));
        assert_eq!(pc.session().head_tick(), t, "seek {t}");
        assert_eq!(pc.session().frame().to_bytes(), frames[t as usize], "frame at tick {t} after seeking");
    }
    // Stepping on from a rewound tick would branch and drop the recorded edit, so
    // re-run the whole recording as a viewer: every frame is bit-identical.
    let stopped = pc.stop_play();
    let mut viewer = PlaySession::<PhysGame>::open_replay(&stopped.replay, cfg(), 0).unwrap();
    assert_eq!(viewer.frame().to_bytes(), frames[0]);
    for (t, expected) in frames.iter().enumerate().skip(1) {
        viewer.step_now().expect("recorded tick");
        assert_eq!(&viewer.frame().to_bytes(), expected, "viewer tick {t}");
    }
}

#[test]
fn branching_after_a_seek_keeps_the_timeline_consistent() {
    let doc = demo_doc();
    let mut pc = start(&doc);
    run(&mut pc, 30);
    pc.control(ControlOp::Seek(10));
    // An edit in the past branches the recording.
    pc.set_field(&target_named(&doc, "body_01"), "orr_physics::Body", "vel", vec2(5, 5)).unwrap();
    assert_eq!(pc.session().head_tick(), 10);
    assert_eq!(pc.session().last_tick(), 10);
    run(&mut pc, 20);
    let live = pc.session().frame().checksum();
    let stopped = pc.stop_play();
    assert_eq!(replay_end_checksum(&stopped.replay).1, live);
}

#[test]
fn capture_scene_reads_the_live_state_back() {
    let doc = demo_doc();
    let mut pc = start(&doc);
    run(&mut pc, 20);
    let ball = pc.spawn(&[("PaddleTag".into(), Value::Struct(vec![("slot".into(), Value::Int(9))]))]).unwrap();
    let scene = pc.capture_scene().unwrap();
    assert_eq!(scene.entities.len(), 50);
    // The named entities keep their GUIDs; the new one got a made one.
    assert!(scene.entities.values().any(|e| e.name.as_deref() == Some("body_01")));
    assert!(pc.view().guid_of(ball).is_none());
    let text = scene.to_yaml();
    let reloaded = orr_edit::EditorDoc::from_yaml(&text, types(), orr_sim::Simulation::<PhysGame>::build_registry(), SEED).unwrap();
    assert_eq!(reloaded.view().entities().len(), 50);
}
