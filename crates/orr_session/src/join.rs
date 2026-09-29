//! Late-join wire messages: how a peer that arrives in a running session
//! gets a confirmed snapshot to catch up from.
//!
//! Three messages, all plain bytes the caller moves over any transport
//! (all integers little-endian, lengths `u32`, never `usize`):
//!
//! - [`join_request`]: joiner -> host. `"ORRQ"`, `version u32`,
//!   `build_hash u64`, `seed u64`, `tick_rate u32`, `player_count u8`,
//!   `slot u8`, `joiner_id u64`, `attempt u32`, `backlog_peers u32`,
//!   `message_checksum u64`. `joiner_id` is a nonce the joiner picks once;
//!   `attempt` counts its requests (1, 2, ...). A `joiner_id` of `0` is an
//!   anonymous one-shot request that can never be retried.
//! - [`Session::serve_join`](crate::Session::serve_join) answers with a
//!   snapshot message: `"ORRJ"`, `version u32`, `build_hash u64`,
//!   `seed u64`, `tick_rate u32`, `player_count u8`, `slot u8`,
//!   `snapshot_tick u64`, `first_input_tick u64`, `checksum u64`, then
//!   `len u32`, an lz4 block (size-prefixed) of `Frame::to_bytes`,
//!   `attempt u32` (echo of the request's) and `message_checksum u64`.
//! - [`Session::backlog_notice`](crate::Session::backlog_notice): existing
//!   peer -> joiner, sent next to the peer's `authored_since` backlog.
//!   `"ORRB"`, `version u32`, `attempt u32`, `sender u8` (the peer's own
//!   slot), `span_count u32`, then per span `slot u8`, `from u64`,
//!   `until u64` (`u64::MAX` = still authoring), `message_checksum u64`.
//!   A span says: "the backlog holds my authored input for `slot` at every
//!   tick in `from..until`". The joiner unions the spans of all peers and
//!   can prove a hole (`JoinError::InputGap`) without any timer.
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
const BACKLOG_MAGIC: &[u8; 4] = b"ORRB";
const VERSION: u32 = 2;
/// Bytes of one encoded [`Span`].
const SPAN_LEN: u64 = 17;

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
    /// The peers' backlogs do not reach back to the tick the joiner needs,
    /// so the hole can never be filled. `needed_from` is the first tick with
    /// no input for `slot`; `available_from` is where the next offered input
    /// starts (`u64::MAX` when no peer offers any). Join again.
    InputGap { slot: PlayerSlot, needed_from: u64, available_from: u64 },
    /// A message belongs to an older or already used join attempt.
    StaleAttempt { current: u32, got: u32 },
    /// The joiner used up `SessionConfig::max_join_attempts`.
    TooManyAttempts { max: u32 },
    /// `Session::mark_slot_vacant` was refused (the text says why).
    InvalidVacate(&'static str),
}

impl core::fmt::Display for JoinError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            JoinError::Truncated => write!(f, "join message truncated"),
            JoinError::BadMagic => write!(f, "not an ORRQ/ORRJ/ORRB join message"),
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
            JoinError::InputGap { slot, needed_from, available_from } => write!(
                f,
                "input gap for slot {}: needed from tick {needed_from}, peers offer from tick {available_from}",
                slot.0
            ),
            JoinError::StaleAttempt { current, got } => {
                write!(f, "stale join attempt {got} (current attempt is {current})")
            }
            JoinError::TooManyAttempts { max } => write!(f, "join failed after {max} attempts"),
            JoinError::InvalidVacate(what) => write!(f, "cannot mark slot vacant: {what}"),
        }
    }
}
impl std::error::Error for JoinError {}

impl From<BuildHashMismatch> for JoinError {
    fn from(e: BuildHashMismatch) -> Self {
        JoinError::BuildHashMismatch(e)
    }
}

/// What every existing peer must know about a join in progress: which
/// inputs to keep (`snapshot_tick` on) and when the join counts as
/// confirmed (an input of `slot` at `first_input_tick` or later arrives).
/// The host returns it from `Session::pending_join`; the caller passes it
/// to the other peers' `Session::hold_inputs_for_join`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JoinTicket {
    pub slot: PlayerSlot,
    pub attempt: u32,
    pub snapshot_tick: u64,
    pub first_input_tick: u64,
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
    /// The echoed attempt, or why the message tail is unusable.
    pub attempt: Result<u32, JoinError>,
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
/// `cfg`, whose `local_slot` is the slot it wants) and its build hash. The
/// retry fields `join_id`, `join_attempt` and `join_backlog_peers` of `cfg`
/// go into the request too; [`JoinAttempts`] fills them in.
pub fn join_request(cfg: &SessionConfig) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(REQUEST_MAGIC);
    JoinHeader::of(cfg, orr_sim::build_hash_of(cfg.build_id, 0)).write(&mut out);
    out.extend_from_slice(&cfg.join_id.to_le_bytes());
    out.extend_from_slice(&cfg.join_attempt.to_le_bytes());
    out.extend_from_slice(&cfg.join_backlog_peers.to_le_bytes());
    seal(&mut out);
    out
}

/// A decoded join request.
pub(crate) struct JoinRequest {
    pub header: JoinHeader,
    pub joiner_id: u64,
    pub attempt: u32,
    pub backlog_peers: u32,
}

pub(crate) fn decode_request(bytes: &[u8]) -> Result<JoinRequest, JoinError> {
    let mut r = Reader { bytes: unseal(bytes)? };
    if r.take(4)? != REQUEST_MAGIC {
        return Err(JoinError::BadMagic);
    }
    let header = JoinHeader::read(&mut r)?;
    let joiner_id = r.u64()?;
    let attempt = r.u32()?;
    let backlog_peers = r.u32()?;
    if !r.bytes.is_empty() {
        return Err(JoinError::Corrupt("trailing bytes"));
    }
    Ok(JoinRequest { header, joiner_id, attempt, backlog_peers })
}

/// The joiner's side of the retry rules: numbers its requests and stops
/// after `SessionConfig::max_join_attempts`. Keep one per join and call
/// [`next_request`](Self::next_request) again after each failed attempt.
/// [`config`](Self::config) carries the current attempt number, which
/// `Session::from_join_snapshot` uses to drop a snapshot of an older attempt.
pub struct JoinAttempts {
    cfg: SessionConfig,
    made: u32,
}

impl JoinAttempts {
    /// `cfg.join_id` must be a nonzero nonce and `cfg.join_backlog_peers`
    /// the number of existing peers that will send a backlog notice, or the
    /// host could not accept a retry safely.
    pub fn new(cfg: SessionConfig) -> Result<Self, JoinError> {
        if cfg.join_id == 0 {
            return Err(JoinError::ConfigMismatch("join id"));
        }
        if cfg.join_backlog_peers == 0 {
            return Err(JoinError::ConfigMismatch("join backlog peers"));
        }
        Ok(Self { cfg, made: 0 })
    }

    /// The next request, or `TooManyAttempts` once the limit is used up.
    pub fn next_request(&mut self) -> Result<Vec<u8>, JoinError> {
        if self.made >= self.cfg.max_join_attempts {
            return Err(JoinError::TooManyAttempts { max: self.cfg.max_join_attempts });
        }
        self.made += 1;
        self.cfg.join_attempt = self.made;
        Ok(join_request(&self.cfg))
    }

    pub fn attempts_made(&self) -> u32 {
        self.made
    }

    /// The config for the current attempt's `Session::from_join_snapshot`.
    pub fn config(&self) -> &SessionConfig {
        &self.cfg
    }
}

/// One range of ticks a peer holds authored input for (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub slot: PlayerSlot,
    pub from: u64,
    pub until: u64,
}

/// A decoded backlog notice.
pub(crate) struct Backlog {
    pub attempt: u32,
    pub sender: PlayerSlot,
    pub spans: Vec<Span>,
}

pub(crate) fn encode_backlog(attempt: u32, sender: PlayerSlot, spans: &[Span]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + spans.len() * SPAN_LEN as usize);
    out.extend_from_slice(BACKLOG_MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&attempt.to_le_bytes());
    out.push(sender.0);
    out.extend_from_slice(&(spans.len() as u32).to_le_bytes());
    for s in spans {
        out.push(s.slot.0);
        out.extend_from_slice(&s.from.to_le_bytes());
        out.extend_from_slice(&s.until.to_le_bytes());
    }
    seal(&mut out);
    out
}

pub(crate) fn decode_backlog(bytes: &[u8]) -> Result<Backlog, JoinError> {
    let mut r = Reader { bytes: unseal(bytes)? };
    if r.take(4)? != BACKLOG_MAGIC {
        return Err(JoinError::BadMagic);
    }
    let version = r.u32()?;
    if version != VERSION {
        return Err(JoinError::UnsupportedVersion(version));
    }
    let attempt = r.u32()?;
    let sender = PlayerSlot(r.u8()?);
    let count = r.u32()?;
    // Checked against the bytes present before anything is allocated.
    if u64::from(count) * SPAN_LEN != r.bytes.len() as u64 {
        return Err(JoinError::Corrupt("span count does not match length"));
    }
    let mut spans = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let slot = PlayerSlot(r.u8()?);
        let (from, until) = (r.u64()?, r.u64()?);
        if until <= from {
            return Err(JoinError::Corrupt("empty span"));
        }
        spans.push(Span { slot, from, until });
    }
    Ok(Backlog { attempt, sender, spans })
}

/// The first tick in `need_from..need_until` that no `(from, until)` range
/// covers, with the start of the next range after it (`u64::MAX` if none).
/// `None` means the ranges cover everything needed.
pub(crate) fn first_gap(ranges: &mut [(u64, u64)], need_from: u64, need_until: u64) -> Option<(u64, u64)> {
    ranges.sort_unstable();
    let mut cursor = need_from;
    for &(from, until) in ranges.iter() {
        if cursor >= need_until {
            return None;
        }
        if from > cursor {
            return Some((cursor, from));
        }
        cursor = cursor.max(until);
    }
    (cursor < need_until).then_some((cursor, u64::MAX))
}

pub(crate) fn encode_snapshot(
    header: &JoinHeader,
    snapshot_tick: u64,
    first_input_tick: u64,
    checksum: u64,
    attempt: u32,
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
    out.extend_from_slice(&attempt.to_le_bytes());
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
    if first_input_tick <= snapshot_tick {
        return Err(JoinError::Corrupt("first input tick not after snapshot tick"));
    }
    let frame_bytes = crate::wire::decompress_bounded(compressed).map_err(JoinError::Decompress)?;
    // The tail is judged by the caller after the frame itself, so damage to
    // the frame is reported before a short tail.
    let attempt = match r.u32() {
        Ok(_) if !r.bytes.is_empty() => Err(JoinError::Corrupt("trailing bytes")),
        other => other,
    };
    Ok(JoinSnapshot { header, snapshot_tick, first_input_tick, checksum, attempt, frame_bytes })
}

#[cfg(test)]
mod tests {
    use super::first_gap;

    #[test]
    fn gap_search_unions_ranges() {
        // Covered by two touching ranges, in any order.
        assert_eq!(first_gap(&mut [(10, 20), (3, 10)], 5, 15), None);
        assert_eq!(first_gap(&mut [(3, u64::MAX)], 5, u64::MAX), None);
        // Nothing offered, hole before the first range, hole between ranges.
        assert_eq!(first_gap(&mut [], 5, 8), Some((5, u64::MAX)));
        assert_eq!(first_gap(&mut [(9, 20)], 5, 15), Some((5, 9)));
        assert_eq!(first_gap(&mut [(3, 7), (9, 20)], 5, 15), Some((7, 9)));
        // A range that ends early leaves the tail open.
        assert_eq!(first_gap(&mut [(3, 12)], 5, u64::MAX), Some((12, u64::MAX)));
        // An empty need is always covered.
        assert_eq!(first_gap(&mut [], 5, 5), None);
    }
}
