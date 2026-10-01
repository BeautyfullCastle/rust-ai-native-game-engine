//! Producing view frames: the game's [`Extractor`] runs on the host side and
//! its output becomes [`ViewFrame`]s.
//!
//! - [`FrameEncoder`] turns one frame (and the one before it) into a
//!   [`ViewFrame`]. It does not care where the frames come from.
//! - [`ViewStreamSource`] drives it from a [`Bridge`]'s snapshots and events:
//!   it sets the rollback and discontinuity flags and turns bridge events into
//!   [`EventRecord`]s.
//! - [`StreamProducer`] is the object-safe face of a source for hosts that
//!   hold frames directly (the ERP server's `viewstream` topic).

use orr_bridge::{Bridge, BridgeEvent, EventStatus, FrameView, Lifecycle, Snapshot};
use orr_ecs::{Entity, Frame};
use orr_sim::Game;
use orr_view::{Extracted, Extracted3, Extractor, Extractor3, InterpMode, Shape, Shape3};

use crate::format::{
    color_to_u8, EntityRecord, EntityRecord3, EventRecord, Pose3, ViewFrame, ViewFrame3, FLAG_DISCONTINUITY, FLAG_PAUSED,
    FLAG_ROLLED_BACK, MODE_NONE, MODE_PREDICTION, MODE_SNAPSHOT, SHAPE3_BOX, SHAPE3_CAPSULE, SHAPE3_PLANE, SHAPE3_SPHERE, SHAPE_CAPSULE,
    SHAPE_CIRCLE, SHAPE_QUAD, STATE_CANCELED, STATE_PREDICTED, STATE_VERIFIED, STYLE_CHECKER,
};
use crate::schema::Schema;

/// The game's vocabulary of entity kinds: which kind an extracted entity is
/// and its custom properties. The kinds and the number and types of the
/// property words must match the [`Schema`] the source was built with.
pub trait StreamKinds: Send {
    /// Returns the kind of `item` and appends its property words (f32 as
    /// `to_bits`, u32, i32 as u32) to `props`.
    fn classify(&self, frame: FrameView<'_>, item: &Extracted, props: &mut Vec<u32>) -> u16;
}

/// Every entity has kind 0 and no properties.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoKinds;

impl StreamKinds for NoKinds {
    fn classify(&self, _frame: FrameView<'_>, _item: &Extracted, _props: &mut Vec<u32>) -> u16 {
        0
    }
}

/// What a frame message says besides its entities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameMeta {
    /// Head tick.
    pub tick: u64,
    /// Newest verified tick.
    pub verified_tick: u64,
    /// `FLAG_*` bits. `FLAG_DISCONTINUITY` makes `prev` equal `cur`.
    pub flags: u8,
    /// Rollback range, if a rollback happened since the last frame.
    pub rollback: Option<(u64, u64)>,
}

/// The stable id of an entity on the wire.
pub fn entity_id(e: Entity) -> u64 {
    u64::from(e.index) | (u64::from(e.version) << 32)
}

/// Turns frames into [`ViewFrame`]s with the game's extractor.
pub struct FrameEncoder<X: Extractor, K: StreamKinds> {
    extractor: X,
    kinds: K,
    seq: u64,
    cur: Vec<Extracted>,
    prev: Vec<Extracted>,
    prev_index: Vec<(Entity, [f32; 3])>,
    words: Vec<u32>,
}

fn transform3(x: &Extracted) -> [f32; 3] {
    [x.transform.pos.x, x.transform.pos.y, x.transform.rot]
}

impl<X: Extractor, K: StreamKinds> FrameEncoder<X, K> {
    /// An encoder for `extractor` and `kinds`.
    pub fn new(extractor: X, kinds: K) -> Self {
        Self { extractor, kinds, seq: 0, cur: Vec::new(), prev: Vec::new(), prev_index: Vec::new(), words: Vec::new() }
    }

    /// Builds the frame message content for `cur` and its predecessor `prev`
    /// (`None`: no previous tick, or a discontinuity; `prev` then equals `cur`).
    pub fn encode(&mut self, cur: FrameView<'_>, prev: Option<FrameView<'_>>, meta: FrameMeta) -> ViewFrame {
        self.cur.clear();
        self.extractor.extract(cur, &mut self.cur);
        self.prev_index.clear();
        let use_prev = prev.is_some() && meta.flags & FLAG_DISCONTINUITY == 0;
        if let (true, Some(p)) = (use_prev, prev) {
            self.prev.clear();
            self.extractor.extract(p, &mut self.prev);
            self.prev_index.extend(self.prev.iter().map(|x| (x.entity, transform3(x))));
            self.prev_index.sort_unstable_by_key(|(e, _)| *e);
        }
        let mut entities = Vec::with_capacity(self.cur.len());
        let mut props: Vec<u8> = Vec::new();
        for item in &self.cur {
            self.words.clear();
            let kind = self.kinds.classify(cur, item, &mut self.words);
            for w in &self.words {
                props.extend_from_slice(&w.to_le_bytes());
            }
            let now = transform3(item);
            let before = self
                .prev_index
                .binary_search_by_key(&item.entity, |(e, _)| *e)
                .map_or(now, |i| self.prev_index[i].1);
            let (shape, mode) = (
                match item.style.shape {
                    Shape::Circle => SHAPE_CIRCLE,
                    Shape::Quad => SHAPE_QUAD,
                    Shape::Capsule => SHAPE_CAPSULE,
                },
                match item.mode {
                    InterpMode::Prediction => MODE_PREDICTION,
                    InterpMode::Snapshot => MODE_SNAPSHOT,
                    InterpMode::None => MODE_NONE,
                },
            );
            let c = item.style.color;
            entities.push(EntityRecord {
                id: entity_id(item.entity),
                kind,
                shape,
                mode,
                size: item.style.size,
                half_y: item.style.half_y,
                rgba: [color_to_u8(c[0]), color_to_u8(c[1]), color_to_u8(c[2]), color_to_u8(c[3])],
                prev: before,
                cur: now,
            });
        }
        self.seq += 1;
        let flags = meta.flags | if meta.rollback.is_some() { FLAG_ROLLED_BACK } else { 0 };
        ViewFrame { flags, tick: meta.tick, verified_tick: meta.verified_tick, seq: self.seq, rollback: meta.rollback, entities, props }
    }
}

/// The object-safe face of a view stream source, for a host that has frames
/// but no bridge (the ERP server).
pub trait StreamProducer: Send {
    /// The schema of this stream.
    fn schema(&self) -> &Schema;

    /// One frame message (bytes) for `cur` and its predecessor.
    fn encode_frame(&mut self, cur: &Frame, prev: Option<&Frame>, meta: FrameMeta) -> Vec<u8>;

    /// The game-defined event type of an event payload (default 0).
    fn event_type(&self, _payload: &[u8]) -> u16 {
        0
    }
}

/// What [`ViewStreamSource::pump`] found.
#[derive(Debug, Default)]
pub struct Pumped {
    /// The newest frame, if the bridge published one since the last pump.
    pub frame: Option<ViewFrame>,
    /// Sim events since the last pump, in sim order.
    pub events: Vec<EventRecord>,
}

type EventTypeFn = Box<dyn Fn(&[u8]) -> u16 + Send>;

/// A view stream producer on top of any [`Bridge`]: each published snapshot
/// becomes one [`ViewFrame`] (extractor run on the predicted frame and the one
/// before it); bridge events become [`EventRecord`]s.
pub struct ViewStreamSource<X: Extractor, K: StreamKinds = NoKinds> {
    enc: FrameEncoder<X, K>,
    schema: Schema,
    event_type: EventTypeFn,
    last_snapshot: Option<u64>,
    last_rollbacks: u64,
    discontinuity: bool,
}

impl<X: Extractor, K: StreamKinds> ViewStreamSource<X, K> {
    /// A source for `extractor` and `kinds` that describes itself with `schema`.
    pub fn new(extractor: X, kinds: K, schema: Schema) -> Self {
        Self {
            enc: FrameEncoder::new(extractor, kinds),
            schema,
            event_type: Box::new(|_| 0),
            last_snapshot: None,
            last_rollbacks: 0,
            discontinuity: false,
        }
    }

    /// Sets how event payloads map to the schema's event type ids.
    pub fn with_event_type(mut self, f: impl Fn(&[u8]) -> u16 + Send + 'static) -> Self {
        self.event_type = Box::new(f);
        self
    }

    /// The schema.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The frame of `snap`, or `None` if it is the snapshot the last call saw.
    /// Sets the rollback flag and range when the bridge's rollback count grew
    /// since then, and the discontinuity flag after a seek or branch noted by
    /// [`pump`](Self::pump).
    pub fn frame_from_snapshot(&mut self, snap: &Snapshot) -> Option<ViewFrame> {
        if self.last_snapshot == Some(snap.seq()) {
            return None;
        }
        self.last_snapshot = Some(snap.seq());
        let mut flags = 0;
        let rollbacks = snap.stats().rollbacks;
        let rollback = (rollbacks > self.last_rollbacks)
            .then(|| snap.last_rollback().map(|r| (r.from_tick, r.to_tick)))
            .flatten();
        self.last_rollbacks = rollbacks;
        if std::mem::take(&mut self.discontinuity) {
            flags |= FLAG_DISCONTINUITY;
        }
        if snap.timeline().is_some_and(|t| !t.playing) {
            flags |= FLAG_PAUSED;
        }
        let meta = FrameMeta { tick: snap.tick(), verified_tick: snap.verified_tick(), flags, rollback };
        Some(self.enc.encode(snap.predicted(), snap.predicted_prev(), meta))
    }

    /// Takes the bridge's newest snapshot and all its pending events.
    pub fn pump<G: Game, B: Bridge<G> + ?Sized>(&mut self, bridge: &mut B) -> Pumped {
        let mut events = Vec::new();
        for ev in bridge.drain_events() {
            match ev {
                BridgeEvent::Sim { key, status } => {
                    let (state, payload) = match &status {
                        EventStatus::Predicted(e) => (STATE_PREDICTED, bytemuck::bytes_of(e).to_vec()),
                        EventStatus::Verified(e) => (STATE_VERIFIED, bytemuck::bytes_of(e).to_vec()),
                        EventStatus::Canceled => (STATE_CANCELED, Vec::new()),
                    };
                    let event_type = (self.event_type)(&payload);
                    events.push(EventRecord { tick: key.tick, system: u32::from(key.system_index), seq: key.seq, state, event_type, payload });
                }
                BridgeEvent::Lifecycle(Lifecycle::Seeked { .. } | Lifecycle::Branched { .. }) => self.discontinuity = true,
                BridgeEvent::Lifecycle(_) => {}
            }
        }
        let frame = bridge.snapshot().and_then(|s| self.frame_from_snapshot(&s));
        Pumped { frame, events }
    }
}

impl<X: Extractor + Send, K: StreamKinds> StreamProducer for ViewStreamSource<X, K> {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn encode_frame(&mut self, cur: &Frame, prev: Option<&Frame>, meta: FrameMeta) -> Vec<u8> {
        self.enc.encode(FrameView::of(cur), prev.map(FrameView::of), meta).encode()
    }

    fn event_type(&self, payload: &[u8]) -> u16 {
        (self.event_type)(payload)
    }
}

// ---- 3D ----

/// The 3D game's vocabulary of entity kinds (see [`StreamKinds`]).
pub trait StreamKinds3: Send {
    /// Returns the kind of `item` and appends its property words to `props`.
    fn classify(&self, frame: FrameView<'_>, item: &Extracted3, props: &mut Vec<u32>) -> u16;
}

/// Every 3D entity has kind 0 and no properties.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoKinds3;

impl StreamKinds3 for NoKinds3 {
    fn classify(&self, _frame: FrameView<'_>, _item: &Extracted3, _props: &mut Vec<u32>) -> u16 {
        0
    }
}

fn pose3(x: &Extracted3) -> Pose3 {
    Pose3 { pos: x.transform.pos.to_array(), rot: x.transform.rot.to_array() }
}

/// Turns 3D frames into [`ViewFrame3`]s with the game's [`Extractor3`].
pub struct FrameEncoder3<X: Extractor3, K: StreamKinds3> {
    extractor: X,
    kinds: K,
    seq: u64,
    cur: Vec<Extracted3>,
    prev: Vec<Extracted3>,
    prev_index: Vec<(Entity, Pose3)>,
    words: Vec<u32>,
}

impl<X: Extractor3, K: StreamKinds3> FrameEncoder3<X, K> {
    /// An encoder for `extractor` and `kinds`.
    pub fn new(extractor: X, kinds: K) -> Self {
        Self { extractor, kinds, seq: 0, cur: Vec::new(), prev: Vec::new(), prev_index: Vec::new(), words: Vec::new() }
    }

    /// Like [`FrameEncoder::encode`], for a 3D game.
    pub fn encode(&mut self, cur: FrameView<'_>, prev: Option<FrameView<'_>>, meta: FrameMeta) -> ViewFrame3 {
        self.cur.clear();
        self.extractor.extract(cur, &mut self.cur);
        self.prev_index.clear();
        let use_prev = prev.is_some() && meta.flags & FLAG_DISCONTINUITY == 0;
        if let (true, Some(p)) = (use_prev, prev) {
            self.prev.clear();
            self.extractor.extract(p, &mut self.prev);
            self.prev_index.extend(self.prev.iter().map(|x| (x.entity, pose3(x))));
            self.prev_index.sort_unstable_by_key(|(e, _)| *e);
        }
        let mut entities = Vec::with_capacity(self.cur.len());
        let mut props: Vec<u8> = Vec::new();
        for item in &self.cur {
            self.words.clear();
            let kind = self.kinds.classify(cur, item, &mut self.words);
            for w in &self.words {
                props.extend_from_slice(&w.to_le_bytes());
            }
            let now = pose3(item);
            let before = self.prev_index.binary_search_by_key(&item.entity, |(e, _)| *e).map_or(now, |i| self.prev_index[i].1);
            let (shape, size) = match item.style.shape {
                Shape3::Sphere { radius } => (SHAPE3_SPHERE, [radius, 0.0, 0.0]),
                Shape3::Box { half } => (SHAPE3_BOX, half),
                Shape3::Capsule { half_length, radius } => (SHAPE3_CAPSULE, [radius, half_length, 0.0]),
                Shape3::Plane { half_x, half_z } => (SHAPE3_PLANE, [half_x, 0.0, half_z]),
            };
            let mode = match item.mode {
                InterpMode::Prediction => MODE_PREDICTION,
                InterpMode::Snapshot => MODE_SNAPSHOT,
                InterpMode::None => MODE_NONE,
            };
            let c = item.style.color;
            entities.push(EntityRecord3 {
                id: entity_id(item.entity),
                kind,
                shape,
                mode,
                size,
                rgba: [color_to_u8(c[0]), color_to_u8(c[1]), color_to_u8(c[2]), color_to_u8(c[3])],
                roughness: color_to_u8(item.style.roughness),
                metallic: color_to_u8(item.style.metallic),
                style_flags: if item.style.checker { STYLE_CHECKER } else { 0 },
                prev: before,
                cur: now,
            });
        }
        self.seq += 1;
        let flags = meta.flags | if meta.rollback.is_some() { FLAG_ROLLED_BACK } else { 0 };
        ViewFrame3 { flags, tick: meta.tick, verified_tick: meta.verified_tick, seq: self.seq, rollback: meta.rollback, entities, props }
    }
}

/// What [`ViewStreamSource3::pump`] found.
#[derive(Debug, Default)]
pub struct Pumped3 {
    /// The newest frame, if the bridge published one since the last pump.
    pub frame: Option<ViewFrame3>,
    /// Sim events since the last pump, in sim order.
    pub events: Vec<EventRecord>,
}

/// A 3D view stream producer on top of any [`Bridge`]; the 3D twin of [`ViewStreamSource`].
/// Its schema must have `dimensions: 3`.
pub struct ViewStreamSource3<X: Extractor3, K: StreamKinds3 = NoKinds3> {
    enc: FrameEncoder3<X, K>,
    schema: Schema,
    event_type: EventTypeFn,
    last_snapshot: Option<u64>,
    last_rollbacks: u64,
    discontinuity: bool,
}

impl<X: Extractor3, K: StreamKinds3> ViewStreamSource3<X, K> {
    /// A source for `extractor` and `kinds` that describes itself with `schema`.
    pub fn new(extractor: X, kinds: K, schema: Schema) -> Self {
        debug_assert_eq!(schema.dimensions, 3, "a 3D source needs a 3D schema");
        Self {
            enc: FrameEncoder3::new(extractor, kinds),
            schema,
            event_type: Box::new(|_| 0),
            last_snapshot: None,
            last_rollbacks: 0,
            discontinuity: false,
        }
    }

    /// Sets how event payloads map to the schema's event type ids.
    pub fn with_event_type(mut self, f: impl Fn(&[u8]) -> u16 + Send + 'static) -> Self {
        self.event_type = Box::new(f);
        self
    }

    /// The schema.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The frame of `snap`, or `None` if it is the snapshot the last call saw.
    pub fn frame_from_snapshot(&mut self, snap: &Snapshot) -> Option<ViewFrame3> {
        if self.last_snapshot == Some(snap.seq()) {
            return None;
        }
        self.last_snapshot = Some(snap.seq());
        let mut flags = 0;
        let rollbacks = snap.stats().rollbacks;
        let rollback = (rollbacks > self.last_rollbacks)
            .then(|| snap.last_rollback().map(|r| (r.from_tick, r.to_tick)))
            .flatten();
        self.last_rollbacks = rollbacks;
        if std::mem::take(&mut self.discontinuity) {
            flags |= FLAG_DISCONTINUITY;
        }
        if snap.timeline().is_some_and(|t| !t.playing) {
            flags |= FLAG_PAUSED;
        }
        let meta = FrameMeta { tick: snap.tick(), verified_tick: snap.verified_tick(), flags, rollback };
        Some(self.enc.encode(snap.predicted(), snap.predicted_prev(), meta))
    }

    /// Takes the bridge's newest snapshot and all its pending events.
    pub fn pump<G: Game, B: Bridge<G> + ?Sized>(&mut self, bridge: &mut B) -> Pumped3 {
        let mut events = Vec::new();
        for ev in bridge.drain_events() {
            match ev {
                BridgeEvent::Sim { key, status } => {
                    let (state, payload) = match &status {
                        EventStatus::Predicted(e) => (STATE_PREDICTED, bytemuck::bytes_of(e).to_vec()),
                        EventStatus::Verified(e) => (STATE_VERIFIED, bytemuck::bytes_of(e).to_vec()),
                        EventStatus::Canceled => (STATE_CANCELED, Vec::new()),
                    };
                    let event_type = (self.event_type)(&payload);
                    events.push(EventRecord { tick: key.tick, system: u32::from(key.system_index), seq: key.seq, state, event_type, payload });
                }
                BridgeEvent::Lifecycle(Lifecycle::Seeked { .. } | Lifecycle::Branched { .. }) => self.discontinuity = true,
                BridgeEvent::Lifecycle(_) => {}
            }
        }
        let frame = bridge.snapshot().and_then(|s| self.frame_from_snapshot(&s));
        Pumped3 { frame, events }
    }
}

impl<X: Extractor3 + Send, K: StreamKinds3> StreamProducer for ViewStreamSource3<X, K> {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn encode_frame(&mut self, cur: &Frame, prev: Option<&Frame>, meta: FrameMeta) -> Vec<u8> {
        self.enc.encode(FrameView::of(cur), prev.map(FrameView::of), meta).encode()
    }

    fn event_type(&self, payload: &[u8]) -> u16 {
        (self.event_type)(payload)
    }
}
