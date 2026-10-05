//! Finite, application-admitted three-peer reliable input mesh (`ORRM` v1).
//!
//! This is separate from the two-peer driver, relay and checked control wire.
//! The application freezes one context: active peers 0/1, donor 0 and joiner 2.
//! Only peer 0 authors vacant slot 2, with portable default input and no commands,
//! until the accepted checked ticket's exact first-input boundary. Each peer
//! admits its two direct edges; there is no discovery, forwarding, reconnect,
//! survivor election, or default takeover. Any edge failure aborts the driver.
//!
//! Adapter instances have distinct application-assigned IDs. Remap their raw
//! connection IDs (a NetLink uses 1) through the admitted registry before routing
//! events. Neither packet fields nor numeric raw IDs establish remote identity.
//! Edge generations are nonzero, agreed independently at both ends, and immutable.
//!
//! Logical canonical evidence is independent from destination obligations. A
//! transport queue accepting one destination does not acknowledge another or
//! prove network delivery. Admission of the joiner's edge creates new obligations
//! for existing local evidence newer than its snapshot. Local verification never
//! retires evidence or pending output. All retention is finite and explicitly
//! bounded in records and encoded bytes, not total allocator/decoded-object memory.
//! Check the driver before and after every Session/bootstrap operation: legacy
//! InputSource methods cannot return errors. Errors are terminal and preserve
//! inert accounting for inspection; no buffered input is delivered after failure.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::marker::PhantomData;
use std::rc::Rc;

use orr_net::SendError;
use orr_proto::{Channel, ConnId};
use orr_session::{
    CheckedJoinContext, CheckedJoinTicket, Game, InputSource, JoinBootstrap, JoinBootstrapStatus,
    PlayerSlot, RemoteInput,
};

use crate::P2pInputCodec;

pub const P2P_MESH_INPUT_VERSION: u16 = 1;
const MAGIC: &[u8; 4] = b"ORRM";
const ENVELOPE: usize = 35;
const LOGICAL_HEADER: usize = 15;
const HEADER: usize = ENVELOPE + LOGICAL_HEADER;
type Key = (u64, PlayerSlot);

/// Configurable downward bounds within the finite tick 1..=512 foundation.
/// Canonical, incoming and pending bytes have separate caps. Delivery metadata
/// includes both pending and queue-accepted obligations for the whole lifetime.
#[derive(Clone, Debug)]
pub struct P2pMeshInputLimits {
    pub first_tick: u64,
    pub end_tick: u64,
    pub max_packet_bytes: usize,
    pub max_input_bytes: usize,
    pub max_commands: usize,
    pub max_command_bytes: usize,
    pub max_logical_records: usize,
    pub max_incoming_records: usize,
    pub max_destination_records: usize,
    pub max_edge_pending_records: usize,
    pub max_logical_encoded_bytes: usize,
    pub max_incoming_encoded_bytes: usize,
    pub max_pending_encoded_bytes: usize,
    pub max_edge_pending_encoded_bytes: usize,
}
impl Default for P2pMeshInputLimits {
    fn default() -> Self {
        Self {
            first_tick: 1,
            end_tick: 513,
            max_packet_bytes: 256,
            max_input_bytes: 20,
            max_commands: 3,
            max_command_bytes: 4,
            max_logical_records: 1536,
            max_incoming_records: 1536,
            max_destination_records: 3072,
            max_edge_pending_records: 1536,
            max_logical_encoded_bytes: 1536 * 256,
            max_incoming_encoded_bytes: 1536 * 256,
            max_pending_encoded_bytes: 3072 * 256,
            max_edge_pending_encoded_bytes: 1536 * 256,
        }
    }
}
impl P2pMeshInputLimits {
    fn validate(&self) -> Result<(), P2pMeshInputError> {
        if self.first_tick == 0
            || self.first_tick >= self.end_tick
            || self.end_tick > 513
            || !(HEADER..=256).contains(&self.max_packet_bytes)
            || self.max_input_bytes > self.max_packet_bytes
            || self.max_commands > 3
            || self.max_command_bytes > self.max_packet_bytes
            || !(1..=1536).contains(&self.max_logical_records)
            || !(1..=1536).contains(&self.max_incoming_records)
            || !(1..=3072).contains(&self.max_destination_records)
            || !(1..=1536).contains(&self.max_edge_pending_records)
            || self.max_logical_encoded_bytes > 1536 * 256
            || self.max_incoming_encoded_bytes > 1536 * 256
            || self.max_pending_encoded_bytes > 3072 * 256
            || self.max_edge_pending_encoded_bytes > 1536 * 256
        {
            return Err(P2pMeshInputError::InvalidConfig);
        }
        Ok(())
    }
    fn tick(&self, tick: u64) -> Result<(), P2pMeshInputError> {
        if (self.first_tick..self.end_tick).contains(&tick) {
            Ok(())
        } else {
            Err(P2pMeshInputError::TickRange)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum P2pMeshInputError {
    InvalidConfig,
    InvalidEdge,
    AlreadyAdmitted,
    NotBound,
    AlreadyBound,
    WrongConnection,
    WrongChannel,
    WrongContext,
    WrongSchema,
    WrongGeneration,
    WrongRecipient,
    WrongAuthority,
    InvalidVacancy,
    InvalidSnapshot,
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
impl core::fmt::Display for P2pMeshInputError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "P2P mesh input: {self:?}")
    }
}
impl std::error::Error for P2pMeshInputError {}

/// One application-admitted direct edge. IDs are never inferred from packets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct P2pMeshEdge {
    pub connection: ConnId,
    pub adapter_id: u64,
    pub raw_connection: ConnId,
    pub remote: PlayerSlot,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum P2pMeshInputAccepted {
    New,
    Duplicate,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct P2pMeshInputFlush {
    pub sent: usize,
    pub pending: usize,
    pub backpressured_edges: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum P2pMeshDelivery {
    Pending,
    QueueAccepted,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Grant {
    snapshot: u64,
    first: u64,
}
struct Evidence {
    author: PlayerSlot,
    bytes: Rc<Vec<u8>>,
}
struct Edge {
    admitted: P2pMeshEdge,
    delivery: BTreeMap<Key, P2pMeshDelivery>,
    pending: VecDeque<(Key, Rc<Vec<u8>>)>,
    pending_bytes: usize,
}
struct State<G: Game, C: P2pInputCodec<G>> {
    local: PlayerSlot,
    context: CheckedJoinContext,
    limits: P2pMeshInputLimits,
    grant: Option<Grant>,
    default_input: Vec<u8>,
    edges: BTreeMap<ConnId, Edge>,
    seen: BTreeMap<Key, Evidence>,
    seen_bytes: usize,
    incoming: VecDeque<(RemoteInput<G>, usize)>,
    incoming_bytes: usize,
    destination_records: usize,
    pending_bytes: usize,
    error: Option<P2pMeshInputError>,
    flushing: bool,
    marker: PhantomData<C>,
}

fn budget(
    current: usize,
    additional: usize,
    limit: usize,
    error: P2pMeshInputError,
) -> Result<(), P2pMeshInputError> {
    if additional > limit.saturating_sub(current) {
        Err(error)
    } else {
        Ok(())
    }
}

/// Canonical payload only: no destination/edge identity and no native Pod bytes.
fn encode<G: Game, C: P2pInputCodec<G>>(
    r: &RemoteInput<G>,
    l: &P2pMeshInputLimits,
) -> Result<Vec<u8>, P2pMeshInputError> {
    l.tick(r.tick)?;
    if r.disconnected || r.slot.0 >= 3 {
        return Err(P2pMeshInputError::WrongAuthority);
    }
    if r.commands.len() > l.max_commands {
        return Err(P2pMeshInputError::CommandLimit);
    }
    let mut input = Vec::new();
    C::encode_input(&r.input, &mut input);
    if input.len() > l.max_input_bytes {
        return Err(P2pMeshInputError::InputLimit);
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&r.tick.to_le_bytes());
    bytes.push(r.slot.0);
    bytes.extend_from_slice(&(input.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(r.commands.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&input);
    for command in &r.commands {
        let mut encoded = Vec::new();
        C::encode_command(command, &mut encoded);
        if encoded.len() > l.max_command_bytes {
            return Err(P2pMeshInputError::CommandLimit);
        }
        bytes.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&encoded);
    }
    if bytes.len() + ENVELOPE > l.max_packet_bytes {
        return Err(P2pMeshInputError::PacketLimit);
    }
    Ok(bytes)
}
fn packet<G: Game, C: P2pInputCodec<G>>(
    context: &CheckedJoinContext,
    edge: P2pMeshEdge,
    canonical: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(ENVELOPE + canonical.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&P2P_MESH_INPUT_VERSION.to_le_bytes());
    out.extend_from_slice(&C::SCHEMA.to_le_bytes());
    out.extend_from_slice(&context.join_id().to_le_bytes());
    out.extend_from_slice(&context.attempt().to_le_bytes());
    out.extend_from_slice(&edge.generation.to_le_bytes());
    out.push(edge.remote.0);
    out.extend_from_slice(canonical);
    out
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], P2pMeshInputError> {
        if n > self.0.len() {
            return Err(P2pMeshInputError::Malformed);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, P2pMeshInputError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, P2pMeshInputError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, P2pMeshInputError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, P2pMeshInputError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}
impl<G: Game, C: P2pInputCodec<G>> State<G, C> {
    fn check(&self) -> Result<(), P2pMeshInputError> {
        self.error.clone().map_or(Ok(()), Err)
    }
    fn fail(&mut self, error: P2pMeshInputError) -> P2pMeshInputError {
        self.error.get_or_insert(error).clone()
    }
    fn authority(
        &self,
        author: PlayerSlot,
        slot: PlayerSlot,
        tick: u64,
    ) -> Result<(), P2pMeshInputError> {
        self.limits.tick(tick)?;
        let allowed = match author.0 {
            0 => {
                slot == PlayerSlot(0)
                    || (slot == PlayerSlot(2) && self.grant.is_none_or(|g| tick < g.first))
            }
            1 => slot == PlayerSlot(1),
            2 => slot == PlayerSlot(2) && self.grant.is_some_and(|g| tick >= g.first),
            _ => false,
        };
        if allowed {
            Ok(())
        } else {
            Err(P2pMeshInputError::WrongAuthority)
        }
    }
    fn vacancy(
        &self,
        author: PlayerSlot,
        record: &RemoteInput<G>,
        canonical: &[u8],
    ) -> Result<(), P2pMeshInputError> {
        if author == PlayerSlot(0)
            && record.slot == PlayerSlot(2)
            && (!record.commands.is_empty() || canonical[LOGICAL_HEADER..] != self.default_input)
        {
            return Err(P2pMeshInputError::InvalidVacancy);
        }
        Ok(())
    }
    fn duplicate(&self, key: Key, bytes: &[u8]) -> Result<bool, P2pMeshInputError> {
        match self.seen.get(&key) {
            Some(e) if e.bytes.as_slice() == bytes => Ok(true),
            Some(_) => Err(P2pMeshInputError::ConflictingDuplicate {
                tick: key.0,
                slot: key.1,
            }),
            None => Ok(false),
        }
    }
    fn evidence_budget(&self, bytes: usize) -> Result<(), P2pMeshInputError> {
        budget(
            self.seen.len(),
            1,
            self.limits.max_logical_records,
            P2pMeshInputError::RecordLimit,
        )?;
        budget(
            self.seen_bytes,
            bytes,
            self.limits.max_logical_encoded_bytes,
            P2pMeshInputError::ByteLimit,
        )
    }
    fn destination_budget(&self, records: usize, bytes: usize) -> Result<(), P2pMeshInputError> {
        budget(
            self.destination_records,
            records,
            self.limits.max_destination_records,
            P2pMeshInputError::RecordLimit,
        )?;
        budget(
            self.pending_bytes,
            bytes,
            self.limits.max_pending_encoded_bytes,
            P2pMeshInputError::ByteLimit,
        )
    }
    fn edge_budget(
        &self,
        edge: &Edge,
        records: usize,
        bytes: usize,
    ) -> Result<(), P2pMeshInputError> {
        budget(
            edge.pending.len(),
            records,
            self.limits.max_edge_pending_records,
            P2pMeshInputError::RecordLimit,
        )?;
        budget(
            edge.pending_bytes,
            bytes,
            self.limits.max_edge_pending_encoded_bytes,
            P2pMeshInputError::ByteLimit,
        )
    }
    fn send(&mut self, record: RemoteInput<G>) -> Result<(), P2pMeshInputError> {
        self.check()?;
        self.authority(self.local, record.slot, record.tick)?;
        let bytes = encode::<G, C>(&record, &self.limits)?;
        self.vacancy(self.local, &record, &bytes)?;
        let key = (record.tick, record.slot);
        if self.duplicate(key, &bytes)? {
            return Ok(());
        }
        self.evidence_budget(bytes.len())?;
        let destinations: Vec<_> = self
            .edges
            .iter()
            .filter(|(_, e)| {
                e.admitted.remote != PlayerSlot(2)
                    || self.grant.is_some_and(|g| record.tick > g.snapshot)
            })
            .map(|(&id, _)| id)
            .collect();
        let packet_bytes = ENVELOPE + bytes.len();
        self.destination_budget(destinations.len(), destinations.len() * packet_bytes)?;
        for id in &destinations {
            self.edge_budget(&self.edges[id], 1, packet_bytes)?;
        }
        // Every queue and ledger was preflighted before the first mutation.
        for id in destinations {
            let edge = self.edges.get_mut(&id).unwrap();
            let packet = Rc::new(packet::<G, C>(&self.context, edge.admitted, &bytes));
            edge.pending.push_back((key, packet));
            edge.pending_bytes += packet_bytes;
            edge.delivery.insert(key, P2pMeshDelivery::Pending);
            self.destination_records += 1;
            self.pending_bytes += packet_bytes;
        }
        self.seen_bytes += bytes.len();
        self.seen.insert(
            key,
            Evidence {
                author: self.local,
                bytes: Rc::new(bytes),
            },
        );
        Ok(())
    }
    fn install_grant(
        &mut self,
        context: &CheckedJoinContext,
        grant: Grant,
    ) -> Result<(), P2pMeshInputError> {
        if &self.context != context {
            return Err(P2pMeshInputError::WrongContext);
        }
        if let Some(old) = self.grant {
            return if old == grant {
                Ok(())
            } else {
                Err(P2pMeshInputError::AlreadyBound)
            };
        }
        self.limits.tick(grant.first)?;
        if grant.snapshot >= grant.first || grant.snapshot + 1 < self.limits.first_tick {
            return Err(P2pMeshInputError::InvalidSnapshot);
        }
        // Installing a cutoff must not retroactively bless prior unauthorized inputs.
        if self.seen.iter().any(|(&(tick, slot), e)| {
            slot == PlayerSlot(2) && (e.author != PlayerSlot(0) || tick >= grant.first)
        }) {
            return Err(P2pMeshInputError::WrongAuthority);
        }
        self.grant = Some(grant);
        Ok(())
    }
    fn admit(&mut self, admitted: P2pMeshEdge) -> Result<(), P2pMeshInputError> {
        if admitted.connection.0 == 0
            || admitted.adapter_id == 0
            || admitted.raw_connection.0 == 0
            || admitted.generation == 0
            || admitted.remote.0 >= 3
            || admitted.remote == self.local
        {
            return Err(P2pMeshInputError::InvalidEdge);
        }
        if self.edges.contains_key(&admitted.connection)
            || self.edges.values().any(|e| {
                e.admitted.remote == admitted.remote
                    || (e.admitted.adapter_id == admitted.adapter_id
                        && e.admitted.raw_connection == admitted.raw_connection)
            })
        {
            return Err(P2pMeshInputError::AlreadyAdmitted);
        }
        let snapshot = if self.local == PlayerSlot(2) || admitted.remote == PlayerSlot(2) {
            Some(self.grant.ok_or(P2pMeshInputError::NotBound)?.snapshot)
        } else {
            None
        };
        let backlog: Vec<_> = self
            .seen
            .iter()
            .filter(|(&(tick, _), e)| e.author == self.local && snapshot.is_none_or(|s| tick > s))
            .map(|(&key, e)| (key, Rc::clone(&e.bytes)))
            .collect();
        let bytes: usize = backlog.iter().map(|(_, b)| ENVELOPE + b.len()).sum();
        let mut edge = Edge {
            admitted,
            delivery: BTreeMap::new(),
            pending: VecDeque::new(),
            pending_bytes: 0,
        };
        self.destination_budget(backlog.len(), bytes)?;
        self.edge_budget(&edge, backlog.len(), bytes)?;
        for (key, canonical) in backlog {
            let packet = Rc::new(packet::<G, C>(&self.context, admitted, &canonical));
            edge.pending_bytes += packet.len();
            edge.pending.push_back((key, packet));
            edge.delivery.insert(key, P2pMeshDelivery::Pending);
        }
        self.destination_records += edge.delivery.len();
        self.pending_bytes += bytes;
        self.edges.insert(admitted.connection, edge);
        Ok(())
    }
    fn receive(
        &mut self,
        connection: ConnId,
        channel: Channel,
        bytes: &[u8],
    ) -> Result<P2pMeshInputAccepted, P2pMeshInputError> {
        let edge = self
            .edges
            .get(&connection)
            .ok_or(P2pMeshInputError::WrongConnection)?
            .admitted;
        if channel != Channel::Reliable {
            return Err(P2pMeshInputError::WrongChannel);
        }
        if bytes.len() > self.limits.max_packet_bytes {
            return Err(P2pMeshInputError::PacketLimit);
        }
        let mut r = Reader(bytes);
        if r.take(4)? != MAGIC || r.u16()? != P2P_MESH_INPUT_VERSION {
            return Err(P2pMeshInputError::Malformed);
        }
        if r.u64()? != C::SCHEMA {
            return Err(P2pMeshInputError::WrongSchema);
        }
        if r.u64()? != self.context.join_id() || r.u32()? != self.context.attempt() {
            return Err(P2pMeshInputError::WrongContext);
        }
        if r.u64()? != edge.generation {
            return Err(P2pMeshInputError::WrongGeneration);
        }
        if r.u8()? != self.local.0 {
            return Err(P2pMeshInputError::WrongRecipient);
        }
        let canonical = r.0;
        let tick = r.u64()?;
        let slot = PlayerSlot(r.u8()?);
        self.authority(edge.remote, slot, tick)?;
        if self.local == PlayerSlot(2) && self.grant.is_some_and(|g| tick <= g.snapshot) {
            return Err(P2pMeshInputError::TickRange);
        }
        let input_len = r.u32()? as usize;
        let command_count = r.u16()? as usize;
        if input_len > self.limits.max_input_bytes {
            return Err(P2pMeshInputError::InputLimit);
        }
        if command_count > self.limits.max_commands {
            return Err(P2pMeshInputError::CommandLimit);
        }
        let input = r.take(input_len)?;
        let mut commands = Vec::with_capacity(command_count);
        for _ in 0..command_count {
            let len = r.u32()? as usize;
            if len > self.limits.max_command_bytes {
                return Err(P2pMeshInputError::CommandLimit);
            }
            commands.push(r.take(len)?);
        }
        if !r.0.is_empty() {
            return Err(P2pMeshInputError::Malformed);
        }
        let key = (tick, slot);
        if self.duplicate(key, canonical)? {
            return Ok(P2pMeshInputAccepted::Duplicate);
        }
        self.evidence_budget(canonical.len())?;
        budget(
            self.incoming.len(),
            1,
            self.limits.max_incoming_records,
            P2pMeshInputError::RecordLimit,
        )?;
        budget(
            self.incoming_bytes,
            bytes.len(),
            self.limits.max_incoming_encoded_bytes,
            P2pMeshInputError::ByteLimit,
        )?;
        // No codec decode or state mutation before envelope/framing/budget validation.
        let input = C::decode_input(input).ok_or(P2pMeshInputError::Malformed)?;
        let commands = commands
            .into_iter()
            .map(|b| C::decode_command(b).ok_or(P2pMeshInputError::Malformed))
            .collect::<Result<Vec<_>, _>>()?;
        let record = RemoteInput {
            tick,
            slot,
            input,
            commands,
            disconnected: false,
        };
        // Re-encoding enforces the codec's canonical representation, including trailing bytes.
        if encode::<G, C>(&record, &self.limits)? != canonical {
            return Err(P2pMeshInputError::Malformed);
        }
        self.vacancy(edge.remote, &record, canonical)?;
        self.seen.insert(
            key,
            Evidence {
                author: edge.remote,
                bytes: Rc::new(canonical.to_vec()),
            },
        );
        self.seen_bytes += canonical.len();
        self.incoming_bytes += bytes.len();
        self.incoming.push_back((record, bytes.len()));
        Ok(P2pMeshInputAccepted::New)
    }
}

pub struct P2pMeshInputDriver<G: Game, C: P2pInputCodec<G>>(Rc<RefCell<State<G, C>>>);
pub struct P2pMeshInputSource<G: Game, C: P2pInputCodec<G>>(Rc<RefCell<State<G, C>>>);
impl<G: Game, C: P2pInputCodec<G>> Clone for P2pMeshInputDriver<G, C> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}
impl<G: Game, C: P2pInputCodec<G>> P2pMeshInputDriver<G, C> {
    pub fn new(
        local: PlayerSlot,
        context: CheckedJoinContext,
        limits: P2pMeshInputLimits,
    ) -> Result<(Self, P2pMeshInputSource<G, C>), P2pMeshInputError> {
        limits.validate()?;
        let r = context.roster();
        if local.0 >= 3
            || r.player_count() != 3
            || r.joiner() != PlayerSlot(2)
            || r.donor() != PlayerSlot(0)
            || r.peers() != [PlayerSlot(0), PlayerSlot(1)]
        {
            return Err(P2pMeshInputError::InvalidConfig);
        }
        let mut default_input = Vec::new();
        C::encode_input(&G::Input::default(), &mut default_input);
        if default_input.len() > limits.max_input_bytes {
            return Err(P2pMeshInputError::InvalidConfig);
        }
        let state = Rc::new(RefCell::new(State {
            local,
            context,
            limits,
            grant: None,
            default_input,
            edges: BTreeMap::new(),
            seen: BTreeMap::new(),
            seen_bytes: 0,
            incoming: VecDeque::new(),
            incoming_bytes: 0,
            destination_records: 0,
            pending_bytes: 0,
            error: None,
            flushing: false,
            marker: PhantomData,
        }));
        Ok((Self(Rc::clone(&state)), P2pMeshInputSource(state)))
    }
    pub fn check(&self) -> Result<(), P2pMeshInputError> {
        self.0.borrow().check()
    }
    /// Existing peers use only an accepted/imported checked ticket, never raw cutoff fields.
    pub fn install_ticket(&self, ticket: &CheckedJoinTicket) -> Result<(), P2pMeshInputError> {
        let mut s = self.0.borrow_mut();
        s.check()?;
        let t = ticket.ticket();
        let result = if s.local == PlayerSlot(2) || t.slot != PlayerSlot(2) {
            Err(P2pMeshInputError::InvalidSnapshot)
        } else {
            s.install_grant(
                ticket.context(),
                Grant {
                    snapshot: t.snapshot_tick,
                    first: t.first_input_tick,
                },
            )
        };
        result.map_err(|e| s.fail(e))
    }
    /// Joiner: immediately after checked bootstrap snapshot acceptance, before advance.
    /// Verifies the exact source identity; a different Session cannot bless this driver.
    pub fn install_joiner_snapshot(
        &self,
        bootstrap: &JoinBootstrap<G, P2pMeshInputSource<G, C>>,
    ) -> Result<(), P2pMeshInputError> {
        let mut s = self.0.borrow_mut();
        s.check()?;
        let result = (|| {
            if s.local != PlayerSlot(2)
                || !s.seen.is_empty()
                || !s.edges.is_empty()
                || !matches!(
                    bootstrap.status(),
                    Ok(JoinBootstrapStatus::Syncing { .. } | JoinBootstrapStatus::Ready)
                )
            {
                return Err(P2pMeshInputError::InvalidSnapshot);
            }
            let session = bootstrap
                .session()
                .ok_or(P2pMeshInputError::InvalidSnapshot)?;
            if !Rc::ptr_eq(&self.0, &session.source().0)
                || !session.authored_since(0).is_empty()
                || session.config().relay
                || session.config().local_slot != PlayerSlot(2)
                || session.config().player_count != 3
            {
                return Err(P2pMeshInputError::InvalidSnapshot);
            }
            let context = bootstrap.context().ok_or(P2pMeshInputError::WrongContext)?;
            s.install_grant(
                &context,
                Grant {
                    snapshot: session.verified_tick(),
                    first: session.next_send_tick(),
                },
            )
        })();
        result.map_err(|e| s.fail(e))
    }
    /// Adds one immutable admitted direct edge and atomically queues its backlog.
    /// The application validates transport identity before calling this method.
    pub fn admit_edge(&self, edge: P2pMeshEdge) -> Result<(), P2pMeshInputError> {
        let mut s = self.0.borrow_mut();
        s.check()?;
        s.admit(edge).map_err(|e| s.fail(e))
    }
    /// Translate an adapter-local event; bare raw IDs from different adapters collide.
    pub fn resolve_connection(
        &self,
        adapter_id: u64,
        raw: ConnId,
    ) -> Result<ConnId, P2pMeshInputError> {
        let s = self.0.borrow();
        s.check()?;
        s.edges
            .values()
            .find(|e| e.admitted.adapter_id == adapter_id && e.admitted.raw_connection == raw)
            .map(|e| e.admitted.connection)
            .ok_or(P2pMeshInputError::WrongConnection)
    }
    pub fn receive(
        &self,
        connection: ConnId,
        channel: Channel,
        bytes: &[u8],
    ) -> Result<P2pMeshInputAccepted, P2pMeshInputError> {
        let mut s = self.0.borrow_mut();
        s.check()?;
        s.receive(connection, channel, bytes).map_err(|e| s.fail(e))
    }
    /// Optional fallible local submission for applications outside Session. The
    /// authority checks are identical; this never forwards third-party records.
    pub fn send_local(&self, record: RemoteInput<G>) -> Result<(), P2pMeshInputError> {
        let mut s = self.0.borrow_mut();
        s.check()?;
        s.send(record).map_err(|e| s.fail(e))
    }
    pub fn pending(&self) -> (usize, usize) {
        let s = self.0.borrow();
        (
            s.edges.values().map(|e| e.pending.len()).sum(),
            s.pending_bytes,
        )
    }
    pub fn retained_evidence(&self) -> (usize, usize) {
        let s = self.0.borrow();
        (s.seen.len(), s.seen_bytes)
    }
    pub fn destination_records(&self) -> usize {
        self.0.borrow().destination_records
    }
    pub fn incoming(&self) -> (usize, usize) {
        let s = self.0.borrow();
        (s.incoming.len(), s.incoming_bytes)
    }
    pub fn delivery(
        &self,
        connection: ConnId,
        tick: u64,
        slot: PlayerSlot,
    ) -> Option<P2pMeshDelivery> {
        self.0
            .borrow()
            .edges
            .get(&connection)?
            .delivery
            .get(&(tick, slot))
            .copied()
    }
    /// Each edge gets only its initial queue budget. A blocked head keeps its
    /// FIFO place while the other edge progresses. Successful destinations are
    /// queue-accepted exactly once. Caller code runs outside the RefCell borrow.
    pub fn flush(
        &self,
        mut send: impl FnMut(ConnId, &[u8]) -> Result<(), SendError>,
    ) -> Result<P2pMeshInputFlush, P2pMeshInputError> {
        let initial: Vec<_> = {
            let mut s = self.0.borrow_mut();
            s.check()?;
            if s.flushing {
                return Err(s.fail(P2pMeshInputError::ReentrantFlush));
            }
            s.flushing = true;
            s.edges
                .iter()
                .map(|(&id, e)| (id, e.pending.len()))
                .collect()
        };
        let result = (|| {
            let mut out = P2pMeshInputFlush::default();
            for (id, count) in initial {
                for _ in 0..count {
                    let bytes = {
                        let s = self.0.borrow();
                        s.check()?;
                        Rc::clone(&s.edges[&id].pending.front().expect("initial FIFO head").1)
                    };
                    let result = send(id, &bytes);
                    let mut s = self.0.borrow_mut();
                    s.check()?;
                    match result {
                        Ok(()) => {
                            let edge = s.edges.get_mut(&id).unwrap();
                            let (key, bytes) =
                                edge.pending.pop_front().expect("head survives callback");
                            edge.pending_bytes -= bytes.len();
                            edge.delivery.insert(key, P2pMeshDelivery::QueueAccepted);
                            s.pending_bytes -= bytes.len();
                            out.sent += 1;
                        }
                        Err(SendError::Backpressure) => {
                            out.backpressured_edges += 1;
                            break;
                        }
                        Err(e) => return Err(s.fail(P2pMeshInputError::Transport(e))),
                    }
                }
            }
            out.pending = self.pending().0;
            Ok(out)
        })();
        self.0.borrow_mut().flushing = false;
        result
    }
    pub fn disconnected(&self, connection: ConnId) -> Result<(), P2pMeshInputError> {
        let mut s = self.0.borrow_mut();
        s.check()?;
        let e = if s.edges.contains_key(&connection) {
            P2pMeshInputError::Disconnected
        } else {
            P2pMeshInputError::WrongConnection
        };
        Err(s.fail(e))
    }
    pub fn cancel(&self) {
        self.0.borrow_mut().fail(P2pMeshInputError::Cancelled);
    }
}
impl<G: Game, C: P2pInputCodec<G>> InputSource<G> for P2pMeshInputSource<G, C> {
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
    // Finite retention deliberately ignores local verification. It is not an ACK.
}
