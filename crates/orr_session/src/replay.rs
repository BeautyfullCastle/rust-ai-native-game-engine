//! `.orrp` replay file format: a header, a per-tick input/command stream
//! (delta-compressed against the previous tick's inputs, then whole-body
//! lz4 compressed), a checksum table, and (v2) optional keyframes.
//!
//! A keyframe is a full `orr_ecs::Frame` byte snapshot (`Frame::to_bytes`)
//! taken every N ticks while recording. [`ReplayReader::seek`] restores the
//! nearest keyframe at or before the target tick and resimulates forward
//! from there, instead of from tick 0. A file without keyframes still seeks
//! (from tick 0).
//!
//! Versions: v1 has no keyframe table. v2 appends `count u32` and, per
//! keyframe, `tick u64, len u32, bytes` after the checksum table inside the
//! compressed body. v3 appends, after the keyframes, `count u32` and, per
//! debug command, `tick u64, len u32, bytes` (`DebugCommand::encode`). A
//! debug command of tick `t` is applied at the boundary before tick `t`
//! runs, in file order. The reader accepts all three; the writer always
//! writes v3.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use orr_ecs::{Frame, FrameDecodeError};
use orr_sim::{DebugCommand, Game, PlayerSlot, SimCommand, Simulation};

const MAGIC: &[u8; 4] = b"ORRP";
const FORMAT_VERSION: u32 = 3;
const MIN_FORMAT_VERSION: u32 = 1;

/// `.orrp` file header.
#[derive(Clone, Debug)]
pub struct ReplayHeader {
    pub format_version: u32,
    pub game_id: String,
    pub build_hash: u64,
    pub seed: u64,
    pub player_count: u8,
    pub tick_rate: u32,
    pub input_size: u32,
}

/// Errors reading back a `.orrp` file.
#[derive(Debug)]
pub enum ReplayError {
    TooShort,
    BadMagic,
    UnsupportedVersion(u32),
    Decompress(String),
    Truncated,
    BadCommand,
    /// The header's player count exceeds the 32 slots the per-tick change
    /// mask can describe.
    BadPlayerCount(u8),
    /// [`replay_verify_checked`] was asked to verify against a build id
    /// whose resulting `Simulation::build_hash()` disagrees with the
    /// header's recorded `build_hash` (see the design doc's hot-patch
    /// decision: a replay recorded under one build/patch generation is not
    /// safe to resimulate-and-compare under another).
    BuildHashMismatch { header: u64, expected: u64 },
    /// A keyframe snapshot did not decode against this game's registry, or
    /// its frame tick disagrees with the tick it was recorded under.
    BadKeyframe { tick: u64, source: FrameDecodeError },
    /// [`ReplayReader::seek`] target is past the last recorded tick.
    TickOutOfRange { tick: u64, last: u64 },
}

impl core::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReplayError::TooShort => write!(f, "replay file too short"),
            ReplayError::BadMagic => write!(f, "not an ORRP replay file"),
            ReplayError::UnsupportedVersion(v) => write!(f, "unsupported replay format version {v}"),
            ReplayError::Decompress(e) => write!(f, "decompression failed: {e}"),
            ReplayError::Truncated => write!(f, "replay body truncated/corrupt"),
            ReplayError::BadCommand => write!(f, "malformed command in replay body"),
            ReplayError::BadPlayerCount(n) => write!(f, "replay player count {n} exceeds 32"),
            ReplayError::BuildHashMismatch { header, expected } => {
                write!(f, "replay build hash {header:#x} does not match expected build hash {expected:#x}")
            }
            ReplayError::BadKeyframe { tick, source } => write!(f, "bad keyframe at tick {tick}: {source}"),
            ReplayError::TickOutOfRange { tick, last } => {
                write!(f, "tick {tick} is past the last recorded tick {last}")
            }
        }
    }
}
impl std::error::Error for ReplayError {}

/// A cancellable parse either observed cancellation or failed to read a replay.
/// The ordinary [`ReplayError`] contract is unchanged.
#[derive(Debug)]
pub enum ReplayParseError {
    Cancelled,
    Replay(ReplayError),
}

impl From<ReplayError> for ReplayParseError {
    fn from(error: ReplayError) -> Self {
        Self::Replay(error)
    }
}

impl core::fmt::Display for ReplayParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("replay parsing was cancelled"),
            Self::Replay(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ReplayParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cancelled => None,
            Self::Replay(error) => Some(error),
        }
    }
}

// Named boundaries also let tests request cancellation deterministically,
// without timing a thread against decompression or a particular record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParseCheckpoint {
    Start,
    BeforeDecompress,
    AfterDecompress,
    BeforeTick,
    AfterTick,
    BeforeCommand,
    AfterCommand,
    BeforeChecksum,
    AfterChecksum,
    BeforeKeyframe,
    AfterKeyframe,
    BeforeDebugCommand,
    AfterDebugCommand,
    Complete,
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn write_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn write_str(out: &mut Vec<u8>, s: &str) {
    write_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], ReplayError> {
        if n > self.remaining() {
            return Err(ReplayError::Truncated);
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, ReplayError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, ReplayError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, ReplayError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn str(&mut self) -> Result<String, ReplayError> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// One recorded tick's confirmed inputs, kept in memory while writing so
/// the previous tick's inputs are available for delta encoding.
pub(crate) struct RecordedTick<G: Game> {
    pub(crate) inputs: Vec<G::Input>,
    pub(crate) commands: Vec<(PlayerSlot, G::Command)>,
}

/// Records a `.orrp` replay incrementally: call [`ReplayWriter::record_tick`]
/// once per confirmed tick (in tick order) and [`ReplayWriter::record_checksum`]
/// whenever a checksum is taken, then [`ReplayWriter::finish`] to get the
/// final compressed file bytes.
pub struct ReplayWriter<G: Game> {
    header: ReplayHeader,
    ticks: BTreeMap<u64, RecordedTick<G>>,
    checksums: Vec<(u64, u64)>,
    /// Ticks between keyframes for [`ReplayWriter::maybe_record_keyframe`];
    /// `0` disables it.
    keyframe_interval: u64,
    /// Serialized `Frame`s by the frame's tick.
    keyframes: BTreeMap<u64, Vec<u8>>,
    /// Debug commands by the tick whose boundary they belong to.
    debug: BTreeMap<u64, Vec<DebugCommand>>,
}

impl<G: Game> ReplayWriter<G> {
    /// The header's `format_version` is ignored: `finish` always writes the
    /// current format version.
    pub fn new(header: ReplayHeader) -> Self {
        Self {
            header,
            ticks: BTreeMap::new(),
            checksums: Vec::new(),
            keyframe_interval: 0,
            keyframes: BTreeMap::new(),
            debug: BTreeMap::new(),
        }
    }

    pub fn header(&self) -> &ReplayHeader {
        &self.header
    }

    /// Records a debug command for the boundary before `tick` runs. Commands
    /// of one tick keep the order they were recorded in.
    pub fn record_debug(&mut self, tick: u64, cmd: DebugCommand) {
        self.debug.entry(tick).or_default().push(cmd);
    }

    pub fn debug_at(&self, tick: u64) -> &[DebugCommand] {
        self.debug.get(&tick).map_or(&[], |v| v.as_slice())
    }

    pub(crate) fn tick_data(&self, tick: u64) -> Option<&RecordedTick<G>> {
        self.ticks.get(&tick)
    }

    /// The last tick with recorded inputs (0 if none).
    pub fn last_tick(&self) -> u64 {
        self.ticks.keys().next_back().copied().unwrap_or(0)
    }

    /// The recorded checksum of `tick`, if one was taken. Checksums are
    /// expected in tick order (the last record of a tick wins).
    pub fn checksum_at(&self, tick: u64) -> Option<u64> {
        let end = self.checksums.partition_point(|&(t, _)| t <= tick);
        self.checksums[..end].last().filter(|&&(t, _)| t == tick).map(|&(_, c)| c)
    }

    /// Recorded `(tick, checksum)` pairs with `from <= tick <= to`, in tick order.
    pub fn checksums_in(&self, from: u64, to: u64) -> &[(u64, u64)] {
        let start = self.checksums.partition_point(|&(t, _)| t < from);
        let end = self.checksums.partition_point(|&(t, _)| t <= to);
        &self.checksums[start..end.max(start)]
    }

    /// Serialized keyframe of exactly `tick`.
    pub fn keyframe(&self, tick: u64) -> Option<&[u8]> {
        self.keyframes.get(&tick).map(Vec::as_slice)
    }

    /// Tick of the latest keyframe at or before `tick`.
    pub fn nearest_keyframe(&self, tick: u64) -> Option<u64> {
        self.keyframes.range(..=tick).next_back().map(|(&t, _)| t)
    }

    pub fn keyframe_ticks(&self) -> impl Iterator<Item = u64> + '_ {
        self.keyframes.keys().copied()
    }

    /// Bytes held by all keyframes.
    pub fn keyframe_bytes(&self) -> usize {
        self.keyframes.values().map(Vec::len).sum()
    }

    /// Drops every keyframe for which `keep(tick)` is false.
    pub fn retain_keyframes(&mut self, mut keep: impl FnMut(u64) -> bool) {
        self.keyframes.retain(|&t, _| keep(t));
    }

    /// Forgets everything recorded after `tick`: inputs, commands, debug
    /// commands, checksums and keyframes. This is the "branch" cut.
    pub fn truncate_after(&mut self, tick: u64) {
        let _ = self.ticks.split_off(&(tick + 1));
        let _ = self.keyframes.split_off(&(tick + 1));
        let _ = self.debug.split_off(&(tick + 1));
        self.checksums.retain(|&(t, _)| t <= tick);
    }

    /// Makes [`maybe_record_keyframe`](Self::maybe_record_keyframe) snapshot
    /// every `ticks` ticks (`0` = never, the default).
    pub fn with_keyframe_interval(mut self, ticks: u64) -> Self {
        self.keyframe_interval = ticks;
        self
    }

    /// Stores a snapshot of `frame`, keyed by `frame.tick()`. Call it after
    /// stepping that tick, with the frame the recorded inputs produced.
    pub fn record_keyframe(&mut self, frame: &Frame) {
        self.keyframes.insert(frame.tick(), frame.to_bytes());
    }

    /// Calls [`record_keyframe`](Self::record_keyframe) when `frame.tick()`
    /// is a positive multiple of the keyframe interval. Cheap to call every
    /// tick.
    pub fn maybe_record_keyframe(&mut self, frame: &Frame) {
        if self.keyframe_interval > 0 && frame.tick() > 0 && frame.tick() % self.keyframe_interval == 0 {
            self.record_keyframe(frame);
        }
    }

    pub fn record_tick(&mut self, tick: u64, inputs: &[G::Input], commands: &[(PlayerSlot, G::Command)]) {
        self.ticks.insert(tick, RecordedTick { inputs: inputs.to_vec(), commands: commands.to_vec() });
    }

    pub fn record_checksum(&mut self, tick: u64, checksum: u64) {
        self.checksums.push((tick, checksum));
    }

    /// Serializes header + delta-encoded body (lz4-compressed) + checksum
    /// table into the final `.orrp` byte string.
    pub fn finish(self) -> Vec<u8> {
        self.to_bytes()
    }

    /// Like [`finish`](Self::finish), but keeps the writer, so a live
    /// session can save its recording and go on.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        write_u32(&mut out, FORMAT_VERSION);
        write_str(&mut out, &self.header.game_id);
        write_u64(&mut out, self.header.build_hash);
        write_u64(&mut out, self.header.seed);
        out.push(self.header.player_count);
        write_u32(&mut out, self.header.tick_rate);
        write_u32(&mut out, self.header.input_size);

        let mut body = Vec::new();
        write_u32(&mut body, self.ticks.len() as u32);
        let mut prev: Option<Vec<G::Input>> = None;
        for (&tick, rec) in &self.ticks {
            write_u64(&mut body, tick);
            // Change bitmask (up to 32 players; `player_count` <= that in
            // practice) then raw bytes for each changed player's input.
            let mut mask: u32 = 0;
            for (i, inp) in rec.inputs.iter().enumerate() {
                let changed = match &prev {
                    Some(p) => p.get(i).map(|pv| pv != inp).unwrap_or(true),
                    None => true,
                };
                if changed {
                    mask |= 1 << i;
                }
            }
            write_u32(&mut body, mask);
            for (i, inp) in rec.inputs.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    body.extend_from_slice(bytemuck::bytes_of(inp));
                }
            }
            write_u32(&mut body, rec.commands.len() as u32);
            for (slot, cmd) in &rec.commands {
                body.push(slot.0);
                let start = body.len();
                write_u32(&mut body, 0); // placeholder length
                cmd.encode(&mut body);
                let len = (body.len() - start - 4) as u32;
                body[start..start + 4].copy_from_slice(&len.to_le_bytes());
            }
            prev = Some(rec.inputs.clone());
        }

        write_u32(&mut body, self.checksums.len() as u32);
        for &(tick, checksum) in &self.checksums {
            write_u64(&mut body, tick);
            write_u64(&mut body, checksum);
        }

        write_u32(&mut body, self.keyframes.len() as u32);
        for (&tick, bytes) in &self.keyframes {
            write_u64(&mut body, tick);
            write_u32(&mut body, bytes.len() as u32);
            body.extend_from_slice(bytes);
        }

        let debug_count: usize = self.debug.values().map(Vec::len).sum();
        write_u32(&mut body, debug_count as u32);
        for (&tick, cmds) in &self.debug {
            for cmd in cmds {
                write_u64(&mut body, tick);
                let start = body.len();
                write_u32(&mut body, 0); // placeholder length
                cmd.encode(&mut body);
                let len = (body.len() - start - 4) as u32;
                body[start..start + 4].copy_from_slice(&len.to_le_bytes());
            }
        }

        let compressed = lz4_flex::block::compress_prepend_size(&body);
        write_u32(&mut out, compressed.len() as u32);
        out.extend_from_slice(&compressed);
        out
    }
}

/// One decoded tick's inputs (indexed by player slot) and commands.
type TickData<G> = (Vec<<G as Game>::Input>, Vec<(PlayerSlot, <G as Game>::Command)>);

/// A `.orrp` file, fully decoded into memory.
pub struct ReplayReader<G: Game> {
    pub header: ReplayHeader,
    ticks: BTreeMap<u64, TickData<G>>,
    pub checksums: Vec<(u64, u64)>,
    keyframes: BTreeMap<u64, Vec<u8>>,
    debug: BTreeMap<u64, Vec<DebugCommand>>,
    cursor: u64,
    first_tick: u64,
    last_tick: u64,
}

impl<G: Game> ReplayReader<G> {
    pub fn parse(bytes: &[u8]) -> Result<Self, ReplayError> {
        Self::parse_with_checkpoint(bytes, |_| Ok(()))
    }

    /// Fully parses a recording, checking `cancel` around decompression and
    /// each tick, command and suffix-table record. Cancellation never returns
    /// a partial reader. A single LZ4 call, allocation/copy or command decoder
    /// is not interruptible; cancellation is observed when it returns.
    pub fn parse_cancellable(bytes: &[u8], cancel: &AtomicBool) -> Result<Self, ReplayParseError> {
        Self::parse_with_checkpoint(bytes, |_| {
            if cancel.load(Ordering::Relaxed) { Err(ReplayParseError::Cancelled) } else { Ok(()) }
        })
    }

    fn parse_with_checkpoint<E: From<ReplayError>>(bytes: &[u8], mut checkpoint: impl FnMut(ParseCheckpoint) -> Result<(), E>) -> Result<Self, E> {
        checkpoint(ParseCheckpoint::Start)?;
        if bytes.len() < 4 {
            return Err(ReplayError::TooShort.into());
        }
        if &bytes[0..4] != MAGIC {
            return Err(ReplayError::BadMagic.into());
        }
        let mut r = Reader::new(&bytes[4..]);
        let format_version = r.u32()?;
        if !(MIN_FORMAT_VERSION..=FORMAT_VERSION).contains(&format_version) {
            return Err(ReplayError::UnsupportedVersion(format_version).into());
        }
        let game_id = r.str()?;
        let build_hash = r.u64()?;
        let seed = r.u64()?;
        let player_count = r.u8()?;
        let tick_rate = r.u32()?;
        let input_size = r.u32()?;
        if player_count > 32 {
            return Err(ReplayError::BadPlayerCount(player_count).into());
        }
        let header = ReplayHeader { format_version, game_id, build_hash, seed, player_count, tick_rate, input_size };

        let compressed_len = r.u32()? as usize;
        let compressed = r.take(compressed_len)?;
        checkpoint(ParseCheckpoint::BeforeDecompress)?;
        let body = crate::wire::decompress_bounded(compressed);
        checkpoint(ParseCheckpoint::AfterDecompress)?;
        let body = body.map_err(ReplayError::Decompress)?;

        let mut br = Reader::new(&body);
        let tick_count = br.u32()?;
        let mut ticks = BTreeMap::new();
        let mut prev: Option<Vec<G::Input>> = None;
        for _ in 0..tick_count {
            checkpoint(ParseCheckpoint::BeforeTick)?;
            let tick = br.u64()?;
            let mask = br.u32()?;
            let mut inputs = prev.clone().unwrap_or_else(|| vec![G::Input::default(); player_count as usize]);
            for (i, slot) in inputs.iter_mut().enumerate() {
                if mask & (1 << i) != 0 {
                    let bytes = br.take(header.input_size as usize)?;
                    *slot = bytemuck::try_pod_read_unaligned(bytes).map_err(|_| ReplayError::Truncated)?;
                }
            }
            let cmd_count = br.u32()?;
            // Each command takes at least 5 bytes (slot + length), so the
            // remaining input bounds the allocation, not the forged count.
            let mut commands = Vec::with_capacity((cmd_count as usize).min(br.remaining() / 5));
            for _ in 0..cmd_count {
                checkpoint(ParseCheckpoint::BeforeCommand)?;
                let slot = PlayerSlot(br.u8()?);
                let len = br.u32()? as usize;
                let bytes = br.take(len)?;
                let cmd = G::Command::decode(bytes);
                checkpoint(ParseCheckpoint::AfterCommand)?;
                let cmd = cmd.ok_or(ReplayError::BadCommand)?;
                commands.push((slot, cmd));
            }
            prev = Some(inputs.clone());
            ticks.insert(tick, (inputs, commands));
            checkpoint(ParseCheckpoint::AfterTick)?;
        }

        let checksum_count = br.u32()?;
        let mut checksums = Vec::with_capacity((checksum_count as usize).min(br.remaining() / 16));
        for _ in 0..checksum_count {
            checkpoint(ParseCheckpoint::BeforeChecksum)?;
            let tick = br.u64()?;
            let checksum = br.u64()?;
            checksums.push((tick, checksum));
            checkpoint(ParseCheckpoint::AfterChecksum)?;
        }

        let mut keyframes = BTreeMap::new();
        if format_version >= 2 {
            let keyframe_count = br.u32()?;
            for _ in 0..keyframe_count {
                checkpoint(ParseCheckpoint::BeforeKeyframe)?;
                let tick = br.u64()?;
                let len = br.u32()? as usize;
                keyframes.insert(tick, br.take(len)?.to_vec());
                checkpoint(ParseCheckpoint::AfterKeyframe)?;
            }
        }

        let mut debug: BTreeMap<u64, Vec<DebugCommand>> = BTreeMap::new();
        if format_version >= 3 {
            let debug_count = br.u32()?;
            for _ in 0..debug_count {
                checkpoint(ParseCheckpoint::BeforeDebugCommand)?;
                let tick = br.u64()?;
                let len = br.u32()? as usize;
                let cmd = DebugCommand::decode(br.take(len)?);
                checkpoint(ParseCheckpoint::AfterDebugCommand)?;
                let cmd = cmd.ok_or(ReplayError::BadCommand)?;
                debug.entry(tick).or_default().push(cmd);
            }
        }

        let first_tick = ticks.keys().next().copied().unwrap_or(1);
        let last_tick = ticks.keys().next_back().copied().unwrap_or(0);
        checkpoint(ParseCheckpoint::Complete)?;
        Ok(Self { header, ticks, checksums, keyframes, debug, cursor: first_tick, first_tick, last_tick })
    }

    pub fn tick_count(&self) -> usize {
        self.ticks.len()
    }

    pub fn last_tick(&self) -> u64 {
        self.last_tick
    }

    pub fn tick(&self, tick: u64) -> Option<&TickData<G>> {
        self.ticks.get(&tick)
    }

    /// The debug commands applied at the boundary before `tick`, in order.
    pub fn debug_commands(&self, tick: u64) -> &[DebugCommand] {
        self.debug.get(&tick).map_or(&[], |v| v.as_slice())
    }

    /// Tick of the earliest recorded input (1 when the file has none).
    pub fn first_tick(&self) -> u64 {
        self.first_tick
    }

    pub fn keyframe_count(&self) -> usize {
        self.keyframes.len()
    }

    /// Tick of the latest keyframe at or before `tick`, if any.
    pub fn nearest_keyframe(&self, tick: u64) -> Option<u64> {
        self.keyframes.range(..=tick).next_back().map(|(&t, _)| t)
    }

    /// Builds a [`Simulation`] positioned at `tick`, identical to one played
    /// from tick 0: restores the nearest keyframe at or before `tick` (or
    /// starts fresh when there is none) and resimulates the recorded inputs
    /// forward. Does not check the header's `build_hash`; see
    /// [`replay_seek_checked`].
    pub fn seek(&self, config: G::Config, tick: u64) -> Result<Simulation<G>, ReplayError> {
        if tick > self.last_tick {
            return Err(ReplayError::TickOutOfRange { tick, last: self.last_tick });
        }
        let mut sim = Simulation::<G>::new(config, self.header.tick_rate, self.header.seed);
        let mut from = 0;
        if let Some(key) = self.nearest_keyframe(tick) {
            let frame = Frame::from_bytes(sim.registry().clone(), &self.keyframes[&key])
                .map_err(|source| ReplayError::BadKeyframe { tick: key, source })?;
            if frame.tick() != key {
                let source = FrameDecodeError::Corrupt("keyframe frame tick differs from its recorded tick");
                return Err(ReplayError::BadKeyframe { tick: key, source });
            }
            sim.restore(&frame);
            from = key;
        }
        // Walk recorded ticks only: `last_tick` comes from the file, so a
        // counting loop up to `tick` could run for ever on a forged file.
        if from < tick {
            for (&t, _) in self.ticks.range(from + 1..=tick) {
                step_recorded(&mut sim, self, t);
            }
        }
        Ok(sim)
    }

    /// Copies the whole recording into a [`ReplayWriter`], so a session can
    /// go on from it (replay viewer, branching). Keyframes are kept as is.
    pub(crate) fn to_writer(&self) -> ReplayWriter<G> {
        let mut w = ReplayWriter::new(self.header.clone());
        for (&tick, (inputs, commands)) in &self.ticks {
            w.record_tick(tick, inputs, commands);
        }
        w.checksums = self.checksums.clone();
        w.checksums.sort_by_key(|&(t, _)| t);
        w.keyframes = self.keyframes.clone();
        w.debug = self.debug.clone();
        w
    }

    /// Resets internal playback position to the first recorded tick, for
    /// reuse as an [`crate::InputSource`] across multiple runs.
    pub fn rewind(&mut self) {
        self.cursor = self.first_tick;
    }
}

impl<G: Game> crate::InputSource<G> for ReplayReader<G> {
    fn send_local(&mut self, _tick: u64, _slot: PlayerSlot, _input: G::Input, _commands: Vec<G::Command>) {
        // Replay drives every player; live local input is ignored.
    }

    fn poll_remote(&mut self) -> Vec<crate::RemoteInput<G>> {
        let mut out = Vec::new();
        if self.cursor > self.last_tick {
            return out;
        }
        if let Some((inputs, commands)) = self.ticks.get(&self.cursor) {
            for (slot, input) in inputs.iter().enumerate() {
                let slot = PlayerSlot(slot as u8);
                let my_commands: Vec<G::Command> =
                    commands.iter().filter(|(s, _)| *s == slot).map(|(_, c)| c.clone()).collect();
                out.push(crate::RemoteInput { tick: self.cursor, slot, input: *input, commands: my_commands, disconnected: false });
            }
        }
        self.cursor += 1;
        out
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use orr_fp::FP;
    use orr_sim::TickInputs;
    use orr_testgame::{Arena, ArenaConfig, ArenaInput, Score, SpawnBulletCmd};

    fn recording() -> Vec<u8> {
        let mut sim = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 9);
        let mut writer = ReplayWriter::<Arena>::new(ReplayHeader {
            format_version: 3, game_id: "arena".into(), build_hash: sim.build_hash(), seed: 9,
            player_count: 2, tick_rate: 60, input_size: std::mem::size_of::<ArenaInput>() as u32,
        }).with_keyframe_interval(1);
        writer.record_checksum(0, sim.checksum());
        writer.record_keyframe(sim.frame());
        for tick in 1..=3 {
            // Tick 2 repeats tick 1; tick 3 changes one slot, exercising deltas.
            let inputs = [ArenaInput::new(FP::ONE, FP::ZERO, false), ArenaInput::new(FP::from_int((tick / 3) as i32), FP::ZERO, false)];
            let commands = vec![(PlayerSlot(0), SpawnBulletCmd { owner: 0 }), (PlayerSlot(1), SpawnBulletCmd { owner: 1 })];
            let score = sim.frame().registry().singleton_id::<Score>().unwrap();
            let debug = vec![
                DebugCommand::SetSingletonField { singleton: score, offset: 0, bytes: (tick as u32).to_le_bytes().to_vec() },
                DebugCommand::SetSingletonField { singleton: score, offset: 0, bytes: (tick as u32 + 10).to_le_bytes().to_vec() },
            ];
            let mut ti = TickInputs::new(tick, 2);
            for (slot, input) in inputs.iter().enumerate() {
                ti.set_input(PlayerSlot(slot as u8), *input);
            }
            ti.set_commands(commands.clone());
            sim.step_with_debug(&ti, &debug);
            writer.record_tick(tick, &inputs, &commands);
            writer.record_checksum(tick, sim.checksum());
            writer.maybe_record_keyframe(sim.frame());
            for command in debug {
                writer.record_debug(tick, command);
            }
        }
        writer.finish()
    }

    #[test]
    fn unset_cancellation_preserves_full_reader() {
        let bytes = recording();
        let ordinary = ReplayReader::<Arena>::parse(&bytes).unwrap();
        let cancellable = ReplayReader::<Arena>::parse_cancellable(&bytes, &AtomicBool::new(false)).unwrap();
        let header = |h: &ReplayHeader| (h.format_version, h.game_id.clone(), h.build_hash, h.seed, h.player_count, h.tick_rate, h.input_size);
        assert_eq!(header(&ordinary.header), header(&cancellable.header));
        assert_eq!(ordinary.ticks, cancellable.ticks);
        assert_eq!(ordinary.checksums, cancellable.checksums);
        assert_eq!(ordinary.keyframes, cancellable.keyframes);
        assert_eq!(ordinary.debug, cancellable.debug);
        assert_eq!((ordinary.cursor, ordinary.first_tick, ordinary.last_tick), (cancellable.cursor, cancellable.first_tick, cancellable.last_tick));
        assert_eq!(ordinary.tick_count(), 3);
        assert_eq!(ordinary.checksums.len(), 4);
        assert_eq!(ordinary.keyframe_count(), 4);
        assert_eq!(ordinary.debug_commands(3).len(), 2);
        assert_eq!(ordinary.to_writer().finish(), bytes);
        assert_eq!(cancellable.to_writer().finish(), bytes);
    }

    #[test]
    fn cancellation_discards_reader_at_every_checkpoint() {
        let bytes = recording();
        assert!(matches!(ReplayReader::<Arena>::parse_cancellable(&bytes, &AtomicBool::new(true)), Err(ReplayParseError::Cancelled)));
        let mut checkpoints = Vec::new();
        ReplayReader::<Arena>::parse_with_checkpoint(&bytes, |point| {
            checkpoints.push(point);
            Ok::<(), ReplayParseError>(())
        }).unwrap();
        for (point, count) in [
            (ParseCheckpoint::Start, 1), (ParseCheckpoint::BeforeDecompress, 1), (ParseCheckpoint::AfterDecompress, 1),
            (ParseCheckpoint::BeforeTick, 3), (ParseCheckpoint::AfterTick, 3),
            (ParseCheckpoint::BeforeCommand, 6), (ParseCheckpoint::AfterCommand, 6),
            (ParseCheckpoint::BeforeChecksum, 4), (ParseCheckpoint::AfterChecksum, 4),
            (ParseCheckpoint::BeforeKeyframe, 4), (ParseCheckpoint::AfterKeyframe, 4),
            (ParseCheckpoint::BeforeDebugCommand, 6), (ParseCheckpoint::AfterDebugCommand, 6), (ParseCheckpoint::Complete, 1),
        ] {
            assert_eq!(checkpoints.iter().filter(|&&p| p == point).count(), count, "{point:?}");
        }
        // Visit every occurrence, including the middle and final record of
        // each table, the final decoder and the last successful-return boundary.
        for stop in 0..checkpoints.len() {
            let cancel = AtomicBool::new(false);
            let mut visited = Vec::new();
            let result = ReplayReader::<Arena>::parse_with_checkpoint(&bytes, |point| {
                visited.push(point);
                if visited.len() == stop + 1 {
                    cancel.store(true, Ordering::Relaxed);
                }
                if cancel.load(Ordering::Relaxed) { Err(ReplayParseError::Cancelled) } else { Ok(()) }
            });
            assert!(matches!(result, Err(ReplayParseError::Cancelled)), "checkpoint {stop}: {visited:?}");
            assert_eq!(visited, checkpoints[..=stop], "no later work after cancellation");
        }
    }
}

/// Report produced by [`replay_verify`]: either every recorded checksum
/// matched a fresh headless resimulation, or the first mismatch found.
#[derive(Debug)]
pub struct VerifyReport {
    pub ticks_simulated: u64,
    pub checksums_checked: u32,
    pub mismatch: Option<(u64, u64, u64)>, // (tick, recorded, resimulated)
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.mismatch.is_none()
    }
}

/// Headlessly resimulates a `.orrp` file's recorded inputs from tick 0 with
/// a fresh [`Simulation<G>`] and compares every recorded checksum against
/// what that resimulation actually produces.
///
/// Does **not** check the header's `build_hash` — use
/// [`replay_verify_checked`] when the caller has a `build_id` to verify
/// against (e.g. a CI job re-verifying a bug-repro replay after a patch,
/// which is exactly when a build-hash check matters most).
pub fn replay_verify<G: Game>(bytes: &[u8], config: G::Config) -> Result<VerifyReport, ReplayError> {
    let reader = ReplayReader::<G>::parse(bytes)?;
    replay_verify_reader::<G>(reader, config)
}

/// Like [`replay_verify`], but first checks the replay's recorded
/// `build_hash` against what `orr_sim::build_hash_of(build_id, 0)` would
/// report (patch generation `0`, i.e. "no patches applied since this build
/// id was recorded" — the normal case for verifying a `.orrp` file someone
/// handed you). Returns [`ReplayError::BuildHashMismatch`] immediately,
/// without resimulating anything, when they disagree; `build_id == 0`
/// (not tracking a build id, matching [`Simulation::build_hash`]'s
/// wildcard) or a header `build_hash` of `0` always passes.
pub fn replay_verify_checked<G: Game>(
    bytes: &[u8],
    config: G::Config,
    build_id: u64,
) -> Result<VerifyReport, ReplayError> {
    let reader = ReplayReader::<G>::parse(bytes)?;
    check_build_hash(&reader.header, build_id)?;
    replay_verify_reader::<G>(reader, config)
}

/// Parses `bytes`, checks the build hash exactly like
/// [`replay_verify_checked`], then returns a [`Simulation`] at `tick`
/// (see [`ReplayReader::seek`]).
pub fn replay_seek_checked<G: Game>(
    bytes: &[u8],
    config: G::Config,
    build_id: u64,
    tick: u64,
) -> Result<Simulation<G>, ReplayError> {
    let reader = ReplayReader::<G>::parse(bytes)?;
    check_build_hash(&reader.header, build_id)?;
    reader.seek(config, tick)
}

pub(crate) fn check_build_hash(header: &ReplayHeader, build_id: u64) -> Result<(), ReplayError> {
    let expected_hash = orr_sim::build_hash_of(build_id, 0);
    if expected_hash != 0 && header.build_hash != 0 && expected_hash != header.build_hash {
        return Err(ReplayError::BuildHashMismatch { header: header.build_hash, expected: expected_hash });
    }
    Ok(())
}

/// Steps `sim` once with the inputs and commands recorded for `tick`.
/// Returns `false` (and does nothing) when the file has no such tick.
fn step_recorded<G: Game>(sim: &mut Simulation<G>, reader: &ReplayReader<G>, tick: u64) -> bool {
    let Some((inputs, commands)) = reader.tick(tick) else { return false };
    let mut tick_inputs = orr_sim::TickInputs::<G::Input, G::Command>::new(tick, inputs.len() as u8);
    let debug = reader.debug_commands(tick);
    for (i, inp) in inputs.iter().enumerate() {
        tick_inputs.set_input(PlayerSlot(i as u8), *inp);
    }
    tick_inputs.set_commands(commands.clone());
    let _events = sim.step_with_debug(&tick_inputs, debug);
    true
}

fn replay_verify_reader<G: Game>(reader: ReplayReader<G>, config: G::Config) -> Result<VerifyReport, ReplayError> {
    let mut sim = Simulation::<G>::new(config, reader.header.tick_rate, reader.header.seed);
    let checksum_lookup: BTreeMap<u64, u64> = reader.checksums.iter().copied().collect();
    let mut checked = 0u32;
    let mut mismatch = None;

    for &tick in reader.ticks.keys() {
        if !step_recorded(&mut sim, &reader, tick) {
            continue;
        }

        if let Some(&recorded) = checksum_lookup.get(&tick) {
            checked += 1;
            let actual = sim.checksum();
            if actual != recorded && mismatch.is_none() {
                mismatch = Some((tick, recorded, actual));
            }
        }
    }

    Ok(VerifyReport { ticks_simulated: reader.last_tick(), checksums_checked: checked, mismatch })
}
