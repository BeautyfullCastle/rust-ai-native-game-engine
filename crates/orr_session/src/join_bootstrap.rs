//! Opt-in, transport-independent P2P late-join coordination.
//!
//! The caller supplies a **completed** membership view, verifies the sender of
//! each message, and moves snapshots, notices and input backlogs over its links.
//! This module neither discovers peers nor authenticates a transport. Every
//! active peer (including the snapshot donor) contributes exactly one notice;
//! vacant slots do not contribute. Unknown membership must not use this path.
//!
//! The default constructor uses generation-fenced ORRQ/ORRJ/ORRB v3 envelopes.
//! Retries retain a nonzero join id and increase attempt. A new membership
//! generation requires a fresh caller-provided join id. Input backlogs remain
//! caller-scoped: control fencing is neither authentication nor input fencing.
//! `new_legacy_v2` explicitly retains the old caller-fenced control protocol.
//!
//! `Ready` proves only advertised input coverage, not receipt of those inputs.
//! Local input is released by the existing `Session` handoff rule at that point.
//! Actual catch-up additionally requires reaching a caller-chosen verified tick.

use std::collections::BTreeMap;

use crate::{
    join, CheckedJoinContext, CheckedJoinError, Game, InputSource, JoinAttempts, JoinError,
    JoinStatus, PlayerSlot, Session, SessionConfig,
};

/// A caller-supplied, completed membership view, frozen for one bootstrap.
/// Construction checks a complete partition of all slots into the joiner,
/// distinct active peers and distinct vacant slots. It cannot verify discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinRoster {
    player_count: u8,
    joiner: PlayerSlot,
    donor: PlayerSlot,
    peers: Vec<PlayerSlot>,
}

impl JoinRoster {
    /// `active` includes the donor and excludes the joiner and vacant slots.
    /// Call only after discovery is complete; an empty/partial roster is invalid.
    pub fn completed(
        player_count: u8,
        joiner: PlayerSlot,
        donor: PlayerSlot,
        mut active: Vec<PlayerSlot>,
        vacant: &[PlayerSlot],
    ) -> Result<Self, JoinBootstrapError> {
        let mut seen = vec![false; usize::from(player_count)];
        for slot in std::iter::once(&joiner).chain(&active).chain(vacant) {
            let Some(occupied) = seen.get_mut(usize::from(slot.0)) else {
                return Err(JoinBootstrapError::InvalidRoster);
            };
            if std::mem::replace(occupied, true) {
                return Err(JoinBootstrapError::InvalidRoster);
            }
        }
        if active.is_empty() || !active.contains(&donor) || seen.iter().any(|s| !s) {
            return Err(JoinBootstrapError::InvalidRoster);
        }
        active.sort_unstable();
        Ok(Self {
            player_count,
            joiner,
            donor,
            peers: active,
        })
    }

    pub fn player_count(&self) -> u8 {
        self.player_count
    }
    pub fn joiner(&self) -> PlayerSlot {
        self.joiner
    }
    pub fn donor(&self) -> PlayerSlot {
        self.donor
    }

    pub fn peers(&self) -> &[PlayerSlot] {
        &self.peers
    }
}

/// Bootstrap lifecycle. `Ready` is coverage, not input delivery or catch-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinBootstrapStatus {
    Idle,
    AwaitingSnapshot {
        received: u32,
        expected: u32,
    },
    Syncing {
        received: u32,
        expected: u32,
    },
    Ready,
    /// All member notices were accepted, but their coverage proves a gap.
    /// Terminal for this attempt; cancel and retry with a fresh snapshot.
    InputGap {
        slot: PlayerSlot,
        needed_from: u64,
        available_from: u64,
    },
    Cancelled,
    Invalidated,
}

/// Rejection without silently falling back to the legacy unchecked join path.
#[derive(Debug)]
pub enum JoinBootstrapError {
    Join(JoinError),
    Checked(CheckedJoinError),
    InvalidRoster,
    InvalidConfig,
    Inactive,
    FailedAttempt,
    Invalidated,
    SessionExists,
    NonMember(PlayerSlot),
    WrongDonor(PlayerSlot),
    SenderMismatch,
    ConflictingNotice(PlayerSlot),
    NoticeBudget { limit: usize },
}

impl core::fmt::Display for JoinBootstrapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Join(e) => write!(f, "{e}"),
            Self::Checked(e) => write!(f, "{e}"),
            Self::InvalidRoster => write!(
                f,
                "join requires a completed, distinct active/vacant roster"
            ),
            Self::InvalidConfig => write!(
                f,
                "join config does not match the P2P roster or notice budget"
            ),
            Self::Inactive => write!(f, "no active join attempt"),
            Self::FailedAttempt => write!(f, "join input gap; cancel before retrying"),
            Self::Invalidated => write!(f, "join membership changed; bootstrap invalidated"),
            Self::SessionExists => {
                write!(f, "cancel and recover the existing session before retrying")
            }
            Self::NonMember(s) => write!(f, "slot {} is not an active roster member", s.0),
            Self::WrongDonor(s) => write!(f, "slot {} is not the snapshot donor", s.0),
            Self::SenderMismatch => write!(f, "notice sender differs from caller-verified sender"),
            Self::ConflictingNotice(s) => {
                write!(f, "slot {} already supplied a different notice", s.0)
            }
            Self::NoticeBudget { limit } => {
                write!(f, "aggregate notice byte budget {limit} exceeded")
            }
        }
    }
}
impl std::error::Error for JoinBootstrapError {}
impl From<JoinError> for JoinBootstrapError {
    fn from(e: JoinError) -> Self {
        Self::Join(e)
    }
}

impl From<CheckedJoinError> for JoinBootstrapError {
    fn from(e: CheckedJoinError) -> Self {
        // Preserve the established error shape for shared codec/attempt errors.
        match e {
            CheckedJoinError::Join(e) => Self::Join(e),
            other => Self::Checked(other),
        }
    }
}

/// Owns the checked join session and bounded notice staging across a single
/// membership generation. The legacy `JoinAttempts`/`Session` APIs are unchanged.
///
/// The caller supplies completed membership and verified sender identities;
/// this helper does not discover peers or authenticate links. The default v3
/// path binds control traffic to the frozen generation and complete roster.
/// Legacy v2 construction still requires caller-fenced control links. Both paths
/// require input delivery to be scoped by the caller to the current generation.
///
/// `Ready` proves advertised coverage only, not input receipt or catch-up.
/// Continue advancing/polling; use `caught_up_to` for a separate verified-tick
/// target. Do not wait for catch-up before advancing: the existing Session
/// handoff needs the joiner's locally authored inputs to make progress.
///
/// Retry after an input gap: call `cancel`, clean up the returned session's links
/// and peer-side holds as appropriate, then `next_request`. Cancellation does not
/// send anything, release remote holds, or undo an already completed handoff.
/// After `Ready`, input may already be in flight; cancellation cannot retract it
/// and does not by itself make slot reassignment or another join safe.
pub struct JoinBootstrap<G: Game, S: InputSource<G>> {
    roster: JoinRoster,
    attempts: JoinAttempts,
    budget: usize,
    snapshot_wire_limit: Option<usize>,
    checked_context: Option<CheckedJoinContext>,
    notices: BTreeMap<PlayerSlot, Vec<u8>>,
    notice_bytes: usize,
    session: Option<Session<G, S>>,
    state: JoinBootstrapStatus,
}

impl<G: Game, S: InputSource<G>> JoinBootstrap<G, S> {
    /// The explicit budget bounds aggregate retained wire bytes, including after
    /// snapshot acceptance. Incoming length is checked before decoding/copying.
    /// `cfg.join_backlog_peers` is derived from the roster, never trusted as input.
    /// Unlike the draft v2 bootstrap, this defaults to checked v3 with no
    /// fallback. The snapshot limit covers the complete incoming envelope and
    /// is checked before inner decoding/decompression. It is a wire-byte limit,
    /// not an absolute decompressed-size cap; legacy LZ4 bounds still apply.
    pub fn new(
        cfg: SessionConfig,
        roster: JoinRoster,
        notice_byte_budget: usize,
        snapshot_wire_limit: usize,
    ) -> Result<Self, JoinBootstrapError> {
        if snapshot_wire_limit == 0 {
            return Err(JoinBootstrapError::InvalidConfig);
        }
        Self::construct(cfg, roster, notice_byte_budget, Some(snapshot_wire_limit))
    }

    /// Explicit compatibility path for unchanged raw v2 control messages.
    /// Callers MUST fence control traffic by generation as well as scope inputs.
    /// The original `Session` and `JoinAttempts` APIs and wire remain unchanged.
    pub fn new_legacy_v2(
        cfg: SessionConfig,
        roster: JoinRoster,
        notice_byte_budget: usize,
    ) -> Result<Self, JoinBootstrapError> {
        Self::construct(cfg, roster, notice_byte_budget, None)
    }

    fn construct(
        mut cfg: SessionConfig,
        roster: JoinRoster,
        notice_byte_budget: usize,
        snapshot_wire_limit: Option<usize>,
    ) -> Result<Self, JoinBootstrapError> {
        if cfg.player_count != roster.player_count
            || cfg.local_slot != roster.joiner
            || cfg.relay
            || !cfg.vacant_slots.is_empty()
            || notice_byte_budget == 0
        {
            return Err(JoinBootstrapError::InvalidConfig);
        }
        cfg.join_backlog_peers = roster.peers.len() as u32;
        Ok(Self {
            roster,
            attempts: JoinAttempts::new(cfg)?,
            budget: notice_byte_budget,
            snapshot_wire_limit,
            checked_context: None,
            notices: BTreeMap::new(),
            notice_bytes: 0,
            session: None,
            state: JoinBootstrapStatus::Idle,
        })
    }

    /// Start/retry with the same frozen roster. Any staged notices are cleared,
    /// including when the retry limit is exhausted. Recover a session using
    /// `cancel` first. An invalidated membership generation cannot be retried.
    pub fn next_request(&mut self) -> Result<Vec<u8>, JoinBootstrapError> {
        if self.state == JoinBootstrapStatus::Invalidated {
            return Err(JoinBootstrapError::Invalidated);
        }
        if self.session.is_some() {
            return Err(JoinBootstrapError::SessionExists);
        }
        self.clear_notices();
        self.checked_context = None;
        self.state = JoinBootstrapStatus::Idle;
        let request = self.attempts.next_request()?;
        self.state = JoinBootstrapStatus::AwaitingSnapshot {
            received: 0,
            expected: self.expected(),
        };
        if self.snapshot_wire_limit.is_some() {
            let context = CheckedJoinContext::new(
                self.config().join_id,
                self.config().join_attempt,
                self.roster.clone(),
            )?;
            let request = context.wrap(b"ORRQ", &request)?;
            self.checked_context = Some(context);
            Ok(request)
        } else {
            Ok(request)
        }
    }

    /// Current checked attempt; `None` before a request, after cancellation, or
    /// in legacy mode. Hosts and peers must obtain their expected context independently
    /// from completed membership, rather than trusting incoming message fields.
    pub fn context(&self) -> Option<CheckedJoinContext> {
        self.checked_context.clone()
    }

    fn payload<'a>(
        &self,
        magic: &[u8; 4],
        message: &'a [u8],
    ) -> Result<&'a [u8], JoinBootstrapError> {
        if self.snapshot_wire_limit.is_some() {
            let context = self
                .checked_context
                .as_ref()
                .ok_or(CheckedJoinError::InvalidContext)?;
            Ok(context.payload(magic, message)?)
        } else {
            Ok(message)
        }
    }

    pub fn config(&self) -> &SessionConfig {
        self.attempts.config()
    }

    /// Distinct notices retained and their total encoded byte length.
    pub fn notice_usage(&self) -> (usize, usize) {
        (self.notices.len(), self.notice_bytes)
    }

    fn expected(&self) -> u32 {
        self.roster.peers.len() as u32
    }

    fn require_active(&self) -> Result<(), JoinBootstrapError> {
        if matches!(self.status()?, JoinBootstrapStatus::InputGap { .. }) {
            return Err(JoinBootstrapError::FailedAttempt);
        }
        match self.state {
            JoinBootstrapStatus::Invalidated => Err(JoinBootstrapError::Invalidated),
            JoinBootstrapStatus::Idle | JoinBootstrapStatus::Cancelled => {
                Err(JoinBootstrapError::Inactive)
            }
            _ => Ok(()),
        }
    }

    /// Route ALL notices here, before or after the snapshot. `sender` must be
    /// verified by the caller's current join link/generation. Exact duplicates
    /// are idempotent; conflicting duplicates never replace the first valid
    /// notice or change accounting. If a peer revises its advertisement, cancel
    /// and retry the attempt instead. Invalid messages cannot advance completion.
    /// A structurally valid final notice can be accepted while returning
    /// `Ok(JoinBootstrapStatus::InputGap { .. })`: its bytes remain counted and the gap is
    /// retained by the session until cancellation/retry.
    pub fn receive_notice(
        &mut self,
        sender: PlayerSlot,
        message: &[u8],
    ) -> Result<JoinBootstrapStatus, JoinBootstrapError> {
        self.require_active()?;
        if self.roster.peers.binary_search(&sender).is_err() {
            return Err(JoinBootstrapError::NonMember(sender));
        }
        let old = self.notices.get(&sender);
        let remaining = self.budget - self.notice_bytes + old.map_or(0, Vec::len);
        if message.len() > remaining {
            return Err(JoinBootstrapError::NoticeBudget { limit: self.budget });
        }
        let payload = self.payload(b"ORRB", message)?;
        let notice = join::decode_backlog(payload)?;
        if notice.sender != sender {
            return Err(JoinBootstrapError::SenderMismatch);
        }
        if notice.attempt != self.config().join_attempt {
            return Err(JoinError::StaleAttempt {
                current: self.config().join_attempt,
                got: notice.attempt,
            }
            .into());
        }
        if notice
            .spans
            .iter()
            .any(|s| s.slot.0 >= self.roster.player_count)
        {
            return Err(JoinError::Corrupt("notice span for an unknown slot").into());
        }
        if let Some(old) = old {
            if old != message {
                return Err(JoinBootstrapError::ConflictingNotice(sender));
            }
            return self.status();
        }
        self.notice_bytes += message.len();
        self.notices.insert(sender, message.to_vec());
        if let Some(session) = &mut self.session {
            match session.receive_backlog(payload) {
                Ok(_) | Err(JoinError::InputGap { .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.status()
    }

    /// Accept only the current attempt's donor snapshot and replay staged
    /// notices in slot order. A proven gap returns the accepted `InputGap`
    /// outcome and leaves the session available for caller cleanup; malformed
    /// snapshots leave staging intact.
    /// As with `Session::from_join_snapshot`, a rejected snapshot drops `source`.
    pub fn receive_snapshot(
        &mut self,
        sender: PlayerSlot,
        config: G::Config,
        source: S,
        message: &[u8],
    ) -> Result<JoinBootstrapStatus, JoinBootstrapError> {
        self.require_active()?;
        if sender != self.roster.donor {
            return Err(JoinBootstrapError::WrongDonor(sender));
        }
        if self.session.is_some() {
            return Err(JoinBootstrapError::SessionExists);
        }
        if let Some(limit) = self.snapshot_wire_limit {
            if message.len() > limit {
                return Err(CheckedJoinError::WireLimit { limit }.into());
            }
        }
        let payload = self.payload(b"ORRJ", message)?;
        let session = Session::from_join_snapshot(config, self.config().clone(), source, payload)?;
        self.session = Some(session);
        let context = &self.checked_context;
        let session = self.session.as_mut().expect("session just installed");
        for notice in self.notices.values() {
            let payload = match &context {
                Some(context) => context.payload(b"ORRB", notice)?,
                None => notice.as_slice(),
            };
            match session.receive_backlog(payload) {
                Ok(_) => {}
                Err(JoinError::InputGap { .. }) => break,
                Err(error) => return Err(error.into()),
            }
        }
        self.status()
    }

    pub fn status(&self) -> Result<JoinBootstrapStatus, JoinBootstrapError> {
        if let Some(session) = &self.session {
            return match session.join_status() {
                Err(JoinError::InputGap {
                    slot,
                    needed_from,
                    available_from,
                }) => Ok(JoinBootstrapStatus::InputGap {
                    slot,
                    needed_from,
                    available_from,
                }),
                Err(error) => Err(error.into()),
                Ok(JoinStatus::Ready) => Ok(JoinBootstrapStatus::Ready),
                Ok(JoinStatus::Syncing { received, expected }) => {
                    Ok(JoinBootstrapStatus::Syncing { received, expected })
                }
                Ok(JoinStatus::Unchecked) => Err(JoinBootstrapError::InvalidConfig),
            };
        }
        match self.state {
            JoinBootstrapStatus::AwaitingSnapshot { .. } => {
                Ok(JoinBootstrapStatus::AwaitingSnapshot {
                    received: self.notices.len() as u32,
                    expected: self.expected(),
                })
            }
            state => Ok(state),
        }
    }

    pub fn session(&self) -> Option<&Session<G, S>> {
        self.session.as_ref()
    }

    /// Access input links without exposing the unchecked notice API.
    pub fn source_mut(&mut self) -> Option<&mut S> {
        self.session.as_mut().map(Session::source_mut)
    }

    /// Advance the accepted session using its unchanged local-input handoff.
    /// `None` means no snapshot has been accepted or the attempt failed/cleared.
    pub fn advance(
        &mut self,
        input: G::Input,
        commands: Vec<G::Command>,
    ) -> Option<crate::AdvanceResult<G>> {
        self.require_active().ok()?;
        self.session.as_mut().map(|s| s.advance(input, commands))
    }

    /// Poll delivered inputs and reconcile verified progress without authoring.
    pub fn poll_confirmed(
        &mut self,
    ) -> Option<(crate::EventBatch<G::Event>, Option<crate::RollbackInfo>)> {
        self.session.as_mut().map(Session::poll_confirmed)
    }

    /// Coverage AND verified progress, never predicted-head progress. The caller
    /// chooses/updates the target using its current view of peer progress.
    pub fn caught_up_to(&self, target_verified_tick: u64) -> Result<bool, JoinBootstrapError> {
        Ok(self.status()? == JoinBootstrapStatus::Ready
            && self
                .session
                .as_ref()
                .is_some_and(|s| s.verified_tick() >= target_verified_tick))
    }

    fn clear_notices(&mut self) {
        self.notices.clear();
        self.notice_bytes = 0;
    }

    /// Clear local staging and return the session for caller link/hold cleanup.
    /// Remote cleanup and safe slot reassignment remain the caller's job.
    pub fn cancel(&mut self) -> Option<Session<G, S>> {
        self.clear_notices();
        self.checked_context = None;
        if self.state != JoinBootstrapStatus::Invalidated {
            self.state = JoinBootstrapStatus::Cancelled;
        }
        self.session.take()
    }

    /// Any membership change invalidates the attempt, even a peer departure.
    /// Never shrink the expected count to manufacture success. A new completed
    /// membership view and caller-coordinated join exchange are required.
    pub fn invalidate_membership(&mut self) -> Option<Session<G, S>> {
        self.state = JoinBootstrapStatus::Invalidated;
        self.cancel()
    }
}
