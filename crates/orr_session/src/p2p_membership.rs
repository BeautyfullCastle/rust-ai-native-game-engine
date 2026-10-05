//! Application-admitted P2P membership and checked control routing.
//!
//! This is not discovery or authentication. The application validates a link
//! before admitting it and coordinates vacancy ownership and promotion across
//! peers. `SessionConfig::vacant_slots` and input history are not membership evidence.
//! A pending joiner overlays a committed vacancy; Connected, requests and Ready
//! never promote it. Disconnect makes an active slot unknown, not vacant.
//!
//! Connection IDs must be unique for the lifetime of this registry, including
//! delayed events. For multiple endpoints, the application must remap their IDs
//! into one namespace. Recreate the registry only after draining old events.
//! No tombstones or unadmitted-connection records are retained.
//!
//! Every membership mutation invalidates all frozen attempts conservatively.
//! Returned cleanup obligations must be coordinated before retry/promotion:
//! release local/remote holds, invalidate join bootstraps and retire old input
//! links. No cleanup wire protocol or input codec is introduced here. In
//! particular, cancellation cannot undo inputs already authored after Ready.

use std::collections::VecDeque;
use std::sync::Arc;

use orr_proto::{Channel, ConnId, ServerEvent};

use crate::{
    checked_backlog_notice, import_checked_join_ticket, serve_checked_join, CheckedJoinContext,
    CheckedJoinError, CheckedJoinTicket, Game, InputSource, JoinBootstrap, JoinBootstrapError,
    JoinBootstrapStatus, JoinRoster, PlayerSlot, Session,
};

/// Room classification, independent of any local default-input authorship.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum P2pSlotState {
    Unknown,
    Active,
    Vacant,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    state: P2pSlotState,
    conn: Option<ConnId>,
}

/// A frozen, locally registered checked attempt. Cloning does not keep it valid
/// after registry mutation or cancellation. Obtain one independently on each peer.
#[derive(Clone, Debug)]
pub struct P2pAttempt {
    registry: Arc<()>,
    revision: u64,
    serial: u64,
    context: CheckedJoinContext,
}
impl P2pAttempt {
    pub fn context(&self) -> &CheckedJoinContext {
        &self.context
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// Explicit obligations, including all former links, even if no control message
/// was successfully queued to the transport. A send success is not delivery.
/// This value never mutates a bootstrap or session. The application must track
/// which retired attempt owns each resource and clean it before replacement;
/// equal wire context alone cannot identify a bootstrap instance.
#[must_use = "coordinate hold, bootstrap and input-link cleanup before retrying"]
#[derive(Debug)]
pub struct P2pCleanup {
    pub context: CheckedJoinContext,
    pub connections: Vec<(PlayerSlot, ConnId)>,
    pub discarded_outbound_bytes: usize,
}

#[derive(Debug)]
pub enum P2pMembershipError {
    InvalidConfig,
    InvalidSlot(PlayerSlot),
    SlotCollision(PlayerSlot),
    ConnectionCollision(ConnId),
    UnknownConnection(ConnId),
    IncompleteMembership,
    AttemptExists(PlayerSlot),
    AttemptMismatch,
    Invalidated,
    RevisionExhausted,
    WrongSender,
    WrongControl,
    OutboundBudget { limit: usize },
    Checked(CheckedJoinError),
    Bootstrap(JoinBootstrapError),
}
impl core::fmt::Display for P2pMembershipError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "P2P membership: {self:?}")
    }
}
impl std::error::Error for P2pMembershipError {}
impl From<CheckedJoinError> for P2pMembershipError {
    fn from(e: CheckedJoinError) -> Self {
        Self::Checked(e)
    }
}
impl From<JoinBootstrapError> for P2pMembershipError {
    fn from(e: JoinBootstrapError) -> Self {
        Self::Bootstrap(e)
    }
}

/// An event is either forwarded intact or identified as checked control from an
/// admitted link. Routing does not validate control contents; use the adapter
/// methods below before applying them. No caller-supplied sender slot is trusted.
#[must_use]
#[derive(Debug)]
pub enum P2pRoutedEvent {
    Forward(ServerEvent),
    Disconnected {
        event: ServerEvent,
        cleanup: Vec<P2pCleanup>,
    },
    Control {
        conn: ConnId,
        data: Vec<u8>,
    },
    Rejected {
        event: ServerEvent,
        error: P2pMembershipError,
    },
}

struct Outbound {
    joiner: PlayerSlot,
    conn: ConnId,
    bytes: Vec<u8>,
}

/// Flush progress. Errors retain the entire failed message and every later
/// message for that connection. `queued` means accepted by the callback, never
/// delivered. At most one error is recorded per admitted connection per pass.
#[derive(Debug)]
pub struct P2pFlush<E> {
    pub queued: usize,
    pub blocked: Vec<(ConnId, E)>,
    pub remaining_bytes: usize,
}

/// Fixed-size slot and attempt tables; no more than `player_count` bindings or
/// pending attempts. Retained outbound payloads share an explicit byte budget.
/// No transport Connected events are stored. The local identity is immutable.
///
/// The existing abstract `Endpoint::send` cannot report backpressure and may
/// silently ignore unknown connections. Therefore `flush` deliberately takes a
/// fallible callback: use a transport enqueue API that reports acceptance, not
/// an unconditional `Ok(())` wrapper around that void method.
pub struct P2pMembership {
    identity: Arc<()>,
    local: PlayerSlot,
    slots: Vec<Slot>,
    revision: u64,
    attempts: Vec<Option<P2pAttempt>>,
    outbound: VecDeque<Outbound>,
    outbound_bytes: usize,
    outbound_budget: usize,
    next_serial: u64,
}
impl P2pMembership {
    /// All slots start unknown; even the local slot requires classification.
    pub fn new(
        player_count: u8,
        local: PlayerSlot,
        outbound_budget: usize,
    ) -> Result<Self, P2pMembershipError> {
        if player_count < 2 || local.0 >= player_count || outbound_budget == 0 {
            return Err(P2pMembershipError::InvalidConfig);
        }
        Ok(Self {
            identity: Arc::new(()),
            local,
            slots: vec![
                Slot {
                    state: P2pSlotState::Unknown,
                    conn: None
                };
                player_count as usize
            ],
            revision: 0,
            attempts: vec![None; player_count as usize],
            outbound: VecDeque::new(),
            outbound_bytes: 0,
            outbound_budget,
            next_serial: 1,
        })
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn local_slot(&self) -> PlayerSlot {
        self.local
    }
    pub fn outbound_usage(&self) -> (usize, usize) {
        (self.outbound.len(), self.outbound_bytes)
    }
    pub fn pending_count(&self) -> usize {
        self.attempts.iter().flatten().count()
    }
    fn slot(&self, slot: PlayerSlot) -> Result<&Slot, P2pMembershipError> {
        self.slots
            .get(slot.0 as usize)
            .ok_or(P2pMembershipError::InvalidSlot(slot))
    }
    pub fn state(&self, slot: PlayerSlot) -> Result<P2pSlotState, P2pMembershipError> {
        Ok(self.slot(slot)?.state)
    }
    pub fn connection(&self, slot: PlayerSlot) -> Result<Option<ConnId>, P2pMembershipError> {
        Ok(self.slot(slot)?.conn)
    }
    pub fn sender(&self, conn: ConnId) -> Result<PlayerSlot, P2pMembershipError> {
        self.slots
            .iter()
            .position(|s| s.conn == Some(conn))
            .map(|i| PlayerSlot(i as u8))
            .ok_or(P2pMembershipError::UnknownConnection(conn))
    }
    fn available_connection(&self, conn: ConnId) -> Result<(), P2pMembershipError> {
        if self.slots.iter().any(|s| s.conn == Some(conn)) {
            Err(P2pMembershipError::ConnectionCollision(conn))
        } else {
            Ok(())
        }
    }
    fn cleanup(&mut self, attempt: P2pAttempt) -> P2pCleanup {
        let joiner = attempt.context.roster().joiner();
        let mut discarded = 0;
        self.outbound.retain(|m| {
            if m.joiner == joiner {
                discarded += m.bytes.len();
                false
            } else {
                true
            }
        });
        self.outbound_bytes -= discarded;
        P2pCleanup {
            context: attempt.context,
            connections: self
                .slots
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.conn.map(|c| (PlayerSlot(i as u8), c)))
                .collect(),
            discarded_outbound_bytes: discarded,
        }
    }
    fn change(
        &mut self,
        slot: PlayerSlot,
        state: P2pSlotState,
        conn: Option<ConnId>,
    ) -> Result<Vec<P2pCleanup>, P2pMembershipError> {
        let old = self.slot(slot)?;
        if old.state == state && old.conn == conn {
            return Ok(vec![]);
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(P2pMembershipError::RevisionExhausted)?;
        let attempts: Vec<_> = self.attempts.iter_mut().filter_map(Option::take).collect();
        let cleanup = attempts.into_iter().map(|a| self.cleanup(a)).collect();
        self.slots[slot.0 as usize] = Slot { state, conn };
        self.revision = revision;
        Ok(cleanup)
    }
    /// Application has independently approved the local active identity.
    pub fn admit_local(&mut self) -> Result<Vec<P2pCleanup>, P2pMembershipError> {
        if self.slot(self.local)?.state == P2pSlotState::Vacant {
            return Err(P2pMembershipError::SlotCollision(self.local));
        }
        self.change(self.local, P2pSlotState::Active, None)
    }
    /// Admit an active remote only into an unknown slot. Vacancy promotion must
    /// instead be explicit via `promote_joiner` after coordinated handoff.
    pub fn admit_active(
        &mut self,
        slot: PlayerSlot,
        conn: ConnId,
    ) -> Result<Vec<P2pCleanup>, P2pMembershipError> {
        if slot == self.local || self.slot(slot)?.state != P2pSlotState::Unknown {
            return Err(P2pMembershipError::SlotCollision(slot));
        }
        self.available_connection(conn)?;
        self.change(slot, P2pSlotState::Active, Some(conn))
    }
    /// Record the application's committed room vacancy. This does not modify
    /// Session default-input ownership or make a disconnected slot safe to reuse.
    pub fn commit_vacant(
        &mut self,
        slot: PlayerSlot,
    ) -> Result<Vec<P2pCleanup>, P2pMembershipError> {
        if self.slot(slot)?.state == P2pSlotState::Vacant {
            return Ok(vec![]);
        }
        self.change(slot, P2pSlotState::Vacant, None)
    }
    pub fn clear_vacancy(
        &mut self,
        slot: PlayerSlot,
    ) -> Result<Vec<P2pCleanup>, P2pMembershipError> {
        if self.slot(slot)?.state != P2pSlotState::Vacant {
            return Err(P2pMembershipError::SlotCollision(slot));
        }
        self.change(slot, P2pSlotState::Unknown, None)
    }
    /// Admit a pending remote joining link without clearing the vacancy.
    pub fn admit_joiner(
        &mut self,
        slot: PlayerSlot,
        conn: ConnId,
    ) -> Result<Vec<P2pCleanup>, P2pMembershipError> {
        let s = self.slot(slot)?;
        if slot == self.local || s.state != P2pSlotState::Vacant || s.conn.is_some() {
            return Err(P2pMembershipError::SlotCollision(slot));
        }
        self.available_connection(conn)?;
        self.change(slot, P2pSlotState::Vacant, Some(conn))
    }
    /// Replace exactly the current binding. Old messages/disconnects cannot
    /// affect this replacement. The new ID must never have been used before.
    pub fn replace_connection(
        &mut self,
        slot: PlayerSlot,
        old: ConnId,
        new: ConnId,
    ) -> Result<Vec<P2pCleanup>, P2pMembershipError> {
        let s = *self.slot(slot)?;
        if s.conn != Some(old) {
            return Err(P2pMembershipError::UnknownConnection(old));
        }
        self.available_connection(new)?;
        self.change(slot, s.state, Some(new))
    }
    /// Application-only coordinated promotion; Ready does not call this. Local
    /// joiners have no transport binding; remote joiners must be admitted first.
    pub fn promote_joiner(
        &mut self,
        slot: PlayerSlot,
    ) -> Result<Vec<P2pCleanup>, P2pMembershipError> {
        let s = *self.slot(slot)?;
        if s.state != P2pSlotState::Vacant || (slot != self.local && s.conn.is_none()) {
            return Err(P2pMembershipError::SlotCollision(slot));
        }
        self.change(slot, P2pSlotState::Active, s.conn)
    }
    pub fn disconnect(&mut self, conn: ConnId) -> Result<Vec<P2pCleanup>, P2pMembershipError> {
        let Ok(slot) = self.sender(conn) else {
            return Ok(vec![]);
        };
        let state = if self.slot(slot)?.state == P2pSlotState::Vacant {
            P2pSlotState::Vacant
        } else {
            P2pSlotState::Unknown
        };
        self.change(slot, state, None)
    }
    pub fn roster(
        &self,
        joiner: PlayerSlot,
        donor: PlayerSlot,
    ) -> Result<JoinRoster, P2pMembershipError> {
        let joining = self.slot(joiner)?;
        if joining.state != P2pSlotState::Vacant || (joiner != self.local && joining.conn.is_none())
        {
            return Err(P2pMembershipError::IncompleteMembership);
        }
        let mut active = Vec::new();
        let mut vacant = Vec::new();
        for (i, slot) in self.slots.iter().enumerate() {
            let id = PlayerSlot(i as u8);
            match slot.state {
                P2pSlotState::Unknown => return Err(P2pMembershipError::IncompleteMembership),
                P2pSlotState::Active => active.push(id),
                P2pSlotState::Vacant if id != joiner => vacant.push(id),
                P2pSlotState::Vacant => {}
            }
        }
        Ok(JoinRoster::completed(
            self.slots.len() as u8,
            joiner,
            donor,
            active,
            &vacant,
        )?)
    }
    /// Freeze independently approved generation/attempt numbers and the exact
    /// current roster. The incoming request must never supply this authority.
    /// Complete prior cleanup before calling. Increase attempt for retries in
    /// one generation and use a fresh join_id after membership changes. Never
    /// reuse a wire (join_id, attempt) pair for a replacement bootstrap; local
    /// handle serials do not fence delayed wire traffic or old cleanup objects.
    pub fn begin_attempt(
        &mut self,
        joiner: PlayerSlot,
        donor: PlayerSlot,
        join_id: u64,
        attempt: u32,
    ) -> Result<P2pAttempt, P2pMembershipError> {
        let roster = self.roster(joiner, donor)?;
        if self.attempts[joiner.0 as usize].is_some() {
            return Err(P2pMembershipError::AttemptExists(joiner));
        }
        let serial = self.next_serial;
        let next_serial = serial
            .checked_add(1)
            .ok_or(P2pMembershipError::RevisionExhausted)?;
        let a = P2pAttempt {
            registry: Arc::clone(&self.identity),
            revision: self.revision,
            serial,
            context: CheckedJoinContext::new(join_id, attempt, roster)?,
        };
        self.next_serial = next_serial;
        self.attempts[joiner.0 as usize] = Some(a.clone());
        Ok(a)
    }
    fn validate(&self, a: &P2pAttempt) -> Result<(), P2pMembershipError> {
        if !Arc::ptr_eq(&self.identity, &a.registry) {
            return Err(P2pMembershipError::AttemptMismatch);
        }
        if a.revision != self.revision {
            return Err(P2pMembershipError::Invalidated);
        }
        let roster = a.context.roster();
        let Some(Some(current)) = self.attempts.get(roster.joiner().0 as usize) else {
            return Err(P2pMembershipError::Invalidated);
        };
        if current.context != a.context
            || current.revision != a.revision
            || current.serial != a.serial
        {
            return Err(P2pMembershipError::AttemptMismatch);
        }
        if &self.roster(roster.joiner(), roster.donor())? != roster {
            return Err(P2pMembershipError::Invalidated);
        }
        Ok(())
    }
    pub fn cancel_attempt(&mut self, a: &P2pAttempt) -> Result<P2pCleanup, P2pMembershipError> {
        self.validate(a)?;
        let current = self.attempts[a.context.roster().joiner().0 as usize]
            .take()
            .expect("validated attempt");
        Ok(self.cleanup(current))
    }
    fn bootstrap_matches<G: Game, S: InputSource<G>>(
        &self,
        a: &P2pAttempt,
        bootstrap: &JoinBootstrap<G, S>,
    ) -> Result<(), P2pMembershipError> {
        self.validate(a)?;
        if self.local != a.context.roster().joiner()
            || bootstrap.context().as_ref() != Some(&a.context)
        {
            return Err(P2pMembershipError::AttemptMismatch);
        }
        Ok(())
    }
    pub fn serve_request<G: Game, S: InputSource<G>>(
        &self,
        a: &P2pAttempt,
        conn: ConnId,
        donor: &mut Session<G, S>,
        request: &[u8],
    ) -> Result<(Vec<u8>, CheckedJoinTicket), P2pMembershipError> {
        self.validate(a)?;
        if self.local != a.context.roster().donor()
            || self.sender(conn)? != a.context.roster().joiner()
        {
            return Err(P2pMembershipError::WrongSender);
        }
        Ok(serve_checked_join(donor, &a.context, request)?)
    }
    pub fn import_ticket<G: Game, S: InputSource<G>>(
        &self,
        a: &P2pAttempt,
        conn: ConnId,
        peer: &Session<G, S>,
        snapshot: &[u8],
        wire_limit: usize,
    ) -> Result<CheckedJoinTicket, P2pMembershipError> {
        self.validate(a)?;
        if peer.config().local_slot != self.local {
            return Err(P2pMembershipError::WrongSender);
        }
        Ok(import_checked_join_ticket(
            peer,
            &a.context,
            self.sender(conn)?,
            snapshot,
            wire_limit,
        )?)
    }
    pub fn backlog_notice<G: Game, S: InputSource<G>>(
        &self,
        a: &P2pAttempt,
        peer: &mut Session<G, S>,
        ticket: &CheckedJoinTicket,
    ) -> Result<Vec<u8>, P2pMembershipError> {
        self.validate(a)?;
        if peer.config().local_slot != self.local {
            return Err(P2pMembershipError::WrongSender);
        }
        Ok(checked_backlog_notice(peer, &a.context, ticket)?)
    }
    pub fn receive_notice<G: Game, S: InputSource<G>>(
        &self,
        a: &P2pAttempt,
        conn: ConnId,
        bootstrap: &mut JoinBootstrap<G, S>,
        data: &[u8],
    ) -> Result<JoinBootstrapStatus, P2pMembershipError> {
        self.bootstrap_matches(a, bootstrap)?;
        Ok(bootstrap.receive_notice(self.sender(conn)?, data)?)
    }
    pub fn receive_snapshot<G: Game, S: InputSource<G>>(
        &self,
        a: &P2pAttempt,
        conn: ConnId,
        bootstrap: &mut JoinBootstrap<G, S>,
        config: G::Config,
        source: S,
        data: &[u8],
    ) -> Result<JoinBootstrapStatus, P2pMembershipError> {
        self.bootstrap_matches(a, bootstrap)?;
        Ok(bootstrap.receive_snapshot(self.sender(conn)?, config, source, data)?)
    }
    /// Poll one existing Endpoint event and pass it here. Unrelated messages
    /// (including input packets) are forwarded unchanged, even when unadmitted.
    /// Checked controls on the unreliable channel are explicitly rejected.
    pub fn route_event(&mut self, event: ServerEvent) -> P2pRoutedEvent {
        match event {
            ServerEvent::Disconnected(conn) => match self.disconnect(conn) {
                Ok(cleanup) => P2pRoutedEvent::Disconnected { event, cleanup },
                Err(error) => P2pRoutedEvent::Rejected { event, error },
            },
            ServerEvent::Message {
                conn,
                channel,
                ref data,
            } if control_magic(data).is_some() => {
                let error = if channel != Channel::Reliable {
                    Some(P2pMembershipError::WrongControl)
                } else {
                    self.sender(conn).err()
                };
                if let Some(error) = error {
                    P2pRoutedEvent::Rejected { event, error }
                } else if let ServerEvent::Message { conn, data, .. } = event {
                    P2pRoutedEvent::Control { conn, data }
                } else {
                    unreachable!()
                }
            }
            _ => P2pRoutedEvent::Forward(event),
        }
    }
    /// Retain a validated control for the admitted destination. Queue rejection
    /// leaves the attempt (and its cleanup obligations) live and the caller's
    /// bytes untouched: retry or explicitly cancel, never assume delivery.
    pub fn queue_control(
        &mut self,
        a: &P2pAttempt,
        target: PlayerSlot,
        data: &[u8],
    ) -> Result<(), P2pMembershipError> {
        self.validate(a)?;
        let conn = self
            .slot(target)?
            .conn
            .ok_or(P2pMembershipError::WrongSender)?;
        let magic = control_magic(data).ok_or(P2pMembershipError::WrongControl)?;
        let roster = a.context.roster();
        let permitted = match magic {
            b"ORRQ" => self.local == roster.joiner() && target == roster.donor(),
            b"ORRJ" => {
                self.local == roster.donor()
                    && (target == roster.joiner() || roster.peers().contains(&target))
            }
            b"ORRB" => roster.peers().contains(&self.local) && target == roster.joiner(),
            _ => false,
        };
        if !permitted {
            return Err(P2pMembershipError::WrongSender);
        }
        if data.len() > self.outbound_budget - self.outbound_bytes {
            return Err(P2pMembershipError::OutboundBudget {
                limit: self.outbound_budget,
            });
        }
        let payload = a.context.payload(magic, data)?;
        if magic == b"ORRB"
            && crate::join::decode_backlog(payload)
                .map_err(CheckedJoinError::from)?
                .sender
                != self.local
        {
            return Err(P2pMembershipError::WrongSender);
        }
        self.outbound.push_back(Outbound {
            joiner: roster.joiner(),
            conn,
            bytes: data.to_vec(),
        });
        self.outbound_bytes += data.len();
        Ok(())
    }
    /// Make one bounded pass over the messages present at entry. A failed
    /// destination is skipped for the remainder of this pass, retaining its
    /// messages in FIFO order; other destinations still get a send opportunity.
    /// The returned error list is bounded by the admitted connection count.
    /// Already accepted bytes cannot be recalled by cancellation: receiver-side
    /// generation fencing and coordinated input cleanup remain necessary.
    /// The callback must return Err only when no bytes were queued. An
    /// uncertain or partially successful transport write is not retry-safe.
    pub fn flush<E>(
        &mut self,
        mut send: impl FnMut(ConnId, Channel, &[u8]) -> Result<(), E>,
    ) -> P2pFlush<E> {
        let mut queued = 0;
        let mut blocked = Vec::new();
        let pending = self.outbound.len();
        for _ in 0..pending {
            let message = self.outbound.pop_front().expect("bounded initial queue");
            if blocked.iter().any(|(conn, _)| *conn == message.conn) {
                self.outbound.push_back(message);
                continue;
            }
            match send(message.conn, Channel::Reliable, &message.bytes) {
                Ok(()) => {
                    self.outbound_bytes -= message.bytes.len();
                    queued += 1;
                }
                Err(error) => {
                    blocked.push((message.conn, error));
                    self.outbound.push_back(message);
                }
            }
        }
        P2pFlush {
            queued,
            blocked,
            remaining_bytes: self.outbound_bytes,
        }
    }
}
fn control_magic(data: &[u8]) -> Option<&'static [u8; 4]> {
    match data.get(..4)? {
        b"ORRQ" => Some(b"ORRQ"),
        b"ORRJ" => Some(b"ORRJ"),
        b"ORRB" => Some(b"ORRB"),
        _ => None,
    }
}
