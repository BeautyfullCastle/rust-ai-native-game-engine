//! The 3D view world: the same model as [`crate::ViewWorld`] (see the crate
//! docs) with poses instead of 2D transforms.
//!
//! - Interpolation between the last two predicted ticks is linear in
//!   position and **slerp** in orientation (shortest arc).
//! - The rollback error offset is a position vector plus a correction
//!   quaternion. The shown pose is `pos + offset` and
//!   `rot_offset * rot`. The position offset decays exponentially
//!   ([`ViewConfig::correction_tau`]); the rotation offset decays the same way
//!   along the arc (`slerp(identity, rot_offset, keep)`). A correction longer
//!   than [`ViewConfig::snap_distance`] is a teleport and is not smoothed.
//! - `Snapshot` entities are played from confirmed frames a little late.

use std::collections::{BTreeMap, VecDeque};

use orr_bridge::Snapshot;
use orr_ecs::Entity;

use crate::extract::InterpMode;
use crate::extract3::{Extracted3, Extractor3, Style3};
use crate::math3::{Quat, Transform3, Vec3};
use crate::world::{ViewConfig, ViewLifecycle};

/// One thing to draw this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderItem3 {
    pub entity: Entity,
    pub transform: Transform3,
    pub style: Style3,
}

struct Tracked {
    prev: Transform3,
    cur: Transform3,
    offset: Vec3,
    rot_offset: Quat,
    mode: InterpMode,
    style: Style3,
}

struct TrackFrame {
    tick: u64,
    items: Vec<Extracted3>,
}

/// The 3D view world: entity poses for drawing, built from bridge snapshots.
pub struct ViewWorld3<X: Extractor3> {
    extractor: X,
    cfg: ViewConfig,
    tick_rate: f32,
    tracked: BTreeMap<Entity, Tracked>,
    head: Option<u64>,
    alpha_raw: f32,
    track: VecDeque<TrackFrame>,
    playback: f64,
    last_seq: Option<u64>,
    rollbacks_seen: Option<u64>,
    lifecycle: Vec<ViewLifecycle>,
    buf_cur: Vec<Extracted3>,
    buf_prev: Vec<Extracted3>,
}

impl<X: Extractor3> ViewWorld3<X> {
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

    /// Forgets all tracked state (after a seek or an edit).
    pub fn reset(&mut self) {
        self.tracked.clear();
        self.head = None;
        self.alpha_raw = 0.0;
        self.track.clear();
        self.playback = 0.0;
        self.last_seq = None;
        self.rollbacks_seen = None;
    }

    pub fn config(&self) -> &ViewConfig {
        &self.cfg
    }

    /// Render alpha in `[0, 1]`.
    pub fn alpha(&self) -> f32 {
        self.alpha_raw.clamp(0.0, 1.0)
    }

    /// Call once per render frame: takes in the newest snapshot (if new) and
    /// advances the view clocks by `dt` seconds.
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

    /// Takes in a new predicted state (see [`crate::ViewWorld::push_predicted`]).
    pub fn push_predicted(&mut self, head: u64, prev: &[Extracted3], cur: &[Extracted3], rolled_back: bool) {
        let ticks_advanced = self.head.map_or(0, |h| head.saturating_sub(h));
        let a_old = self.alpha_raw.clamp(0.0, 1.0);
        let mut next_alpha = if self.head.is_none() { 0.0 } else { self.alpha_raw - ticks_advanced as f32 };
        if ticks_advanced > 0 && next_alpha > 0.0 {
            next_alpha *= 1.0 - self.cfg.phase_correction;
        }
        let next_alpha = next_alpha.clamp(0.0, 2.0);
        let a_new = next_alpha.clamp(0.0, 1.0);

        let prev_by_entity: BTreeMap<Entity, Transform3> = prev.iter().map(|e| (e.entity, e.transform)).collect();
        let mut next: BTreeMap<Entity, Tracked> = BTreeMap::new();
        for item in cur.iter().filter(|e| e.mode != InterpMode::Snapshot) {
            let new_prev = prev_by_entity.get(&item.entity).copied().unwrap_or(item.transform);
            let tracked = match self.tracked.remove(&item.entity) {
                Some(old) => {
                    let mut t = Tracked {
                        prev: new_prev,
                        cur: item.transform,
                        offset: Vec3::ZERO,
                        rot_offset: Quat::IDENTITY,
                        mode: item.mode,
                        style: item.style,
                    };
                    if item.mode == InterpMode::Prediction && old.mode == InterpMode::Prediction {
                        let (was, is) = match ticks_advanced {
                            0 => (old.prev.lerp(old.cur, a_old), new_prev.lerp(item.transform, a_old)),
                            1 => (old.cur, new_prev),
                            _ if rolled_back => (old.prev.lerp(old.cur, a_old), new_prev.lerp(item.transform, a_new)),
                            _ => (old.cur, old.cur),
                        };
                        let offset = old.offset + (was.pos - is.pos);
                        if offset.length() <= self.cfg.snap_distance {
                            t.offset = offset;
                            // Shown before: old_off * was. Shown after: new_off * is. Equal when
                            // new_off = old_off * was * is^-1.
                            t.rot_offset = (old.rot_offset * was.rot * is.rot.conjugate()).normalize();
                        }
                    }
                    t
                }
                None => {
                    self.lifecycle.push(ViewLifecycle::Spawned(item.entity));
                    Tracked {
                        prev: new_prev,
                        cur: item.transform,
                        offset: Vec3::ZERO,
                        rot_offset: Quat::IDENTITY,
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

    /// Takes in a confirmed frame for `Snapshot`-mode entities.
    pub fn push_verified(&mut self, tick: u64, items: &[Extracted3]) {
        if self.track.back().is_some_and(|f| f.tick >= tick) {
            return;
        }
        let items: Vec<Extracted3> = items.iter().filter(|e| e.mode == InterpMode::Snapshot).copied().collect();
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

    /// Advances the view clocks and decays correction offsets.
    pub fn advance(&mut self, dt: f32) {
        self.alpha_raw = (self.alpha_raw + dt * self.tick_rate).min(2.0);

        let keep = if self.cfg.correction_tau > 0.0 { (-dt / self.cfg.correction_tau).exp() } else { 0.0 };
        for t in self.tracked.values_mut() {
            t.offset = t.offset * keep;
            t.rot_offset = Quat::IDENTITY.slerp(t.rot_offset, keep);
            if t.offset.length() < self.cfg.settle_epsilon {
                t.offset = Vec3::ZERO;
            }
            if t.rot_offset.angle_to(Quat::IDENTITY) < 1e-4 {
                t.rot_offset = Quat::IDENTITY;
            }
        }

        if let Some(newest) = self.track.back().map(|f| f.tick) {
            let lag = newest as f32 - self.playback as f32;
            let speed = (1.0 + self.cfg.snapshot_catchup_gain * (lag - self.cfg.snapshot_delay_ticks)).clamp(0.5, 2.0);
            self.playback += f64::from(dt * self.tick_rate * speed);
            self.clamp_playback();
            while self.track.len() > 2 && self.track[1].tick as f64 <= self.playback {
                self.track.pop_front();
            }
        }
    }

    /// Everything to draw now, in a stable order (`Prediction`/`None`
    /// entities by id, then `Snapshot` entities by id).
    pub fn render_items(&self, out: &mut Vec<RenderItem3>) {
        let alpha = self.alpha();
        for (&entity, t) in &self.tracked {
            let base = match t.mode {
                InterpMode::None => t.cur,
                _ => t.prev.lerp(t.cur, alpha),
            };
            let transform = Transform3 { pos: base.pos + t.offset, rot: t.rot_offset * base.rot };
            out.push(RenderItem3 { entity, transform, style: t.style });
        }
        self.snapshot_items(out);
    }

    fn snapshot_items(&self, out: &mut Vec<RenderItem3>) {
        let Some(newest) = self.track.back() else { return };
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
        let before_by: BTreeMap<Entity, &Extracted3> = before.items.iter().map(|e| (e.entity, e)).collect();
        for item in &after.items {
            let transform = match before_by.get(&item.entity) {
                Some(b) if before.tick != after.tick => b.transform.lerp(item.transform, f),
                _ => item.transform,
            };
            out.push(RenderItem3 { entity: item.entity, transform, style: item.style });
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

    /// Current length of the position correction offset of `entity`.
    pub fn correction_offset(&self, entity: Entity) -> Option<f32> {
        self.tracked.get(&entity).map(|t| t.offset.length())
    }

    /// Current angle (radians) of the rotation correction of `entity`.
    pub fn correction_angle(&self, entity: Entity) -> Option<f32> {
        self.tracked.get(&entity).map(|t| t.rot_offset.angle_to(Quat::IDENTITY))
    }
}
