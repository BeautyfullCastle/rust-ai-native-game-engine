//! Rollback smoothing and the client counters of the view state: pure functions of the messages
//! and the clock that is passed in.

use std::time::{Duration, Instant};

use orr_viewstream::{
    EntityRecord, EntityRecord3, EventBatch, EventRecord, Pose3, ViewFrame, ViewFrame3, FLAG_DISCONTINUITY, FLAG_EVENTS_RESET, FLAG_PAUSED,
    FLAG_ROLLED_BACK, MODE_PREDICTION, SHAPE_CIRCLE, SHAPE3_SPHERE, STATE_CANCELED, STATE_PREDICTED, STATE_VERIFIED,
};

use crate::schema::{KindInfo, ViewSchema};
use crate::source::{Incoming, NetState, NetStatus};
use crate::state::{ViewState, SMOOTH};

fn schema() -> ViewSchema {
    ViewSchema {
        game: "T".into(),
        dimensions: 2,
        tick_rate: 60,
        player_count: 2,
        kinds: vec![KindInfo { id: 0, name: "ball".into(), props: Vec::new() }],
        input_size: 0,
        input_fields: Vec::new(),
        events: Vec::new(),
    }
}

fn schema3() -> ViewSchema {
    let mut schema = schema();
    schema.dimensions = 3;
    schema.game = "Yard3D".into();
    schema
}

fn ball(id: u64, prev: [f32; 3], cur: [f32; 3]) -> EntityRecord {
    EntityRecord { id, kind: 0, shape: SHAPE_CIRCLE, mode: MODE_PREDICTION, size: 1.0, half_y: 0.0, rgba: [255; 4], prev, cur }
}

fn frame(tick: u64, flags: u8, rollback: Option<(u64, u64)>, balls: Vec<EntityRecord>) -> Incoming {
    Incoming::Frame(ViewFrame { flags, tick, verified_tick: tick - 1, seq: tick, rollback, entities: balls, props: Vec::new() }.encode())
}

fn ball3(id: u64, prev: [f32; 3], cur: [f32; 3]) -> EntityRecord3 {
    EntityRecord3 {
        id,
        kind: 0,
        shape: SHAPE3_SPHERE,
        mode: MODE_PREDICTION,
        size: [1.0, 0.0, 0.0],
        rgba: [255; 4],
        roughness: 128,
        metallic: 0,
        style_flags: 0,
        prev: Pose3 { pos: prev, rot: Pose3::IDENTITY.rot },
        cur: Pose3 { pos: cur, rot: Pose3::IDENTITY.rot },
    }
}

fn frame3(tick: u64, flags: u8, rollback: Option<(u64, u64)>, balls: Vec<EntityRecord3>) -> Incoming {
    Incoming::Frame3(
        ViewFrame3 { flags, tick, verified_tick: tick.saturating_sub(1), seq: tick, rollback, entities: balls, props: Vec::new() }.encode(),
    )
}

/// The x the viewer draws the first ball at, at alpha 0 (`prev`) or alpha 1 (`cur`).
fn x_of(state: &ViewState, now: Instant, alpha_one: bool) -> f32 {
    let f = state.display_frame(now).unwrap();
    let e = &f.entities[0];
    if alpha_one {
        e.cur[0]
    } else {
        e.prev[0]
    }
}

#[test]
fn a_rollback_correction_fades_out_instead_of_snapping() {
    let t0 = Instant::now();
    let mut s = ViewState::new(schema(), t0);
    // The ball is drawn at x = 10 (its `cur`, frame fully played).
    s.ingest(&frame(5, 0, None, vec![ball(1, [9.0, 0.0, 0.0], [10.0, 0.0, 0.0])]), t0);
    assert_eq!(x_of(&s, t0, true), 10.0);
    // A rollback corrects the past: the next frame has the ball at x = 4 (a 6 unit jump).
    let t1 = t0 + Duration::from_millis(16);
    s.ingest(&frame(6, FLAG_ROLLED_BACK, Some((3, 5)), vec![ball(1, [4.0, 0.0, 0.0], [5.0, 0.0, 0.0])]), t1);
    assert_eq!(s.rolled_back_frames, 1);
    assert_eq!(s.max_rollback_depth, 3);
    // Right after: drawn (almost) where it was, not at the corrected position.
    let first = x_of(&s, t1, false);
    assert!(first > 9.0, "no snap: {first}");
    // Halfway through the fade: in between. After it: exactly the new position.
    let mid = x_of(&s, t1 + SMOOTH / 2, false);
    assert!(mid > 4.5 && mid < first, "{mid} {first}");
    assert_eq!(x_of(&s, t1 + SMOOTH, false), 4.0);
    assert_eq!(x_of(&s, t1 + SMOOTH * 3, true), 5.0);
}

#[test]
fn a_frame_without_rollback_or_after_a_jump_is_drawn_as_is() {
    let t0 = Instant::now();
    let mut s = ViewState::new(schema(), t0);
    s.ingest(&frame(5, 0, None, vec![ball(1, [9.0, 0.0, 0.0], [10.0, 0.0, 0.0])]), t0);
    let t1 = t0 + Duration::from_millis(16);
    s.ingest(&frame(6, 0, None, vec![ball(1, [10.0, 0.0, 0.0], [11.0, 0.0, 0.0])]), t1);
    assert_eq!(x_of(&s, t1, false), 10.0);
    // A discontinuity (seek, new session) never smooths.
    let t2 = t0 + Duration::from_millis(32);
    s.ingest(&frame(2, FLAG_DISCONTINUITY | FLAG_ROLLED_BACK, None, vec![ball(1, [0.0; 3], [0.0; 3])]), t2);
    assert_eq!(x_of(&s, t2, false), 0.0);
}

fn event(tick: u64, seq: u32, state: u8) -> EventRecord {
    EventRecord { tick, system: 0, seq, state, event_type: 0, payload: if state == STATE_CANCELED { Vec::new() } else { vec![0; 4] } }
}

#[test]
fn events_are_counted_by_state_and_by_transition() {
    let t0 = Instant::now();
    let mut s = ViewState::new(schema(), t0);
    let batch = |events| Incoming::Events(EventBatch { events }.encode());
    s.ingest(&batch(vec![event(1, 0, STATE_PREDICTED), event(2, 0, STATE_PREDICTED), event(3, 0, STATE_PREDICTED)]), t0);
    s.ingest(&batch(vec![event(1, 0, STATE_VERIFIED), event(2, 0, STATE_CANCELED)]), t0);
    // Verified without an earlier predicted record: counted by state, not as a transition.
    s.ingest(&batch(vec![event(9, 0, STATE_VERIFIED)]), t0);
    assert_eq!(s.event_counts, [3, 2, 1]);
    assert_eq!((s.verified_after_predicted, s.canceled_after_predicted), (1, 1));
}

#[test]
fn event_reset_discards_unsettled_predictions_without_faking_a_transition() {
    let t0 = Instant::now();
    let mut s = ViewState::new(schema(), t0);
    let batch = |events| Incoming::Events(EventBatch { events }.encode());
    s.ingest(&batch(vec![event(1, 7, STATE_PREDICTED)]), t0);

    let t1 = t0 + Duration::from_millis(16);
    s.ingest(&frame(8, FLAG_DISCONTINUITY | FLAG_EVENTS_RESET, None, vec![]), t1);
    assert!(s.status_line(t1).contains("EVENTS_RESET"));

    // A settle record arriving after the cut is counted, but cannot settle the
    // predicted event whose announcement was discarded.
    s.ingest(&batch(vec![event(1, 7, STATE_VERIFIED)]), t1);
    assert_eq!(s.event_counts, [1, 1, 0]);
    assert_eq!(s.verified_after_predicted, 0);
}

#[test]
fn the_client_status_line_shows_slot_rtt_delay_rollbacks_and_depth() {
    let t0 = Instant::now();
    let mut s = ViewState::new(schema(), t0);
    s.ingest(&frame(40, FLAG_ROLLED_BACK, Some((36, 39)), vec![ball(1, [0.0; 3], [0.0; 3])]), t0);
    let net = |rollbacks| NetStatus {
        state: NetState::Playing,
        slot: 1,
        players: 2,
        rtt_ms: 83,
        input_delay: 4,
        head_tick: 40,
        verified_tick: 33,
        rollbacks,
        last_rollback: (36, 39),
        ..NetStatus::default()
    };
    s.set_net(net(2), t0);
    s.set_net(net(12), t0 + Duration::from_millis(500));
    let line = s.status_line(t0 + Duration::from_millis(500));
    println!("{line}");
    // The ticks come from the frame (tick 40, verified 39).
    for part in ["playing", "slot 1/2", "rtt 83 ms", "delay 4", "tick 40 verified 39", "rollbacks 12 (10.0/s)", "last depth 4", "ROLLED_BACK"] {
        assert!(line.contains(part), "{part:?} missing in {line}");
    }
}

#[test]
fn three_dimensional_state_interpolates_tracks_flags_and_rejects_the_wrong_flavor() {
    let t0 = Instant::now();
    let mut s = ViewState::new(schema3(), t0);
    s.ingest(&frame3(5, 0, None, vec![ball3(1, [0.0; 3], [2.0, 4.0, 6.0])]), t0);
    assert!(s.frame.is_none());
    assert!(s.camera.is_none());
    assert!(s.camera3.is_some());
    assert_eq!(s.display_frame3(t0).unwrap().entities[0].prev.pos, [0.0; 3]);
    assert_eq!(s.display_frame3(t0 + Duration::from_millis(8)).unwrap().entities[0].cur.pos, [2.0, 4.0, 6.0]);
    assert!((s.alpha(t0 + Duration::from_millis(8)) - 0.48).abs() < 0.02);

    s.ingest(&frame3(6, FLAG_PAUSED, None, vec![ball3(1, [2.0, 4.0, 6.0], [3.0, 4.0, 6.0])]), t0 + Duration::from_millis(16));
    assert_eq!(s.alpha(t0 + Duration::from_millis(17)), 1.0);
    let previous_tick = s.frame3.as_ref().unwrap().tick;

    s.ingest(&frame(7, 0, None, vec![ball(1, [0.0; 3], [1.0; 3])]), t0 + Duration::from_millis(32));
    assert_eq!(s.frame3.as_ref().unwrap().tick, previous_tick, "a 2D frame cannot replace 3D state");
    assert!(s.notice.as_deref().unwrap().contains("does not match 3D schema"));
    assert!(s.status_line(t0 + Duration::from_millis(32)).contains("3D XZ"));
    assert!(s.status_line(t0 + Duration::from_millis(32)).contains("PAUSED"));
}

#[test]
fn three_dimensional_rollback_offsets_fade_and_events_reset_clears_predictions() {
    let t0 = Instant::now();
    let mut s = ViewState::new(schema3(), t0);
    s.ingest(&frame3(5, 0, None, vec![ball3(1, [9.0, 1.0, 0.0], [10.0, 1.0, 0.0])]), t0);
    let t1 = t0 + Duration::from_millis(16);
    s.ingest(&frame3(6, FLAG_ROLLED_BACK, Some((3, 5)), vec![ball3(1, [4.0, 2.0, 0.0], [5.0, 2.0, 0.0])]), t1);
    let corrected = s.display_frame3(t1).unwrap();
    assert!(corrected.entities[0].prev.pos[0] > 9.0, "no positional snap: {:?}", corrected.entities[0].prev.pos);
    assert_eq!(s.display_frame3(t1 + SMOOTH).unwrap().entities[0].prev.pos, [4.0, 2.0, 0.0]);

    let batch = Incoming::Events(EventBatch { events: vec![event(7, 1, STATE_PREDICTED)] }.encode());
    s.ingest(&batch, t1);
    s.ingest(&frame3(7, FLAG_DISCONTINUITY | FLAG_EVENTS_RESET, None, vec![ball3(1, [2.0; 3], [2.0; 3])]), t1 + SMOOTH);
    s.ingest(&Incoming::Events(EventBatch { events: vec![event(7, 1, STATE_VERIFIED)] }.encode()), t1 + SMOOTH);
    assert_eq!(s.verified_after_predicted, 0);
    assert_eq!(s.display_frame3(t1 + SMOOTH).unwrap().entities[0].prev.pos, [2.0; 3]);
}

#[test]
fn malformed_three_dimensional_quaternion_does_not_replace_the_last_good_frame() {
    let t0 = Instant::now();
    let mut s = ViewState::new(schema3(), t0);
    s.ingest(&frame3(5, 0, None, vec![ball3(1, [0.0; 3], [1.0; 3])]), t0);
    let mut malformed = ball3(1, [1.0; 3], [2.0; 3]);
    malformed.cur.rot = [0.0, 0.0, 0.0, 2.0];
    s.ingest(&frame3(6, 0, None, vec![malformed]), t0 + Duration::from_millis(16));
    assert_eq!(s.frame3.as_ref().unwrap().tick, 5);
    assert!(s.notice.as_deref().unwrap().contains("unit quaternion"));
}
