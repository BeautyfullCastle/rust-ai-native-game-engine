//! Bounded reliable two-peer inputs, separate from relay and checked-v3 controls.
//!
//! `ORRI` v1 uses explicit little-endian framing and a caller-defined portable
//! payload codec. It is not a raw-Pod format. A driver binds exactly one admitted
//! connection to an independently agreed checked context and accepted snapshot.
//! The finite constructor retains duplicate evidence for its entire tick window.
//! Opt-in rolling mode retires evidence only behind locally verified progress;
//! local verification never acknowledges delivery of bound outbound records.
//! Exhaustion or a recent conflicting duplicate fails the whole driver.
//! Call `check` before and after each Session/bootstrap operation; InputSource's
//! legacy void methods cannot return transport failures themselves. Never keep
//! advancing a failed session. `flush` retries the same FIFO head on Backpressure.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::marker::PhantomData;
use std::rc::Rc;

use orr_net::SendError;
use orr_proto::{Channel, ConnId};
use orr_session::{
    CheckedJoinContext, DepartureBarrier, DepartureFence, Game, InputSource, LocallyVerifiedTick,
    PlayerSlot, RemoteInput, Session,
};

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
        if self.first_tick >= self.end_tick {
            return Err(P2pInputError::InvalidConfig);
        }
        self.validate_payload()
    }
    fn validate_payload(&self) -> Result<(), P2pInputError> {
        if self.max_packet_bytes < HEADER
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

/// Opt-in rolling retention. Both distances must be nonzero. Keep at most
/// `recent_ticks` verified ticks of exact duplicate evidence; additionally accept
/// ticks up to `progress + future_ticks` (inclusive). Progress starts at zero and
/// comes only from Session's local verification callback or a validated binding.
/// The two ticks beyond the inclusive horizon must also fit. This lets Session
/// pre-increment its send cursor even for the first rejected send, including
/// when verification is stalled; the caller then observes a terminal error.
/// The finite limits' `first_tick`/`end_tick` do not apply in rolling mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct P2pRollingWindow {
    pub recent_ticks: u64,
    pub future_ticks: u64,
}
impl P2pRollingWindow {
    fn end(self, progress: u64) -> Result<u64, P2pInputError> {
        progress
            .checked_add(self.future_ticks)
            .and_then(|last| last.checked_add(2))
            .map(|reserved_end| reserved_end - 1)
            .ok_or(P2pInputError::TickExhausted)
    }
}

#[derive(Clone, Copy)]
struct Rolling {
    window: P2pRollingWindow,
    progress: u64,
    retired_through: u64,
    discarded_output_through: u64,
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
    /// The rolling future horizon would overflow; the session must terminate.
    TickExhausted,
    /// A pre-bind host already discarded output newer than this snapshot.
    SnapshotTooOld,
    /// Source replacement does not meet the chosen host recovery policy.
    UnsafeReplacement,
    /// Ingress/output is fenced; previously admitted input can still be drained.
    Fenced,
    /// The fenced target has not yet been verified by this exact source/Session.
    UnverifiedContinuation,
    /// Required unverified host-authored or admitted remote history is unavailable.
    MissingHistory {
        tick: u64,
        slot: PlayerSlot,
    },
    Malformed,
    PacketLimit,
    InputLimit,
    CommandLimit,
    RecordLimit,
    ByteLimit,
    ConflictingDuplicate {
        tick: u64,
        slot: PlayerSlot,
    },
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

/// Outcome after identity, authority, framing and byte-bound validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum P2pInputAccepted {
    New,
    Duplicate,
    /// Too old to compare: no application decode, enqueue or ledger insertion.
    /// This is not an assertion that the retired bytes were an exact duplicate.
    IgnoredRetired,
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
    encode_payload::<G, C>(context, record, limits)
}

fn encode_payload<G: Game, C: P2pInputCodec<G>>(
    context: &CheckedJoinContext,
    record: &RemoteInput<G>,
    limits: &P2pInputLimits,
) -> Result<Vec<u8>, P2pInputError> {
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

#[derive(Clone, Copy)]
struct Continuation {
    target: u64,
    next_send: u64,
}

struct State<G: Game, C: P2pInputCodec<G>> {
    local: PlayerSlot,
    context: CheckedJoinContext,
    limits: P2pInputLimits,
    rolling: Option<Rolling>,
    binding: Option<Binding>,
    error: Option<P2pInputError>,
    seen: BTreeMap<(u64, PlayerSlot), Vec<u8>>,
    seen_bytes: usize,
    outgoing: VecDeque<(u64, Vec<u8>)>,
    outgoing_bytes: usize,
    incoming: VecDeque<(RemoteInput<G>, usize)>,
    incoming_bytes: usize,
    flushing: bool,
    admitted_remote: bool,
    accepted_remote_max: Option<u64>,
    fenced: bool,
    continuation: Option<Continuation>,
    marker: PhantomData<C>,
}
impl<G: Game, C: P2pInputCodec<G>> State<G, C> {
    fn tick(&self, tick: u64) -> Result<(), P2pInputError> {
        if let Some(r) = self.rolling {
            let end = r.window.end(r.progress)?;
            if tick == 0 || tick >= end {
                return Err(P2pInputError::TickRange);
            }
            Ok(())
        } else {
            self.limits.tick(tick)
        }
    }
    fn retired(&self, tick: u64) -> bool {
        self.rolling.is_some_and(|r| tick <= r.retired_through)
    }
    fn progress(&mut self, tick: u64) -> Result<(), P2pInputError> {
        self.check()?;
        let Some(mut r) = self.rolling else {
            return Ok(());
        };
        r.progress = r.progress.max(tick);
        r.window.end(r.progress)?;
        r.retired_through = r
            .retired_through
            .max(r.progress.saturating_sub(r.window.recent_ticks));
        while self
            .seen
            .first_key_value()
            .is_some_and(|(&(tick, _), _)| tick <= r.retired_through)
        {
            let (_, bytes) = self.seen.pop_first().unwrap();
            self.seen_bytes -= bytes.len();
        }
        // Before admission a fresh snapshot will cover these records. Binding
        // requires a snapshot at least this fresh. Once bound, verification is
        // NOT peer receipt: even retired records remain in the outbound FIFO.
        if self.binding.is_none() && self.local == PlayerSlot(0) {
            self.outgoing.retain(|(tick, bytes)| {
                if *tick <= r.progress {
                    self.outgoing_bytes -= bytes.len();
                    r.discarded_output_through = r.discarded_output_through.max(*tick);
                    false
                } else {
                    true
                }
            });
        }
        self.rolling = Some(r);
        Ok(())
    }
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
        self.tick(tick)?;
        let allowed = match self.binding {
            Some(b) if self.rolling.is_none() && tick <= b.snapshot_tick => false,
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
        if self.fenced {
            return Err(P2pInputError::Fenced);
        }
        self.authority(self.local, record.slot, record.tick)?;
        // Never resurrect retired local records, even after their evidence is gone.
        if self.retired(record.tick) {
            return Ok(());
        }
        let bytes = encode_payload::<G, C>(&self.context, &record, &self.limits)?;
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
        if self.retired(tick) {
            return Ok(P2pInputAccepted::IgnoredRetired);
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
        self.admitted_remote = true;
        self.accepted_remote_max = Some(self.accepted_remote_max.map_or(tick, |old| old.max(tick)));
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
/// A driver is single-session/single-connection. Explicit host continuation can
/// replace it with a fresh generation; there is no general repair protocol.
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
        Self::create(local, context, limits, None)
    }
    /// Rolling mode has no fixed session end tick. The recent evidence and
    /// each pending queue retain independent record/byte limits. A stalled
    /// verifier cannot advance the future horizon by receiving newer packets.
    /// This source belongs to one Session lifetime: cancel/replace it before
    /// restoring or replacing that Session. Explicit continuation below preserves
    /// the Session; arbitrary restore/replay is unsupported.
    pub fn new_rolling(
        local: PlayerSlot,
        context: CheckedJoinContext,
        limits: P2pInputLimits,
        window: P2pRollingWindow,
    ) -> Result<(Self, P2pInputSource<G, C>), P2pInputError> {
        limits.validate_payload()?;
        if window.recent_ticks == 0 || window.future_ticks == 0 {
            return Err(P2pInputError::InvalidConfig);
        }
        window.end(0).map_err(|_| P2pInputError::InvalidConfig)?;
        Self::create(
            local,
            context,
            limits,
            Some(Rolling {
                window,
                progress: 0,
                retired_through: 0,
                discarded_output_through: 0,
            }),
        )
    }
    /// Replace only a cancelled rolling host source, preserving the actual Session/world.
    /// This is a trusted application boundary, not an established-player reconnect API:
    /// the caller MUST first fence the old route, cancel its membership attempt, retire
    /// its owned link/hold, and prove the final readiness notice was never successfully
    /// enqueued. No joiner input may have been admitted by this source, even if
    /// not yet polled or already verified. Older-generation remote history is
    /// allowed only at or below the actual verified checkpoint.
    ///
    /// The replacement starts at the Session's locally verified checkpoint and rebuilds
    /// its unverified authored tail under a fresh generation. All coverage, authority and
    /// queue budgets (including defaults a subsequent `mark_slot_vacant` will emit) are
    /// checked before changing the Session source. On failure neither source nor world
    /// changes. After success the caller must immediately vacate the pending grant from
    /// its original first-input tick, before polling, advancing, or admitting a new peer.
    /// The old cancelled handle remains inert; no wire format or Session reset is used.
    pub fn replace_cancelled_host_rolling(
        &self,
        session: &mut Session<G, P2pInputSource<G, C>>,
        context: CheckedJoinContext,
    ) -> Result<Self, P2pInputError> {
        let old = self.0.borrow();
        let host = PlayerSlot(0);
        let joiner = PlayerSlot(1);
        let verified = session.verified_tick();
        let next = session.next_send_tick();
        if !Rc::ptr_eq(&self.0, &session.source().0)
            || old.local != host
            || old.error != Some(P2pInputError::Cancelled)
            || old.flushing
            || old.admitted_remote
            || session.config().local_slot != host
            || session.config().player_count != 2
            || session.config().relay
            || verified < u64::from(session.config().input_delay)
            || session.verified_frame().is_none()
            || session
                .last_remote_tick(joiner)
                .is_some_and(|tick| tick > verified)
            || context.join_id() == old.context.join_id()
            || context.roster() != old.context.roster()
        {
            return Err(P2pInputError::UnsafeReplacement);
        }
        let rolling = old.rolling.ok_or(P2pInputError::UnsafeReplacement)?;
        let from = match (old.binding, session.pending_join(joiner)) {
            (Some(binding), Some(ticket))
                if ticket.snapshot_tick == binding.snapshot_tick
                    && ticket.first_input_tick == binding.first_input_tick
                    && ticket.attempt == old.context.attempt()
                    && verified < ticket.first_input_tick
                    && ticket.first_input_tick <= next =>
            {
                Some(ticket.first_input_tick)
            }
            (None, None) => None,
            _ => return Err(P2pInputError::UnsafeReplacement),
        };
        // Bound the coverage pass before iterating or allocating from tick distances.
        if next > rolling.window.end(verified)? || next <= verified {
            return Err(P2pInputError::TickRange);
        }
        let (replacement, source) =
            Self::new_rolling(host, context, old.limits.clone(), rolling.window)?;
        drop(old);
        {
            let mut fresh = replacement.0.borrow_mut();
            let r = fresh.rolling.as_mut().unwrap();
            r.progress = verified;
            r.retired_through = verified;
            r.discarded_output_through = verified;
            let mut covered = BTreeSet::new();
            for record in session.authored_since(verified) {
                if record.tick >= next
                    || record.disconnected
                    || (record.slot == joiner && from.is_some_and(|cutoff| record.tick >= cutoff))
                    || !covered.insert((record.tick, record.slot))
                {
                    return Err(P2pInputError::UnsafeReplacement);
                }
                fresh.send(record)?;
            }
            for tick in verified + 1..next {
                for slot in [host, joiner] {
                    if (slot == host || from.is_none_or(|cutoff| tick < cutoff))
                        && !covered.contains(&(tick, slot))
                    {
                        return Err(P2pInputError::MissingHistory { tick, slot });
                    }
                }
            }
            // Vacancy fills only this missing suffix. Reserve its exact encoded cost
            // without emitting or changing Session state before the source is installed.
            let mut reserved_records = 0;
            let mut reserved_bytes = 0;
            if let Some(from) = from {
                for tick in from..next {
                    fresh.authority(host, joiner, tick)?;
                    let bytes = encode_payload::<G, C>(
                        &fresh.context,
                        &RemoteInput {
                            tick,
                            slot: joiner,
                            input: G::Input::default(),
                            commands: Vec::new(),
                            disconnected: false,
                        },
                        &fresh.limits,
                    )?;
                    fresh.budget(
                        fresh.seen.len() + reserved_records,
                        fresh.seen_bytes + reserved_bytes,
                        bytes.len(),
                    )?;
                    fresh.budget(
                        fresh.outgoing.len() + reserved_records,
                        fresh.outgoing_bytes + reserved_bytes,
                        bytes.len(),
                    )?;
                    reserved_records += 1;
                    reserved_bytes += bytes.len();
                }
            }
        }
        *session.source_mut() = source;
        Ok(replacement)
    }
    /// Explicit two-player host-continuation policy: only input already admitted
    /// by this healthy driver survives the lost connection. Fence the application
    /// route first; do NOT call `cancel`/`disconnected` before this operation.
    /// Never-received transport bytes are not acknowledged or reconstructed.
    ///
    /// The stable maximum includes accepted-but-unpolled records. Every required
    /// tick through it must already have exact driver evidence, with host-owned
    /// defaults before the accepted grant's boundary. Gaps, failed sources and
    /// targets beyond the host's authored horizon are unrecoverable here. A failed
    /// coverage check leaves ingress fenced, never authorizes filling a gap.
    /// After success use `verify_host_continuation`, without authoring new input.
    /// Calling `advance`/`send_local` while fenced is terminal misuse: it changes
    /// the frozen authored horizon and poisons recovery, requiring teardown.
    pub fn fence_host_continuation(
        &self,
        session: &Session<G, P2pInputSource<G, C>>,
        conn: ConnId,
    ) -> Result<u64, P2pInputError> {
        let mut old = self.0.borrow_mut();
        old.check()?;
        let host = PlayerSlot(0);
        let joiner = PlayerSlot(1);
        let verified = session.verified_tick();
        let next = session.next_send_tick();
        let binding = old.binding.ok_or(P2pInputError::UnsafeReplacement)?;
        let rolling = old.rolling.ok_or(P2pInputError::UnsafeReplacement)?;
        if !Rc::ptr_eq(&self.0, &session.source().0)
            || old.local != host
            || binding.conn != conn
            || old.flushing
            || old.fenced
            || session.config().local_slot != host
            || session.config().player_count != 2
            || session.config().relay
            || verified < binding.snapshot_tick
            || rolling.progress != verified
            || session.verified_frame().is_none()
        {
            return Err(P2pInputError::UnsafeReplacement);
        }
        old.fenced = true;
        // Defaults already authored before the grant cannot be overwritten either.
        let target = verified
            .max(u64::from(session.config().input_delay))
            .max(binding.first_input_tick - 1)
            .max(old.accepted_remote_max.unwrap_or(0));
        target.checked_add(1).ok_or(P2pInputError::TickExhausted)?;
        if target >= next || next > rolling.window.end(verified)? {
            return Err(P2pInputError::TickRange);
        }
        // This loop is bounded by the rolling horizon, never a packet-supplied span.
        for tick in verified + 1..=target {
            for slot in [host, joiner] {
                // Initial delay ticks are Session-preconfirmed, not authored records.
                if tick <= u64::from(session.config().input_delay) {
                    continue;
                }
                if !old.seen.contains_key(&(tick, slot)) {
                    return Err(P2pInputError::MissingHistory { tick, slot });
                }
            }
        }
        // Session's polled maximum must never exceed the actual driver admissions.
        if session
            .last_remote_tick(joiner)
            .is_some_and(|tick| tick > verified && Some(tick) > old.accepted_remote_max)
        {
            return Err(P2pInputError::UnsafeReplacement);
        }
        old.continuation = Some(Continuation {
            target,
            next_send: next,
        });
        Ok(target)
    }

    /// Drain retained records and simulate/verify the exact fenced target in the
    /// same Session. `step` advances simulation only, never authors new input.
    /// No caller-reported maximum/checksum can stand in for this verification.
    pub fn verify_host_continuation(
        &self,
        session: &mut Session<G, P2pInputSource<G, C>>,
    ) -> Result<u64, P2pInputError> {
        let fence = {
            let old = self.0.borrow();
            old.check()?;
            if !old.fenced || !Rc::ptr_eq(&self.0, &session.source().0) {
                return Err(P2pInputError::UnsafeReplacement);
            }
            old.continuation.ok_or(P2pInputError::UnsafeReplacement)?
        };
        if session.next_send_tick() != fence.next_send || session.verified_tick() > fence.target {
            return Err(P2pInputError::UnsafeReplacement);
        }
        session.poll_confirmed();
        self.check()?;
        while session.head_tick() < fence.target {
            let head = session.head_tick();
            session.step();
            self.check()?;
            if session.head_tick() <= head {
                return Err(P2pInputError::UnverifiedContinuation);
            }
        }
        session.poll_confirmed();
        self.check()?;
        if session.verified_tick() != fence.target || !self.0.borrow().incoming.is_empty() {
            return Err(P2pInputError::UnverifiedContinuation);
        }
        Ok(fence.target)
    }

    /// Install a fresh unbound generation and immediately commit the unchanged
    /// one-survivor DepartureBarrier at target+1. The old generation is retired
    /// permanently. The returner must discard its old speculation and bootstrap
    /// from a fresh checked snapshot; this is not a general repair protocol.
    /// All host-tail/default budgets are checked before swapping the source.
    pub fn replace_fenced_host_rolling(
        &self,
        session: &mut Session<G, P2pInputSource<G, C>>,
        context: CheckedJoinContext,
    ) -> Result<Self, P2pInputError> {
        let old = self.0.borrow();
        old.check()?;
        let fence = old.continuation.ok_or(P2pInputError::UnsafeReplacement)?;
        let host = PlayerSlot(0);
        let joiner = PlayerSlot(1);
        let verified = session.verified_tick();
        let next = session.next_send_tick();
        let rolling = old.rolling.ok_or(P2pInputError::UnsafeReplacement)?;
        if !old.fenced
            || !Rc::ptr_eq(&self.0, &session.source().0)
            || old.local != host
            || old.flushing
            || session.config().local_slot != host
            || session.config().player_count != 2
            || session.config().relay
            || next != fence.next_send
            || context.join_id() <= old.context.join_id()
            || context.roster() != old.context.roster()
        {
            return Err(P2pInputError::UnsafeReplacement);
        }
        if !old.incoming.is_empty()
            || verified != fence.target
            || rolling.progress != verified
            || session.verified_frame().is_none()
        {
            return Err(P2pInputError::UnverifiedContinuation);
        }
        let cutoff = verified
            .checked_add(1)
            .ok_or(P2pInputError::TickExhausted)?;
        if cutoff > next || next > rolling.window.end(verified)? {
            return Err(P2pInputError::TickRange);
        }
        let mut barrier = DepartureBarrier::new(
            context.join_id(),
            0,
            joiner,
            host,
            vec![host],
            &[],
            session.config(),
        )
        .map_err(|_| P2pInputError::UnsafeReplacement)?;
        barrier
            .report_fence(DepartureFence {
                recovery_id: context.join_id(),
                revision: 0,
                survivor: host,
                verified_tick: verified,
                departed_max: old.accepted_remote_max,
            })
            .map_err(|_| P2pInputError::UnsafeReplacement)?;
        let ack = barrier
            .acknowledgment(session)
            .map_err(|_| P2pInputError::UnverifiedContinuation)?;
        barrier
            .acknowledge(ack)
            .map_err(|_| P2pInputError::UnsafeReplacement)?;
        let (replacement, source) =
            Self::new_rolling(host, context, old.limits.clone(), rolling.window)?;
        drop(old);
        {
            let mut fresh = replacement.0.borrow_mut();
            let r = fresh.rolling.as_mut().unwrap();
            r.progress = verified;
            r.retired_through = verified;
            r.discarded_output_through = verified;
            let mut covered = BTreeSet::new();
            for record in session.authored_since(verified) {
                if record.tick >= next
                    || record.slot != host
                    || record.disconnected
                    || !covered.insert(record.tick)
                {
                    return Err(P2pInputError::UnsafeReplacement);
                }
                fresh.send(record)?;
            }
            for tick in cutoff..next {
                if !covered.contains(&tick) {
                    return Err(P2pInputError::MissingHistory { tick, slot: host });
                }
            }
            let mut reserved_bytes = 0;
            for (reserved_records, tick) in (cutoff..next).enumerate() {
                let bytes = encode_payload::<G, C>(
                    &fresh.context,
                    &RemoteInput {
                        tick,
                        slot: joiner,
                        input: G::Input::default(),
                        commands: Vec::new(),
                        disconnected: false,
                    },
                    &fresh.limits,
                )?;
                fresh.authority(host, joiner, tick)?;
                fresh.budget(
                    fresh.seen.len() + reserved_records,
                    fresh.seen_bytes + reserved_bytes,
                    bytes.len(),
                )?;
                fresh.budget(
                    fresh.outgoing.len() + reserved_records,
                    fresh.outgoing_bytes + reserved_bytes,
                    bytes.len(),
                )?;
                reserved_bytes += bytes.len();
            }
        }
        let prior = std::mem::replace(session.source_mut(), source);
        if barrier.commit(session, 0, &[host]).is_err() {
            *session.source_mut() = prior;
            return Err(P2pInputError::UnsafeReplacement);
        }
        self.cancel();
        replacement.check()?;
        Ok(replacement)
    }
    fn create(
        local: PlayerSlot,
        context: CheckedJoinContext,
        limits: P2pInputLimits,
        rolling: Option<Rolling>,
    ) -> Result<(Self, P2pInputSource<G, C>), P2pInputError> {
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
            rolling,
            binding: None,
            error: None,
            seen: BTreeMap::new(),
            seen_bytes: 0,
            outgoing: VecDeque::new(),
            outgoing_bytes: 0,
            incoming: VecDeque::new(),
            incoming_bytes: 0,
            flushing: false,
            admitted_remote: false,
            accepted_remote_max: None,
            fenced: false,
            continuation: None,
            marker: PhantomData,
        }));
        Ok((Self(state.clone()), P2pInputSource(state)))
    }
    /// Bind the actual admitted connection once. Derive tick bounds from the
    /// accepted checked ticket (host), or the accepted bootstrap Session's
    /// verified_tick/next_send_tick (joiner), never unvalidated packet fields.
    /// Context equality is checked before queued input can reach Session. This
    /// is a trusted application boundary: scalar tick arguments cannot prove
    /// snapshot validation themselves. The caller must validate the checked
    /// snapshot/ticket first; never pass a peer's claimed progress directly.
    pub fn bind(
        &self,
        conn: ConnId,
        context: &CheckedJoinContext,
        snapshot_tick: u64,
        first_input_tick: u64,
    ) -> Result<(), P2pInputError> {
        let mut s = self.0.borrow_mut();
        s.check()?;
        if s.fenced {
            return Err(P2pInputError::Fenced);
        }
        let result = (|| {
            if s.binding.is_some() {
                return Err(P2pInputError::AlreadyBound);
            }
            if &s.context != context {
                return Err(P2pInputError::WrongContext);
            }
            if snapshot_tick >= first_input_tick {
                return Err(P2pInputError::TickRange);
            }
            if let Some(r) = s.rolling {
                if snapshot_tick < r.discarded_output_through {
                    return Err(P2pInputError::SnapshotTooOld);
                }
                let end = r.window.end(r.progress.max(snapshot_tick))?;
                if first_input_tick >= end {
                    return Err(P2pInputError::TickRange);
                }
            } else {
                s.limits.tick(first_input_tick)?;
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
            // The snapshot already contains these ticks. Never send old records
            // to the freshly bootstrapped Session. Finite mode keeps their exact
            // evidence; rolling mode retires it below after all binding checks.
            let mut retained = VecDeque::new();
            while let Some((tick, bytes)) = s.outgoing.pop_front() {
                if tick > snapshot_tick {
                    retained.push_back((tick, bytes));
                } else {
                    s.outgoing_bytes -= bytes.len();
                }
            }
            s.outgoing = retained;
            if let Some(r) = &mut s.rolling {
                // Seed only after all binding checks pass. The caller must
                // supply a validated checked snapshot, not claimed progress.
                r.retired_through = r.retired_through.max(snapshot_tick);
                s.progress(snapshot_tick)?;
            }
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
    /// Retained exact duplicate evidence: (records, encoded bytes), excluding
    /// the independently bounded incoming and outgoing queues.
    pub fn retained_evidence(&self) -> (usize, usize) {
        let s = self.0.borrow();
        (s.seen.len(), s.seen_bytes)
    }
    /// Rolling (locally verified/snapshot watermark, irreversible retirement
    /// floor), or None for the legacy finite constructor.
    pub fn rolling_progress(&self) -> Option<(u64, u64)> {
        self.0
            .borrow()
            .rolling
            .map(|r| (r.progress, r.retired_through))
    }
    /// Invalid traffic is terminal: callers must close/retire this admitted link.
    pub fn receive(
        &self,
        conn: ConnId,
        channel: Channel,
        bytes: &[u8],
    ) -> Result<P2pInputAccepted, P2pInputError> {
        let mut s = self.0.borrow_mut();
        s.check()?;
        if s.fenced {
            return Err(P2pInputError::Fenced);
        }
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
            if s.fenced {
                return Err(P2pInputError::Fenced);
            }
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
        s.check()?;
        if s.fenced {
            return Err(P2pInputError::Fenced);
        }
        let e = if s.binding.is_some_and(|b| b.conn == conn) {
            P2pInputError::Disconnected
        } else {
            P2pInputError::WrongConnection
        };
        Err(s.fail(e))
    }
}
impl<G: Game, C: P2pInputCodec<G>> InputSource<G> for P2pInputSource<G, C> {
    fn on_locally_verified(&mut self, tick: LocallyVerifiedTick<'_, G>) {
        let mut s = self.0.borrow_mut();
        if let Err(e) = s.progress(tick.simulated.tick()) {
            s.fail(e);
        }
    }
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
