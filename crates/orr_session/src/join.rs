//! Late-join wire messages: how a peer that arrives in a running session
//! gets a confirmed snapshot to catch up from.
//!
//! Two messages, both plain bytes the caller moves over any transport
//! (all integers little-endian, lengths `u32`, never `usize`):
//!
//! - [`join_request`]: joiner -> host. `"ORRQ"`, `version u32`,
//!   `build_hash u64`, `seed u64`, `tick_rate u32`, `player_count u8`,
//!   `slot u8`, `message_checksum u64`.
//! - [`Session::serve_join`](crate::Session::serve_join) answers with a
//!   snapshot message: `"ORRJ"`, `version u32`, `build_hash u64`,
//!   `seed u64`, `tick_rate u32`, `player_count u8`, `slot u8`,
//!   `snapshot_tick u64`, `first_input_tick u64`, `checksum u64`, then
//!   `len u32`, an lz4 block (size-prefixed) of `Frame::to_bytes`, and
//!   `message_checksum u64`.
//!
//! `message_checksum` is xxh3 over every earlier byte of the message. It
//! catches transport corruption of any field, including the header. The
//! snapshot has its own `checksum` (of the `Frame`, as `Frame::checksum`),
//! which the joiner compares against the rebuilt frame.
//!
//! `snapshot_tick` is always a *verified* (fully confirmed) tick of the
//! host. `first_input_tick` is the first tick whose input for the joiner's
//! slot the joiner itself supplies; every earlier tick of that slot was
//! authored by the host as default input.
use orr_ecs::FrameDecodeError;
use orr_sim::PlayerSlot;

use crate::session::{BuildHashMismatch, SessionConfig};

const REQUEST_MAGIC: &[u8; 4] = b"ORRQ";
const SNAPSHOT_MAGIC: &[u8; 4] = b"ORRJ";
const VERSION: u32 = 1;

/// Why a late-join message or exchange was rejected.
#[derive(Debug)]
pub enum JoinError {
    /// The message ended early.
    Truncated,
    BadMagic,
    UnsupportedVersion(u32),
    /// The two peers run different builds or patch generations.
    BuildHashMismatch(BuildHashMismatch),
    /// The peers disagree on a session setting (`what` names it).
    ConfigMismatch(&'static str),
    /// The requested slot is not one this peer fills with default input, or
    /// it was already handed to another joiner.
    SlotNotVacant(PlayerSlot),
    /// The host has no stored frame for its verified tick.
    NoVerifiedFrame,
    Decompress(String),
    BadSnapshot(FrameDecodeError),
    /// The snapshot's tick, or its checksum, differs from what the message
    /// header promised.
    SnapshotMismatch { expected: u64, actual: u64 },
    /// The message's own trailing checksum does not match its bytes.
    BadMessageChecksum,
    /// The message header is self-contradictory.
    Corrupt(&'static str),
}

impl core::fmt::Display for JoinError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            JoinError::Truncated => write!(f, "join message truncated"),
            JoinError::BadMagic => write!(f, "not an ORRQ/ORRJ join message"),
            JoinError::UnsupportedVersion(v) => write!(f, "unsupported join message version {v}"),
            JoinError::BuildHashMismatch(e) => write!(f, "{e}"),
            JoinError::ConfigMismatch(what) => write!(f, "join rejected: {what} differs between peers"),
            JoinError::SlotNotVacant(s) => write!(f, "slot {} is not open for joining", s.0),
            JoinError::NoVerifiedFrame => write!(f, "host has no stored frame for its verified tick"),
            JoinError::Decompress(e) => write!(f, "snapshot decompression failed: {e}"),
            JoinError::BadSnapshot(e) => write!(f, "bad snapshot: {e}"),
            JoinError::SnapshotMismatch { expected, actual } => {
                write!(f, "snapshot mismatch: header says {expected:#x}, snapshot has {actual:#x}")
            }
            JoinError::BadMessageChecksum => write!(f, "join message checksum mismatch"),
            JoinError::Corrupt(what) => write!(f, "corrupt join message: {what}"),
        }
    }
}
impl std::error::Error for JoinError {}

impl From<BuildHashMismatch> for JoinError {
    fn from(e: BuildHashMismatch) -> Self {
        JoinError::BuildHashMismatch(e)
    }
}

/// Settings both sides must agree on, sent in both messages.
pub(crate) struct JoinHeader {
    pub build_hash: u64,
    pub seed: u64,
    pub tick_rate: u32,
    pub player_count: u8,
    pub slot: PlayerSlot,
}

impl JoinHeader {
    fn of(cfg: &SessionConfig, build_hash: u64) -> Self {
        Self {
            build_hash,
            seed: cfg.seed,
            tick_rate: cfg.tick_rate,
            player_count: cfg.player_count,
            slot: cfg.local_slot,
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.build_hash.to_le_bytes());
        out.extend_from_slice(&self.seed.to_le_bytes());
        out.extend_from_slice(&self.tick_rate.to_le_bytes());
        out.push(self.player_count);
        out.push(self.slot.0);
    }

    fn read(r: &mut Reader) -> Result<Self, JoinError> {
        let version = r.u32()?;
        if version != VERSION {
            return Err(JoinError::UnsupportedVersion(version));
        }
        Ok(Self {
            build_hash: r.u64()?,
            seed: r.u64()?,
            tick_rate: r.u32()?,
            player_count: r.u8()?,
            slot: PlayerSlot(r.u8()?),
        })
    }

    /// Checks the settings a joiner and host must share. `slot` is checked
    /// separately since the two sides read it differently.
    pub(crate) fn check_against(&self, cfg: &SessionConfig, local_build_hash: u64) -> Result<(), JoinError> {
        crate::session::require_same_build_hash(local_build_hash, self.build_hash)?;
        if self.seed != cfg.seed {
            return Err(JoinError::ConfigMismatch("seed"));
        }
        if self.tick_rate != cfg.tick_rate {
            return Err(JoinError::ConfigMismatch("tick rate"));
        }
        if self.player_count != cfg.player_count {
            return Err(JoinError::ConfigMismatch("player count"));
        }
        Ok(())
    }
}

/// A decoded snapshot message.
pub(crate) struct JoinSnapshot {
    pub header: JoinHeader,
    pub snapshot_tick: u64,
    pub first_input_tick: u64,
    pub checksum: u64,
    /// `Frame::to_bytes` output, already decompressed.
    pub frame_bytes: Vec<u8>,
}

struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], JoinError> {
        if n > self.bytes.len() {
            return Err(JoinError::Truncated);
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, JoinError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, JoinError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, JoinError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

/// Appends the xxh3 of everything written so far.
fn seal(out: &mut Vec<u8>) {
    let sum = xxhash_rust::xxh3::xxh3_64(out);
    out.extend_from_slice(&sum.to_le_bytes());
}

/// Splits off and verifies the trailing checksum, returning the body.
fn unseal(bytes: &[u8]) -> Result<&[u8], JoinError> {
    let Some(split) = bytes.len().checked_sub(8) else { return Err(JoinError::Truncated) };
    let (body, tail) = bytes.split_at(split);
    if u64::from_le_bytes(tail.try_into().unwrap()) != xxhash_rust::xxh3::xxh3_64(body) {
        return Err(JoinError::BadMessageChecksum);
    }
    Ok(body)
}

/// Builds the request a joining peer sends to the host: its settings (from
/// `cfg`, whose `local_slot` is the slot it wants) and its build hash.
pub fn join_request(cfg: &SessionConfig) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(REQUEST_MAGIC);
    JoinHeader::of(cfg, orr_sim::build_hash_of(cfg.build_id, 0)).write(&mut out);
    seal(&mut out);
    out
}

pub(crate) fn decode_request(bytes: &[u8]) -> Result<JoinHeader, JoinError> {
    let mut r = Reader { bytes: unseal(bytes)? };
    if r.take(4)? != REQUEST_MAGIC {
        return Err(JoinError::BadMagic);
    }
    let header = JoinHeader::read(&mut r)?;
    if !r.bytes.is_empty() {
        return Err(JoinError::Corrupt("trailing bytes"));
    }
    Ok(header)
}

pub(crate) fn encode_snapshot(
    header: &JoinHeader,
    snapshot_tick: u64,
    first_input_tick: u64,
    checksum: u64,
    frame_bytes: &[u8],
) -> Vec<u8> {
    let compressed = lz4_flex::block::compress_prepend_size(frame_bytes);
    let mut out = Vec::with_capacity(compressed.len() + 64);
    out.extend_from_slice(SNAPSHOT_MAGIC);
    header.write(&mut out);
    out.extend_from_slice(&snapshot_tick.to_le_bytes());
    out.extend_from_slice(&first_input_tick.to_le_bytes());
    out.extend_from_slice(&checksum.to_le_bytes());
    out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    out.extend_from_slice(&compressed);
    seal(&mut out);
    out
}

pub(crate) fn decode_snapshot(bytes: &[u8]) -> Result<JoinSnapshot, JoinError> {
    let mut r = Reader { bytes: unseal(bytes)? };
    if r.take(4)? != SNAPSHOT_MAGIC {
        return Err(JoinError::BadMagic);
    }
    let header = JoinHeader::read(&mut r)?;
    let snapshot_tick = r.u64()?;
    let first_input_tick = r.u64()?;
    let checksum = r.u64()?;
    let len = r.u32()? as usize;
    let compressed = r.take(len)?;
    if !r.bytes.is_empty() {
        return Err(JoinError::Corrupt("trailing bytes"));
    }
    if first_input_tick <= snapshot_tick {
        return Err(JoinError::Corrupt("first input tick not after snapshot tick"));
    }
    let frame_bytes = crate::wire::decompress_bounded(compressed).map_err(JoinError::Decompress)?;
    Ok(JoinSnapshot { header, snapshot_tick, first_input_tick, checksum, frame_bytes })
}
