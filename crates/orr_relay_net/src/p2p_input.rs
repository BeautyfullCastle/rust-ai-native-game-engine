//! Bounded reliable two-peer inputs, separate from relay and checked-v3 controls.
//!
//! `ORRI` v1 uses explicit little-endian framing and a caller-defined portable
//! payload codec. It is not a raw-Pod format. A driver binds exactly one admitted
//! connection to an independently agreed checked context and accepted snapshot.
//! This deliberately finite source retains duplicate evidence for its entire
//! tick window. Exhaustion or a conflicting duplicate fails the whole driver.
//! Call `check` before and after each Session/bootstrap operation; InputSource's
//! legacy void methods cannot return transport failures themselves. Never keep
//! advancing a failed session. `flush` retries the same FIFO head on Backpressure.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::marker::PhantomData;
use std::rc::Rc;

use orr_net::SendError;
use orr_proto::{Channel, ConnId};
use orr_session::{CheckedJoinContext, Game, InputSource, PlayerSlot, RemoteInput};

pub const P2P_INPUT_VERSION: u16 = 1;
const MAGIC: &[u8; 4] = b"ORRI";
const HEADER: usize = 41;

/// A canonical, portable application encoding. Encode/decode each field with
/// specified widths/endianness and validate semantic ranges. Commands remain in
/// exact order, including repeats. SCHEMA must change with the encoding contract.
/// Implementations are trusted application code: they must not allocate based
/// on unchecked payload values, and must consume the complete supplied slice.
/// This interface deliberately does not use Game::Input's Pod representation or
/// SimCommand's potentially platform-native encoding as a portability promise.
pub trait P2pInputCodec<G: Game> {
    const SCHEMA: u64;
    fn encode_input(input: &G::Input, out: &mut Vec<u8>);
    fn decode_input(bytes: &[u8]) -> Option<G::Input>;
    fn encode_command(command: &G::Command, out: &mut Vec<u8>);
    fn decode_command(bytes: &[u8]) -> Option<G::Command>;
}

/// Finite session window and per-packet/retained-queue bounds. Each of the
/// duplicate ledger, outbound queue and incoming queue has the record and byte
/// limit below, so retained encoded payload is at most three times the budget.
/// Decoded commands additionally use at most max_records * max_commands objects.
#[derive(Clone, Debug)]
pub struct P2pInputLimits {
    pub first_tick: u64,
    pub end_tick: u64,
    pub max_packet_bytes: usize,
    pub max_input_bytes: usize,
    pub max_commands: usize,
    pub max_command_bytes: usize,
    pub max_records: usize,
    pub max_retained_bytes: usize,
}
impl Default for P2pInputLimits {
    fn default() -> Self {
        Self {
            first_tick: 1,
            end_tick: 4097,
            max_packet_bytes: 64 * 1024,
            max_input_bytes: 1024,
            max_commands: 64,
            max_command_bytes: 1024,
            max_records: 8192,
            max_retained_bytes: 2 * 1024 * 1024,
        }
    }
}
impl P2pInputLimits {
    fn validate(&self) -> Result<(), P2pInputError> {
        if self.first_tick >= self.end_tick
            || self.max_packet_bytes < HEADER
            || self.max_packet_bytes > u32::MAX as usize
            || self.max_input_bytes > self.max_packet_bytes
            || self.max_commands > u16::MAX as usize
            || self.max_command_bytes > self.max_packet_bytes
            || self.max_records == 0
            || self.max_retained_bytes < self.max_packet_bytes
        {
            return Err(P2pInputError::InvalidConfig);
        }
        Ok(())
    }
    fn tick(&self, tick: u64) -> Result<(), P2pInputError> {
        if !(self.first_tick..self.end_tick).contains(&tick) {
            Err(P2pInputError::TickRange)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum P2pInputError {
    InvalidConfig,
    NotBound,
    AlreadyBound,
    WrongConnection,
    WrongChannel,
    WrongContext,
    WrongSchema,
    WrongAuthority,
    TickRange,
    Malformed,
    PacketLimit,
    InputLimit,
    CommandLimit,
    RecordLimit,
    ByteLimit,
    ConflictingDuplicate { tick: u64, slot: PlayerSlot },
    Disconnected,
    Cancelled,
    ReentrantFlush,
    Transport(SendError),
}
impl core::fmt::Display for P2pInputError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "P2P input: {self:?}")
    }
}
impl std::error::Error for P2pInputError {}

/// Whether an accepted packet added a record or was an exact duplicate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum P2pInputAccepted {
    New,
    Duplicate,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct P2pInputFlush {
    pub sent: usize,
    pub pending: usize,
    pub backpressured: bool,
}

/// Encode a single input with ordered commands. Bounds are checked before the
/// final packet allocation. Application codec temporary allocations are trusted.
pub fn encode_p2p_input<G: Game, C: P2pInputCodec<G>>(
    context: &CheckedJoinContext,
    record: &RemoteInput<G>,
    limits: &P2pInputLimits,
) -> Result<Vec<u8>, P2pInputError> {
    limits.validate()?;
    limits.tick(record.tick)?;
    if record.disconnected || record.slot.0 >= 2 {
        return Err(P2pInputError::WrongAuthority);
    }
    if record.commands.len() > limits.max_commands {
        return Err(P2pInputError::CommandLimit);
    }
    let mut input = Vec::new();
    C::encode_input(&record.input, &mut input);
    if input.len() > limits.max_input_bytes {
        return Err(P2pInputError::InputLimit);
    }
    let mut size = HEADER
        .checked_add(input.len())
        .ok_or(P2pInputError::PacketLimit)?;
    let mut commands = Vec::new();
    for command in &record.commands {
        let mut bytes = Vec::new();
        C::encode_command(command, &mut bytes);
        if bytes.len() > limits.max_command_bytes {
            return Err(P2pInputError::CommandLimit);
        }
        size = size
            .checked_add(4 + bytes.len())
            .ok_or(P2pInputError::PacketLimit)?;
        if size > limits.max_packet_bytes {
            return Err(P2pInputError::PacketLimit);
        }
        commands.push(bytes);
    }
    if size > limits.max_packet_bytes {
        return Err(P2pInputError::PacketLimit);
    }
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&P2P_INPUT_VERSION.to_le_bytes());
    out.extend_from_slice(&C::SCHEMA.to_le_bytes());
    out.extend_from_slice(&context.join_id().to_le_bytes());
    out.extend_from_slice(&context.attempt().to_le_bytes());
    out.push(record.slot.0);
    out.extend_from_slice(&record.tick.to_le_bytes());
    out.extend_from_slice(&(input.len() as u32).to_le_bytes());
    out.extend_from_slice(&(commands.len() as u16).to_le_bytes());
    out.extend_from_slice(&input);
    for bytes in commands {
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&bytes);
    }
    Ok(out)
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], P2pInputError> {
        if n > self.0.len() {
            return Err(P2pInputError::Malformed);
        }
        let (out, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(out)
    }
    fn u16(&mut self) -> Result<u16, P2pInputError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, P2pInputError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, P2pInputError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

#[derive(Clone, Copy)]
struct Binding {
    conn: ConnId,
    snapshot_tick: u64,
    first_input_tick: u64,
}

struct State<G: Game, C: P2pInputCodec<G>> {
    local: PlayerSlot,
    context: CheckedJoinContext,
    limits: P2pInputLimits,
    binding: Option<Binding>,
    error: Option<P2pInputError>,
    seen: BTreeMap<(u64, PlayerSlot), Vec<u8>>,
    seen_bytes: usize,
    outgoing: VecDeque<(u64, Vec<u8>)>,
    outgoing_bytes: usize,
    incoming: VecDeque<(RemoteInput<G>, usize)>,
    incoming_bytes: usize,
    flushing: bool,
    marker: PhantomData<C>,
}
impl<G: Game, C: P2pInputCodec<G>> State<G, C> {
    fn check(&self) -> Result<(), P2pInputError> {
        self.error.clone().map_or(Ok(()), Err)
    }
    fn fail(&mut self, error: P2pInputError) -> P2pInputError {
        if self.error.is_none() {
            self.error = Some(error);
        }
        self.seen.clear();
        self.outgoing.clear();
        self.incoming.clear();
        self.seen_bytes = 0;
        self.outgoing_bytes = 0;
        self.incoming_bytes = 0;
        self.error.clone().unwrap()
    }
    fn authority(
        &self,
        author: PlayerSlot,
        slot: PlayerSlot,
        tick: u64,
    ) -> Result<(), P2pInputError> {
        self.limits.tick(tick)?;
        let allowed = match self.binding {
            Some(b) if tick <= b.snapshot_tick => false,
            Some(b) => {
                if author == PlayerSlot(0) {
                    slot == PlayerSlot(0) || (slot == PlayerSlot(1) && tick < b.first_input_tick)
                } else {
                    slot == PlayerSlot(1) && tick >= b.first_input_tick
                }
            }
            None => author == PlayerSlot(0) && slot.0 < 2,
        };
        if allowed {
            Ok(())
        } else {
            Err(P2pInputError::WrongAuthority)
        }
    }
    fn duplicate(&self, tick: u64, slot: PlayerSlot, bytes: &[u8]) -> Result<bool, P2pInputError> {
        match self.seen.get(&(tick, slot)) {
            Some(old) if old == bytes => Ok(true),
            Some(_) => Err(P2pInputError::ConflictingDuplicate { tick, slot }),
            None => Ok(false),
        }
    }
    fn budget(&self, count: usize, bytes: usize, additional: usize) -> Result<(), P2pInputError> {
        if count >= self.limits.max_records {
            return Err(P2pInputError::RecordLimit);
        }
        if additional > self.limits.max_retained_bytes.saturating_sub(bytes) {
            return Err(P2pInputError::ByteLimit);
        }
        Ok(())
    }
    fn send(&mut self, record: RemoteInput<G>) -> Result<(), P2pInputError> {
        self.check()?;
        self.authority(self.local, record.slot, record.tick)?;
        let bytes = encode_p2p_input::<G, C>(&self.context, &record, &self.limits)?;
        if self.duplicate(record.tick, record.slot, &bytes)? {
            return Ok(());
        }
        self.budget(self.seen.len(), self.seen_bytes, bytes.len())?;
        self.budget(self.outgoing.len(), self.outgoing_bytes, bytes.len())?;
        self.seen_bytes += bytes.len();
        self.outgoing_bytes += bytes.len();
        self.seen.insert((record.tick, record.slot), bytes.clone());
        self.outgoing.push_back((record.tick, bytes));
        Ok(())
    }
    fn receive(
        &mut self,
        conn: ConnId,
        channel: Channel,
        bytes: &[u8],
    ) -> Result<P2pInputAccepted, P2pInputError> {
        self.check()?;
        let binding = self.binding.ok_or(P2pInputError::NotBound)?;
        if conn != binding.conn {
            return Err(P2pInputError::WrongConnection);
        }
        if channel != Channel::Reliable {
            return Err(P2pInputError::WrongChannel);
        }
        if bytes.len() > self.limits.max_packet_bytes {
            return Err(P2pInputError::PacketLimit);
        }
        let mut r = Reader(bytes);
        if r.take(4)? != MAGIC || r.u16()? != P2P_INPUT_VERSION {
            return Err(P2pInputError::Malformed);
        }
        if r.u64()? != C::SCHEMA {
            return Err(P2pInputError::WrongSchema);
        }
        if r.u64()? != self.context.join_id() || r.u32()? != self.context.attempt() {
            return Err(P2pInputError::WrongContext);
        }
        let slot = PlayerSlot(r.take(1)?[0]);
        let tick = r.u64()?;
        self.authority(PlayerSlot(1 - self.local.0), slot, tick)?;
        let input_len = r.u32()? as usize;
        let count = usize::from(r.u16()?);
        if input_len > self.limits.max_input_bytes {
            return Err(P2pInputError::InputLimit);
        }
        if count > self.limits.max_commands {
            return Err(P2pInputError::CommandLimit);
        }
        let input = r.take(input_len)?;
        let command_bytes = r.0;
        // Validate the entire framing before allocating or calling application decoders.
        for _ in 0..count {
            let len = r.u32()? as usize;
            if len > self.limits.max_command_bytes {
                return Err(P2pInputError::CommandLimit);
            }
            r.take(len)?;
        }
        if !r.0.is_empty() {
            return Err(P2pInputError::Malformed);
        }
        if self.duplicate(tick, slot, bytes)? {
            return Ok(P2pInputAccepted::Duplicate);
        }
        self.budget(self.seen.len(), self.seen_bytes, bytes.len())?;
        self.budget(self.incoming.len(), self.incoming_bytes, bytes.len())?;
        let input = C::decode_input(input).ok_or(P2pInputError::Malformed)?;
        let mut r = Reader(command_bytes);
        let mut commands = Vec::with_capacity(count);
        for _ in 0..count {
            let len = r.u32()? as usize;
            commands.push(C::decode_command(r.take(len)?).ok_or(P2pInputError::Malformed)?);
        }
        self.seen_bytes += bytes.len();
        self.incoming_bytes += bytes.len();
        self.seen.insert((tick, slot), bytes.to_vec());
        self.incoming.push_back((
            RemoteInput {
                tick,
                slot,
                input,
                commands,
                disconnected: false,
            },
            bytes.len(),
        ));
        Ok(P2pInputAccepted::New)
    }
}

/// Application-side handle. Keep it outside Session to pump admitted input
/// packets, retry outbound FIFO, observe failures and retire all buffered input.
/// A driver is single-session/single-connection; no reconnect or repair protocol.
pub struct P2pInputDriver<G: Game, C: P2pInputCodec<G>>(Rc<RefCell<State<G, C>>>);
/// Session-side half, created exactly once together with its driver.
pub struct P2pInputSource<G: Game, C: P2pInputCodec<G>>(Rc<RefCell<State<G, C>>>);

impl<G: Game, C: P2pInputCodec<G>> P2pInputDriver<G, C> {
    /// Only the completed two-slot roster (host=0, joiner=1) is supported.
    /// The host may advance before admission; its bounded authored queue becomes
    /// the backlog after bind. A joiner must not author before bind/Ready.
    pub fn new(
        local: PlayerSlot,
        context: CheckedJoinContext,
        limits: P2pInputLimits,
    ) -> Result<(Self, P2pInputSource<G, C>), P2pInputError> {
        limits.validate()?;
        let roster = context.roster();
        if local.0 > 1
            || roster.player_count() != 2
            || roster.donor() != PlayerSlot(0)
            || roster.joiner() != PlayerSlot(1)
            || roster.peers() != [PlayerSlot(0)]
        {
            return Err(P2pInputError::InvalidConfig);
        }
        let state = Rc::new(RefCell::new(State {
            local,
            context,
            limits,
            binding: None,
            error: None,
            seen: BTreeMap::new(),
            seen_bytes: 0,
            outgoing: VecDeque::new(),
            outgoing_bytes: 0,
            incoming: VecDeque::new(),
            incoming_bytes: 0,
            flushing: false,
            marker: PhantomData,
        }));
        Ok((Self(state.clone()), P2pInputSource(state)))
    }
    /// Bind the actual admitted connection once. Derive tick bounds from the
    /// accepted checked ticket (host), or the accepted bootstrap Session's
    /// verified_tick/next_send_tick (joiner), never unvalidated packet fields.
    /// Context equality is checked before queued input can reach Session.
    pub fn bind(
        &self,
        conn: ConnId,
        context: &CheckedJoinContext,
        snapshot_tick: u64,
        first_input_tick: u64,
    ) -> Result<(), P2pInputError> {
        let mut s = self.0.borrow_mut();
        let result = (|| {
            s.check()?;
            if s.binding.is_some() {
                return Err(P2pInputError::AlreadyBound);
            }
            if &s.context != context {
                return Err(P2pInputError::WrongContext);
            }
            if snapshot_tick >= first_input_tick
                || first_input_tick < s.limits.first_tick
                || first_input_tick >= s.limits.end_tick
            {
                return Err(P2pInputError::TickRange);
            }
            s.binding = Some(Binding {
                conn,
                snapshot_tick,
                first_input_tick,
            });
            // All pre-bind evidence is locally authored. It must obey the
            // accepted grant too; a later cutoff cannot bless unauthorized data.
            for &(tick, slot) in s.seen.keys() {
                if tick > snapshot_tick {
                    s.authority(s.local, slot, tick)?;
                }
            }
            // The snapshot already contains these ticks. Retain duplicate evidence,
            // but never send old records to the freshly bootstrapped Session.
            let mut retained = VecDeque::new();
            while let Some((tick, bytes)) = s.outgoing.pop_front() {
                if tick > snapshot_tick {
                    retained.push_back((tick, bytes));
                } else {
                    s.outgoing_bytes -= bytes.len();
                }
            }
            s.outgoing = retained;
            Ok(())
        })();
        result.map_err(|e| s.fail(e))
    }
    pub fn check(&self) -> Result<(), P2pInputError> {
        self.0.borrow().check()
    }
    pub fn pending(&self) -> (usize, usize) {
        let s = self.0.borrow();
        (s.outgoing.len(), s.outgoing_bytes)
    }
    /// Invalid traffic is terminal: callers must close/retire this admitted link.
    pub fn receive(
        &self,
        conn: ConnId,
        channel: Channel,
        bytes: &[u8],
    ) -> Result<P2pInputAccepted, P2pInputError> {
        let mut s = self.0.borrow_mut();
        s.receive(conn, channel, bytes).map_err(|e| s.fail(e))
    }
    /// Retry without removing a Backpressure-blocked head. Other transport
    /// errors fail the driver and discard all buffered input explicitly.
    pub fn flush(
        &self,
        mut send: impl FnMut(ConnId, &[u8]) -> Result<(), SendError>,
    ) -> Result<P2pInputFlush, P2pInputError> {
        let (b, initial) = {
            let mut s = self.0.borrow_mut();
            s.check()?;
            if s.flushing {
                return Err(s.fail(P2pInputError::ReentrantFlush));
            }
            let b = s.binding.ok_or(P2pInputError::NotBound)?;
            s.flushing = true;
            (b, s.outgoing.len())
        };
        let result = (|| {
            let mut out = P2pInputFlush::default();
            // Never hold RefCell across caller code. Process only this initial
            // queue budget even if the callback appends more local inputs.
            for _ in 0..initial {
                let bytes = {
                    let s = self.0.borrow();
                    s.check()?;
                    let Some((_, bytes)) = s.outgoing.front() else {
                        break;
                    };
                    bytes.clone()
                };
                let result = send(b.conn, &bytes);
                let mut s = self.0.borrow_mut();
                s.check()?;
                match result {
                    Ok(()) => {
                        let (_, bytes) = s
                            .outgoing
                            .pop_front()
                            .expect("head retained while callback ran");
                        s.outgoing_bytes -= bytes.len();
                        out.sent += 1;
                    }
                    Err(SendError::Backpressure) => {
                        out.backpressured = true;
                        break;
                    }
                    Err(e) => return Err(s.fail(P2pInputError::Transport(e))),
                }
            }
            out.pending = self.0.borrow().outgoing.len();
            Ok(out)
        })();
        self.0.borrow_mut().flushing = false;
        result
    }

    /// Retire application buffers alongside P2pCleanup's exact transport/hold
    /// retirement. This does not authorize reusing the vacant slot or reconnecting.
    pub fn cancel(&self) {
        self.0.borrow_mut().fail(P2pInputError::Cancelled);
    }
    pub fn disconnected(&self, conn: ConnId) -> Result<(), P2pInputError> {
        let mut s = self.0.borrow_mut();
        let e = if s.binding.is_some_and(|b| b.conn == conn) {
            P2pInputError::Disconnected
        } else {
            P2pInputError::WrongConnection
        };
        Err(s.fail(e))
    }
}
impl<G: Game, C: P2pInputCodec<G>> InputSource<G> for P2pInputSource<G, C> {
    fn send_local(
        &mut self,
        tick: u64,
        slot: PlayerSlot,
        input: G::Input,
        commands: Vec<G::Command>,
    ) {
        let mut s = self.0.borrow_mut();
        if let Err(e) = s.send(RemoteInput {
            tick,
            slot,
            input,
            commands,
            disconnected: false,
        }) {
            s.fail(e);
        }
    }
    fn poll_remote(&mut self) -> Vec<RemoteInput<G>> {
        let mut s = self.0.borrow_mut();
        if s.error.is_some() {
            return Vec::new();
        }
        s.incoming_bytes = 0;
        s.incoming.drain(..).map(|(r, _)| r).collect()
    }
}
