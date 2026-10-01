//! What the viewer knows about the stream: the newest frame and when it arrived, counters for
//! the status line, the last events. Time is passed in, so this is testable.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use orr_viewstream::{
    EventBatch, ViewFrame, FLAG_DISCONTINUITY, FLAG_PAUSED, FLAG_ROLLED_BACK, STATE_CANCELED, STATE_PREDICTED, STATE_VERIFIED,
};

use crate::render::Camera;
use crate::schema::ViewSchema;
use crate::source::{Incoming, NetState, NetStatus};

/// How many recent events the viewer lists.
pub const RECENT_EVENTS: usize = 5;
/// How long a rollback or discontinuity stays in the status line, so it can be seen.
const FLAG_HOLD: Duration = Duration::from_millis(1000);
/// How long the visual correction of a rollback takes to fade out (the spec's error offset).
pub const SMOOTH: Duration = Duration::from_millis(100);
/// The window over which rollbacks per second are counted.
const RATE_WINDOW: Duration = Duration::from_secs(5);

/// One event as listed on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventLine {
    pub tick: u64,
    pub name: String,
    pub state: u8,
}

impl EventLine {
    pub fn state_name(&self) -> &'static str {
        match self.state {
            STATE_PREDICTED => "predicted",
            STATE_VERIFIED => "verified",
            STATE_CANCELED => "canceled",
            _ => "?",
        }
    }
}

pub struct ViewState {
    pub schema: ViewSchema,
    pub frame: Option<ViewFrame>,
    pub arrived: Instant,
    pub frame_bytes: usize,
    pub frames_total: u64,
    pub events_total: u64,
    pub recent_events: VecDeque<EventLine>,
    pub camera: Option<Camera>,
    /// Arrival times of the frames of the last second (for frames/s).
    arrivals: VecDeque<Instant>,
    last_rollback: Option<Instant>,
    last_discontinuity: Option<Instant>,
    /// The latest problem (a refused call, a bad message), shown until the next one.
    pub notice: Option<String>,
    /// The session of a network client (see [`ViewState::set_net`]); `None` for a local host.
    pub net: Option<NetStatus>,
    /// `(when, rollbacks so far)` samples of the last [`RATE_WINDOW`].
    rollback_samples: VecDeque<(Instant, u64)>,
    /// Events heard by state: predicted, verified, canceled.
    pub event_counts: [u64; 3],
    /// Predicted events that were later verified, and later canceled.
    pub verified_after_predicted: u64,
    pub canceled_after_predicted: u64,
    /// Keys of events announced as predicted and not settled yet.
    open_events: BTreeSet<(u64, u32, u32)>,
    /// Frames that had the rollback flag, and the deepest range.
    pub rolled_back_frames: u64,
    pub max_rollback_depth: u64,
    /// The rollback correction still to fade out: per entity id the offset (x, y) to add to the
    /// frame's positions, and when it started.
    offsets: BTreeMap<u64, [f32; 2]>,
    smooth_start: Option<Instant>,
}

impl ViewState {
    pub fn new(schema: ViewSchema, now: Instant) -> ViewState {
        ViewState {
            schema,
            frame: None,
            arrived: now,
            frame_bytes: 0,
            frames_total: 0,
            events_total: 0,
            recent_events: VecDeque::new(),
            camera: None,
            arrivals: VecDeque::new(),
            last_rollback: None,
            last_discontinuity: None,
            notice: None,
            net: None,
            rollback_samples: VecDeque::new(),
            event_counts: [0; 3],
            verified_after_predicted: 0,
            canceled_after_predicted: 0,
            open_events: BTreeSet::new(),
            rolled_back_frames: 0,
            max_rollback_depth: 0,
            offsets: BTreeMap::new(),
            smooth_start: None,
        }
    }

    /// Takes one message of the stream.
    pub fn ingest(&mut self, msg: &Incoming, now: Instant) {
        match msg {
            Incoming::Frame(bytes) => match ViewFrame::decode(bytes) {
                Ok(f) => self.ingest_frame(f, bytes.len(), now),
                Err(e) => self.notice = Some(format!("bad frame: {e}")),
            },
            Incoming::Events(bytes) => match EventBatch::decode(bytes) {
                Ok(b) => {
                    for e in b.events {
                        self.events_total += 1;
                        self.count_event(e.state, (e.tick, e.system, e.seq));
                        self.recent_events.push_back(EventLine { tick: e.tick, name: self.schema.event_name(e.event_type), state: e.state });
                        while self.recent_events.len() > RECENT_EVENTS {
                            self.recent_events.pop_front();
                        }
                    }
                }
                Err(e) => self.notice = Some(format!("bad event batch: {e}")),
            },
            Incoming::Error(e) => self.notice = Some(e.clone()),
        }
    }

    fn count_event(&mut self, state: u8, key: (u64, u32, u32)) {
        if let Some(n) = self.event_counts.get_mut(usize::from(state)) {
            *n += 1;
        }
        match state {
            STATE_PREDICTED => {
                self.open_events.insert(key);
            }
            STATE_VERIFIED if self.open_events.remove(&key) => self.verified_after_predicted += 1,
            STATE_CANCELED if self.open_events.remove(&key) => self.canceled_after_predicted += 1,
            _ => {}
        }
        // A viewer that never settles events must not grow without bound.
        while self.open_events.len() > 4096 {
            self.open_events.pop_first();
        }
    }

    /// Takes the session status of a network client (the viewer asks its source once in a while).
    pub fn set_net(&mut self, status: NetStatus, now: Instant) {
        if self.rollback_samples.back().map(|(_, n)| *n) != Some(status.rollbacks) {
            self.rollback_samples.push_back((now, status.rollbacks));
        }
        while self.rollback_samples.len() > 1 && self.rollback_samples.front().is_some_and(|(t, _)| now.saturating_duration_since(*t) > RATE_WINDOW) {
            self.rollback_samples.pop_front();
        }
        self.net = Some(status);
    }

    /// Rollbacks per second over the last few seconds (network client).
    pub fn rollbacks_per_s(&self, now: Instant) -> f32 {
        let (Some((t0, n0)), Some(net)) = (self.rollback_samples.front(), &self.net) else { return 0.0 };
        let span = now.saturating_duration_since(*t0).as_secs_f32().max(1.0);
        net.rollbacks.saturating_sub(*n0) as f32 / span
    }

    fn ingest_frame(&mut self, f: ViewFrame, len: usize, now: Instant) {
        if f.has(FLAG_ROLLED_BACK) {
            self.last_rollback = Some(now);
            self.rolled_back_frames += 1;
            if let Some((from, to)) = f.rollback {
                self.max_rollback_depth = self.max_rollback_depth.max(to + 1 - from.min(to + 1));
            }
        }
        if f.has(FLAG_DISCONTINUITY) {
            self.last_discontinuity = Some(now);
            // A jump may also be a new scene: look at it again.
            self.camera = None;
            self.offsets.clear();
            self.smooth_start = None;
        } else if f.has(FLAG_ROLLED_BACK) {
            self.start_smoothing(&f, now);
        }
        if self.camera.is_none() {
            self.camera = Some(Camera::fit(&f));
        }
        self.frame_bytes = len;
        self.frames_total += 1;
        self.arrivals.push_back(now);
        self.frame = Some(f);
        self.arrived = now;
    }

    /// A rollback moved entities: keep where they were drawn and fade to where they are now over
    /// [`SMOOTH`] instead of snapping (the error offset of the design). Positions only.
    fn start_smoothing(&mut self, new: &ViewFrame, now: Instant) {
        let Some(old) = &self.frame else { return };
        let alpha = self.alpha(now);
        let fade = self.fade(now);
        let shown_before: BTreeMap<u64, [f32; 2]> = old
            .entities
            .iter()
            .map(|e| {
                let off = self.offsets.get(&e.id).copied().unwrap_or([0.0; 2]);
                let at = |i: usize| e.prev[i] + (e.cur[i] - e.prev[i]) * alpha + off[i] * fade;
                (e.id, [at(0), at(1)])
            })
            .collect();
        let mut offsets = BTreeMap::new();
        for e in &new.entities {
            if let Some(before) = shown_before.get(&e.id) {
                // Where the new frame draws it right now: alpha 0, its `prev`.
                let off = [before[0] - e.prev[0], before[1] - e.prev[1]];
                if off[0].abs() > 1e-4 || off[1].abs() > 1e-4 {
                    offsets.insert(e.id, off);
                }
            }
        }
        self.smooth_start = (!offsets.is_empty()).then_some(now);
        self.offsets = offsets;
    }

    /// 1 right after a rollback, down to 0 after [`SMOOTH`].
    fn fade(&self, now: Instant) -> f32 {
        match self.smooth_start {
            Some(t) => (1.0 - now.saturating_duration_since(t).as_secs_f32() / SMOOTH.as_secs_f32()).clamp(0.0, 1.0),
            None => 0.0,
        }
    }

    /// The frame to draw: the newest one, with the rollback correction still fading out added to
    /// the positions of the entities it moved.
    pub fn display_frame(&self, now: Instant) -> Option<Cow<'_, ViewFrame>> {
        let f = self.frame.as_ref()?;
        let fade = self.fade(now);
        if fade <= 0.0 || self.offsets.is_empty() {
            return Some(Cow::Borrowed(f));
        }
        let mut shown = f.clone();
        for e in &mut shown.entities {
            if let Some(off) = self.offsets.get(&e.id) {
                for (i, o) in off.iter().enumerate() {
                    e.prev[i] += o * fade;
                    e.cur[i] += o * fade;
                }
            }
        }
        Some(Cow::Owned(shown))
    }

    /// The viewer's own interpolation factor: `(now - arrival) * tick_rate`, clamped to `0..=1`;
    /// 1 after a discontinuity and while paused.
    pub fn alpha(&self, now: Instant) -> f32 {
        match &self.frame {
            Some(f) if !f.has(FLAG_DISCONTINUITY) && !f.has(FLAG_PAUSED) => {
                (now.saturating_duration_since(self.arrived).as_secs_f32() * self.schema.tick_rate as f32).clamp(0.0, 1.0)
            }
            _ => 1.0,
        }
    }

    /// Frames per second over the last second.
    pub fn fps(&mut self, now: Instant) -> f32 {
        while self.arrivals.front().is_some_and(|t| now.saturating_duration_since(*t) > Duration::from_secs(1)) {
            self.arrivals.pop_front();
        }
        self.arrivals.len() as f32
    }

    /// The one status line.
    pub fn status_line(&mut self, now: Instant) -> String {
        let fps = self.fps(now);
        if let Some(net) = self.net {
            return self.net_line(&net, fps, now);
        }
        let Some(f) = &self.frame else {
            return format!("{}: waiting for the first frame (a paused session sends one per tick: s steps, space plays)", self.schema.game);
        };
        let hold = |t: Option<Instant>| t.is_some_and(|t| now.saturating_duration_since(t) < FLAG_HOLD);
        let mut flags = Vec::new();
        if hold(self.last_rollback) {
            flags.push("ROLLED_BACK");
        }
        if hold(self.last_discontinuity) {
            flags.push("DISCONTINUITY");
        }
        if f.has(FLAG_PAUSED) {
            flags.push("PAUSED");
        }
        format!(
            "tick {} verified {} | {} entities | {} B/frame | {:.0} frames/s | flags [{}] | events {}",
            f.tick,
            f.verified_tick,
            f.entities.len(),
            self.frame_bytes,
            fps,
            flags.join(" "),
            self.events_total
        )
    }

    /// The status line of a network client: who and how the connection is, then the prediction.
    fn net_line(&self, net: &NetStatus, fps: f32, now: Instant) -> String {
        let hold = |t: Option<Instant>| t.is_some_and(|t| now.saturating_duration_since(t) < FLAG_HOLD);
        let rolled = if hold(self.last_rollback) { " ROLLED_BACK" } else { "" };
        let desync = if net.desyncs > 0 { " DESYNC" } else { "" };
        let [p, v, c] = self.event_counts;
        match net.state {
            NetState::Connecting | NetState::Failed => format!("{}: {} the server", self.schema.game, net.state.name()),
            _ => format!(
                "{} slot {}/{} rtt {} ms delay {} | tick {} verified {} | rollbacks {} ({:.1}/s) last depth {} | events P{p} V{v} X{c} (P->V {}, P->X {}) | {:.0} fps{rolled}{desync}",
                net.state.name(),
                net.slot,
                net.players,
                net.rtt_ms,
                net.input_delay,
                self.frame.as_ref().map_or(net.head_tick, |f| f.tick),
                self.frame.as_ref().map_or(net.verified_tick, |f| f.verified_tick),
                net.rollbacks,
                self.rollbacks_per_s(now),
                net.last_depth(),
                self.verified_after_predicted,
                self.canceled_after_predicted,
                fps
            ),
        }
    }
}
