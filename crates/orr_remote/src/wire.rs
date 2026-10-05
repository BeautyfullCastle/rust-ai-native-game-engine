//! Wire forms shared by the server and the `Remote` client: checksums,
//! timelines, raw debug commands and the binary frame message.
//!
//! # Checksums
//!
//! A `u64` checksum is the JSON string `"0x"` + 16 lowercase hex digits
//! (JSON numbers lose bits above 2^53 in many clients).
//!
//! # Frame message (WebSocket binary)
//!
//! ```text
//! "ORRS" | version u8 (=1) | 3 reserved bytes | meta_len u32 LE | meta (JSON, UTF-8) | payload
//! payload = lz4 block with a u32 LE size prefix of Frame::to_bytes() (ORRF v2, checksummed)
//! meta = { tick, epoch, tick_rate, player_count, sent_at_us, timeline: {...} }
//! ```
//!
//! `sent_at_us` is the sender's wall clock (microseconds since the Unix
//! epoch), meaningful for latency only when both sides share a clock (the same machine).

use std::{fmt, sync::Arc};

use orr_ecs::{ComponentId, Entity, SingletonId};
use orr_session::{PlayMode, Speed, Timeline};
use orr_sim::{DebugCommand, DebugError};
use serde_json::{json, Map, Value as J};

use crate::codec::{hex_decode, hex_encode};
use crate::frame_delta::{FrameRecord, FrameScope, FrameStamp};
use crate::json::{handle_text, parse_handle};

/// `"0x"` and 16 hex digits.
pub fn checksum_text(c: u64) -> String {
    format!("0x{c:016x}")
}

/// Parses a checksum text (`0x...` hex), or a plain integer.
pub fn parse_checksum(j: &J) -> Option<u64> {
    match j {
        J::String(s) => u64::from_str_radix(s.strip_prefix("0x")?, 16).ok(),
        J::Number(n) => n.as_u64(),
        _ => None,
    }
}

/// A timeline as JSON.
pub fn timeline_to_json(t: &Timeline) -> J {
    json!({
        "mode": match t.mode { PlayMode::Record => "record", PlayMode::Viewer => "viewer" },
        "tick": t.tick,
        "verified_tick": t.verified_tick,
        "first_tick": t.first_tick,
        "last_tick": t.last_tick,
        "playing": t.playing,
        "speed_permille": t.speed.permille(),
        "checksum": checksum_text(t.checksum),
        "keyframes": t.keyframes.iter().collect::<Vec<_>>(),
        "recent_checksums": t.recent_checksums.iter().map(|(k, c)| json!([k, checksum_text(*c)])).collect::<Vec<_>>(),
        "pending_edits": t.pending_edits,
        "branches": t.branches,
        "epoch": t.epoch,
    })
}

/// The timeline of [`timeline_to_json`]. `None` if a field is missing or malformed.
pub fn timeline_from_json(j: &J) -> Option<Timeline> {
    let u = |k: &str| j.get(k)?.as_u64();
    let recent: Vec<(u64, u64)> = j
        .get("recent_checksums")?
        .as_array()?
        .iter()
        .map(|p| Some((p.get(0)?.as_u64()?, parse_checksum(p.get(1)?)?)))
        .collect::<Option<_>>()?;
    let keyframes: Vec<u64> = j
        .get("keyframes")?
        .as_array()?
        .iter()
        .map(J::as_u64)
        .collect::<Option<_>>()?;
    Some(Timeline {
        mode: match j.get("mode")?.as_str()? {
            "record" => PlayMode::Record,
            "viewer" => PlayMode::Viewer,
            _ => return None,
        },
        tick: u("tick")?,
        verified_tick: u("verified_tick")?,
        first_tick: u("first_tick")?,
        last_tick: u("last_tick")?,
        playing: j.get("playing")?.as_bool()?,
        speed: Speed(u32::try_from(u("speed_permille")?).ok()?),
        checksum: parse_checksum(j.get("checksum")?)?,
        keyframes: Arc::from(keyframes),
        recent_checksums: Arc::from(recent),
        pending_edits: u32::try_from(u("pending_edits")?).ok()?,
        branches: u32::try_from(u("branches")?).ok()?,
        epoch: u("epoch")?,
    })
}

/// The name of a debug error on the wire.
pub fn debug_error_name(e: DebugError) -> &'static str {
    match e {
        DebugError::EntityNotAlive => "entity_not_alive",
        DebugError::UnknownComponent => "unknown_component",
        DebugError::UnknownSingleton => "unknown_singleton",
        DebugError::MissingComponent => "missing_component",
        DebugError::OutOfBounds => "out_of_bounds",
        DebugError::SizeMismatch => "size_mismatch",
        DebugError::DuplicateComponent => "duplicate_component",
        DebugError::ReadOnly => "read_only",
        DebugError::Unsupported => "unsupported",
    }
}

/// The error of [`debug_error_name`]; an unknown name is `Unsupported`.
pub fn debug_error_from_name(name: &str) -> DebugError {
    match name {
        "entity_not_alive" => DebugError::EntityNotAlive,
        "unknown_component" => DebugError::UnknownComponent,
        "unknown_singleton" => DebugError::UnknownSingleton,
        "missing_component" => DebugError::MissingComponent,
        "out_of_bounds" => DebugError::OutOfBounds,
        "size_mismatch" => DebugError::SizeMismatch,
        "duplicate_component" => DebugError::DuplicateComponent,
        "read_only" => DebugError::ReadOnly,
        _ => DebugError::Unsupported,
    }
}

fn entity_json(e: Entity) -> J {
    J::String(handle_text(e))
}

/// A raw debug command as the params of `sim.debug` (numeric ids, hex bytes).
pub fn debug_to_json(cmd: &DebugCommand) -> J {
    match cmd {
        DebugCommand::SetField {
            entity,
            component,
            offset,
            bytes,
        } => json!({
            "cmd": "set_field", "entity": entity_json(*entity), "component": component.0, "offset": offset, "bytes": hex_encode(bytes),
        }),
        DebugCommand::SetSingletonField {
            singleton,
            offset,
            bytes,
        } => json!({
            "cmd": "set_singleton_field", "singleton": singleton.0, "offset": offset, "bytes": hex_encode(bytes),
        }),
        DebugCommand::Spawn { components } => json!({
            "cmd": "spawn",
            "components": components.iter().map(|(c, b)| json!({"component": c.0, "bytes": hex_encode(b)})).collect::<Vec<_>>(),
        }),
        DebugCommand::Despawn { entity } => {
            json!({"cmd": "despawn", "entity": entity_json(*entity)})
        }
        DebugCommand::AddComponent {
            entity,
            component,
            bytes,
        } => json!({
            "cmd": "add_component", "entity": entity_json(*entity), "component": component.0, "bytes": hex_encode(bytes),
        }),
        DebugCommand::RemoveComponent { entity, component } => json!({
            "cmd": "remove_component", "entity": entity_json(*entity), "component": component.0,
        }),
    }
}

fn get_u<T: TryFrom<u64>>(o: &Map<String, J>, key: &str) -> Result<T, String> {
    o.get(key)
        .and_then(J::as_u64)
        .and_then(|n| T::try_from(n).ok())
        .ok_or_else(|| format!("'{key}' must be a non-negative integer in range"))
}

fn get_bytes(o: &Map<String, J>, key: &str) -> Result<Vec<u8>, String> {
    let s = o
        .get(key)
        .and_then(J::as_str)
        .ok_or_else(|| format!("'{key}' must be a hex string"))?;
    hex_decode(s).ok_or_else(|| format!("'{key}' is not valid hex"))
}

fn get_entity(o: &Map<String, J>) -> Result<Entity, String> {
    let s = o
        .get("entity")
        .and_then(J::as_str)
        .ok_or("'entity' must be a handle text like \"12v0\"")?;
    parse_handle(s).ok_or_else(|| format!("'{s}' is not an entity handle like \"12v0\""))
}

/// The command of [`debug_to_json`] (the params of `sim.debug`).
pub fn debug_from_json(o: &Map<String, J>) -> Result<DebugCommand, String> {
    let cmd = o.get("cmd").and_then(J::as_str).ok_or("'cmd' must be set_field, set_singleton_field, spawn, despawn, add_component or remove_component")?;
    Ok(match cmd {
        "set_field" => DebugCommand::SetField {
            entity: get_entity(o)?,
            component: ComponentId(get_u(o, "component")?),
            offset: get_u(o, "offset")?,
            bytes: get_bytes(o, "bytes")?,
        },
        "set_singleton_field" => DebugCommand::SetSingletonField {
            singleton: SingletonId(get_u(o, "singleton")?),
            offset: get_u(o, "offset")?,
            bytes: get_bytes(o, "bytes")?,
        },
        "spawn" => {
            let list = o
                .get("components")
                .and_then(J::as_array)
                .ok_or("'components' must be a list of {component, bytes}")?;
            let mut components = Vec::new();
            for c in list {
                let c = c
                    .as_object()
                    .ok_or("each component must be an object {component, bytes}")?;
                components.push((ComponentId(get_u(c, "component")?), get_bytes(c, "bytes")?));
            }
            DebugCommand::Spawn { components }
        }
        "despawn" => DebugCommand::Despawn {
            entity: get_entity(o)?,
        },
        "add_component" => DebugCommand::AddComponent {
            entity: get_entity(o)?,
            component: ComponentId(get_u(o, "component")?),
            bytes: get_bytes(o, "bytes")?,
        },
        "remove_component" => DebugCommand::RemoveComponent {
            entity: get_entity(o)?,
            component: ComponentId(get_u(o, "component")?),
        },
        other => return Err(format!("unknown debug command '{other}'")),
    })
}

const MAGIC: &[u8; 4] = b"ORRS";
const VERSION: u8 = 1;
/// The most bytes a decoded frame may have (a hostile size prefix must not allocate more).
pub const MAX_FRAME_BYTES: usize = 512 << 20;

/// Builds a frame message from its meta and `Frame::to_bytes()` output.
pub fn encode_frame_message(meta: &J, frame_bytes: &[u8]) -> Vec<u8> {
    let meta = meta.to_string();
    let payload = lz4_flex::compress_prepend_size(frame_bytes);
    let mut out = Vec::with_capacity(12 + meta.len() + payload.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&(meta.len() as u32).to_le_bytes());
    out.extend_from_slice(meta.as_bytes());
    out.extend_from_slice(&payload);
    out
}

/// Splits a frame message into its meta and the decompressed `Frame` bytes.
pub fn decode_frame_message(msg: &[u8]) -> Result<(J, Vec<u8>), String> {
    if msg.len() < 12 || &msg[..4] != MAGIC {
        return Err("not a frame message".into());
    }
    if msg[4] != VERSION {
        return Err(format!("frame message version {} is not supported", msg[4]));
    }
    let meta_len = u32::from_le_bytes([msg[8], msg[9], msg[10], msg[11]]) as usize;
    let rest = &msg[12..];
    if meta_len > rest.len() {
        return Err("frame message is truncated".into());
    }
    let meta: J =
        serde_json::from_slice(&rest[..meta_len]).map_err(|e| format!("bad frame meta: {e}"))?;
    let payload = &rest[meta_len..];
    if payload.len() < 4 {
        return Err("frame message is truncated".into());
    }
    let size = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    if size > MAX_FRAME_BYTES {
        return Err(format!("frame of {size} bytes is over the limit"));
    }
    let frame = lz4_flex::decompress_size_prepended(payload)
        .map_err(|e| format!("bad frame payload: {e}"))?;
    Ok((meta, frame))
}

/// Codec capability version carried inside the negotiated `frame_codec` metadata.
pub const FRAME_CODEC_VERSION: u64 = 1;
/// ORRS envelope version for negotiated Full/Delta frame records.
pub const FRAME_CODEC_ENVELOPE_VERSION: u8 = 2;
/// Hard per-connection ceiling for negotiated ORRS v2 full frames.
pub const MAX_FRAME_CODEC_FRAME_BYTES: usize = 8 << 20;
/// Hard per-connection ceiling for one retained complete-frame baseline.
pub const MAX_FRAME_CODEC_BASELINE_BYTES: usize = 8 << 20;
/// Hard per-message ceiling for a complete negotiated ORRS v2 message.
pub const MAX_FRAME_CODEC_MESSAGE_BYTES: usize = 16 << 20;
/// Hard metadata ceiling for a negotiated ORRS v2 message.
pub const MAX_FRAME_CODEC_METADATA_BYTES: usize = 64 << 10;
/// Hard server-wide retained-baseline budget across negotiated subscribers.
pub const MAX_FRAME_CODEC_TOTAL_BASELINE_BYTES: usize = 64 << 20;

/// Finite per-subscription limits for negotiated frame records.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameCodecLimits {
    pub max_frame_bytes: usize,
    pub max_baseline_bytes: usize,
    pub max_message_bytes: usize,
}

impl FrameCodecLimits {
    /// Rejects zero or out-of-policy values before negotiation is activated.
    pub fn validate(self) -> Result<(), FrameWireError> {
        if self.max_frame_bytes == 0
            || self.max_frame_bytes > MAX_FRAME_CODEC_FRAME_BYTES
            || self.max_baseline_bytes > MAX_FRAME_CODEC_BASELINE_BYTES
            || self.max_message_bytes == 0
            || self.max_message_bytes > MAX_FRAME_CODEC_MESSAGE_BYTES
        {
            return Err(FrameWireError::InvalidLimits);
        }
        Ok(())
    }
}

/// Conservative default accepted policy for negotiated frame streams.
pub const DEFAULT_FRAME_CODEC_LIMITS: FrameCodecLimits = FrameCodecLimits {
    max_frame_bytes: MAX_FRAME_CODEC_FRAME_BYTES,
    max_baseline_bytes: MAX_FRAME_CODEC_BASELINE_BYTES,
    max_message_bytes: MAX_FRAME_CODEC_MESSAGE_BYTES,
};

/// Negotiated stream identity carried on every ORRS v2 frame record.
///
/// Values are encoded as canonical decimal JSON strings to preserve all u64
/// bits in JavaScript clients.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameCodecMeta {
    pub subscription: u64,
    pub sequence: u64,
    pub reset_generation: u64,
}

/// Errors while encoding or bounded-decoding negotiated ORRS v2 frame records.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameWireError {
    InvalidLimits,
    MessageTooLarge { actual: usize, max: usize },
    MetadataTooLarge { actual: usize, max: usize },
    FrameTooLarge { actual: usize, max: usize },
    InvalidEnvelope(&'static str),
    InvalidMetadata(&'static str),
    InvalidRecord(&'static str),
    Compression(String),
}

impl fmt::Display for FrameWireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => f.write_str("invalid negotiated frame codec limits"),
            Self::MessageTooLarge { actual, max } => {
                write!(f, "frame message is {actual} bytes; limit is {max}")
            }
            Self::MetadataTooLarge { actual, max } => {
                write!(f, "frame metadata is {actual} bytes; limit is {max}")
            }
            Self::FrameTooLarge { actual, max } => {
                write!(f, "frame record is {actual} bytes; limit is {max}")
            }
            Self::InvalidEnvelope(reason) => write!(f, "invalid ORRS v2 envelope: {reason}"),
            Self::InvalidMetadata(reason) => write!(f, "invalid frame codec metadata: {reason}"),
            Self::InvalidRecord(reason) => write!(f, "invalid frame record: {reason}"),
            Self::Compression(reason) => write!(f, "invalid compressed frame record: {reason}"),
        }
    }
}

impl std::error::Error for FrameWireError {}

/// Encodes a tagged Full or Delta as a bounded ORRS v2 binary message.
///
/// The JSON metadata is the existing delivery metadata plus a `frame_codec`
/// object containing the capability version, stream identity, record kind,
/// and exact base/target stamps. The payload is one LZ4 size-prefixed block.
pub fn encode_frame_record_message(
    meta: &J,
    identity: FrameCodecMeta,
    record: &FrameRecord,
    limits: FrameCodecLimits,
) -> Result<Vec<u8>, FrameWireError> {
    limits.validate()?;
    let mut meta_object = meta
        .as_object()
        .cloned()
        .ok_or(FrameWireError::InvalidMetadata(
            "delivery metadata must be an object",
        ))?;
    if meta_object.contains_key("frame_codec") {
        return Err(FrameWireError::InvalidMetadata(
            "delivery metadata already contains frame_codec",
        ));
    }

    let (record_kind, target, base) = match record {
        FrameRecord::Full { target, bytes } => {
            if bytes.len() > limits.max_frame_bytes {
                return Err(FrameWireError::FrameTooLarge {
                    actual: bytes.len(),
                    max: limits.max_frame_bytes,
                });
            }
            ("full", *target, None)
        }
        FrameRecord::Delta {
            base,
            target,
            target_len,
            prefix_len,
            suffix_len,
            inserted,
        } => {
            if base.scope != target.scope || target.tick <= base.tick {
                return Err(FrameWireError::InvalidRecord(
                    "delta stamps must share scope and move forward",
                ));
            }
            let target_len = usize::try_from(*target_len).map_err(|_| {
                FrameWireError::InvalidRecord("target length exceeds address space")
            })?;
            if target_len > limits.max_frame_bytes {
                return Err(FrameWireError::FrameTooLarge {
                    actual: target_len,
                    max: limits.max_frame_bytes,
                });
            }
            let normalized_target_len =
                target_len
                    .checked_sub(16)
                    .ok_or(FrameWireError::InvalidRecord(
                        "Delta target is shorter than ORRF v2 header",
                    ))?;
            let prefix_len = usize::try_from(*prefix_len).map_err(|_| {
                FrameWireError::InvalidRecord("prefix length exceeds address space")
            })?;
            let suffix_len = usize::try_from(*suffix_len).map_err(|_| {
                FrameWireError::InvalidRecord("suffix length exceeds address space")
            })?;
            if inserted.len() > limits.max_frame_bytes {
                return Err(FrameWireError::FrameTooLarge {
                    actual: inserted.len(),
                    max: limits.max_frame_bytes,
                });
            }
            let covered_len = prefix_len
                .checked_add(suffix_len)
                .ok_or(FrameWireError::InvalidRecord("splice length overflow"))?;
            let rebuilt_len = covered_len
                .checked_add(inserted.len())
                .ok_or(FrameWireError::InvalidRecord("splice length overflow"))?;
            if covered_len > normalized_target_len || rebuilt_len != normalized_target_len {
                return Err(FrameWireError::InvalidRecord(
                    "Delta splice lengths do not match target length",
                ));
            }
            ("delta", *target, Some(*base))
        }
    };
    validate_record_identity(identity, target, base)?;

    let codec_meta = json!({
        "version": FRAME_CODEC_VERSION,
        "subscription": identity.subscription.to_string(),
        "sequence": identity.sequence.to_string(),
        "reset_generation": identity.reset_generation.to_string(),
        "record": record_kind,
        "base": base.map(stamp_to_json),
        "target": stamp_to_json(target),
    });
    meta_object.insert("frame_codec".to_owned(), codec_meta);
    let meta = J::Object(meta_object).to_string().into_bytes();
    if meta.len() > MAX_FRAME_CODEC_METADATA_BYTES {
        return Err(FrameWireError::MetadataTooLarge {
            actual: meta.len(),
            max: MAX_FRAME_CODEC_METADATA_BYTES,
        });
    }
    let meta_len = u32::try_from(meta.len()).map_err(|_| FrameWireError::MetadataTooLarge {
        actual: meta.len(),
        max: u32::MAX as usize,
    })?;

    // The maximum decoded record is a full frame plus one tag. A Delta also
    // carries three u64 lengths, so its largest legal payload is frame+25.
    let max_record_bytes = limits
        .max_frame_bytes
        .checked_add(25)
        .ok_or(FrameWireError::InvalidLimits)?;
    let mut record_bytes = Vec::new();
    match record {
        FrameRecord::Full { bytes, .. } => {
            let capacity = bytes
                .len()
                .checked_add(1)
                .ok_or(FrameWireError::InvalidRecord("record length overflow"))?;
            if capacity > max_record_bytes {
                return Err(FrameWireError::FrameTooLarge {
                    actual: capacity,
                    max: max_record_bytes,
                });
            }
            record_bytes.reserve(capacity);
            record_bytes.push(0);
            record_bytes.extend_from_slice(bytes);
        }
        FrameRecord::Delta {
            target_len,
            prefix_len,
            suffix_len,
            inserted,
            ..
        } => {
            let capacity = inserted
                .len()
                .checked_add(25)
                .ok_or(FrameWireError::InvalidRecord("record length overflow"))?;
            if capacity > max_record_bytes {
                return Err(FrameWireError::FrameTooLarge {
                    actual: capacity,
                    max: max_record_bytes,
                });
            }
            record_bytes.reserve(capacity);
            record_bytes.push(1);
            record_bytes.extend_from_slice(&target_len.to_le_bytes());
            record_bytes.extend_from_slice(&prefix_len.to_le_bytes());
            record_bytes.extend_from_slice(&suffix_len.to_le_bytes());
            record_bytes.extend_from_slice(inserted);
        }
    }

    let payload = lz4_flex::compress_prepend_size(&record_bytes);
    let message_len = 12usize
        .checked_add(meta.len())
        .and_then(|length| length.checked_add(payload.len()))
        .ok_or(FrameWireError::MessageTooLarge {
            actual: usize::MAX,
            max: limits.max_message_bytes,
        })?;
    if message_len > limits.max_message_bytes {
        return Err(FrameWireError::MessageTooLarge {
            actual: message_len,
            max: limits.max_message_bytes,
        });
    }

    let mut message = Vec::with_capacity(message_len);
    message.extend_from_slice(MAGIC);
    message.push(FRAME_CODEC_ENVELOPE_VERSION);
    message.extend_from_slice(&[0, 0, 0]);
    message.extend_from_slice(&meta_len.to_le_bytes());
    message.extend_from_slice(&meta);
    message.extend_from_slice(&payload);
    Ok(message)
}

/// Bounded-decodes an ORRS v2 tagged Full or Delta without changing codec state.
pub fn decode_frame_record_message(
    message: &[u8],
    limits: FrameCodecLimits,
) -> Result<(J, FrameCodecMeta, FrameRecord), FrameWireError> {
    limits.validate()?;
    if message.len() > limits.max_message_bytes {
        return Err(FrameWireError::MessageTooLarge {
            actual: message.len(),
            max: limits.max_message_bytes,
        });
    }
    if message.len() < 12 || &message[..4] != MAGIC {
        return Err(FrameWireError::InvalidEnvelope("missing ORRS header"));
    }
    if message[4] != FRAME_CODEC_ENVELOPE_VERSION {
        return Err(FrameWireError::InvalidEnvelope("version is not 2"));
    }
    if message[5..8] != [0, 0, 0] {
        return Err(FrameWireError::InvalidEnvelope(
            "reserved bytes must be zero",
        ));
    }
    let meta_len = u32::from_le_bytes(message[8..12].try_into().expect("fixed header")) as usize;
    if meta_len > MAX_FRAME_CODEC_METADATA_BYTES {
        return Err(FrameWireError::MetadataTooLarge {
            actual: meta_len,
            max: MAX_FRAME_CODEC_METADATA_BYTES,
        });
    }
    let payload_start = 12usize
        .checked_add(meta_len)
        .ok_or(FrameWireError::InvalidEnvelope("metadata length overflow"))?;
    if payload_start > message.len() || message.len() - payload_start < 4 {
        return Err(FrameWireError::InvalidEnvelope(
            "truncated metadata or payload",
        ));
    }
    let meta: J = serde_json::from_slice(&message[12..payload_start])
        .map_err(|_| FrameWireError::InvalidMetadata("invalid JSON"))?;
    if !meta.is_object() {
        return Err(FrameWireError::InvalidMetadata(
            "delivery metadata must be an object",
        ));
    }
    let codec_meta =
        meta.get("frame_codec")
            .and_then(J::as_object)
            .ok_or(FrameWireError::InvalidMetadata(
                "missing frame_codec object",
            ))?;
    if codec_meta.get("version").and_then(J::as_u64) != Some(FRAME_CODEC_VERSION) {
        return Err(FrameWireError::InvalidMetadata("unsupported codec version"));
    }
    let identity = FrameCodecMeta {
        subscription: parse_decimal(codec_meta, "subscription")?,
        sequence: parse_decimal(codec_meta, "sequence")?,
        reset_generation: parse_decimal(codec_meta, "reset_generation")?,
    };
    let target = stamp_from_json(
        codec_meta
            .get("target")
            .ok_or(FrameWireError::InvalidMetadata("missing target stamp"))?,
    )?;
    let base = match codec_meta.get("base") {
        Some(J::Null) => None,
        Some(value) => Some(stamp_from_json(value)?),
        None => return Err(FrameWireError::InvalidMetadata("missing base stamp field")),
    };
    let record_kind = codec_meta
        .get("record")
        .and_then(J::as_str)
        .ok_or(FrameWireError::InvalidMetadata("missing record kind"))?;
    validate_record_identity(identity, target, base)?;

    let compressed = &message[payload_start..];
    let declared_len =
        u32::from_le_bytes(compressed[..4].try_into().expect("size prefix")) as usize;
    let max_record_bytes = limits
        .max_frame_bytes
        .checked_add(25)
        .ok_or(FrameWireError::InvalidLimits)?;
    if declared_len > max_record_bytes {
        return Err(FrameWireError::FrameTooLarge {
            actual: declared_len,
            max: max_record_bytes,
        });
    }
    let record_bytes = lz4_flex::decompress_size_prepended(compressed)
        .map_err(|error| FrameWireError::Compression(error.to_string()))?;
    if record_bytes.len() != declared_len {
        return Err(FrameWireError::InvalidRecord(
            "decompressed record length mismatch",
        ));
    }
    let tag = *record_bytes
        .first()
        .ok_or(FrameWireError::InvalidRecord("empty record payload"))?;
    let record = match tag {
        0 => {
            if record_kind != "full" || base.is_some() {
                return Err(FrameWireError::InvalidMetadata("Full kind/base mismatch"));
            }
            let bytes = &record_bytes[1..];
            if bytes.len() > limits.max_frame_bytes {
                return Err(FrameWireError::FrameTooLarge {
                    actual: bytes.len(),
                    max: limits.max_frame_bytes,
                });
            }
            FrameRecord::Full {
                target,
                bytes: Arc::from(bytes),
            }
        }
        1 => {
            if record_kind != "delta" {
                return Err(FrameWireError::InvalidMetadata("Delta kind mismatch"));
            }
            let base = base.ok_or(FrameWireError::InvalidMetadata(
                "Delta is missing base stamp",
            ))?;
            if record_bytes.len() < 25 {
                return Err(FrameWireError::InvalidRecord("truncated Delta header"));
            }
            let target_len = u64::from_le_bytes(record_bytes[1..9].try_into().expect("fixed u64"));
            let prefix_len = u64::from_le_bytes(record_bytes[9..17].try_into().expect("fixed u64"));
            let suffix_len =
                u64::from_le_bytes(record_bytes[17..25].try_into().expect("fixed u64"));
            let target_len_usize = usize::try_from(target_len).map_err(|_| {
                FrameWireError::InvalidRecord("target length exceeds address space")
            })?;
            if target_len_usize > limits.max_frame_bytes {
                return Err(FrameWireError::FrameTooLarge {
                    actual: target_len_usize,
                    max: limits.max_frame_bytes,
                });
            }
            let inserted = &record_bytes[25..];
            if inserted.len() > limits.max_frame_bytes {
                return Err(FrameWireError::FrameTooLarge {
                    actual: inserted.len(),
                    max: limits.max_frame_bytes,
                });
            }
            FrameRecord::Delta {
                base,
                target,
                target_len,
                prefix_len,
                suffix_len,
                inserted: inserted.to_vec(),
            }
        }
        _ => return Err(FrameWireError::InvalidRecord("unknown record tag")),
    };
    Ok((meta, identity, record))
}

fn stamp_to_json(stamp: FrameStamp) -> J {
    json!({
        "scope": {
            "stream_generation": stamp.scope.stream_generation.to_string(),
            "play_epoch": stamp.scope.play_epoch.to_string(),
            "timeline_epoch": stamp.scope.timeline_epoch.to_string(),
        },
        "tick": stamp.tick.to_string(),
        "frame_checksum": stamp.frame_checksum.to_string(),
    })
}

fn validate_record_identity(
    identity: FrameCodecMeta,
    target: FrameStamp,
    base: Option<FrameStamp>,
) -> Result<(), FrameWireError> {
    if identity.subscription == 0 || identity.sequence == 0 || identity.reset_generation == 0 {
        return Err(FrameWireError::InvalidMetadata(
            "wire records require positive subscription, sequence and reset generation",
        ));
    }
    if target.scope.stream_generation != identity.subscription
        || target.scope.timeline_epoch != identity.reset_generation
    {
        return Err(FrameWireError::InvalidMetadata(
            "target scope does not match subscription/reset generation",
        ));
    }
    if let Some(base) = base {
        if base.scope != target.scope {
            return Err(FrameWireError::InvalidMetadata(
                "Delta base and target scopes differ",
            ));
        }
        if base.scope.stream_generation != identity.subscription
            || base.scope.timeline_epoch != identity.reset_generation
        {
            return Err(FrameWireError::InvalidMetadata(
                "Delta base scope does not match subscription/reset generation",
            ));
        }
    }
    Ok(())
}

fn stamp_from_json(value: &J) -> Result<FrameStamp, FrameWireError> {
    let object = value
        .as_object()
        .ok_or(FrameWireError::InvalidMetadata("stamp must be an object"))?;
    let scope =
        object
            .get("scope")
            .and_then(J::as_object)
            .ok_or(FrameWireError::InvalidMetadata(
                "stamp scope must be an object",
            ))?;
    Ok(FrameStamp {
        scope: FrameScope {
            stream_generation: parse_decimal(scope, "stream_generation")?,
            play_epoch: parse_decimal(scope, "play_epoch")?,
            timeline_epoch: parse_decimal(scope, "timeline_epoch")?,
        },
        tick: parse_decimal(object, "tick")?,
        frame_checksum: parse_decimal(object, "frame_checksum")?,
    })
}

fn parse_decimal(object: &Map<String, J>, key: &'static str) -> Result<u64, FrameWireError> {
    let text = object
        .get(key)
        .and_then(J::as_str)
        .ok_or(FrameWireError::InvalidMetadata(
            "u64 identity must be a decimal string",
        ))?;
    let value = text
        .parse::<u64>()
        .map_err(|_| FrameWireError::InvalidMetadata("u64 decimal string is out of range"))?;
    if value.to_string() != text {
        return Err(FrameWireError::InvalidMetadata(
            "u64 decimal string is not in canonical form",
        ));
    }
    let _ = key;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_message_roundtrip() {
        let meta = json!({"tick": 5});
        let bytes: Vec<u8> = (0..5000u32).map(|i| (i % 7) as u8).collect();
        let msg = encode_frame_message(&meta, &bytes);
        let (m, b) = decode_frame_message(&msg).unwrap();
        assert_eq!(m, meta);
        assert_eq!(b, bytes);
        assert!(decode_frame_message(&msg[..10]).is_err());
        assert!(decode_frame_message(b"ORRS\x01\0\0\0\xff\xff\xff\xffxxxx").is_err());
    }

    #[test]
    fn debug_roundtrip() {
        let cmds = [
            DebugCommand::SetField {
                entity: Entity {
                    index: 3,
                    version: 1,
                },
                component: ComponentId(2),
                offset: 8,
                bytes: vec![1, 2, 3],
            },
            DebugCommand::SetSingletonField {
                singleton: SingletonId(1),
                offset: 0,
                bytes: vec![9],
            },
            DebugCommand::Spawn {
                components: vec![(ComponentId(0), vec![0; 4]), (ComponentId(1), vec![7])],
            },
            DebugCommand::Despawn {
                entity: Entity {
                    index: 3,
                    version: 1,
                },
            },
            DebugCommand::AddComponent {
                entity: Entity {
                    index: 3,
                    version: 1,
                },
                component: ComponentId(2),
                bytes: vec![5; 3],
            },
            DebugCommand::RemoveComponent {
                entity: Entity {
                    index: 3,
                    version: 1,
                },
                component: ComponentId(2),
            },
        ];
        for c in cmds {
            let j = debug_to_json(&c);
            assert_eq!(debug_from_json(j.as_object().unwrap()).unwrap(), c);
        }
        assert!(debug_from_json(json!({"cmd": "fly"}).as_object().unwrap()).is_err());
        assert!(debug_from_json(
            json!({"cmd": "despawn", "entity": "x"})
                .as_object()
                .unwrap()
        )
        .is_err());
    }
}
