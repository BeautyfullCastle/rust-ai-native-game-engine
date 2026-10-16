//! Producing view frames: the game's [`Extractor`] runs on the host side and
//! its output becomes [`ViewFrame`]s.
//!
//! - [`FrameEncoder`] turns one frame (and the one before it) into a
//!   [`ViewFrame`]. It does not care where the frames come from.
//! - [`ViewStreamSource`] drives it from a [`Bridge`]'s pinned view updates:
//!   it sets rollback, discontinuity, and event-reset flags, then converts
//!   post-cut bridge events into [`EventRecord`]s.
//! - [`StreamProducer`] is the object-safe face of a source for hosts that
//!   hold frames directly (the ERP server's `viewstream` topic).

use orr_bridge::{Bridge, BridgeEvent, EventStatus, FrameView, Lifecycle, Snapshot};
use orr_ecs::{Entity, Frame};
use orr_sim::Game;
use orr_view::{Extracted, Extracted3, Extractor, Extractor3, InterpMode, Shape, Shape3};

use crate::format::{
    color_to_u8, EntityRecord, EntityRecord3, EventRecord, Pose3, ViewFrame, ViewFrame3, FLAG_DISCONTINUITY, FLAG_PAUSED,
    FLAG_EVENTS_RESET, FLAG_ROLLED_BACK, MODE_NONE, MODE_PREDICTION, MODE_SNAPSHOT, SHAPE3_BOX, SHAPE3_CAPSULE, SHAPE3_PLANE, SHAPE3_SPHERE,
    SHAPE_CAPSULE, SHAPE_CIRCLE, SHAPE_QUAD, STATE_CANCELED, STATE_PREDICTED, STATE_VERIFIED, STYLE_CHECKER,
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
    /// `FLAG_*` bits. `FLAG_DISCONTINUITY` makes `prev` equal `cur`;
    /// `FLAG_EVENTS_RESET` tells consumers to clear pending predicted effects.
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
    events_reset: bool,
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
            events_reset: false,
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
        if std::mem::take(&mut self.events_reset) {
            flags |= FLAG_DISCONTINUITY | FLAG_EVENTS_RESET;
        }
        if snap.timeline().is_some_and(|t| !t.playing) {
            flags |= FLAG_PAUSED;
        }
        let meta = FrameMeta { tick: snap.tick(), verified_tick: snap.verified_tick(), flags, rollback };
        Some(self.enc.encode(snap.predicted(), snap.predicted_prev(), meta))
    }

    /// Takes one bounded, coherent bridge view update. A recovery emits a
    /// complete discontinuity frame with `FLAG_EVENTS_RESET` even when the
    /// pinned snapshot's sequence did not advance. Resync summaries are not
    /// event records; recovery emits no events and later polls deliver only
    /// post-cut records.
    pub fn pump<G: Game, B: Bridge<G> + ?Sized>(&mut self, bridge: &mut B) -> Pumped {
        let update = bridge.poll_view();
        let reset_events = update.resync.is_some()
            || update.events.iter().any(|ev| matches!(ev, BridgeEvent::ViewResynced(_)));
        if reset_events {
            // Use the snapshot paired with this cut; a separate latest-snapshot
            // read could race ahead of it.
            self.last_snapshot = None;
            self.discontinuity = true;
            self.events_reset = true;
        }
        // Custom/legacy adapters may report recovery before any snapshot exists.
        // Hold later events until the reset baseline can accompany them.
        let recovery_waiting_for_baseline = self.events_reset && update.snapshot.is_none();
        let mut events = Vec::new();
        if !reset_events && !recovery_waiting_for_baseline {
            for ev in update.events {
                match ev {
                    BridgeEvent::ViewResynced(_) => unreachable!("recovery marker was detected before event conversion"),
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
        }
        let frame = update.snapshot.as_ref().and_then(|s| self.frame_from_snapshot(s));
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
    events_reset: bool,
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
            events_reset: false,
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
        if std::mem::take(&mut self.events_reset) {
            flags |= FLAG_DISCONTINUITY | FLAG_EVENTS_RESET;
        }
        if snap.timeline().is_some_and(|t| !t.playing) {
            flags |= FLAG_PAUSED;
        }
        let meta = FrameMeta { tick: snap.tick(), verified_tick: snap.verified_tick(), flags, rollback };
        Some(self.enc.encode(snap.predicted(), snap.predicted_prev(), meta))
    }

    /// Takes one bounded, coherent bridge view update. A recovery emits a
    /// complete discontinuity frame with `FLAG_EVENTS_RESET` even when the
    /// pinned snapshot's sequence did not advance; only later, post-cut event
    /// records are converted.
    pub fn pump<G: Game, B: Bridge<G> + ?Sized>(&mut self, bridge: &mut B) -> Pumped3 {
        let update = bridge.poll_view();
        let reset_events = update.resync.is_some()
            || update.events.iter().any(|ev| matches!(ev, BridgeEvent::ViewResynced(_)));
        if reset_events {
            self.last_snapshot = None;
            self.discontinuity = true;
            self.events_reset = true;
        }
        let recovery_waiting_for_baseline = self.events_reset && update.snapshot.is_none();
        let mut events = Vec::new();
        if !reset_events && !recovery_waiting_for_baseline {
            for ev in update.events {
                match ev {
                    BridgeEvent::ViewResynced(_) => unreachable!("recovery marker was detected before event conversion"),
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
        }
        let frame = update.snapshot.as_ref().and_then(|s| self.frame_from_snapshot(s));
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
#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::time::Duration;

    use orr_bridge::{
        Bridge, BridgeError, BridgeEvent, BridgeStats, EventStatus, FrameView, Snapshot,
        SnapshotParts, ViewResync, ViewUpdate,
    };
    use orr_ecs::{ComponentRegistryBuilder, Frame};
    use orr_sim::{Game, PlayerSlot, SimCommand, System};
    use orr_view::{
        Extracted, Extracted3, Extractor, Extractor3, InterpMode, Quat, Shape, Shape3, Style,
        Style3, Transform2, Transform3, Vec2, Vec3,
    };

    use super::{NoKinds, NoKinds3, ViewStreamSource, ViewStreamSource3};
    use crate::format::{FLAG_DISCONTINUITY, FLAG_EVENTS_RESET};
    use crate::schema::{InputLayout, Schema};

    #[derive(Clone)]
    struct NoCommand;
    impl SimCommand for NoCommand {
        fn encode(&self, _out: &mut Vec<u8>) {}
        fn decode(bytes: &[u8]) -> Option<Self> {
            bytes.is_empty().then_some(Self)
        }
    }

    struct TestGame;
    impl Game for TestGame {
        type Input = u8;
        type Command = NoCommand;
        type Event = u32;
        type Config = ();

        fn register(_builder: &mut ComponentRegistryBuilder) {}
        fn setup(_frame: &mut Frame, _config: &Self::Config) {}
        fn systems() -> Vec<Box<dyn System<Self>>> {
            Vec::new()
        }
    }

    #[derive(Clone, Copy)]
    struct Position;
    impl Extractor for Position {
        fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted>) {
            for (entity, x) in frame.iter::<u32>() {
                out.push(Extracted {
                    entity,
                    transform: Transform2::new(Vec2::new(*x as f32, 0.0), 0.0),
                    mode: InterpMode::Prediction,
                    style: Style {
                        shape: Shape::Circle,
                        size: 1.0,
                        half_y: 0.0,
                        color: [1.0; 4],
                    },
                });
            }
        }
    }

    #[derive(Clone, Copy)]
    struct Position3;
    impl Extractor3 for Position3 {
        fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted3>) {
            for (entity, x) in frame.iter::<u32>() {
                out.push(Extracted3 {
                    entity,
                    transform: Transform3::new(Vec3::new(*x as f32, 0.0, 0.0), Quat::IDENTITY),
                    mode: InterpMode::Prediction,
                    style: Style3::new(Shape3::Sphere { radius: 1.0 }, [1.0; 3]),
                });
            }
        }
    }

    struct MockBridge {
        updates: VecDeque<ViewUpdate<u32>>,
        latest: Option<Snapshot>,
    }

    impl Bridge<TestGame> for MockBridge {
        fn tick_rate(&self) -> u32 {
            60
        }
        fn local_slot(&self) -> PlayerSlot {
            PlayerSlot(0)
        }
        fn player_count(&self) -> u8 {
            1
        }
        fn set_input(&mut self, _player: PlayerSlot, _input: u8) -> Result<(), BridgeError> {
            Ok(())
        }
        fn send_command(&mut self, _command: NoCommand) -> Result<(), BridgeError> {
            Ok(())
        }
        fn update(&mut self, _elapsed: Duration) {}
        fn snapshot(&self) -> Option<Snapshot> {
            self.latest.clone()
        }
        fn drain_events(&mut self) -> Vec<BridgeEvent<u32>> {
            Vec::new()
        }
        fn poll_view(&mut self) -> ViewUpdate<u32> {
            self.updates.pop_front().unwrap_or_else(|| ViewUpdate {
                snapshot: self.latest.clone(),
                events: Vec::new(),
                resync: None,
            })
        }
        fn is_alive(&self) -> bool {
            true
        }
    }

    fn snapshot(
        seq: u64,
        prev_x: u32,
        cur_x: u32,
        registry: Arc<orr_ecs::ComponentRegistry>,
    ) -> Snapshot {
        let mut prev = Frame::new(registry.clone());
        let entity = prev.spawn();
        let _ = prev.add(entity, prev_x);
        let mut cur = Frame::new(registry);
        let cur_entity = cur.spawn();
        let _ = cur.add(cur_entity, cur_x);
        Snapshot::from_parts(SnapshotParts {
            seq,
            tick: 2,
            verified_tick: 2,
            tick_rate: 60,
            predicted: Arc::new(cur),
            predicted_prev: Some(Arc::new(prev)),
            verified: None,
            stats: BridgeStats::default(),
            last_rollback: None,
            timeline: None,
        })
    }

    fn schema() -> Schema {
        schema_for(2)
    }

    fn schema_for(dimensions: u8) -> Schema {
        Schema {
            game: "test".into(),
            dimensions,
            build_id: 1,
            tick_rate: 60,
            player_count: 1,
            kinds: Vec::new(),
            input: InputLayout {
                size: 0,
                fields: Vec::new(),
            },
            command_size: 0,
            events: Vec::new(),
        }
    }

    fn resync() -> ViewResync {
        ViewResync {
            generation: 1,
            discarded_events: 12,
            head_tick: 2,
            verified_tick: 2,
            lifecycle: Vec::new(),
            disconnected: false,
            last_desync: None,
        }
    }

    #[test]
    fn recovery_uses_pinned_snapshot_discards_events_and_forces_same_seq_frame() {
        let mut builder = ComponentRegistryBuilder::new();
        let _ = builder.register_component::<u32>("position");
        let registry = builder.build();
        let pinned = snapshot(7, 9, 10, registry.clone());
        let unrelated_latest = snapshot(8, 99, 100, registry);
        let resync = resync();
        let mut bridge = MockBridge {
            latest: Some(unrelated_latest),
            updates: VecDeque::from([
                ViewUpdate {
                    snapshot: Some(pinned.clone()),
                    events: Vec::new(),
                    resync: None,
                },
                // The real bounded receiver guarantees an empty `events` vec on
                // reset. Supplying an event here verifies the source fails
                // closed if an adapter violates that contract.
                ViewUpdate {
                    snapshot: Some(pinned.clone()),
                    events: vec![BridgeEvent::Sim {
                        key: orr_sim::EventKey::new(1, 0, 0),
                        status: EventStatus::Predicted(42),
                    }],
                    resync: Some(resync.clone()),
                },
                ViewUpdate {
                    snapshot: Some(pinned.clone()),
                    events: vec![BridgeEvent::ViewResynced(resync)],
                    resync: None,
                },
                ViewUpdate {
                    snapshot: Some(pinned),
                    events: Vec::new(),
                    resync: None,
                },
            ]),
        };
        let mut source = ViewStreamSource::new(Position, NoKinds, schema());

        let first = source.pump(&mut bridge);
        assert_eq!(
            first.frame.unwrap().entities[0].cur[0],
            10.0,
            "pump must use its pinned update snapshot"
        );

        let recovered = source.pump(&mut bridge);
        let frame = recovered
            .frame
            .expect("resync must force a frame even with unchanged snapshot seq");
        assert!(frame.has(FLAG_DISCONTINUITY));
        assert!(frame.has(FLAG_EVENTS_RESET));
        assert_eq!(
            frame.entities[0].prev, frame.entities[0].cur,
            "recovery frame must not interpolate from stale state"
        );
        assert!(
            recovered.events.is_empty(),
            "pre-cut effects must not leak through recovery"
        );

        // Adapters using the compatibility split-read path surface recovery
        // as a distinct marker; the source must convert it into the same
        // frame protocol signal rather than silently swallowing it.
        let legacy = source.pump(&mut bridge);
        let legacy_frame = legacy
            .frame
            .expect("legacy recovery marker must force a frame");
        assert!(legacy_frame.has(FLAG_EVENTS_RESET));
        assert!(legacy_frame.has(FLAG_DISCONTINUITY));
        assert!(legacy.events.is_empty());

        assert!(
            source.pump(&mut bridge).frame.is_none(),
            "unchanged seq is suppressed again after the one reset frame"
        );
    }

    #[test]
    fn recovery_3d_uses_pinned_snapshot_and_forces_a_discontinuity_frame() {
        let mut builder = ComponentRegistryBuilder::new();
        let _ = builder.register_component::<u32>("position");
        let registry = builder.build();
        let pinned = snapshot(7, 9, 10, registry.clone());
        let unrelated_latest = snapshot(8, 99, 100, registry);
        let mut bridge = MockBridge {
            latest: Some(unrelated_latest),
            updates: VecDeque::from([
                ViewUpdate {
                    snapshot: Some(pinned.clone()),
                    events: Vec::new(),
                    resync: None,
                },
                ViewUpdate {
                    snapshot: Some(pinned.clone()),
                    events: Vec::new(),
                    resync: Some(resync()),
                },
            ]),
        };
        let mut source = ViewStreamSource3::new(Position3, NoKinds3, schema_for(3));

        let first = source.pump(&mut bridge).frame.unwrap();
        assert_eq!(first.entities[0].cur.pos, [10.0, 0.0, 0.0]);
        let recovered = source.pump(&mut bridge);
        let frame = recovered
            .frame
            .expect("resync must force a frame even with unchanged snapshot seq");
        assert!(frame.has(FLAG_DISCONTINUITY));
        assert!(frame.has(FLAG_EVENTS_RESET));
        assert_eq!(frame.entities[0].prev.pos, frame.entities[0].cur.pos);
        assert!(recovered.events.is_empty());
    }

    #[test]
    fn recovery_without_snapshot_withholds_events_until_the_baseline_frame() {
        let mut builder = ComponentRegistryBuilder::new();
        let _ = builder.register_component::<u32>("position");
        let registry = builder.build();
        let pinned = snapshot(7, 9, 10, registry);
        let predicted = || BridgeEvent::Sim {
            key: orr_sim::EventKey::new(3, 0, 1),
            status: EventStatus::Predicted(42),
        };
        let mut bridge = MockBridge {
            latest: Some(pinned.clone()),
            updates: VecDeque::from([
                ViewUpdate {
                    snapshot: Some(pinned.clone()),
                    events: Vec::new(),
                    resync: None,
                },
                ViewUpdate {
                    snapshot: None,
                    events: Vec::new(),
                    resync: Some(resync()),
                },
                ViewUpdate {
                    snapshot: None,
                    events: vec![predicted()],
                    resync: None,
                },
                ViewUpdate {
                    snapshot: Some(pinned),
                    events: vec![predicted()],
                    resync: None,
                },
            ]),
        };
        let mut source = ViewStreamSource::new(Position, NoKinds, schema());

        assert!(source.pump(&mut bridge).frame.is_some());
        let reset = source.pump(&mut bridge);
        assert!(reset.frame.is_none());
        assert!(reset.events.is_empty());
        let blocked = source.pump(&mut bridge);
        assert!(blocked.frame.is_none());
        assert!(
            blocked.events.is_empty(),
            "post-cut events wait until consumers have a reset baseline"
        );
        let recovered = source.pump(&mut bridge);
        assert!(recovered.frame.unwrap().has(FLAG_EVENTS_RESET));
        assert_eq!(
            recovered.events.len(),
            1,
            "subsequent event records resume after the baseline frame"
        );
    }
}
