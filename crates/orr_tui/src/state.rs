//! What the viewer knows about the stream: the newest frame and when it arrived, counters for
//! the status line, the last events. Time is passed in, so this is testable.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use orr_viewstream::{
    EventBatch, ViewFrame, FLAG_DISCONTINUITY, FLAG_PAUSED, FLAG_ROLLED_BACK, STATE_CANCELED, STATE_PREDICTED, STATE_VERIFIED,
};

use crate::render::Camera;
use crate::schema::ViewSchema;
use crate::source::Incoming;

/// How many recent events the viewer lists.
pub const RECENT_EVENTS: usize = 5;
/// How long a rollback or discontinuity stays in the status line, so it can be seen.
const FLAG_HOLD: Duration = Duration::from_millis(1000);

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

    fn ingest_frame(&mut self, f: ViewFrame, len: usize, now: Instant) {
        if f.has(FLAG_ROLLED_BACK) {
            self.last_rollback = Some(now);
        }
        if f.has(FLAG_DISCONTINUITY) {
            self.last_discontinuity = Some(now);
            // A jump may also be a new scene: look at it again.
            self.camera = None;
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
}
