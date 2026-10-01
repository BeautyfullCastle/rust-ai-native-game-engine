//! Rollback smoothing and the client counters of the view state: pure functions of the messages
//! and the clock that is passed in.

use std::time::{Duration, Instant};

use orr_viewstream::{
    EntityRecord, EventBatch, EventRecord, ViewFrame, FLAG_DISCONTINUITY, FLAG_ROLLED_BACK, MODE_PREDICTION, SHAPE_CIRCLE, STATE_CANCELED,
    STATE_PREDICTED, STATE_VERIFIED,
};

use crate::schema::{KindInfo, ViewSchema};
use crate::source::{Incoming, NetState, NetStatus};
use crate::state::{ViewState, SMOOTH};

fn schema() -> ViewSchema {
    ViewSchema {
        game: "T".into(),
        tick_rate: 60,
        player_count: 2,
        kinds: vec![KindInfo { id: 0, name: "ball".into(), props: Vec::new() }],
        input_size: 0,
        input_fields: Vec::new(),
        events: Vec::new(),
    }
}

fn ball(id: u64, prev: [f32; 3], cur: [f32; 3]) -> EntityRecord {
    EntityRecord { id, kind: 0, shape: SHAPE_CIRCLE, mode: MODE_PREDICTION, size: 1.0, half_y: 0.0, rgba: [255; 4], prev, cur }
}

fn frame(tick: u64, flags: u8, rollback: Option<(u64, u64)>, balls: Vec<EntityRecord>) -> Incoming {
    Incoming::Frame(ViewFrame { flags, tick, verified_tick: tick - 1, seq: tick, rollback, entities: balls, props: Vec::new() }.encode())
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
