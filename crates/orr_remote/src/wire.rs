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
//! payload = lz4 block with a u32 LE size prefix of Frame::to_bytes() (ORRF v1, checksummed)
//! meta = { tick, epoch, tick_rate, player_count, sent_at_us, timeline: {...} }
//! ```
//!
//! `sent_at_us` is the sender's wall clock (microseconds since the Unix
//! epoch), meaningful for latency only when both sides share a clock (the same machine).

use std::sync::Arc;

use orr_ecs::{ComponentId, Entity, SingletonId};
use orr_session::{PlayMode, Speed, Timeline};
use orr_sim::{DebugCommand, DebugError};
use serde_json::{json, Map, Value as J};

use crate::codec::{hex_decode, hex_encode};
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
    let keyframes: Vec<u64> = j.get("keyframes")?.as_array()?.iter().map(J::as_u64).collect::<Option<_>>()?;
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
        DebugCommand::SetField { entity, component, offset, bytes } => json!({
            "cmd": "set_field", "entity": entity_json(*entity), "component": component.0, "offset": offset, "bytes": hex_encode(bytes),
        }),
        DebugCommand::SetSingletonField { singleton, offset, bytes } => json!({
            "cmd": "set_singleton_field", "singleton": singleton.0, "offset": offset, "bytes": hex_encode(bytes),
        }),
        DebugCommand::Spawn { components } => json!({
            "cmd": "spawn",
            "components": components.iter().map(|(c, b)| json!({"component": c.0, "bytes": hex_encode(b)})).collect::<Vec<_>>(),
        }),
        DebugCommand::Despawn { entity } => json!({"cmd": "despawn", "entity": entity_json(*entity)}),
        DebugCommand::AddComponent { entity, component, bytes } => json!({
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
    let s = o.get(key).and_then(J::as_str).ok_or_else(|| format!("'{key}' must be a hex string"))?;
    hex_decode(s).ok_or_else(|| format!("'{key}' is not valid hex"))
}

fn get_entity(o: &Map<String, J>) -> Result<Entity, String> {
    let s = o.get("entity").and_then(J::as_str).ok_or("'entity' must be a handle text like \"12v0\"")?;
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
            let list = o.get("components").and_then(J::as_array).ok_or("'components' must be a list of {component, bytes}")?;
            let mut components = Vec::new();
            for c in list {
                let c = c.as_object().ok_or("each component must be an object {component, bytes}")?;
                components.push((ComponentId(get_u(c, "component")?), get_bytes(c, "bytes")?));
            }
            DebugCommand::Spawn { components }
        }
        "despawn" => DebugCommand::Despawn { entity: get_entity(o)? },
        "add_component" => DebugCommand::AddComponent {
            entity: get_entity(o)?,
            component: ComponentId(get_u(o, "component")?),
            bytes: get_bytes(o, "bytes")?,
        },
        "remove_component" => DebugCommand::RemoveComponent { entity: get_entity(o)?, component: ComponentId(get_u(o, "component")?) },
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
    let meta: J = serde_json::from_slice(&rest[..meta_len]).map_err(|e| format!("bad frame meta: {e}"))?;
    let payload = &rest[meta_len..];
    if payload.len() < 4 {
        return Err("frame message is truncated".into());
    }
    let size = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    if size > MAX_FRAME_BYTES {
        return Err(format!("frame of {size} bytes is over the limit"));
    }
    let frame = lz4_flex::decompress_size_prepended(payload).map_err(|e| format!("bad frame payload: {e}"))?;
    Ok((meta, frame))
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
            DebugCommand::SetField { entity: Entity { index: 3, version: 1 }, component: ComponentId(2), offset: 8, bytes: vec![1, 2, 3] },
            DebugCommand::SetSingletonField { singleton: SingletonId(1), offset: 0, bytes: vec![9] },
            DebugCommand::Spawn { components: vec![(ComponentId(0), vec![0; 4]), (ComponentId(1), vec![7])] },
            DebugCommand::Despawn { entity: Entity { index: 3, version: 1 } },
            DebugCommand::AddComponent { entity: Entity { index: 3, version: 1 }, component: ComponentId(2), bytes: vec![5; 3] },
            DebugCommand::RemoveComponent { entity: Entity { index: 3, version: 1 }, component: ComponentId(2) },
        ];
        for c in cmds {
            let j = debug_to_json(&c);
            assert_eq!(debug_from_json(j.as_object().unwrap()).unwrap(), c);
        }
        assert!(debug_from_json(json!({"cmd": "fly"}).as_object().unwrap()).is_err());
        assert!(debug_from_json(json!({"cmd": "despawn", "entity": "x"}).as_object().unwrap()).is_err());
    }
}
