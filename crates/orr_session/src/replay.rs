//! `.orrp` replay file format: a header, a per-tick input/command stream
//! (delta-compressed against the previous tick's inputs, then whole-body
//! lz4 compressed), and a checksum table.
//!
//! Full-Frame snapshots for scrubbing (per the design doc's §6.3) are a
//! documented TODO: `orr_ecs` doesn't expose a byte (de)serialization API
//! for `Frame` (only `Clone`/`copy_from`, which need a live registry, not a
//! byte stream) and this task must not modify `orr_ecs`. Until that API
//! exists, `.orrp` stores only per-checkpoint checksums (fine for
//! verification/CI/bug-repro) and reconstructs any tick by resimulating
//! from tick 0 — the whole point of the format being replayable input logs.
use std::collections::BTreeMap;

use orr_sim::{Game, PlayerSlot, SimCommand, Simulation};

const MAGIC: &[u8; 4] = b"ORRP";
const FORMAT_VERSION: u32 = 1;

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
    /// [`replay_verify_checked`] was asked to verify against a build id
    /// whose resulting `Simulation::build_hash()` disagrees with the
    /// header's recorded `build_hash` (see the design doc's hot-patch
    /// decision: a replay recorded under one build/patch generation is not
    /// safe to resimulate-and-compare under another).
    BuildHashMismatch { header: u64, expected: u64 },
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
            ReplayError::BuildHashMismatch { header, expected } => {
                write!(f, "replay build hash {header:#x} does not match expected build hash {expected:#x}")
            }
        }
    }
}
impl std::error::Error for ReplayError {}

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
    fn take(&mut self, n: usize) -> Result<&'a [u8], ReplayError> {
        if self.pos + n > self.bytes.len() {
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
struct RecordedTick<G: Game> {
    inputs: Vec<G::Input>,
    commands: Vec<(PlayerSlot, G::Command)>,
}

/// Records a `.orrp` replay incrementally: call [`ReplayWriter::record_tick`]
/// once per confirmed tick (in tick order) and [`ReplayWriter::record_checksum`]
/// whenever a checksum is taken, then [`ReplayWriter::finish`] to get the
/// final compressed file bytes.
pub struct ReplayWriter<G: Game> {
    header: ReplayHeader,
    ticks: BTreeMap<u64, RecordedTick<G>>,
    checksums: Vec<(u64, u64)>,
}

impl<G: Game> ReplayWriter<G> {
    pub fn new(header: ReplayHeader) -> Self {
        Self { header, ticks: BTreeMap::new(), checksums: Vec::new() }
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
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        write_u32(&mut out, self.header.format_version);
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
    cursor: u64,
    first_tick: u64,
    last_tick: u64,
}

impl<G: Game> ReplayReader<G> {
    pub fn parse(bytes: &[u8]) -> Result<Self, ReplayError> {
        if bytes.len() < 4 {
            return Err(ReplayError::TooShort);
        }
        if &bytes[0..4] != MAGIC {
            return Err(ReplayError::BadMagic);
        }
        let mut r = Reader::new(&bytes[4..]);
        let format_version = r.u32()?;
        if format_version != FORMAT_VERSION {
            return Err(ReplayError::UnsupportedVersion(format_version));
        }
        let game_id = r.str()?;
        let build_hash = r.u64()?;
        let seed = r.u64()?;
        let player_count = r.u8()?;
        let tick_rate = r.u32()?;
        let input_size = r.u32()?;
        let header = ReplayHeader { format_version, game_id, build_hash, seed, player_count, tick_rate, input_size };

        let compressed_len = r.u32()? as usize;
        let compressed = r.take(compressed_len)?;
        let body = lz4_flex::block::decompress_size_prepended(compressed)
            .map_err(|e| ReplayError::Decompress(e.to_string()))?;

        let mut br = Reader::new(&body);
        let tick_count = br.u32()?;
        let mut ticks = BTreeMap::new();
        let mut prev: Option<Vec<G::Input>> = None;
        for _ in 0..tick_count {
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
            let mut commands = Vec::with_capacity(cmd_count as usize);
            for _ in 0..cmd_count {
                let slot = PlayerSlot(br.u8()?);
                let len = br.u32()? as usize;
                let bytes = br.take(len)?;
                let cmd = G::Command::decode(bytes).ok_or(ReplayError::BadCommand)?;
                commands.push((slot, cmd));
            }
            prev = Some(inputs.clone());
            ticks.insert(tick, (inputs, commands));
        }

        let checksum_count = br.u32()?;
        let mut checksums = Vec::with_capacity(checksum_count as usize);
        for _ in 0..checksum_count {
            let tick = br.u64()?;
            let checksum = br.u64()?;
            checksums.push((tick, checksum));
        }

        let first_tick = ticks.keys().next().copied().unwrap_or(1);
        let last_tick = ticks.keys().next_back().copied().unwrap_or(0);
        Ok(Self { header, ticks, checksums, cursor: first_tick, first_tick, last_tick })
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
                out.push(crate::RemoteInput { tick: self.cursor, slot, input: *input, commands: my_commands });
            }
        }
        self.cursor += 1;
        out
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
    let expected_hash = orr_sim::build_hash_of(build_id, 0);
    if expected_hash != 0 && reader.header.build_hash != 0 && expected_hash != reader.header.build_hash {
        return Err(ReplayError::BuildHashMismatch { header: reader.header.build_hash, expected: expected_hash });
    }
    replay_verify_reader::<G>(reader, config)
}

fn replay_verify_reader<G: Game>(reader: ReplayReader<G>, config: G::Config) -> Result<VerifyReport, ReplayError> {
    let mut sim = Simulation::<G>::new(config, reader.header.tick_rate, reader.header.seed);
    let checksum_lookup: BTreeMap<u64, u64> = reader.checksums.iter().copied().collect();
    let mut checked = 0u32;
    let mut mismatch = None;

    for tick in 1..=reader.last_tick() {
        let Some((inputs, commands)) = reader.tick(tick) else { continue };
        let mut tick_inputs = orr_sim::TickInputs::<G::Input, G::Command>::new(tick, inputs.len() as u8);
        for (i, inp) in inputs.iter().enumerate() {
            tick_inputs.set_input(PlayerSlot(i as u8), *inp);
        }
        tick_inputs.set_commands(commands.clone());
        let _events = sim.step(&tick_inputs);

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
