//! The binary messages of the view stream. See `docs/view-stream.md` for the
//! byte layouts; this module is their only implementation in Rust.
//!
//! Everything is little-endian, fixed-size records, no padding surprises:
//! a foreign reader needs nothing but the layout tables.

/// First four bytes of every binary view-stream message.
pub const MAGIC: [u8; 4] = *b"OVS1";
/// Format version 1 (the `u16` after the magic): 2D frames and event batches.
/// Their bytes never change; they are written with this version.
pub const VERSION: u16 = 1;
/// Format version 2 adds the 3D frame ([`MSG_FRAME3D`]), which is written with this
/// version. Version 1 messages are unchanged.
pub const VERSION_3D: u16 = 2;
/// The newest version this reader knows. A reader refuses a larger one.
pub const MAX_VERSION: u16 = VERSION_3D;

/// Message type of a [`ViewFrame`].
pub const MSG_FRAME: u8 = 1;
/// Message type of an [`EventBatch`].
pub const MSG_EVENTS: u8 = 2;
/// Message type of a [`ViewFrame3`] (format version 2).
pub const MSG_FRAME3D: u8 = 3;

/// Size of a frame message header, in bytes.
pub const HEADER_LEN: usize = 56;
/// Size of one entity record, in bytes.
pub const RECORD_LEN: usize = 48;
/// Size of one 3D entity record, in bytes.
pub const RECORD3D_LEN: usize = 88;
/// Size of an event batch header, in bytes.
pub const EVENT_BATCH_HEADER_LEN: usize = 16;
/// Size of an event record without its payload, in bytes.
pub const EVENT_HEAD_LEN: usize = 24;

/// Frame flag: a rollback corrected the past since the previous frame sent.
pub const FLAG_ROLLED_BACK: u8 = 1;
/// Frame flag: the timeline jumped (seek, branch, edit): show this frame as it
/// is, with no blend from what was shown before. `prev` equals `cur` then.
pub const FLAG_DISCONTINUITY: u8 = 2;
/// Frame flag: the simulation is paused (the tick does not advance by itself).
pub const FLAG_PAUSED: u8 = 4;
/// Frame flag: the bounded event stream lost notifications; clear speculative
/// effects and history before processing later event batches. The frame is a
/// self-contained discontinuity baseline; the lost count is not on the wire.
pub const FLAG_EVENTS_RESET: u8 = 8;

/// Entity shape code: a circle, `size` is the radius.
pub const SHAPE_CIRCLE: u8 = 0;
/// A rectangle: `size` is the half width, `half_y` the half height (0 = `size`).
pub const SHAPE_QUAD: u8 = 1;
/// A segment along local x grown by a radius: `size` is the half length, `half_y` the radius.
pub const SHAPE_CAPSULE: u8 = 2;

/// 3D shape code: a sphere, `size[0]` is the radius.
pub const SHAPE3_SPHERE: u8 = 0;
/// A box: `size` is the half extents along the local axes.
pub const SHAPE3_BOX: u8 = 1;
/// A segment along local y grown by a radius: `size[0]` is the radius, `size[1]` the half length.
pub const SHAPE3_CAPSULE: u8 = 2;
/// A horizontal rectangle (local xz, normal local y): `size[0]` is the half x, `size[2]` the half z.
pub const SHAPE3_PLANE: u8 = 3;

/// 3D style flag: draw a checker pattern (ground planes).
pub const STYLE_CHECKER: u8 = 1;

/// Interpolation mode code: blend the last two predicted ticks.
pub const MODE_PREDICTION: u8 = 0;
/// Play the confirmed frames slightly late. This stream carries predicted transforms
/// only, so a consumer shows these like `MODE_PREDICTION` unless it buffers frames itself.
pub const MODE_SNAPSHOT: u8 = 1;
/// Show the newest tick as it is (`prev` equals `cur` for a static entity).
pub const MODE_NONE: u8 = 2;

/// Event state: emitted by a predicted tick, a rollback may still cancel it.
pub const STATE_PREDICTED: u8 = 0;
/// The event's tick is verified; it will never be rolled back. Announced once.
pub const STATE_VERIFIED: u8 = 1;
/// A rollback removed a predicted event; treat it as if it never happened.
pub const STATE_CANCELED: u8 = 2;

/// Why a message could not be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Shorter than its header or its declared content.
    Truncated,
    /// Does not start with [`MAGIC`].
    BadMagic,
    /// A newer format version than this reader knows.
    UnsupportedVersion(u16),
    /// A message type this function does not decode.
    WrongType(u8),
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DecodeError::Truncated => f.write_str("view stream message is truncated"),
            DecodeError::BadMagic => f.write_str("not a view stream message (bad magic)"),
            DecodeError::UnsupportedVersion(v) => write!(f, "view stream version {v} is newer than this reader ({MAX_VERSION})"),
            DecodeError::WrongType(t) => write!(f, "unexpected view stream message type {t}"),
        }
    }
}
impl std::error::Error for DecodeError {}

#[cfg(test)]
mod reset_frame_tests {
    use super::*;

    #[test]
    fn reset_frame_events_marks_2d_baseline_and_collapses_interpolation() {
        let input = ViewFrame {
            flags: FLAG_PAUSED,
            tick: 10,
            verified_tick: 9,
            seq: 42,
            rollback: None,
            entities: vec![EntityRecord {
                id: 3,
                kind: 0,
                shape: SHAPE_CIRCLE,
                mode: MODE_PREDICTION,
                size: 1.0,
                half_y: 0.0,
                rgba: [255; 4],
                prev: [1.0, 2.0, 0.0],
                cur: [4.0, 5.0, 0.5],
            }],
            props: vec![1, 2, 3, 4],
        };
        let reset = ViewFrame::decode(&reset_frame_events(&input.encode()).unwrap()).unwrap();
        assert!(reset.has(FLAG_DISCONTINUITY | FLAG_EVENTS_RESET | FLAG_PAUSED));
        assert_eq!(reset.entities[0].prev, reset.entities[0].cur);
        assert_eq!(reset.props, input.props);
    }

    #[test]
    fn reset_frame_events_handles_3d_and_rejects_nonframes() {
        let input = ViewFrame3 {
            flags: 0,
            tick: 10,
            verified_tick: 9,
            seq: 42,
            rollback: None,
            entities: vec![EntityRecord3 {
                id: 3,
                kind: 0,
                shape: SHAPE3_SPHERE,
                mode: MODE_PREDICTION,
                size: [1.0, 0.0, 0.0],
                rgba: [255; 4],
                roughness: 160,
                metallic: 0,
                style_flags: 0,
                prev: Pose3 { pos: [1.0, 2.0, 3.0], rot: [0.0, 0.0, 0.0, 1.0] },
                cur: Pose3 { pos: [4.0, 5.0, 6.0], rot: [0.0, 0.0, 1.0, 0.0] },
            }],
            props: Vec::new(),
        };
        let reset = ViewFrame3::decode(&reset_frame_events(&input.encode()).unwrap()).unwrap();
        assert!(reset.has(FLAG_DISCONTINUITY | FLAG_EVENTS_RESET));
        assert_eq!(reset.entities[0].prev, reset.entities[0].cur);
        assert_eq!(reset.seq, input.seq);
        assert_eq!(reset_frame_events(&EventBatch::default().encode()), Err(DecodeError::WrongType(MSG_EVENTS)));
    }
}

/// One entity as a view draws it: identity, look, and its transform at the
/// previous and the current tick. The consumer interpolates
/// `prev + (cur - prev) * alpha` with its own `alpha` (rotation along the
/// shorter way round).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityRecord {
    /// Stable identity: `entity index | version << 32`. A reused index gets a new version.
    pub id: u64,
    /// Game-defined kind (see the schema's `kinds`).
    pub kind: u16,
    /// `SHAPE_*`.
    pub shape: u8,
    /// `MODE_*`.
    pub mode: u8,
    /// Radius, half width or half segment length, in world units.
    pub size: f32,
    /// Quad: half height. Capsule: radius.
    pub half_y: f32,
    /// Color, 8 bits per channel.
    pub rgba: [u8; 4],
    /// `[x, y, rotation in radians]` at the previous tick.
    pub prev: [f32; 3],
    /// `[x, y, rotation in radians]` at the current tick.
    pub cur: [f32; 3],
}

/// One published tick.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewFrame {
    /// `FLAG_*` bits.
    pub flags: u8,
    /// The predicted head tick these records show.
    pub tick: u64,
    /// The newest fully confirmed tick (equals `tick` without prediction).
    pub verified_tick: u64,
    /// Counts up by one per frame the producer built.
    pub seq: u64,
    /// `(from_tick, to_tick)` of the rollback, when `FLAG_ROLLED_BACK` is set.
    pub rollback: Option<(u64, u64)>,
    /// The entities, in the game extractor's order.
    pub entities: Vec<EntityRecord>,
    /// Custom properties: for each entity in order, the kind's property words
    /// (32-bit: f32, u32 or i32 as the schema says), concatenated, little-endian.
    pub props: Vec<u8>,
}

impl ViewFrame {
    /// True if `flag` is set.
    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    /// The property bytes of each entity, given how many 32-bit words a kind has.
    /// Returns `None` if the declared property bytes do not add up.
    pub fn props_by_entity(&self, words_of_kind: impl Fn(u16) -> usize) -> Option<Vec<&[u8]>> {
        let mut out = Vec::with_capacity(self.entities.len());
        let mut at = 0usize;
        for e in &self.entities {
            let n = words_of_kind(e.kind) * 4;
            out.push(self.props.get(at..at + n)?);
            at += n;
        }
        (at == self.props.len()).then_some(out)
    }

    /// The bytes of the message (see `docs/view-stream.md`).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.entities.len() * RECORD_LEN + self.props.len());
        self.encode_into(&mut out);
        out
    }

    /// Like [`encode`](Self::encode), into `out` (cleared first), so a producer can reuse a buffer.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.clear();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.push(MSG_FRAME);
        let flags = if self.rollback.is_some() { self.flags | FLAG_ROLLED_BACK } else { self.flags };
        out.push(flags);
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.extend_from_slice(&self.verified_tick.to_le_bytes());
        out.extend_from_slice(&self.seq.to_le_bytes());
        let (from, to) = self.rollback.unwrap_or((0, 0));
        out.extend_from_slice(&from.to_le_bytes());
        out.extend_from_slice(&to.to_le_bytes());
        out.extend_from_slice(&(self.entities.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.props.len() as u32).to_le_bytes());
        for e in &self.entities {
            out.extend_from_slice(&e.id.to_le_bytes());
            out.extend_from_slice(&e.kind.to_le_bytes());
            out.push(e.shape);
            out.push(e.mode);
            out.extend_from_slice(&e.size.to_le_bytes());
            out.extend_from_slice(&e.half_y.to_le_bytes());
            out.extend_from_slice(&e.rgba);
            for v in e.prev.iter().chain(e.cur.iter()) {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out.extend_from_slice(&self.props);
    }

    /// Reads a frame message.
    pub fn decode(bytes: &[u8]) -> Result<ViewFrame, DecodeError> {
        let mut r = Reader::new(bytes);
        let (msg_type, flags) = r.preamble()?;
        if msg_type != MSG_FRAME {
            return Err(DecodeError::WrongType(msg_type));
        }
        let tick = r.u64()?;
        let verified_tick = r.u64()?;
        let seq = r.u64()?;
        let (from, to) = (r.u64()?, r.u64()?);
        let count = r.u32()? as usize;
        let props_len = r.u32()? as usize;
        let body = count.checked_mul(RECORD_LEN).ok_or(DecodeError::Truncated)?;
        if r.left() < body.saturating_add(props_len) {
            return Err(DecodeError::Truncated);
        }
        let mut entities = Vec::with_capacity(count);
        for _ in 0..count {
            let id = r.u64()?;
            let kind = r.u16()?;
            let (shape, mode) = (r.u8()?, r.u8()?);
            let (size, half_y) = (r.f32()?, r.f32()?);
            let rgba = [r.u8()?, r.u8()?, r.u8()?, r.u8()?];
            let prev = [r.f32()?, r.f32()?, r.f32()?];
            let cur = [r.f32()?, r.f32()?, r.f32()?];
            entities.push(EntityRecord { id, kind, shape, mode, size, half_y, rgba, prev, cur });
        }
        let props = r.take(props_len)?.to_vec();
        let rollback = (flags & FLAG_ROLLED_BACK != 0).then_some((from, to));
        Ok(ViewFrame { flags, tick, verified_tick, seq, rollback, entities, props })
    }
}

/// Makes an already-encoded frame a sticky event-reset baseline for transports
/// that retain or overwrite only the latest frame. Sets both recovery flags
/// and collapses interpolation endpoints. The discarded count is not on wire.
pub fn reset_frame_events(bytes: &[u8]) -> Result<Vec<u8>, DecodeError> {
    match message_type(bytes)? {
        MSG_FRAME => {
            let mut frame = ViewFrame::decode(bytes)?;
            frame.flags |= FLAG_DISCONTINUITY | FLAG_EVENTS_RESET;
            for entity in &mut frame.entities {
                entity.prev = entity.cur;
            }
            Ok(frame.encode())
        }
        MSG_FRAME3D => {
            let mut frame = ViewFrame3::decode(bytes)?;
            frame.flags |= FLAG_DISCONTINUITY | FLAG_EVENTS_RESET;
            for entity in &mut frame.entities {
                entity.prev = entity.cur;
            }
            Ok(frame.encode())
        }
        t => Err(DecodeError::WrongType(t)),
    }
}

/// A 3D pose: position and orientation as a unit quaternion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose3 {
    /// `[x, y, z]`, world units, y up, right handed.
    pub pos: [f32; 3],
    /// Unit quaternion `[x, y, z, w]`.
    pub rot: [f32; 4],
}

impl Pose3 {
    /// The pose at the origin with no rotation.
    pub const IDENTITY: Pose3 = Pose3 { pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0] };
}

/// One 3D entity as a view draws it: identity, look, and its pose at the
/// previous and the current tick. The consumer interpolates with its own
/// `alpha`: position linearly, rotation by slerp (shortest arc).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityRecord3 {
    /// Stable identity: `entity index | version << 32`.
    pub id: u64,
    /// Game-defined kind (see the schema's `kinds`).
    pub kind: u16,
    /// `SHAPE3_*`.
    pub shape: u8,
    /// `MODE_*`.
    pub mode: u8,
    /// Shape size, see the `SHAPE3_*` codes (unused components are 0).
    pub size: [f32; 3],
    /// Color, 8 bits per channel (linear RGB, straight alpha).
    pub rgba: [u8; 4],
    /// Roughness, `0..=255` for `0.0..=1.0`.
    pub roughness: u8,
    /// Metallic, `0..=255` for `0.0..=1.0`.
    pub metallic: u8,
    /// `STYLE_*` bits.
    pub style_flags: u8,
    /// Pose at the previous tick.
    pub prev: Pose3,
    /// Pose at the current tick.
    pub cur: Pose3,
}

/// One published tick of a 3D game (format version 2, message type 3). Same
/// header and meaning as [`ViewFrame`]; the records are 88 bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewFrame3 {
    /// `FLAG_*` bits.
    pub flags: u8,
    /// The predicted head tick these records show.
    pub tick: u64,
    /// The newest fully confirmed tick.
    pub verified_tick: u64,
    /// Counts up by one per frame the producer built.
    pub seq: u64,
    /// `(from_tick, to_tick)` of the rollback, when `FLAG_ROLLED_BACK` is set.
    pub rollback: Option<(u64, u64)>,
    /// The entities.
    pub entities: Vec<EntityRecord3>,
    /// Custom properties, as in [`ViewFrame::props`].
    pub props: Vec<u8>,
}

impl ViewFrame3 {
    /// True if `flag` is set.
    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    /// The property bytes of each entity (see [`ViewFrame::props_by_entity`]).
    pub fn props_by_entity(&self, words_of_kind: impl Fn(u16) -> usize) -> Option<Vec<&[u8]>> {
        let mut out = Vec::with_capacity(self.entities.len());
        let mut at = 0usize;
        for e in &self.entities {
            let n = words_of_kind(e.kind) * 4;
            out.push(self.props.get(at..at + n)?);
            at += n;
        }
        (at == self.props.len()).then_some(out)
    }

    /// The bytes of the message (see `docs/view-stream.md`).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.entities.len() * RECORD3D_LEN + self.props.len());
        self.encode_into(&mut out);
        out
    }

    /// Like [`encode`](Self::encode), into `out` (cleared first).
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.clear();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION_3D.to_le_bytes());
        out.push(MSG_FRAME3D);
        let flags = if self.rollback.is_some() { self.flags | FLAG_ROLLED_BACK } else { self.flags };
        out.push(flags);
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.extend_from_slice(&self.verified_tick.to_le_bytes());
        out.extend_from_slice(&self.seq.to_le_bytes());
        let (from, to) = self.rollback.unwrap_or((0, 0));
        out.extend_from_slice(&from.to_le_bytes());
        out.extend_from_slice(&to.to_le_bytes());
        out.extend_from_slice(&(self.entities.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.props.len() as u32).to_le_bytes());
        for e in &self.entities {
            out.extend_from_slice(&e.id.to_le_bytes());
            out.extend_from_slice(&e.kind.to_le_bytes());
            out.push(e.shape);
            out.push(e.mode);
            for v in e.size {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&e.rgba);
            out.push(e.roughness);
            out.push(e.metallic);
            out.push(e.style_flags);
            out.push(0);
            for p in [&e.prev, &e.cur] {
                for v in p.pos.iter().chain(p.rot.iter()) {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        out.extend_from_slice(&self.props);
    }

    /// Reads a 3D frame message.
    pub fn decode(bytes: &[u8]) -> Result<ViewFrame3, DecodeError> {
        let mut r = Reader::new(bytes);
        let (msg_type, flags) = r.preamble()?;
        if msg_type != MSG_FRAME3D {
            return Err(DecodeError::WrongType(msg_type));
        }
        let tick = r.u64()?;
        let verified_tick = r.u64()?;
        let seq = r.u64()?;
        let (from, to) = (r.u64()?, r.u64()?);
        let count = r.u32()? as usize;
        let props_len = r.u32()? as usize;
        let body = count.checked_mul(RECORD3D_LEN).ok_or(DecodeError::Truncated)?;
        if r.left() < body.saturating_add(props_len) {
            return Err(DecodeError::Truncated);
        }
        let mut entities = Vec::with_capacity(count);
        for _ in 0..count {
            let id = r.u64()?;
            let kind = r.u16()?;
            let (shape, mode) = (r.u8()?, r.u8()?);
            let size = [r.f32()?, r.f32()?, r.f32()?];
            let rgba = [r.u8()?, r.u8()?, r.u8()?, r.u8()?];
            let (roughness, metallic, style_flags) = (r.u8()?, r.u8()?, r.u8()?);
            r.u8()?;
            let prev = Pose3 { pos: [r.f32()?, r.f32()?, r.f32()?], rot: [r.f32()?, r.f32()?, r.f32()?, r.f32()?] };
            let cur = Pose3 { pos: [r.f32()?, r.f32()?, r.f32()?], rot: [r.f32()?, r.f32()?, r.f32()?, r.f32()?] };
            entities.push(EntityRecord3 { id, kind, shape, mode, size, rgba, roughness, metallic, style_flags, prev, cur });
        }
        let props = r.take(props_len)?.to_vec();
        let rollback = (flags & FLAG_ROLLED_BACK != 0).then_some((from, to));
        Ok(ViewFrame3 { flags, tick, verified_tick, seq, rollback, entities, props })
    }
}

/// One sim event as a view hears of it: which event (the deterministic key),
/// in which state, and its game-defined payload bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRecord {
    /// The tick that emitted the event. With `system` and `seq` it is the
    /// event's identity, the same across a rollback resimulation: a
    /// `STATE_VERIFIED` or `STATE_CANCELED` record matches the `STATE_PREDICTED`
    /// one with the same key.
    pub tick: u64,
    /// Index of the emitting system.
    pub system: u32,
    /// Sequence number within the system and tick.
    pub seq: u32,
    /// `STATE_*`.
    pub state: u8,
    /// Game-defined event type (see the schema's `events`).
    pub event_type: u16,
    /// The event's bytes (empty for `STATE_CANCELED`).
    pub payload: Vec<u8>,
}

/// Events that arrived since the last batch, in sim order.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EventBatch {
    /// The events.
    pub events: Vec<EventRecord>,
}

impl EventBatch {
    /// The bytes of the message (see `docs/view-stream.md`).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(EVENT_BATCH_HEADER_LEN + self.events.len() * (EVENT_HEAD_LEN + 16));
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.push(MSG_EVENTS);
        out.push(0);
        out.extend_from_slice(&(self.events.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        for e in &self.events {
            out.extend_from_slice(&e.tick.to_le_bytes());
            out.extend_from_slice(&e.system.to_le_bytes());
            out.extend_from_slice(&e.seq.to_le_bytes());
            out.push(e.state);
            out.push(0);
            out.extend_from_slice(&e.event_type.to_le_bytes());
            out.extend_from_slice(&(e.payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&e.payload);
            // Records start on 8-byte boundaries.
            let pad = (8 - e.payload.len() % 8) % 8;
            out.extend_from_slice(&[0u8; 8][..pad]);
        }
        out
    }

    /// Reads an event batch message.
    pub fn decode(bytes: &[u8]) -> Result<EventBatch, DecodeError> {
        let mut r = Reader::new(bytes);
        let (msg_type, _) = r.preamble()?;
        if msg_type != MSG_EVENTS {
            return Err(DecodeError::WrongType(msg_type));
        }
        let count = r.u32()? as usize;
        r.take(4)?;
        let mut events = Vec::with_capacity(count.min(1 << 16));
        for _ in 0..count {
            let tick = r.u64()?;
            let system = r.u32()?;
            let seq = r.u32()?;
            let state = r.u8()?;
            r.u8()?;
            let event_type = r.u16()?;
            let len = r.u32()? as usize;
            let payload = r.take(len)?.to_vec();
            r.take((8 - len % 8) % 8)?;
            events.push(EventRecord { tick, system, seq, state, event_type, payload });
        }
        Ok(EventBatch { events })
    }
}

/// The message type of `bytes` (after checking magic and version).
pub fn message_type(bytes: &[u8]) -> Result<u8, DecodeError> {
    Reader::new(bytes).preamble().map(|(t, _)| t)
}

/// Quantizes a color channel in `0.0..=1.0` to 8 bits.
pub fn color_to_u8(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0 }
    }
    fn left(&self) -> usize {
        self.b.len() - self.at
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.at.checked_add(n).ok_or(DecodeError::Truncated)?;
        let s = self.b.get(self.at..end).ok_or(DecodeError::Truncated)?;
        self.at = end;
        Ok(s)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        self.take(N)?.try_into().map_err(|_| DecodeError::Truncated)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.array()?))
    }
    /// Magic, version, message type, flags.
    fn preamble(&mut self) -> Result<(u8, u8), DecodeError> {
        if self.take(4)? != MAGIC {
            return Err(DecodeError::BadMagic);
        }
        let version = self.u16()?;
        if version > MAX_VERSION {
            return Err(DecodeError::UnsupportedVersion(version));
        }
        Ok((self.u8()?, self.u8()?))
    }
}
