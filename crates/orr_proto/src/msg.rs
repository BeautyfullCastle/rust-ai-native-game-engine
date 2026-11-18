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
//! | 1 | `Hello` (with the client's protocol version; v5) | reliable |
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
//! | 11 | `Bye` (code: see `BYE_*`) | reliable |
//! | 12 | `Correction` (authoritative mode: server snapshot for a diverged client; v3) | reliable |
//! | 13 | `SeatChange` (a returning player or an expired seat; v3) | reliable |
//!
//! # Versions
//!
//! A message carries the lowest protocol version that can express it, so the
//! server messages of a relay room without signals are byte-identical to
//! version 2 and old clients can read them. The client `Input` and the
//! `Hello` of a version 5 client are the exception (see version 5 below).
//! Version 3 adds the authoritative mode: `Welcome`
//! with a trailing `flags` byte (only written when a flag is set) and
//! `Correction`. It also adds `SeatChange` for the player signals: a slot's
//! player came back (`SEAT_RECONNECTED`) or its reserved seat ran out
//! (`SEAT_REMOVED`). `Presence` is unchanged: `present = true` for a slot
//! that had no player is a new player (`Added`), `present = false` is a
//! player that left while its seat is still reserved (`Disconnected`). A
//! decoder accepts `MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION`;
//! a version-2 client cannot join an authoritative room (it fails to decode
//! the version-3 `Welcome`).
//!
//! Version 4 adds the signal fence. The server numbers its player signals
//! (`Presence` of a relay room and `SeatChange`) 1, 2, 3, ... in the order it
//! announces them, and every signal message carries its `seq`. A `Bundle`
//! carries `signals`: how many signals have a tick at or before the bundle's
//! tick. A client must have all of them before it may verify that tick, so a
//! signal that is late on the reliable channel holds the bundles back instead
//! of arriving for a tick that is already verified. `Confirmed` writes the
//! counts after the bundles (see below), `JoinSnapshot` carries the count at
//! the snapshot tick. A message whose counts are all zero (an authoritative
//! room, or a room that never had a signal) is still written as version 2 or 3.
//!
//! Version 5 adds the client's side of the compact encoding. `Hello` ends with
//! `client_version`, the newest protocol version the client speaks (a `Hello`
//! without it, `client_version == 0`, is written as version 2: a client older
//! than version 5, which can be version 2, 3 or 4). The server refuses a
//! client below `RoomConfig::min_client_version` with `RejectReason::BadVersion`,
//! which names the oldest version it accepts. A version 5 `Input` writes, per
//! entry, a flags byte, then the tick (a full `u64` for the first entry, later
//! a one-byte delta to the previous entry, 255 escapes to a full `u64`), then
//! the input bytes, then the commands only if the flags say so. An entry whose
//! input equals the input of the entry before it in the same message has the
//! `INPUT_SAME` flag and leaves the input bytes out. The first entry is never
//! "same". The rule looks at the same message only, never at an earlier
//! packet or at what the server acked. Every `Input` packet is then decodable
//! alone, so a lost or reordered packet cannot make the server read a wrong
//! input. A client sends a version 5 `Input` always. A server decodes the
//! version 2 form of old clients as before.
//!
//! `Confirmed` is compact, because the server repeats unacked bundles in
//! every packet: the first bundle has a `u64` tick, later ones a one-byte
//! delta (255 escapes to a full `u64`); a slot's flags byte tells whether
//! its input equals the same slot's input in the previous bundle of the
//! message (then the input bytes are left out) and whether commands follow.
//! In version 4 a `Confirmed` with a nonzero signal count ends with: the
//! count of the first bundle, the number of later bundles whose count differs
//! from the bundle before, and for each such bundle its index and count (all
//! varints). Counts that stay the same cost two bytes per message.
use crate::codec::{Reader, Writer};

/// Newest protocol version (adds the compact client `Input` and the version
/// in `Hello`, see the module docs).
pub const PROTOCOL_VERSION: u32 = 5;
/// Oldest version a decoder still accepts (the relay-only protocol).
pub const MIN_PROTOCOL_VERSION: u32 = 2;
/// First version that has `Welcome::flags` and `ServerMsg::Correction`.
const V_AUTHORITATIVE: u32 = 3;
/// First version with signal sequence numbers and counts (the signal fence).
const V_SIGNAL_FENCE: u32 = 4;
/// First version with the compact client `Input` and `Hello::client_version`.
const V_CLIENT_COMPACT: u32 = 5;
const MAGIC: &[u8; 4] = b"ORRN";

/// `Hello::want_slot` value meaning "any free slot".
pub const NO_SLOT: u8 = 0xFF;

/// Slot number the server uses for itself in `Desync` reports and `.orrd`
/// dumps (its own checksum, in authoritative mode).
pub const SERVER_SLOT: u8 = 0xFF;

/// `SeatChange::change`: the player of the slot came back with its token.
pub const SEAT_RECONNECTED: u8 = 1;
/// `SeatChange::change`: the reserved seat of the slot ran out; the slot is free.
pub const SEAT_REMOVED: u8 = 2;

/// `Welcome::flags`: the room is authoritative (the server simulates, judges
/// checksums and sends `Correction`s).
pub const WELCOME_AUTHORITATIVE: u8 = 1;

/// `Bye` code: the client fell further behind than the server keeps bundles.
pub const BYE_BEHIND: u8 = 1;
/// `Bye` code: kicked for diverging again and again after corrections.
pub const BYE_KICKED_DESYNC: u8 = 2;
/// `Bye` code: kicked for violating the authoritative checks (cheating).
pub const BYE_KICKED_CHEAT: u8 = 3;

/// `SlotConfirmed::flags`: the server did not receive this slot's input for
/// the tick in time and filled it with the slot's previous input.
pub const FLAG_REPEATED: u8 = 1;
/// `SlotConfirmed::flags`: nobody plays this slot right now.
pub const FLAG_ABSENT: u8 = 2;
/// Wire only: the slot has commands (else no command count follows).
const WIRE_HAS_COMMANDS: u8 = 4;
/// Wire only: the input equals this slot's input in the previous bundle of
/// the same message (the input bytes are left out).
const WIRE_SAME_INPUT: u8 = 8;
/// Wire only, `Input` entry: the input equals the previous entry's input.
const INPUT_SAME: u8 = 1;
/// Wire only, `Input` entry: commands follow.
const INPUT_HAS_COMMANDS: u8 = 2;
/// Wire only: after a bundle tick delta of this value, a full `u64` tick.
const TICK_ESCAPE: u8 = 255;

pub const MAX_INPUT_SIZE: u32 = 1024;
pub const MAX_SLOTS: u8 = 64;
/// Most commands one slot can submit per tick, and the largest encoded command.
pub const MAX_COMMANDS_PER_SLOT: usize = 64;
pub const MAX_COMMAND_BYTES: usize = 4096;
const MAX_COMMANDS: u32 = MAX_COMMANDS_PER_SLOT as u32;
const MAX_COMMAND_LEN: u32 = MAX_COMMAND_BYTES as u32;
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
    /// How many player signals of the room have a tick at or before `tick`
    /// (the signal fence, see the module docs). `0` when there are none.
    pub signals: u32,
    pub slots: Vec<SlotConfirmed>,
}

fn commands_len(commands: &[Vec<u8>]) -> usize {
    if commands.is_empty() {
        0
    } else {
        4 + commands.iter().map(|c| 4 + c.len()).sum::<usize>()
    }
}

impl Bundle {
    /// Upper bound of this bundle's bytes inside a `Confirmed` message (as
    /// the first bundle: full tick, every input in full).
    pub fn encoded_len(&self, input_size: u32) -> usize {
        8 + self.slots.iter().map(|s| 1 + input_size as usize + commands_len(&s.commands)).sum::<usize>()
    }

    /// Bytes of this bundle inside a `Confirmed` message that has `prev` as
    /// the bundle before it: a one-byte tick delta, and inputs left out where
    /// they equal `prev`'s.
    pub fn encoded_len_after(&self, prev: &Bundle, input_size: u32) -> usize {
        let tick = if prev.tick < self.tick && self.tick - prev.tick < u64::from(TICK_ESCAPE) { 1 } else { 9 };
        let mut n = tick;
        for (i, s) in self.slots.iter().enumerate() {
            let same = prev.slots.get(i).is_some_and(|p| p.input == s.input);
            n += 1 + if same { 0 } else { input_size as usize } + commands_len(&s.commands);
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
    /// The newest protocol version the client speaks (`PROTOCOL_VERSION` of
    /// its build), or `0` when it does not say (a client older than version 5).
    pub client_version: u32,
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
    /// `WELCOME_*` bits. Nonzero needs protocol version 3 on the wire.
    pub flags: u8,
}

impl Welcome {
    /// Whether the room is in authoritative mode.
    pub fn authoritative(&self) -> bool {
        self.flags & WELCOME_AUTHORITATIVE != 0
    }
}

/// Why a `Hello` was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    BuildHashMismatch { server: u64, client: u64 },
    InputSizeMismatch { server: u32, client: u32 },
    /// `server`: the newest protocol version the server speaks. `min_client`:
    /// the oldest client version it accepts (`0`: not said).
    BadVersion { server: u32, min_client: u32 },
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
            RejectReason::BadVersion { server, min_client } => (3, u64::from(server), u64::from(min_client)),
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
            3 => RejectReason::BadVersion { server: a as u32, min_client: b as u32 },
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
    /// `signals`: the signal count at `tick` (see [`Bundle::signals`]).
    JoinSnapshot { tick: u64, checksum: u64, data: Vec<u8>, signals: u32 },
    /// `seq`: the number of this signal (see the module docs), `0` for a
    /// message that is no signal (authoritative rooms).
    Presence { slot: u8, present: bool, from_tick: u64, seq: u32 },
    Bye { code: u8 },
    /// Authoritative mode: the client's checksum at `from_tick` differed from
    /// the server's. `data` is the server's frame at `tick` (lz4, the same
    /// format as `JoinSnapshot`), `checksum` its checksum. The client
    /// restores it and goes on with the confirmed bundles after `tick`.
    Correction { from_tick: u64, tick: u64, checksum: u64, data: Vec<u8> },
    /// A change of a slot that `Presence` cannot say: `change` is
    /// `SEAT_RECONNECTED` or `SEAT_REMOVED`. It takes effect from `from_tick`,
    /// like `Presence`.
    SeatChange { slot: u8, change: u8, from_tick: u64, seq: u32 },
}

fn seal(mut w: Writer) -> Vec<u8> {
    let sum = xxhash_rust::xxh3::xxh3_64(&w.out);
    w.u64(sum);
    w.out
}

fn open(bytes: &[u8]) -> Result<(u8, u32, Reader<'_>), ProtoError> {
    let Some(split) = bytes.len().checked_sub(8) else { return Err(ProtoError::Truncated) };
    let (body, tail) = bytes.split_at(split);
    let mut r = Reader::new(body);
    if r.take(4).map_err(|_| ProtoError::Truncated)? != MAGIC {
        return Err(ProtoError::BadMagic);
    }
    let version = r.u32()?;
    if !(MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&version) {
        return Err(ProtoError::UnsupportedVersion(version));
    }
    if u64::from_le_bytes(tail.try_into().unwrap()) != xxhash_rust::xxh3::xxh3_64(body) {
        return Err(ProtoError::BadChecksum);
    }
    let kind = r.u8()?;
    Ok((kind, version, r))
}

/// Starts a version-2 message (everything the relay protocol had).
fn start(kind: u8) -> Writer {
    start_v(kind, MIN_PROTOCOL_VERSION)
}

fn start_v(kind: u8, version: u32) -> Writer {
    let mut w = Writer::new();
    w.raw(MAGIC);
    w.u32(version);
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
                let mut w = start_v(1, if h.client_version != 0 { V_CLIENT_COMPACT } else { MIN_PROTOCOL_VERSION });
                w.u64(h.build_hash);
                w.u64(h.room);
                w.u32(h.input_size);
                w.u8(h.want_slot);
                w.u64(h.token);
                if h.client_version != 0 {
                    w.u32(h.client_version);
                }
                seal(w)
            }
            ClientMsg::Input { slot, input_size, ack_tick, entries } => {
                let mut w = start_v(2, V_CLIENT_COMPACT);
                w.u8(*slot);
                w.u32(*input_size);
                w.u64(*ack_tick);
                w.varint(entries.len() as u32);
                let mut prev: Option<&InputEntry> = None;
                for e in entries {
                    debug_assert_eq!(e.input.len() as u32, *input_size);
                    let same = prev.is_some_and(|p| p.input == e.input);
                    let mut flags = 0;
                    if same {
                        flags |= INPUT_SAME;
                    }
                    if !e.commands.is_empty() {
                        flags |= INPUT_HAS_COMMANDS;
                    }
                    w.u8(flags);
                    match prev {
                        None => w.u64(e.tick),
                        Some(p) if p.tick < e.tick && e.tick - p.tick < u64::from(TICK_ESCAPE) => w.u8((e.tick - p.tick) as u8),
                        Some(_) => {
                            w.u8(TICK_ESCAPE);
                            w.u64(e.tick);
                        }
                    }
                    if !same {
                        w.raw(&e.input);
                    }
                    if !e.commands.is_empty() {
                        write_commands(&mut w, &e.commands);
                    }
                    prev = Some(e);
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
        let (kind, version, mut r) = open(bytes)?;
        let msg = match kind {
            1 => ClientMsg::Hello(Hello {
                build_hash: r.u64()?,
                room: r.u64()?,
                input_size: r.u32()?,
                want_slot: r.u8()?,
                token: r.u64()?,
                client_version: if version >= V_CLIENT_COMPACT { r.u32()? } else { 0 },
            }),
            2 if version >= V_CLIENT_COMPACT => {
                let slot = r.u8()?;
                let input_size = r.u32()?;
                check_input_size(input_size)?;
                let ack_tick = r.u64()?;
                let n = r.count_varint(2, MAX_ENTRIES)?;
                let mut entries: Vec<InputEntry> = Vec::with_capacity(n as usize);
                for k in 0..n as usize {
                    let flags = r.u8()?;
                    if flags & !(INPUT_SAME | INPUT_HAS_COMMANDS) != 0 {
                        return Err(ProtoError::Corrupt("input flags"));
                    }
                    let tick = if k == 0 {
                        r.u64()?
                    } else {
                        match r.u8()? {
                            TICK_ESCAPE => r.u64()?,
                            0 => return Err(ProtoError::Corrupt("tick delta 0")),
                            d => entries[k - 1].tick.checked_add(u64::from(d)).ok_or(ProtoError::Corrupt("tick overflow"))?,
                        }
                    };
                    let input = if flags & INPUT_SAME != 0 {
                        if k == 0 {
                            return Err(ProtoError::Corrupt("same input without a previous entry"));
                        }
                        entries[k - 1].input.clone()
                    } else {
                        read_input(&mut r, input_size)?
                    };
                    let commands = if flags & INPUT_HAS_COMMANDS != 0 { read_commands(&mut r)? } else { Vec::new() };
                    entries.push(InputEntry { tick, input, commands });
                }
                ClientMsg::Input { slot, input_size, ack_tick, entries }
            }
            2 => {
                // Version 2 to 4 clients: a full entry each.
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
                let mut w = start_v(1, if m.flags != 0 { V_AUTHORITATIVE } else { MIN_PROTOCOL_VERSION });
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
                if m.flags != 0 {
                    w.u8(m.flags);
                }
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
                let fenced = bundles.iter().any(|b| b.signals != 0);
                let mut w = start_v(5, if fenced { V_SIGNAL_FENCE } else { MIN_PROTOCOL_VERSION });
                w.u32(*input_size);
                let slots = bundles.first().map_or(0, |b| b.slots.len());
                w.u8(slots as u8);
                w.u32(bundles.len() as u32);
                let mut prev: Option<&Bundle> = None;
                for b in bundles {
                    debug_assert_eq!(b.slots.len(), slots);
                    match prev {
                        None => w.u64(b.tick),
                        Some(p) if p.tick < b.tick && b.tick - p.tick < u64::from(TICK_ESCAPE) => {
                            w.u8((b.tick - p.tick) as u8)
                        }
                        Some(_) => {
                            w.u8(TICK_ESCAPE);
                            w.u64(b.tick);
                        }
                    }
                    for (i, s) in b.slots.iter().enumerate() {
                        debug_assert_eq!(s.input.len() as u32, *input_size);
                        let same = prev.and_then(|p| p.slots.get(i)).is_some_and(|p| p.input == s.input);
                        let mut flags = s.flags & (FLAG_REPEATED | FLAG_ABSENT);
                        if same {
                            flags |= WIRE_SAME_INPUT;
                        }
                        if !s.commands.is_empty() {
                            flags |= WIRE_HAS_COMMANDS;
                        }
                        w.u8(flags);
                        if !same {
                            w.raw(&s.input);
                        }
                        if !s.commands.is_empty() {
                            write_commands(&mut w, &s.commands);
                        }
                    }
                    prev = Some(b);
                }
                if fenced {
                    w.varint(bundles[0].signals);
                    let changes: Vec<(usize, u32)> = bundles
                        .windows(2)
                        .enumerate()
                        .filter(|(_, pair)| pair[0].signals != pair[1].signals)
                        .map(|(i, pair)| (i + 1, pair[1].signals))
                        .collect();
                    w.varint(changes.len() as u32);
                    for (index, signals) in changes {
                        w.varint(index as u32);
                        w.varint(signals);
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
            ServerMsg::JoinSnapshot { tick, checksum, data, signals } => {
                let mut w = start_v(9, if *signals != 0 { V_SIGNAL_FENCE } else { MIN_PROTOCOL_VERSION });
                w.u64(*tick);
                w.u64(*checksum);
                w.bytes(data);
                if *signals != 0 {
                    w.varint(*signals);
                }
                seal(w)
            }
            ServerMsg::Presence { slot, present, from_tick, seq } => {
                let mut w = start_v(10, if *seq != 0 { V_SIGNAL_FENCE } else { MIN_PROTOCOL_VERSION });
                w.u8(*slot);
                w.u8(u8::from(*present));
                w.u64(*from_tick);
                if *seq != 0 {
                    w.varint(*seq);
                }
                seal(w)
            }
            ServerMsg::Bye { code } => {
                let mut w = start(11);
                w.u8(*code);
                seal(w)
            }
            ServerMsg::Correction { from_tick, tick, checksum, data } => {
                let mut w = start_v(12, V_AUTHORITATIVE);
                w.u64(*from_tick);
                w.u64(*tick);
                w.u64(*checksum);
                w.bytes(data);
                seal(w)
            }
            ServerMsg::SeatChange { slot, change, from_tick, seq } => {
                let mut w = start_v(13, if *seq != 0 { V_SIGNAL_FENCE } else { V_AUTHORITATIVE });
                w.u8(*slot);
                w.u8(*change);
                w.u64(*from_tick);
                if *seq != 0 {
                    w.varint(*seq);
                }
                seal(w)
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtoError> {
        let (kind, version, mut r) = open(bytes)?;
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
                flags: if version >= V_AUTHORITATIVE { r.u8()? } else { 0 },
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
                let per_bundle = 1 + u64::from(slots);
                let n = r.count(per_bundle, MAX_BUNDLES)?;
                let mut bundles: Vec<Bundle> = Vec::with_capacity(n as usize);
                for k in 0..n as usize {
                    let tick = if k == 0 {
                        r.u64()?
                    } else {
                        let prev_tick = bundles[k - 1].tick;
                        match r.u8()? {
                            TICK_ESCAPE => r.u64()?,
                            0 => return Err(ProtoError::Corrupt("tick delta 0")),
                            d => prev_tick.checked_add(u64::from(d)).ok_or(ProtoError::Corrupt("tick overflow"))?,
                        }
                    };
                    let mut list: Vec<SlotConfirmed> = Vec::with_capacity(slots as usize);
                    for i in 0..slots as usize {
                        let wire = r.u8()?;
                        if wire & !(FLAG_REPEATED | FLAG_ABSENT | WIRE_HAS_COMMANDS | WIRE_SAME_INPUT) != 0 {
                            return Err(ProtoError::Corrupt("flags"));
                        }
                        let input = if wire & WIRE_SAME_INPUT != 0 {
                            if k == 0 {
                                return Err(ProtoError::Corrupt("same input without a previous bundle"));
                            }
                            bundles[k - 1].slots[i].input.clone()
                        } else {
                            read_input(&mut r, input_size)?
                        };
                        let commands = if wire & WIRE_HAS_COMMANDS != 0 { read_commands(&mut r)? } else { Vec::new() };
                        list.push(SlotConfirmed { input, commands, flags: wire & (FLAG_REPEATED | FLAG_ABSENT) });
                    }
                    bundles.push(Bundle { tick, signals: 0, slots: list });
                }
                if version >= V_SIGNAL_FENCE && !bundles.is_empty() {
                    let mut signals = r.varint()?;
                    let changes = r.varint()?;
                    if u64::from(changes) >= bundles.len() as u64 {
                        return Err(ProtoError::Corrupt("signal count changes"));
                    }
                    let mut at = 0usize;
                    let mut last_index = 0u64;
                    for c in 0..=changes {
                        // The run `at..end` has `signals`; the next change starts the next run.
                        let (end, next) = if c < changes {
                            let index = u64::from(r.varint()?);
                            if index <= last_index || index >= bundles.len() as u64 {
                                return Err(ProtoError::Corrupt("signal count index"));
                            }
                            last_index = index;
                            (index as usize, Some(r.varint()?))
                        } else {
                            (bundles.len(), None)
                        };
                        for b in &mut bundles[at..end] {
                            b.signals = signals;
                        }
                        at = end;
                        if let Some(n) = next {
                            signals = n;
                        }
                    }
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
            9 => ServerMsg::JoinSnapshot {
                tick: r.u64()?,
                checksum: r.u64()?,
                data: r.bytes(MAX_SNAPSHOT_LEN)?,
                signals: if version >= V_SIGNAL_FENCE { r.varint()? } else { 0 },
            },
            10 => ServerMsg::Presence {
                slot: r.u8()?,
                present: r.u8()? != 0,
                from_tick: r.u64()?,
                seq: if version >= V_SIGNAL_FENCE { r.varint()? } else { 0 },
            },
            11 => ServerMsg::Bye { code: r.u8()? },
            12 if version >= V_AUTHORITATIVE => ServerMsg::Correction {
                from_tick: r.u64()?,
                tick: r.u64()?,
                checksum: r.u64()?,
                data: r.bytes(MAX_SNAPSHOT_LEN)?,
            },
            13 if version >= V_AUTHORITATIVE => {
                let (slot, change, from_tick) = (r.u8()?, r.u8()?, r.u64()?);
                if change != SEAT_RECONNECTED && change != SEAT_REMOVED {
                    return Err(ProtoError::Corrupt("unknown seat change"));
                }
                let seq = if version >= V_SIGNAL_FENCE { r.varint()? } else { 0 };
                ServerMsg::SeatChange { slot, change, from_tick, seq }
            }
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
            signals: 0,
            slots: vec![
                SlotConfirmed { input: vec![1, 2, 3, 4], commands: vec![vec![9, 9]], flags: 0 },
                SlotConfirmed { input: vec![5, 6, 7, 8], commands: vec![], flags: FLAG_REPEATED },
            ],
        }
    }

    fn fenced_bundle(tick: u64, signals: u32) -> Bundle {
        Bundle { signals, ..sample_bundle(tick) }
    }

    /// Same inputs next to each other and apart, a tick gap, an escaped tick,
    /// commands on a "same" entry.
    fn mixed_entries() -> Vec<InputEntry> {
        let e = |tick: u64, byte: u8, commands: Vec<Vec<u8>>| InputEntry { tick, input: vec![byte; 4], commands };
        vec![
            e(100, 1, vec![]),
            e(101, 1, vec![]),
            e(102, 2, vec![vec![5]]),
            e(103, 2, vec![vec![6, 7]]),
            e(150, 2, vec![]),
            e(500, 1, vec![]),
            e(499, 1, vec![]),
            e(u64::MAX, 3, vec![]),
        ]
    }

    fn all_client() -> Vec<ClientMsg> {
        vec![
            ClientMsg::Hello(Hello { build_hash: 7, room: 3, input_size: 4, want_slot: NO_SLOT, token: 0, client_version: 0 }),
            ClientMsg::Hello(Hello {
                build_hash: 7,
                room: 3,
                input_size: 4,
                want_slot: NO_SLOT,
                token: 0,
                client_version: PROTOCOL_VERSION,
            }),
            ClientMsg::Input {
                slot: 1,
                input_size: 4,
                ack_tick: 9,
                entries: vec![InputEntry { tick: 10, input: vec![1, 2, 3, 4], commands: vec![vec![1], vec![2, 3]] }],
            },
            ClientMsg::Input { slot: 1, input_size: 4, ack_tick: 9, entries: vec![] },
            ClientMsg::Input { slot: 1, input_size: 4, ack_tick: 9, entries: mixed_entries() },
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
                flags: 0,
            }),
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
                running: false,
                t0_us: 0,
                server_time_us: 2000,
                finalized_tick: 0,
                config: vec![],
                flags: WELCOME_AUTHORITATIVE,
            }),
            ServerMsg::Reject(RejectReason::BuildHashMismatch { server: 1, client: 2 }),
            ServerMsg::Reject(RejectReason::BadVersion { server: 5, min_client: 4 }),
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
            ServerMsg::JoinSnapshot { tick: 90, checksum: 8, data: vec![1; 50], signals: 0 },
            ServerMsg::JoinSnapshot { tick: 90, checksum: 8, data: vec![1; 50], signals: 300 },
            ServerMsg::Presence { slot: 1, present: false, from_tick: 44, seq: 0 },
            ServerMsg::Presence { slot: 1, present: true, from_tick: 44, seq: 3 },
            ServerMsg::Bye { code: 2 },
            ServerMsg::Correction { from_tick: 60, tick: 91, checksum: 0xABCD, data: vec![7; 40] },
            ServerMsg::SeatChange { slot: 2, change: SEAT_RECONNECTED, from_tick: 77, seq: 0 },
            ServerMsg::SeatChange { slot: 0, change: SEAT_REMOVED, from_tick: 900, seq: 129 },
            ServerMsg::Confirmed { input_size: 4, bundles: vec![fenced_bundle(5, 2), fenced_bundle(6, 2), fenced_bundle(9, 7)] },
        ]
    }

    fn version_of(bytes: &[u8]) -> u32 {
        u32::from_le_bytes(bytes[4..8].try_into().unwrap())
    }

    #[test]
    fn messages_without_signals_stay_version_2_and_authoritative_ones_are_version_3() {
        for m in all_client() {
            // An input and a Hello that states its version are version 5.
            let v5 = matches!(&m, ClientMsg::Input { .. }) || matches!(&m, ClientMsg::Hello(h) if h.client_version != 0);
            assert_eq!(version_of(&m.encode()), if v5 { 5 } else { 2 }, "{m:?}");
        }
        for m in all_server() {
            let v = version_of(&m.encode());
            let v4 = match &m {
                ServerMsg::JoinSnapshot { signals, .. } => *signals != 0,
                ServerMsg::Presence { seq, .. } | ServerMsg::SeatChange { seq, .. } => *seq != 0,
                ServerMsg::Confirmed { bundles, .. } => bundles.iter().any(|b| b.signals != 0),
                _ => false,
            };
            let v3 = matches!(&m, ServerMsg::Correction { .. } | ServerMsg::SeatChange { .. }) || matches!(&m, ServerMsg::Welcome(w) if w.flags != 0);
            assert_eq!(v, if v4 { 4 } else if v3 { 3 } else { 2 }, "{m:?}");
        }
    }

    #[test]
    fn a_future_version_and_a_correction_in_a_version_2_message_are_refused() {
        let mut bytes = ServerMsg::Bye { code: 1 }.encode();
        bytes[4..8].copy_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());
        // The header checksum no longer matches, so re-seal.
        let n = bytes.len() - 8;
        let sum = xxhash_rust::xxh3::xxh3_64(&bytes[..n]);
        bytes[n..].copy_from_slice(&sum.to_le_bytes());
        assert_eq!(ServerMsg::decode(&bytes), Err(ProtoError::UnsupportedVersion(PROTOCOL_VERSION + 1)));

        let mut bytes = ServerMsg::Correction { from_tick: 1, tick: 2, checksum: 3, data: vec![] }.encode();
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
        let n = bytes.len() - 8;
        let sum = xxhash_rust::xxh3::xxh3_64(&bytes[..n]);
        bytes[n..].copy_from_slice(&sum.to_le_bytes());
        assert_eq!(ServerMsg::decode(&bytes), Err(ProtoError::UnknownKind(12)));
    }

    #[test]
    fn a_seat_change_needs_version_3_and_a_known_change() {
        let m = ServerMsg::SeatChange { slot: 1, change: SEAT_REMOVED, from_tick: 5, seq: 0 };
        let reseal = |bytes: &mut Vec<u8>| {
            let n = bytes.len() - 8;
            let sum = xxhash_rust::xxh3::xxh3_64(&bytes[..n]);
            bytes[n..].copy_from_slice(&sum.to_le_bytes());
        };
        let mut bytes = m.encode();
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
        reseal(&mut bytes);
        assert_eq!(ServerMsg::decode(&bytes), Err(ProtoError::UnknownKind(13)));

        let mut bytes = m.encode();
        bytes[9 + 1] = 9; // kind at 8, slot at 9, change at 10
        reseal(&mut bytes);
        assert_eq!(ServerMsg::decode(&bytes), Err(ProtoError::Corrupt("unknown seat change")));
    }

    #[test]
    fn confirmed_compact_encoding_round_trips_and_shrinks() {
        let b = |tick: u64, a: u8, c: u8, cmds: bool| Bundle {
            tick,
            signals: 0,
            slots: vec![
                SlotConfirmed { input: vec![a; 4], commands: if cmds { vec![vec![1, 2]] } else { vec![] }, flags: 0 },
                SlotConfirmed { input: vec![c; 4], commands: vec![], flags: FLAG_ABSENT | FLAG_REPEATED },
            ],
        };
        // Same inputs, changed input, a tick gap, a gap over the delta range, a backwards tick.
        let bundles = vec![b(10, 1, 2, false), b(11, 1, 2, true), b(12, 3, 2, false), b(15, 3, 9, false), b(400, 3, 9, false), b(7, 3, 9, false)];
        let msg = ServerMsg::Confirmed { input_size: 4, bundles: bundles.clone() };
        let bytes = msg.encode();
        assert_eq!(ServerMsg::decode(&bytes).unwrap(), msg);
        let full: usize = bundles.iter().map(|x| x.encoded_len(4)).sum();
        let est = bundles[0].encoded_len(4) + bundles.windows(2).map(|w| w[1].encoded_len_after(&w[0], 4)).sum::<usize>();
        assert!(est < full);
        // header: magic 4 + version 4 + kind 1 + input_size 4 + slots 1 + count 4 + checksum 8
        assert_eq!(bytes.len(), est + 26);
    }

    #[test]
    fn signal_counts_cost_two_bytes_while_they_stay_the_same_and_round_trip() {
        let plain = ServerMsg::Confirmed { input_size: 4, bundles: vec![sample_bundle(5), sample_bundle(6), sample_bundle(7)] };
        let same = ServerMsg::Confirmed { input_size: 4, bundles: (5..8).map(|t| fenced_bundle(t, 3)).collect() };
        assert_eq!(same.encode().len(), plain.encode().len() + 2);
        // Counts that rise inside the message: one more index and count for every change.
        let rising = ServerMsg::Confirmed {
            input_size: 4,
            bundles: vec![fenced_bundle(5, 3), fenced_bundle(6, 3), fenced_bundle(7, 4), fenced_bundle(8, 4), fenced_bundle(9, 200)],
        };
        assert_eq!(ServerMsg::decode(&rising.encode()).unwrap(), rising);
        assert_eq!(ServerMsg::decode(&same.encode()).unwrap(), same);
        // A count of zero in the first bundle and a rise later.
        let late = ServerMsg::Confirmed { input_size: 4, bundles: vec![sample_bundle(5), fenced_bundle(6, 1)] };
        assert_eq!(ServerMsg::decode(&late.encode()).unwrap(), late);
        // A single bundle and a message of version 4 without bundles.
        let one = ServerMsg::Confirmed { input_size: 4, bundles: vec![fenced_bundle(5, 1)] };
        assert_eq!(ServerMsg::decode(&one.encode()).unwrap(), one);
    }

    #[test]
    fn bad_signal_count_trailers_are_errors() {
        let encode = |trailer: &[u32]| {
            let mut w = start_v(5, V_SIGNAL_FENCE);
            w.u32(4);
            w.u8(1);
            w.u32(2);
            w.u64(5);
            w.u8(0);
            w.raw(&[1, 2, 3, 4]);
            w.u8(1);
            w.u8(WIRE_SAME_INPUT);
            for v in trailer {
                w.varint(*v);
            }
            seal(w)
        };
        assert!(ServerMsg::decode(&encode(&[2, 1, 1, 5])).is_ok());
        // More changes than later bundles; an index of 0 or past the end; indexes that do not rise.
        assert!(ServerMsg::decode(&encode(&[2, 2, 1, 5, 1, 6])).is_err());
        assert!(ServerMsg::decode(&encode(&[2, 1, 0, 5])).is_err());
        assert!(ServerMsg::decode(&encode(&[2, 1, 2, 5])).is_err());
        // The trailer is missing.
        assert!(ServerMsg::decode(&encode(&[])).is_err());
    }

    #[test]
    fn varint_round_trips_and_refuses_overflow() {
        for v in [0u32, 1, 127, 128, 300, 16_383, 16_384, u32::MAX] {
            let mut w = Writer::new();
            w.varint(v);
            let len = w.out.len();
            assert_eq!(len, if v < 128 { 1 } else if v < 16_384 { 2 } else if v == u32::MAX { 5 } else { 3 }, "{v}");
            assert_eq!(Reader::new(&w.out).varint().unwrap(), v);
        }
        assert!(Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0x1F]).varint().is_err());
        assert!(Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01]).varint().is_err());
    }

    #[test]
    fn confirmed_same_input_on_first_bundle_is_an_error() {
        let mut w = start(5);
        w.u32(4);
        w.u8(1);
        w.u32(1);
        w.u64(5);
        w.u8(WIRE_SAME_INPUT);
        assert!(ServerMsg::decode(&seal(w)).is_err());
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

    fn input_msg(entries: Vec<InputEntry>) -> ClientMsg {
        ClientMsg::Input { slot: 1, input_size: 24, ack_tick: 9, entries }
    }

    fn entry(tick: u64, byte: u8) -> InputEntry {
        InputEntry { tick, input: vec![byte; 24], commands: vec![] }
    }

    #[test]
    fn an_input_equal_to_the_one_before_it_costs_two_bytes_and_round_trips() {
        let one = input_msg(vec![entry(100, 7)]).encode().len();
        let same: Vec<_> = (100..104).map(|t| entry(t, 7)).collect();
        let four_same = input_msg(same).encode();
        // Each repeat is a flags byte and a one-byte tick delta.
        assert_eq!(four_same.len(), one + 3 * 2);
        let differing: Vec<_> = (100..104).map(|t| entry(t, t as u8)).collect();
        let four_different = input_msg(differing.clone()).encode();
        assert_eq!(four_different.len(), one + 3 * (2 + 24));
        // The old form: 12 bytes and the input for every entry, a 4-byte count.
        let old_form = 4 * (12 + 24) + 34;
        assert!(four_same.len() * 2 < old_form, "{} bytes against {old_form}", four_same.len());
        for entries in [(100..104).map(|t| entry(t, 7)).collect::<Vec<_>>(), differing, mixed_entries_24()] {
            let msg = input_msg(entries);
            assert_eq!(ClientMsg::decode(&msg.encode()), Ok(msg));
        }
    }

    fn mixed_entries_24() -> Vec<InputEntry> {
        mixed_entries().into_iter().map(|e| InputEntry { input: e.input.repeat(6), ..e }).collect()
    }

    #[test]
    fn a_same_flag_never_needs_another_packet() {
        // Every packet of a stream decodes alone, whichever packets are lost:
        // the "same" entries of a packet point at the entry before them in
        // that packet.
        let stream: Vec<InputEntry> = (1..=40).map(|t| entry(t, if t % 9 == 0 { 2 } else { 1 })).collect();
        for end in 4..=stream.len() {
            let window = stream[end - 4..end].to_vec();
            let msg = input_msg(window);
            assert_eq!(ClientMsg::decode(&msg.encode()), Ok(msg));
        }
    }

    #[test]
    fn a_version_2_input_still_decodes() {
        let mut w = start(2);
        w.u8(1);
        w.u32(4);
        w.u64(9);
        w.u32(2);
        for (tick, byte) in [(10u64, 1u8), (11, 1)] {
            w.u64(tick);
            w.raw(&[byte; 4]);
            write_commands(&mut w, &[]);
        }
        let want = ClientMsg::Input {
            slot: 1,
            input_size: 4,
            ack_tick: 9,
            entries: vec![
                InputEntry { tick: 10, input: vec![1; 4], commands: vec![] },
                InputEntry { tick: 11, input: vec![1; 4], commands: vec![] },
            ],
        };
        assert_eq!(ClientMsg::decode(&seal(w)), Ok(want));
    }

    #[test]
    fn bad_compact_inputs_are_errors() {
        let build = |entries: &dyn Fn(&mut Writer)| {
            let mut w = start_v(2, V_CLIENT_COMPACT);
            w.u8(1);
            w.u32(4);
            w.u64(9);
            w.varint(2);
            entries(&mut w);
            seal(w)
        };
        // The first entry cannot repeat an earlier one.
        let bytes = build(&|w| {
            w.u8(INPUT_SAME);
            w.u64(10);
            w.u8(0);
            w.u8(1);
        });
        assert!(matches!(ClientMsg::decode(&bytes), Err(ProtoError::Corrupt(_))));
        // A tick delta of zero.
        let bytes = build(&|w| {
            w.u8(0);
            w.u64(10);
            w.raw(&[1; 4]);
            w.u8(INPUT_SAME);
            w.u8(0);
        });
        assert!(matches!(ClientMsg::decode(&bytes), Err(ProtoError::Corrupt(_))));
        // An unknown flag.
        let bytes = build(&|w| {
            w.u8(0x80);
            w.u64(10);
        });
        assert!(matches!(ClientMsg::decode(&bytes), Err(ProtoError::Corrupt(_))));
        // More entries than the bytes can hold.
        let mut w = start_v(2, V_CLIENT_COMPACT);
        w.u8(1);
        w.u32(4);
        w.u64(9);
        w.varint(200);
        assert!(ClientMsg::decode(&seal(w)).is_err());
    }

    #[test]
    fn a_hello_states_its_version_and_an_old_hello_states_none() {
        let hello = Hello { build_hash: 1, room: 2, input_size: 4, want_slot: NO_SLOT, token: 3, client_version: PROTOCOL_VERSION };
        let ClientMsg::Hello(back) = ClientMsg::decode(&ClientMsg::Hello(hello.clone()).encode()).unwrap() else { panic!() };
        assert_eq!(back, hello);
        // What a version 2 to 4 client sends: no version at the end.
        let mut w = start(1);
        w.u64(1);
        w.u64(2);
        w.u32(4);
        w.u8(NO_SLOT);
        w.u64(3);
        let ClientMsg::Hello(old) = ClientMsg::decode(&seal(w)).unwrap() else { panic!() };
        assert_eq!(old, Hello { client_version: 0, ..hello });
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
