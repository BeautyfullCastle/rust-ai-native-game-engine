//! Desync diagnostic dump (`.orrd`): what a client writes when the server
//! reports that clients' checksums disagree.
//!
//! It holds a verified frame from before the mismatch (an "anchor",
//! `Frame::to_bytes`, lz4) and every confirmed input the client received
//! after it, so the run can be replayed offline: [`DesyncDump::replay`]
//! resimulates from the anchor and returns the checksums, which can then be
//! compared with `local_checksum` and the other clients' `reports`.
//!
//! Layout (little endian, lengths `u32`, never `usize`):
//!
//! ```text
//! "ORRD" version u32
//! build_hash u64  seed u64  tick_rate u32  player_count u8  input_size u32
//! checksum_interval u32  local_slot u8
//! desync_tick u64  local_checksum u64 (0 = none)
//! report_count u32, per report: slot u8, checksum u64
//! anchor_tick u64  anchor_checksum u64  frame_len u32  lz4(size-prefixed) frame
//!   (anchor_tick 0 with an empty frame = start from Game::setup)
//! tick_count u32, per tick: tick u64, per slot: flags u8, input bytes,
//!   command_count u32, per command: len u32, bytes
//! checksum u64 (xxh3 of everything before)
//! ```
use orr_ecs::{Frame, FrameDecodeError};
use orr_proto::{Bundle, SlotConfirmed};
use orr_sim::{Game, PlayerSlot, SimCommand, Simulation, TickInputs};

const MAGIC: &[u8; 4] = b"ORRD";
const VERSION: u32 = 1;

/// Why a dump could not be read or replayed.
#[derive(Debug)]
pub enum DumpError {
    Truncated,
    BadMagic,
    UnsupportedVersion(u32),
    BadChecksum,
    Corrupt(&'static str),
    Decompress(String),
    BadSnapshot(FrameDecodeError),
    /// The dump does not match the game or build it is replayed with.
    Mismatch(&'static str),
}

impl core::fmt::Display for DumpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DumpError::Truncated => write!(f, "dump truncated"),
            DumpError::BadMagic => write!(f, "not an ORRD dump"),
            DumpError::UnsupportedVersion(v) => write!(f, "unsupported dump version {v}"),
            DumpError::BadChecksum => write!(f, "dump checksum mismatch"),
            DumpError::Corrupt(what) => write!(f, "corrupt dump: {what}"),
            DumpError::Decompress(e) => write!(f, "dump decompression failed: {e}"),
            DumpError::BadSnapshot(e) => write!(f, "bad snapshot in dump: {e}"),
            DumpError::Mismatch(what) => write!(f, "dump does not match this game: {what}"),
        }
    }
}
impl std::error::Error for DumpError {}

/// A decoded (or about to be written) desync dump.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesyncDump {
    pub build_hash: u64,
    pub seed: u64,
    pub tick_rate: u32,
    pub player_count: u8,
    pub input_size: u32,
    pub checksum_interval: u32,
    pub local_slot: u8,
    /// The tick the server found different checksums at.
    pub desync_tick: u64,
    /// This client's own checksum at `desync_tick` (`0` if it had none).
    pub local_checksum: u64,
    /// What every reporting slot sent for `desync_tick`.
    pub reports: Vec<(u8, u64)>,
    pub anchor_tick: u64,
    pub anchor_checksum: u64,
    /// `Frame::to_bytes` of the anchor (empty when `anchor_tick` is 0).
    pub anchor_frame: Vec<u8>,
    /// Confirmed bundles after the anchor, in tick order, no gaps.
    pub ticks: Vec<Bundle>,
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: u64) -> Result<&'a [u8], DumpError> {
        if n > self.bytes.len() as u64 {
            return Err(DumpError::Truncated);
        }
        let (head, rest) = self.bytes.split_at(n as usize);
        self.bytes = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, DumpError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, DumpError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, DumpError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    /// A count whose elements are each at least `min` bytes, checked against
    /// what is left before anything is allocated.
    fn count(&mut self, min: u64) -> Result<u32, DumpError> {
        let n = self.u32()?;
        if u64::from(n) * min > self.bytes.len() as u64 {
            return Err(DumpError::Truncated);
        }
        Ok(n)
    }
}

impl DesyncDump {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::new();
        o.extend_from_slice(MAGIC);
        o.extend_from_slice(&VERSION.to_le_bytes());
        o.extend_from_slice(&self.build_hash.to_le_bytes());
        o.extend_from_slice(&self.seed.to_le_bytes());
        o.extend_from_slice(&self.tick_rate.to_le_bytes());
        o.push(self.player_count);
        o.extend_from_slice(&self.input_size.to_le_bytes());
        o.extend_from_slice(&self.checksum_interval.to_le_bytes());
        o.push(self.local_slot);
        o.extend_from_slice(&self.desync_tick.to_le_bytes());
        o.extend_from_slice(&self.local_checksum.to_le_bytes());
        o.extend_from_slice(&(self.reports.len() as u32).to_le_bytes());
        for (slot, sum) in &self.reports {
            o.push(*slot);
            o.extend_from_slice(&sum.to_le_bytes());
        }
        o.extend_from_slice(&self.anchor_tick.to_le_bytes());
        o.extend_from_slice(&self.anchor_checksum.to_le_bytes());
        let frame = if self.anchor_frame.is_empty() {
            Vec::new()
        } else {
            lz4_flex::block::compress_prepend_size(&self.anchor_frame)
        };
        o.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        o.extend_from_slice(&frame);
        o.extend_from_slice(&(self.ticks.len() as u32).to_le_bytes());
        for b in &self.ticks {
            o.extend_from_slice(&b.tick.to_le_bytes());
            for s in &b.slots {
                o.push(s.flags);
                o.extend_from_slice(&s.input);
                o.extend_from_slice(&(s.commands.len() as u32).to_le_bytes());
                for c in &s.commands {
                    o.extend_from_slice(&(c.len() as u32).to_le_bytes());
                    o.extend_from_slice(c);
                }
            }
        }
        let sum = xxhash_rust::xxh3::xxh3_64(&o);
        o.extend_from_slice(&sum.to_le_bytes());
        o
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DumpError> {
        let Some(split) = bytes.len().checked_sub(8) else { return Err(DumpError::Truncated) };
        let (body, tail) = bytes.split_at(split);
        let mut r = Reader { bytes: body };
        if r.take(4)? != MAGIC {
            return Err(DumpError::BadMagic);
        }
        let version = r.u32()?;
        if version != VERSION {
            return Err(DumpError::UnsupportedVersion(version));
        }
        if u64::from_le_bytes(tail.try_into().unwrap()) != xxhash_rust::xxh3::xxh3_64(body) {
            return Err(DumpError::BadChecksum);
        }
        let build_hash = r.u64()?;
        let seed = r.u64()?;
        let tick_rate = r.u32()?;
        let player_count = r.u8()?;
        let input_size = r.u32()?;
        if input_size > 1 << 16 || player_count > 64 {
            return Err(DumpError::Corrupt("size out of range"));
        }
        let checksum_interval = r.u32()?;
        let local_slot = r.u8()?;
        let desync_tick = r.u64()?;
        let local_checksum = r.u64()?;
        let n = r.count(9)?;
        let mut reports = Vec::with_capacity(n as usize);
        for _ in 0..n {
            reports.push((r.u8()?, r.u64()?));
        }
        let anchor_tick = r.u64()?;
        let anchor_checksum = r.u64()?;
        let len = r.u32()?;
        let compressed = r.take(u64::from(len))?;
        let anchor_frame = if compressed.is_empty() {
            Vec::new()
        } else {
            crate::wire::decompress_bounded(compressed).map_err(DumpError::Decompress)?
        };
        let per_slot = 5 + u64::from(input_size);
        let n = r.count(8 + u64::from(player_count) * per_slot)?;
        let mut ticks = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let tick = r.u64()?;
            let mut slots = Vec::with_capacity(player_count as usize);
            for _ in 0..player_count {
                let flags = r.u8()?;
                let input = r.take(u64::from(input_size))?.to_vec();
                let nc = r.count(4)?;
                let mut commands = Vec::with_capacity(nc as usize);
                for _ in 0..nc {
                    let l = r.u32()?;
                    commands.push(r.take(u64::from(l))?.to_vec());
                }
                slots.push(SlotConfirmed { input, commands, flags });
            }
            ticks.push(Bundle { tick, slots });
        }
        if !r.bytes.is_empty() {
            return Err(DumpError::Corrupt("trailing bytes"));
        }
        Ok(Self {
            build_hash,
            seed,
            tick_rate,
            player_count,
            input_size,
            checksum_interval,
            local_slot,
            desync_tick,
            local_checksum,
            reports,
            anchor_tick,
            anchor_checksum,
            anchor_frame,
            ticks,
        })
    }

    /// Resimulates the dump on a clean simulation of `G`: restores the
    /// anchor (or starts from `Game::setup`) and steps through every
    /// recorded tick. Returns `(tick, checksum)` for every tick that is a
    /// multiple of `checksum_interval`, in tick order. On a deterministic
    /// game these equal the checksums the healthy clients reported, so the
    /// entry at `desync_tick` shows which side diverged.
    pub fn replay<G: Game>(&self, config: G::Config, build_id: u64) -> Result<Vec<(u64, u64)>, DumpError> {
        let mut sim = Simulation::<G>::with_build_id(config, self.tick_rate, self.seed, build_id);
        if std::mem::size_of::<G::Input>() as u32 != self.input_size {
            return Err(DumpError::Mismatch("input size"));
        }
        if self.anchor_tick > 0 {
            let frame = Frame::from_bytes(sim.registry().clone(), &self.anchor_frame).map_err(DumpError::BadSnapshot)?;
            if frame.tick() != self.anchor_tick || frame.checksum() != self.anchor_checksum {
                return Err(DumpError::Mismatch("anchor frame"));
            }
            sim.restore(&frame);
        }
        let mut out = Vec::new();
        let mut expect = self.anchor_tick + 1;
        for b in &self.ticks {
            if b.tick != expect || b.slots.len() != self.player_count as usize {
                return Err(DumpError::Corrupt("ticks are not contiguous"));
            }
            expect += 1;
            let mut ti = TickInputs::<G::Input, G::Command>::new(b.tick, self.player_count);
            let mut cmds = Vec::new();
            for (i, s) in b.slots.iter().enumerate() {
                let slot = PlayerSlot(i as u8);
                let input = bytemuck::try_pod_read_unaligned::<G::Input>(&s.input)
                    .map_err(|_| DumpError::Corrupt("input size"))?;
                ti.set_input(slot, input);
                for raw in &s.commands {
                    if let Some(c) = <G::Command as SimCommand>::decode(raw) {
                        cmds.push((slot, c));
                    }
                }
            }
            ti.set_commands(cmds);
            sim.step(&ti);
            if self.checksum_interval > 0 && b.tick % u64::from(self.checksum_interval) == 0 {
                out.push((b.tick, sim.checksum()));
            }
        }
        Ok(out)
    }
}
