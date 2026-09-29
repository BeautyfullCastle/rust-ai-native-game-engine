use std::collections::{BTreeMap, VecDeque};

use orr_bridge::Snapshot;
use orr_ecs::Entity;

use crate::extract::{Extracted, Extractor, InterpMode, Style};
use crate::math::{wrap_angle, Transform2, Vec2};

/// Tuning of interpolation and correction.
#[derive(Clone, Copy, Debug)]
pub struct ViewConfig {
    /// Time constant (seconds) of the rollback error offset decay. Larger is
    /// smoother but lets the entity lag longer behind the sim truth. `0`
    /// turns smoothing off (corrections show as jumps).
    pub correction_tau: f32,
    /// A correction longer than this (world units) is a teleport: shown at
    /// once, not smoothed.
    pub snap_distance: f32,
    /// The offset is set to zero when shorter than this.
    pub settle_epsilon: f32,
    /// `Snapshot` mode: how many ticks behind the newest verified frame the
    /// playback aims to stay.
    pub snapshot_delay_ticks: f32,
    /// Each new tick drops this fraction of the render clock's lead (time
    /// shown beyond the previous tick's end). It pulls the render phase to
    /// "one tick behind the newest" and keeps a start-up offset from lasting.
    pub phase_correction: f32,
    /// `Snapshot` mode: playback speeds up or slows down by this factor per
    /// tick of distance from the target delay (0.2 = 20 % per tick).
    pub snapshot_catchup_gain: f32,
}

impl Default for ViewConfig {
    fn default() -> Self {
        Self {
            correction_tau: 0.12,
            snap_distance: 400.0,
            settle_epsilon: 0.01,
            snapshot_delay_ticks: 2.0,
            phase_correction: 0.2,
            snapshot_catchup_gain: 0.2,
        }
    }
}

/// A change in the set of `Prediction` and `None` entities, seen by the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewLifecycle {
    Spawned(Entity),
    Despawned(Entity),
}

/// One thing to draw this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderItem {
    pub entity: Entity,
    pub transform: Transform2,
    pub style: Style,
}

struct Tracked {
    prev: Transform2,
    cur: Transform2,
    offset: Vec2,
    rot_offset: f32,
    mode: InterpMode,
    style: Style,
}

struct TrackFrame {
    tick: u64,
    items: Vec<Extracted>,
}

/// The view world: entity state for drawing, built from bridge snapshots.
/// See the crate docs for the model.
pub struct ViewWorld<X: Extractor> {
    extractor: X,
    cfg: ViewConfig,
    tick_rate: f32,
    // Prediction / None entities, keyed by (index, version).
    tracked: BTreeMap<Entity, Tracked>,
    head: Option<u64>,
    /// Time since the head tick's predecessor, in ticks. Kept unclamped up
    /// to 2 so a late snapshot carries the extra time into the next tick.
    alpha_raw: f32,
    // Snapshot-mode entities: confirmed frames and the playback position.
    track: VecDeque<TrackFrame>,
    playback: f64,
    last_seq: Option<u64>,
    rollbacks_seen: Option<u64>,
    lifecycle: Vec<ViewLifecycle>,
    buf_cur: Vec<Extracted>,
    buf_prev: Vec<Extracted>,
}

impl<X: Extractor> ViewWorld<X> {
    pub fn new(extractor: X, cfg: ViewConfig) -> Self {
        Self {
            extractor,
            cfg,
            tick_rate: 60.0,
            tracked: BTreeMap::new(),
            head: None,
            alpha_raw: 0.0,
            track: VecDeque::new(),
            playback: 0.0,
            last_seq: None,
            rollbacks_seen: None,
            lifecycle: Vec::new(),
            buf_cur: Vec::new(),
            buf_prev: Vec::new(),
        }
    }

    pub fn config(&self) -> &ViewConfig {
        &self.cfg
    }

    /// Render alpha in `[0, 1]`: how far the shown time is from the previous
    /// predicted tick to the newest one.
    pub fn alpha(&self) -> f32 {
        self.alpha_raw.clamp(0.0, 1.0)
    }

    /// Call once per render frame: takes in the newest snapshot (if it is
    /// new) and advances the view clocks by `dt` seconds.
    pub fn update(&mut self, dt: f32, snapshot: Option<&Snapshot>) {
        if let Some(snap) = snapshot {
            self.tick_rate = snap.tick_rate() as f32;
            if self.last_seq != Some(snap.seq()) {
                self.last_seq = Some(snap.seq());
                self.ingest(snap);
            }
        }
        self.advance(dt);
    }

    fn ingest(&mut self, snap: &Snapshot) {
        let mut cur = std::mem::take(&mut self.buf_cur);
        let mut prev = std::mem::take(&mut self.buf_prev);
        cur.clear();
        prev.clear();
        self.extractor.extract(snap.predicted(), &mut cur);
        if let Some(p) = snap.predicted_prev() {
            self.extractor.extract(p, &mut prev);
        }
        let rolled_back = self.rollbacks_seen.is_some_and(|n| snap.stats().rollbacks > n);
        self.rollbacks_seen = Some(snap.stats().rollbacks);
        self.push_predicted(snap.tick(), &prev, &cur, rolled_back);
        self.buf_cur = cur;
        self.buf_prev = prev;

        if let Some(v) = snap.verified() {
            if self.track.back().is_none_or(|f| f.tick < v.tick()) {
                let mut items = Vec::new();
                self.extractor.extract(v, &mut items);
                self.push_verified(v.tick(), &items);
            }
        }
    }

    /// Takes in a new predicted state: `cur` is the frame of tick `head`,
    /// `prev` the frame of tick `head - 1` as the sim knows it now (empty at
    /// tick 0). `rolled_back` says the sim did a rollback since the last call.
    /// Normally called by [`update`](Self::update); public so tools and tests
    /// can feed synthetic states.
    pub fn push_predicted(&mut self, head: u64, prev: &[Extracted], cur: &[Extracted], rolled_back: bool) {
        let ticks_advanced = self.head.map_or(0, |h| head.saturating_sub(h));
        let a_old = self.alpha_raw.clamp(0.0, 1.0);
        // The clock after this tick: the ticks consumed are subtracted, and part
        // of any lead (time already shown beyond the old head) is dropped.
        let mut next_alpha = if self.head.is_none() { 0.0 } else { self.alpha_raw - ticks_advanced as f32 };
        if ticks_advanced > 0 && next_alpha > 0.0 {
            next_alpha *= 1.0 - self.cfg.phase_correction;
        }
        let next_alpha = next_alpha.clamp(0.0, 2.0);
        let a_new = next_alpha.clamp(0.0, 1.0);

        let prev_by_entity: BTreeMap<Entity, Transform2> = prev.iter().map(|e| (e.entity, e.transform)).collect();
        let mut next: BTreeMap<Entity, Tracked> = BTreeMap::new();
        for item in cur.iter().filter(|e| e.mode != InterpMode::Snapshot) {
            let new_prev = prev_by_entity.get(&item.entity).copied().unwrap_or(item.transform);
            // Same key means same entity: the version is part of `Entity`, so a
            // reused index never matches an old entry.
            let tracked = match self.tracked.remove(&item.entity) {
                Some(old) => {
                    let mut t = Tracked {
                        prev: new_prev,
                        cur: item.transform,
                        offset: Vec2::ZERO,
                        rot_offset: 0.0,
                        mode: item.mode,
                        style: item.style,
                    };
                    if item.mode == InterpMode::Prediction && old.mode == InterpMode::Prediction {
                        // The error is how much the sim changed its mind about the
                        // same moment: the old and the new track are compared at one
                        // tick, so render-clock jitter never becomes error.
                        let (was, is) = match ticks_advanced {
                            // Same head (stall, or a rollback that only changed
                            // the newest ticks): compare at the shown moment.
                            0 => (old.prev.lerp(old.cur, a_old), new_prev.lerp(item.transform, a_old)),
                            // The old head is now the previous tick.
                            1 => (old.cur, new_prev),
                            // Ticks were skipped: the old head is not in the new pair.
                            // After a rollback, fall back to keeping the screen continuous;
                            // otherwise nothing changed in the past.
                            _ if rolled_back => (old.prev.lerp(old.cur, a_old), new_prev.lerp(item.transform, a_new)),
                            _ => (old.cur, old.cur),
                        };
                        let offset = old.offset + (was.pos - is.pos);
                        if offset.length() <= self.cfg.snap_distance {
                            t.offset = offset;
                            t.rot_offset = old.rot_offset + wrap_angle(was.rot - is.rot);
                        }
                    }
                    t
                }
                None => {
                    // New to the view. It interpolates from its position in the
                    // previous tick only if the sim had it there (same index *and*
                    // version); an entity spawned this tick has nothing to start from.
                    self.lifecycle.push(ViewLifecycle::Spawned(item.entity));
                    Tracked {
                        prev: new_prev,
                        cur: item.transform,
                        offset: Vec2::ZERO,
                        rot_offset: 0.0,
                        mode: item.mode,
                        style: item.style,
                    }
                }
            };
            next.insert(item.entity, tracked);
        }
        for gone in self.tracked.keys() {
            self.lifecycle.push(ViewLifecycle::Despawned(*gone));
        }
        self.tracked = next;
        self.head = Some(head);
        self.alpha_raw = next_alpha;
    }

    /// Takes in a confirmed frame for `Snapshot`-mode entities. Items of
    /// other modes are ignored. Frames must arrive in tick order; an older
    /// or equal tick is dropped. Normally called by [`update`](Self::update).
    pub fn push_verified(&mut self, tick: u64, items: &[Extracted]) {
        if self.track.back().is_some_and(|f| f.tick >= tick) {
            return;
        }
        let items: Vec<Extracted> = items.iter().filter(|e| e.mode == InterpMode::Snapshot).copied().collect();
        if self.track.is_empty() {
            self.playback = tick as f64 - f64::from(self.cfg.snapshot_delay_ticks);
        }
        self.track.push_back(TrackFrame { tick, items });
        self.clamp_playback();
    }

    fn clamp_playback(&mut self) {
        if let (Some(front), Some(back)) = (self.track.front(), self.track.back()) {
            self.playback = self.playback.clamp(front.tick as f64, back.tick as f64);
        }
    }

    /// Advances the view clocks and decays correction offsets. Called by
    /// [`update`](Self::update).
    pub fn advance(&mut self, dt: f32) {
        self.alpha_raw = (self.alpha_raw + dt * self.tick_rate).min(2.0);

        let keep = if self.cfg.correction_tau > 0.0 { (-dt / self.cfg.correction_tau).exp() } else { 0.0 };
        for t in self.tracked.values_mut() {
            t.offset = t.offset * keep;
            t.rot_offset *= keep;
            if t.offset.length() < self.cfg.settle_epsilon {
                t.offset = Vec2::ZERO;
            }
            if t.rot_offset.abs() < 1e-4 {
                t.rot_offset = 0.0;
            }
        }

        if let Some(newest) = self.track.back().map(|f| f.tick) {
            let lag = newest as f32 - self.playback as f32;
            let speed = (1.0 + self.cfg.snapshot_catchup_gain * (lag - self.cfg.snapshot_delay_ticks)).clamp(0.5, 2.0);
            self.playback += f64::from(dt * self.tick_rate * speed);
            self.clamp_playback();
            // Drop frames that are entirely behind the playback position.
            while self.track.len() > 2 && self.track[1].tick as f64 <= self.playback {
                self.track.pop_front();
            }
        }
    }

    /// Everything to draw now, in a stable order (`Prediction`/`None`
    /// entities by id, then `Snapshot` entities by id).
    pub fn render_items(&self, out: &mut Vec<RenderItem>) {
        let alpha = self.alpha();
        for (&entity, t) in &self.tracked {
            let base = match t.mode {
                InterpMode::None => t.cur,
                _ => t.prev.lerp(t.cur, alpha),
            };
            let transform = Transform2 { pos: base.pos + t.offset, rot: base.rot + t.rot_offset };
            out.push(RenderItem { entity, transform, style: t.style });
        }
        self.snapshot_items(out);
    }

    fn snapshot_items(&self, out: &mut Vec<RenderItem>) {
        let Some(newest) = self.track.back() else { return };
        // Frame pair around the playback position (`track` is short).
        let mut after = newest;
        let mut before = newest;
        for w in self.track.iter().collect::<Vec<_>>().windows(2) {
            if (w[1].tick as f64) >= self.playback {
                before = w[0];
                after = w[1];
                break;
            }
        }
        let span = (after.tick - before.tick).max(1) as f64;
        let f = (((self.playback - before.tick as f64) / span).clamp(0.0, 1.0)) as f32;
        let before_by: BTreeMap<Entity, &Extracted> = before.items.iter().map(|e| (e.entity, e)).collect();
        for item in &after.items {
            let transform = match before_by.get(&item.entity) {
                Some(b) if before.tick != after.tick => b.transform.lerp(item.transform, f),
                _ => item.transform,
            };
            out.push(RenderItem { entity: item.entity, transform, style: item.style });
        }
    }

    /// Takes the spawn/despawn notices since the last call.
    pub fn take_lifecycle(&mut self) -> Vec<ViewLifecycle> {
        std::mem::take(&mut self.lifecycle)
    }

    /// The predicted head tick the view is on.
    pub fn head_tick(&self) -> Option<u64> {
        self.head
    }

    /// The tick (fractional) `Snapshot` entities are shown at, if any frame arrived.
    pub fn playback_tick(&self) -> Option<f64> {
        (!self.track.is_empty()).then_some(self.playback)
    }

    /// Current length of the correction offset of `entity` (tests, debug overlay).
    pub fn correction_offset(&self, entity: Entity) -> Option<f32> {
        self.tracked.get(&entity).map(|t| t.offset.length())
    }
}
