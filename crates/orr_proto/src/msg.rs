//! Relay protocol messages.
//!
//! Every message is one byte string:
//!
//! ```text
//! "ORRN"  version u32  kind u8  payload...  checksum u64
//! ```
//!
//! All integers are little endian; lengths and counts are `u32`, never
//! `usize`. `checksum` is xxh3-64 over every earlier byte, so any damaged
//! field (including the header) is caught. Decoding untrusted bytes never
//! panics and never allocates from a count before checking it against the
//! bytes left.
//!
//! Client to server ([`ClientMsg`]):
//!
//! | kind | message | channel |
//! |---|---|---|
//! | 1 | `Hello` | reliable |
//! | 2 | `Input`: last K unacked inputs (tick, input, commands) + `ack_tick` | unreliable |
//! | 3 | `Ping` | unreliable |
//! | 4 | `Checksum` (tick, xxh3 of the verified frame) | reliable |
//! | 5 | `SnapshotUpload` (answer to `SnapshotRequest`) | reliable |
//! | 6 | `SnapshotDecline` | reliable |
//! | 7 | `Ready` (clock synced, or late join caught up) | reliable |
//! | 8 | `Leave` | reliable |
//!
//! Server to client ([`ServerMsg`]):
//!
//! | kind | message | channel |
//! |---|---|---|
//! | 1 | `Welcome` (slot, tick rate, seed, config) | reliable |
//! | 2 | `Reject` | reliable |
//! | 3 | `Start` (room clock origin) | reliable |
//! | 4 | `Pong` | unreliable |
//! | 5 | `Confirmed` (bundles: every slot's input for a tick) | unreliable (live), reliable (backlog) |
//! | 6 | `TimeSync` (input arrival slack of this client) | unreliable |
//! | 7 | `Desync` (checksum mismatch notice) | reliable |
//! | 8 | `SnapshotRequest` (to a donor client) | reliable |
//! | 9 | `JoinSnapshot` (relayed snapshot, to the joiner) | reliable |
//! | 10 | `Presence` (a slot got a player or lost it) | reliable |
//! | 11 | `Bye` | reliable |
use crate::codec::{Reader, Writer};

/// Protocol version carried by every message.
pub const PROTOCOL_VERSION: u32 = 1;
const MAGIC: &[u8; 4] = b"ORRN";

/// `Hello::want_slot` value meaning "any free slot".
pub const NO_SLOT: u8 = 0xFF;

/// `SlotConfirmed::flags`: the server did not receive this slot's input for
/// the tick in time and filled it with the slot's previous input.
pub const FLAG_REPEATED: u8 = 1;
/// `SlotConfirmed::flags`: nobody plays this slot right now.
pub const FLAG_ABSENT: u8 = 2;

pub const MAX_INPUT_SIZE: u32 = 1024;
pub const MAX_SLOTS: u8 = 64;
const MAX_COMMANDS: u32 = 64;
const MAX_COMMAND_LEN: u32 = 4096;
const MAX_ENTRIES: u32 = 256;
const MAX_BUNDLES: u32 = 65_536;
const MAX_CONFIG_LEN: u32 = 1 << 20;
const MAX_SNAPSHOT_LEN: u32 = 256 << 20;
const MAX_REPORTS: u32 = 256;

/// Why a message could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtoError {
    Truncated,
    BadMagic,
    UnsupportedVersion(u32),
    BadChecksum,
    UnknownKind(u8),
    /// A length or count is above its limit.
    TooLarge,
    Corrupt(&'static str),
}

impl core::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ProtoError::Truncated => write!(f, "message truncated"),
            ProtoError::BadMagic => write!(f, "not an ORRN message"),
            ProtoError::UnsupportedVersion(v) => write!(f, "unsupported protocol version {v}"),
            ProtoError::BadChecksum => write!(f, "message checksum mismatch"),
            ProtoError::UnknownKind(k) => write!(f, "unknown message kind {k}"),
            ProtoError::TooLarge => write!(f, "message field too large"),
            ProtoError::Corrupt(what) => write!(f, "corrupt message: {what}"),
        }
    }
}
impl std::error::Error for ProtoError {}

/// One input a client submits for one tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputEntry {
    pub tick: u64,
    pub input: Vec<u8>,
    /// Encoded `SimCommand`s submitted together with the input.
    pub commands: Vec<Vec<u8>>,
}

/// One slot's confirmed input for one tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotConfirmed {
    pub input: Vec<u8>,
    pub commands: Vec<Vec<u8>>,
    /// [`FLAG_REPEATED`] | [`FLAG_ABSENT`].
    pub flags: u8,
}

/// Everything the server confirmed for one tick: one entry per slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    pub tick: u64,
    pub slots: Vec<SlotConfirmed>,
}

impl Bundle {
    /// Bytes of this bundle inside a `Confirmed` message.
    pub fn encoded_len(&self, input_size: u32) -> usize {
        let mut n = 8usize;
        for s in &self.slots {
            n += 1 + input_size as usize + 4;
            n += s.commands.iter().map(|c| 4 + c.len()).sum::<usize>();
        }
        n
    }
}

/// Client request to join a room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    pub build_hash: u64,
    pub room: u64,
    /// Byte size of one input; the room must use the same.
    pub input_size: u32,
    /// Slot the client wants, or [`NO_SLOT`].
    pub want_slot: u8,
    /// Token from an earlier `Welcome` to reclaim the same slot, or `0`.
    pub token: u64,
}

/// Server answer to an accepted `Hello`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    pub room: u64,
    pub slot: u8,
    pub player_count: u8,
    pub tick_rate: u32,
    pub seed: u64,
    pub build_hash: u64,
    pub input_size: u32,
    pub checksum_interval: u32,
    /// Secret that lets this client reclaim its slot after a disconnect.
    pub token: u64,
    /// The room already runs: a `JoinSnapshot` follows.
    pub running: bool,
    /// Server time (microseconds) of tick 0; only meaningful if `running`.
    pub t0_us: u64,
    pub server_time_us: u64,
    pub finalized_tick: u64,
    /// Opaque game/room configuration for the client's `Game::Config`.
    pub config: Vec<u8>,
}

/// Why a `Hello` was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    BuildHashMismatch { server: u64, client: u64 },
    InputSizeMismatch { server: u32, client: u32 },
    BadVersion { server: u32 },
    NoSuchRoom,
    RoomFull,
    SlotTaken,
    /// A late join found no client that can supply a snapshot.
    NoDonor,
    JoinTimedOut,
    BadRequest,
}

impl RejectReason {
    fn write(&self, w: &mut Writer) {
        let (code, a, b) = match *self {
            RejectReason::BuildHashMismatch { server, client } => (1, server, client),
            RejectReason::InputSizeMismatch { server, client } => (2, u64::from(server), u64::from(client)),
            RejectReason::BadVersion { server } => (3, u64::from(server), 0),
            RejectReason::NoSuchRoom => (4, 0, 0),
            RejectReason::RoomFull => (5, 0, 0),
            RejectReason::SlotTaken => (6, 0, 0),
            RejectReason::NoDonor => (7, 0, 0),
            RejectReason::JoinTimedOut => (8, 0, 0),
            RejectReason::BadRequest => (9, 0, 0),
        };
        w.u8(code);
        w.u64(a);
        w.u64(b);
    }

    fn read(r: &mut Reader) -> Result<Self, ProtoError> {
        let code = r.u8()?;
        let (a, b) = (r.u64()?, r.u64()?);
        Ok(match code {
            1 => RejectReason::BuildHashMismatch { server: a, client: b },
            2 => RejectReason::InputSizeMismatch { server: a as u32, client: b as u32 },
            3 => RejectReason::BadVersion { server: a as u32 },
            4 => RejectReason::NoSuchRoom,
            5 => RejectReason::RoomFull,
            6 => RejectReason::SlotTaken,
            7 => RejectReason::NoDonor,
            8 => RejectReason::JoinTimedOut,
            9 => RejectReason::BadRequest,
            _ => return Err(ProtoError::Corrupt("unknown reject reason")),
        })
    }
}

/// How early or late this client's inputs reached the server over the last
/// window of ticks (see the server docs). Positive slack = early.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSync {
    /// First and last tick of the window (ticks finalized since the last
    /// message).
    pub window_from: u64,
    pub finalized: u64,
    /// Smallest slack in the window (microseconds; negative = late).
    pub min_slack_us: i32,
    pub avg_slack_us: i32,
    /// Ticks in the window whose input was first seen at the server.
    pub samples: u32,
    /// Ticks in the window the server had to repeat for this client.
    pub late: u32,
}

/// Messages a client sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMsg {
    Hello(Hello),
    /// `ack_tick`: the client holds the confirmed bundles of every tick up
    /// to it (the server stops resending them).
    Input { slot: u8, input_size: u32, ack_tick: u64, entries: Vec<InputEntry> },
    Ping { seq: u32, client_time_us: u64, rtt_hint_us: u32 },
    Checksum { tick: u64, checksum: u64 },
    /// `data` is opaque to the server (the client's compressed frame).
    SnapshotUpload { request_id: u32, tick: u64, checksum: u64, data: Vec<u8> },
    SnapshotDecline { request_id: u32 },
    Ready,
    Leave,
}

/// Messages the server sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerMsg {
    Welcome(Welcome),
    Reject(RejectReason),
    Start { t0_us: u64, server_time_us: u64 },
    /// `t0_us` is `u64::MAX` while the room has not started.
    Pong { seq: u32, client_time_us: u64, server_time_us: u64, t0_us: u64, finalized_tick: u64 },
    Confirmed { input_size: u32, bundles: Vec<Bundle> },
    TimeSync(TimeSync),
    Desync { tick: u64, reports: Vec<(u8, u64)> },
    SnapshotRequest { request_id: u32 },
    JoinSnapshot { tick: u64, checksum: u64, data: Vec<u8> },
    Presence { slot: u8, present: bool, from_tick: u64 },
    Bye { code: u8 },
}

fn seal(mut w: Writer) -> Vec<u8> {
    let sum = xxhash_rust::xxh3::xxh3_64(&w.out);
    w.u64(sum);
    w.out
}

fn open(bytes: &[u8]) -> Result<(u8, Reader<'_>), ProtoError> {
    let Some(split) = bytes.len().checked_sub(8) else { return Err(ProtoError::Truncated) };
    let (body, tail) = bytes.split_at(split);
    let mut r = Reader::new(body);
    if r.take(4).map_err(|_| ProtoError::Truncated)? != MAGIC {
        return Err(ProtoError::BadMagic);
    }
    let version = r.u32()?;
    if version != PROTOCOL_VERSION {
        return Err(ProtoError::UnsupportedVersion(version));
    }
    if u64::from_le_bytes(tail.try_into().unwrap()) != xxhash_rust::xxh3::xxh3_64(body) {
        return Err(ProtoError::BadChecksum);
    }
    let kind = r.u8()?;
    Ok((kind, r))
}

fn start(kind: u8) -> Writer {
    let mut w = Writer::new();
    w.raw(MAGIC);
    w.u32(PROTOCOL_VERSION);
    w.u8(kind);
    w
}

fn write_commands(w: &mut Writer, commands: &[Vec<u8>]) {
    w.u32(commands.len() as u32);
    for c in commands {
        w.bytes(c);
    }
}

fn read_commands(r: &mut Reader) -> Result<Vec<Vec<u8>>, ProtoError> {
    let n = r.count(4, MAX_COMMANDS)?;
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        out.push(r.bytes(MAX_COMMAND_LEN)?);
    }
    Ok(out)
}

fn read_input(r: &mut Reader, input_size: u32) -> Result<Vec<u8>, ProtoError> {
    Ok(r.take(u64::from(input_size))?.to_vec())
}

fn check_input_size(input_size: u32) -> Result<(), ProtoError> {
    if input_size > MAX_INPUT_SIZE {
        Err(ProtoError::TooLarge)
    } else {
        Ok(())
    }
}

impl ClientMsg {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ClientMsg::Hello(h) => {
                let mut w = start(1);
                w.u64(h.build_hash);
                w.u64(h.room);
                w.u32(h.input_size);
                w.u8(h.want_slot);
                w.u64(h.token);
                seal(w)
            }
            ClientMsg::Input { slot, input_size, ack_tick, entries } => {
                let mut w = start(2);
                w.u8(*slot);
                w.u32(*input_size);
                w.u64(*ack_tick);
                w.u32(entries.len() as u32);
                for e in entries {
                    w.u64(e.tick);
                    debug_assert_eq!(e.input.len() as u32, *input_size);
                    w.raw(&e.input);
                    write_commands(&mut w, &e.commands);
                }
                seal(w)
            }
            ClientMsg::Ping { seq, client_time_us, rtt_hint_us } => {
                let mut w = start(3);
                w.u32(*seq);
                w.u64(*client_time_us);
                w.u32(*rtt_hint_us);
                seal(w)
            }
            ClientMsg::Checksum { tick, checksum } => {
                let mut w = start(4);
                w.u64(*tick);
                w.u64(*checksum);
                seal(w)
            }
            ClientMsg::SnapshotUpload { request_id, tick, checksum, data } => {
                let mut w = start(5);
                w.u32(*request_id);
                w.u64(*tick);
                w.u64(*checksum);
                w.bytes(data);
                seal(w)
            }
            ClientMsg::SnapshotDecline { request_id } => {
                let mut w = start(6);
                w.u32(*request_id);
                seal(w)
            }
            ClientMsg::Ready => seal(start(7)),
            ClientMsg::Leave => seal(start(8)),
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtoError> {
        let (kind, mut r) = open(bytes)?;
        let msg = match kind {
            1 => ClientMsg::Hello(Hello {
                build_hash: r.u64()?,
                room: r.u64()?,
                input_size: r.u32()?,
                want_slot: r.u8()?,
                token: r.u64()?,
            }),
            2 => {
                let slot = r.u8()?;
                let input_size = r.u32()?;
                check_input_size(input_size)?;
                let ack_tick = r.u64()?;
                let n = r.count(12 + u64::from(input_size), MAX_ENTRIES)?;
                let mut entries = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    let tick = r.u64()?;
                    let input = read_input(&mut r, input_size)?;
                    let commands = read_commands(&mut r)?;
                    entries.push(InputEntry { tick, input, commands });
                }
                ClientMsg::Input { slot, input_size, ack_tick, entries }
            }
            3 => ClientMsg::Ping { seq: r.u32()?, client_time_us: r.u64()?, rtt_hint_us: r.u32()? },
            4 => ClientMsg::Checksum { tick: r.u64()?, checksum: r.u64()? },
            5 => ClientMsg::SnapshotUpload {
                request_id: r.u32()?,
                tick: r.u64()?,
                checksum: r.u64()?,
                data: r.bytes(MAX_SNAPSHOT_LEN)?,
            },
            6 => ClientMsg::SnapshotDecline { request_id: r.u32()? },
            7 => ClientMsg::Ready,
            8 => ClientMsg::Leave,
            k => return Err(ProtoError::UnknownKind(k)),
        };
        r.finish()?;
        Ok(msg)
    }
}

impl ServerMsg {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ServerMsg::Welcome(m) => {
                let mut w = start(1);
                w.u64(m.room);
                w.u8(m.slot);
                w.u8(m.player_count);
                w.u32(m.tick_rate);
                w.u64(m.seed);
                w.u64(m.build_hash);
                w.u32(m.input_size);
                w.u32(m.checksum_interval);
                w.u64(m.token);
                w.u8(u8::from(m.running));
                w.u64(m.t0_us);
                w.u64(m.server_time_us);
                w.u64(m.finalized_tick);
                w.bytes(&m.config);
                seal(w)
            }
            ServerMsg::Reject(reason) => {
                let mut w = start(2);
                reason.write(&mut w);
                seal(w)
            }
            ServerMsg::Start { t0_us, server_time_us } => {
                let mut w = start(3);
                w.u64(*t0_us);
                w.u64(*server_time_us);
                seal(w)
            }
            ServerMsg::Pong { seq, client_time_us, server_time_us, t0_us, finalized_tick } => {
                let mut w = start(4);
                w.u32(*seq);
                w.u64(*client_time_us);
                w.u64(*server_time_us);
                w.u64(*t0_us);
                w.u64(*finalized_tick);
                seal(w)
            }
            ServerMsg::Confirmed { input_size, bundles } => {
                let mut w = start(5);
                w.u32(*input_size);
                let slots = bundles.first().map_or(0, |b| b.slots.len());
                w.u8(slots as u8);
                w.u32(bundles.len() as u32);
                for b in bundles {
                    debug_assert_eq!(b.slots.len(), slots);
                    w.u64(b.tick);
                    for s in &b.slots {
                        w.u8(s.flags);
                        debug_assert_eq!(s.input.len() as u32, *input_size);
                        w.raw(&s.input);
                        write_commands(&mut w, &s.commands);
                    }
                }
                seal(w)
            }
            ServerMsg::TimeSync(t) => {
                let mut w = start(6);
                w.u64(t.window_from);
                w.u64(t.finalized);
                w.i32(t.min_slack_us);
                w.i32(t.avg_slack_us);
                w.u32(t.samples);
                w.u32(t.late);
                seal(w)
            }
            ServerMsg::Desync { tick, reports } => {
                let mut w = start(7);
                w.u64(*tick);
                w.u32(reports.len() as u32);
                for (slot, sum) in reports {
                    w.u8(*slot);
                    w.u64(*sum);
                }
                seal(w)
            }
            ServerMsg::SnapshotRequest { request_id } => {
                let mut w = start(8);
                w.u32(*request_id);
                seal(w)
            }
            ServerMsg::JoinSnapshot { tick, checksum, data } => {
                let mut w = start(9);
                w.u64(*tick);
                w.u64(*checksum);
                w.bytes(data);
                seal(w)
            }
            ServerMsg::Presence { slot, present, from_tick } => {
                let mut w = start(10);
                w.u8(*slot);
                w.u8(u8::from(*present));
                w.u64(*from_tick);
                seal(w)
            }
            ServerMsg::Bye { code } => {
                let mut w = start(11);
                w.u8(*code);
                seal(w)
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtoError> {
        let (kind, mut r) = open(bytes)?;
        let msg = match kind {
            1 => ServerMsg::Welcome(Welcome {
                room: r.u64()?,
                slot: r.u8()?,
                player_count: r.u8()?,
                tick_rate: r.u32()?,
                seed: r.u64()?,
                build_hash: r.u64()?,
                input_size: r.u32()?,
                checksum_interval: r.u32()?,
                token: r.u64()?,
                running: r.u8()? != 0,
                t0_us: r.u64()?,
                server_time_us: r.u64()?,
                finalized_tick: r.u64()?,
                config: r.bytes(MAX_CONFIG_LEN)?,
            }),
            2 => ServerMsg::Reject(RejectReason::read(&mut r)?),
            3 => ServerMsg::Start { t0_us: r.u64()?, server_time_us: r.u64()? },
            4 => ServerMsg::Pong {
                seq: r.u32()?,
                client_time_us: r.u64()?,
                server_time_us: r.u64()?,
                t0_us: r.u64()?,
                finalized_tick: r.u64()?,
            },
            5 => {
                let input_size = r.u32()?;
                check_input_size(input_size)?;
                let slots = r.u8()?;
                if slots > MAX_SLOTS {
                    return Err(ProtoError::TooLarge);
                }
                let per_bundle = 8 + u64::from(slots) * (5 + u64::from(input_size));
                let n = r.count(per_bundle, MAX_BUNDLES)?;
                let mut bundles = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    let tick = r.u64()?;
                    let mut list = Vec::with_capacity(slots as usize);
                    for _ in 0..slots {
                        let flags = r.u8()?;
                        let input = read_input(&mut r, input_size)?;
                        let commands = read_commands(&mut r)?;
                        list.push(SlotConfirmed { input, commands, flags });
                    }
                    bundles.push(Bundle { tick, slots: list });
                }
                ServerMsg::Confirmed { input_size, bundles }
            }
            6 => ServerMsg::TimeSync(TimeSync {
                window_from: r.u64()?,
                finalized: r.u64()?,
                min_slack_us: r.i32()?,
                avg_slack_us: r.i32()?,
                samples: r.u32()?,
                late: r.u32()?,
            }),
            7 => {
                let tick = r.u64()?;
                let n = r.count(9, MAX_REPORTS)?;
                let mut reports = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    reports.push((r.u8()?, r.u64()?));
                }
                ServerMsg::Desync { tick, reports }
            }
            8 => ServerMsg::SnapshotRequest { request_id: r.u32()? },
            9 => ServerMsg::JoinSnapshot { tick: r.u64()?, checksum: r.u64()?, data: r.bytes(MAX_SNAPSHOT_LEN)? },
            10 => ServerMsg::Presence { slot: r.u8()?, present: r.u8()? != 0, from_tick: r.u64()? },
            11 => ServerMsg::Bye { code: r.u8()? },
            k => return Err(ProtoError::UnknownKind(k)),
        };
        r.finish()?;
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bundle(tick: u64) -> Bundle {
        Bundle {
            tick,
            slots: vec![
                SlotConfirmed { input: vec![1, 2, 3, 4], commands: vec![vec![9, 9]], flags: 0 },
                SlotConfirmed { input: vec![5, 6, 7, 8], commands: vec![], flags: FLAG_REPEATED },
            ],
        }
    }

    fn all_client() -> Vec<ClientMsg> {
        vec![
            ClientMsg::Hello(Hello { build_hash: 7, room: 3, input_size: 4, want_slot: NO_SLOT, token: 0 }),
            ClientMsg::Input {
                slot: 1,
                input_size: 4,
                ack_tick: 9,
                entries: vec![InputEntry { tick: 10, input: vec![1, 2, 3, 4], commands: vec![vec![1], vec![2, 3]] }],
            },
            ClientMsg::Ping { seq: 1, client_time_us: 99, rtt_hint_us: 5 },
            ClientMsg::Checksum { tick: 30, checksum: 0xDEAD },
            ClientMsg::SnapshotUpload { request_id: 4, tick: 60, checksum: 1, data: vec![7; 100] },
            ClientMsg::SnapshotDecline { request_id: 4 },
            ClientMsg::Ready,
            ClientMsg::Leave,
        ]
    }

    fn all_server() -> Vec<ServerMsg> {
        vec![
            ServerMsg::Welcome(Welcome {
                room: 1,
                slot: 2,
                player_count: 4,
                tick_rate: 60,
                seed: 5,
                build_hash: 6,
                input_size: 4,
                checksum_interval: 30,
                token: 77,
                running: true,
                t0_us: 1000,
                server_time_us: 2000,
                finalized_tick: 12,
                config: vec![1, 2, 3],
            }),
            ServerMsg::Reject(RejectReason::BuildHashMismatch { server: 1, client: 2 }),
            ServerMsg::Start { t0_us: 5, server_time_us: 6 },
            ServerMsg::Pong { seq: 1, client_time_us: 2, server_time_us: 3, t0_us: u64::MAX, finalized_tick: 0 },
            ServerMsg::Confirmed { input_size: 4, bundles: vec![sample_bundle(5), sample_bundle(6)] },
            ServerMsg::TimeSync(TimeSync {
                window_from: 1,
                finalized: 6,
                min_slack_us: -500,
                avg_slack_us: 9000,
                samples: 6,
                late: 1,
            }),
            ServerMsg::Desync { tick: 30, reports: vec![(0, 1), (1, 2)] },
            ServerMsg::SnapshotRequest { request_id: 3 },
            ServerMsg::JoinSnapshot { tick: 90, checksum: 8, data: vec![1; 50] },
            ServerMsg::Presence { slot: 1, present: false, from_tick: 44 },
            ServerMsg::Bye { code: 2 },
        ]
    }

    #[test]
    fn round_trip() {
        for m in all_client() {
            assert_eq!(ClientMsg::decode(&m.encode()).unwrap(), m);
        }
        for m in all_server() {
            assert_eq!(ServerMsg::decode(&m.encode()).unwrap(), m);
        }
    }

    #[test]
    fn every_truncation_and_bit_flip_is_an_error_not_a_panic() {
        for m in all_client() {
            let bytes = m.encode();
            for len in 0..bytes.len() {
                assert!(ClientMsg::decode(&bytes[..len]).is_err());
            }
            for i in 0..bytes.len() {
                let mut b = bytes.clone();
                b[i] ^= 0x40;
                assert!(ClientMsg::decode(&b).is_err(), "flip at {i} accepted");
            }
        }
        for m in all_server() {
            let bytes = m.encode();
            for len in 0..bytes.len() {
                assert!(ServerMsg::decode(&bytes[..len]).is_err());
            }
            for i in 0..bytes.len() {
                let mut b = bytes.clone();
                b[i] ^= 0x40;
                assert!(ServerMsg::decode(&b).is_err(), "flip at {i} accepted");
            }
        }
    }

    #[test]
    fn a_client_message_is_not_a_server_message() {
        // Same kind numbers, different meaning: decoding with the wrong
        // side must not panic (it may error or decode something else).
        for m in all_client() {
            let _ = ServerMsg::decode(&m.encode());
        }
        for m in all_server() {
            let _ = ClientMsg::decode(&m.encode());
        }
    }
}
