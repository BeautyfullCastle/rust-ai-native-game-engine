//! Application-coordinated departure fencing and verified-history repair.
//!
//! This is not discovery, authentication, or distributed consensus. The application
//! supplies complete membership, authenticates every sender, fences every old-author
//! route, and settles already admitted traffic before reporting. It transports repair
//! records explicitly and acknowledges only after reconciliation and verification.
//! Missing history never permits skipping a survivor or lowering the target. Notify
//! this barrier of every membership change and invalidate it before Session replacement
//! or restore, even if the replacement has identical configuration and state.

use crate::{Game, InputSource, JoinError, PlayerSlot, Session, SessionConfig};
use std::collections::BTreeMap;

/// One survivor's application-confirmed stable post-fence checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepartureFence {
    pub recovery_id: u64,
    pub revision: u64,
    pub survivor: PlayerSlot,
    pub verified_tick: u64,
    pub departed_max: Option<u64>,
}

/// Frozen decision carried in every verification acknowledgment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepartureTarget {
    pub recovery_id: u64,
    pub revision: u64,
    pub departed: PlayerSlot,
    pub owner: PlayerSlot,
    pub target: u64,
    pub cutoff: u64,
}

/// Application assertion of verified (not predicted or received) state at target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepartureAck {
    pub target: DepartureTarget,
    pub survivor: PlayerSlot,
    pub checksum: u64,
}

#[derive(Debug)]
pub enum DepartureError {
    InvalidConfig,
    Invalidated,
    Committed,
    UnknownSurvivor,
    StaleContext,
    Contradiction,
    MembershipChanged,
    TickExhausted,
    MissingFences,
    MissingAcknowledgments,
    SessionMismatch,
    UnverifiedTarget,
    Vacate(JoinError),
}
impl std::fmt::Display for DepartureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "departure barrier: {self:?}")
    }
}
impl std::error::Error for DepartureError {}

/// A bounded one-shot barrier. No wire messages or Session policy are changed.
/// Reports are trusted application assertions, not cryptographic proofs.
///
/// The application supplies complete membership, authenticates each sender,
/// fences every departed-author route, and settles all previously admitted
/// traffic before reporting. It transports repairs and acknowledges only after
/// reconciliation and verification. This helper provides neither discovery nor
/// distributed consensus. Missing history never allows dropping a survivor.
///
/// Use a fresh, never-reused recovery ID for each recovery. Advance the membership
/// revision on every membership, peer identity, or designated-owner change and
/// notify this barrier with [`Self::check_membership`] or [`Self::invalidate`].
/// Before any participating Session is replaced or restored, invalidate even if
/// its configuration and state are identical: Session identity is not observable
/// here. Do not continue on stale acknowledgments after such changes.
pub struct DepartureBarrier {
    recovery_id: u64,
    revision: u64,
    departed: PlayerSlot,
    owner: PlayerSlot,
    survivors: Vec<PlayerSlot>,
    config: SessionConfig,
    fences: BTreeMap<PlayerSlot, DepartureFence>,
    target: Option<DepartureTarget>,
    acks: BTreeMap<PlayerSlot, DepartureAck>,
    invalidated: bool,
    committed: bool,
}
impl DepartureBarrier {
    /// `survivors` is the complete authenticated active set, including the owner;
    /// `vacant` completes the partition of slots. No unknown slot is permitted.
    pub fn new(
        recovery_id: u64,
        revision: u64,
        departed: PlayerSlot,
        owner: PlayerSlot,
        mut survivors: Vec<PlayerSlot>,
        vacant: &[PlayerSlot],
        config: &SessionConfig,
    ) -> Result<Self, DepartureError> {
        let mut seen = vec![false; usize::from(config.player_count)];
        for slot in std::iter::once(&departed).chain(&survivors).chain(vacant) {
            let Some(present) = seen.get_mut(usize::from(slot.0)) else {
                return Err(DepartureError::InvalidConfig);
            };
            if std::mem::replace(present, true) {
                return Err(DepartureError::InvalidConfig);
            }
        }
        if recovery_id == 0
            || config.relay
            || owner != config.local_slot
            || !survivors.contains(&owner)
            || seen.iter().any(|p| !p)
        {
            return Err(DepartureError::InvalidConfig);
        }
        survivors.sort_unstable();
        Ok(Self {
            recovery_id,
            revision,
            departed,
            owner,
            survivors,
            config: config.clone(),
            fences: BTreeMap::new(),
            target: None,
            acks: BTreeMap::new(),
            invalidated: false,
            committed: false,
        })
    }
    pub fn invalidate(&mut self) {
        self.invalidated = true;
    }
    pub fn is_committed(&self) -> bool {
        self.committed
    }
    fn active(&self) -> Result<(), DepartureError> {
        if self.invalidated {
            Err(DepartureError::Invalidated)
        } else if self.committed {
            Err(DepartureError::Committed)
        } else {
            Ok(())
        }
    }
    fn poison<T>(&mut self, error: DepartureError) -> Result<T, DepartureError> {
        self.invalidate();
        Err(error)
    }
    /// Call on every membership notification, including survivor replacement.
    /// Any revision change invalidates, even if the survivor slots are unchanged.
    pub fn check_membership(
        &mut self,
        revision: u64,
        survivors: &[PlayerSlot],
    ) -> Result<(), DepartureError> {
        self.active()?;
        let mut current = survivors.to_vec();
        current.sort_unstable();
        if revision != self.revision || current != self.survivors {
            return self.poison(DepartureError::MembershipChanged);
        }
        Ok(())
    }
    /// Submit only after all old-author routes are fenced and previously admitted
    /// traffic is settled. An exact duplicate is idempotent; a changed report is fatal.
    pub fn report_fence(&mut self, report: DepartureFence) -> Result<(), DepartureError> {
        self.active()?;
        if report.recovery_id != self.recovery_id || report.revision != self.revision {
            return Err(DepartureError::StaleContext);
        }
        if !self.survivors.contains(&report.survivor) {
            return Err(DepartureError::UnknownSurvivor);
        }
        if let Some(prior) = self.fences.get(&report.survivor) {
            return if *prior == report {
                Ok(())
            } else {
                self.poison(DepartureError::Contradiction)
            };
        }
        self.fences.insert(report.survivor, report);
        if self.fences.len() == self.survivors.len() {
            let target = self
                .fences
                .values()
                .fold(u64::from(self.config.input_delay), |t, r| {
                    t.max(r.verified_tick).max(r.departed_max.unwrap_or(0))
                });
            let Some(cutoff) = target.checked_add(1) else {
                return self.poison(DepartureError::TickExhausted);
            };
            self.target = Some(DepartureTarget {
                recovery_id: self.recovery_id,
                revision: self.revision,
                departed: self.departed,
                owner: self.owner,
                target,
                cutoff,
            });
        }
        Ok(())
    }
    pub fn target(&self) -> Result<DepartureTarget, DepartureError> {
        self.active()?;
        self.target.ok_or(DepartureError::MissingFences)
    }
    /// Required inclusive repair interval. `None` needs no export. Initial
    /// preconfirmed delay ticks are never requested from a history archive.
    /// Even with no export, the survivor must simulate and verify through target
    /// (including the input-delay floor) before acknowledging.
    pub fn repair_range(&self, survivor: PlayerSlot) -> Result<Option<(u64, u64)>, DepartureError> {
        let target = self.target()?.target;
        let report = self
            .fences
            .get(&survivor)
            .ok_or(DepartureError::UnknownSurvivor)?;
        let base = report.verified_tick.max(u64::from(self.config.input_delay));
        Ok(if base >= target {
            None
        } else {
            Some((
                base.checked_add(1).ok_or(DepartureError::TickExhausted)?,
                target,
            ))
        })
    }
    /// Construct an acknowledgment only from the exact current verified frame.
    /// The caller still authenticates and transports it; historical checksum lists,
    /// predicted heads, and highest-received ticks are deliberately not used.
    pub fn acknowledgment<G: Game, S: InputSource<G>>(
        &self,
        session: &Session<G, S>,
    ) -> Result<DepartureAck, DepartureError> {
        let target = self.target()?;
        let survivor = session.config().local_slot;
        if !self.survivors.contains(&survivor) {
            return Err(DepartureError::UnknownSurvivor);
        }
        if !self.compatible(session.config(), survivor) {
            return Err(DepartureError::SessionMismatch);
        }
        if session.verified_tick() != target.target {
            return Err(DepartureError::UnverifiedTarget);
        }
        let checksum = session
            .verified_frame()
            .ok_or(DepartureError::UnverifiedTarget)?
            .checksum();
        Ok(DepartureAck {
            target,
            survivor,
            checksum,
        })
    }
    pub fn acknowledge(&mut self, ack: DepartureAck) -> Result<(), DepartureError> {
        let target = self.target()?;
        if !self.survivors.contains(&ack.survivor) {
            return Err(DepartureError::UnknownSurvivor);
        }
        if ack.target != target {
            return Err(DepartureError::StaleContext);
        }
        if self
            .acks
            .values()
            .any(|prior| prior.checksum != ack.checksum)
        {
            return self.poison(DepartureError::Contradiction);
        }
        self.acks.insert(ack.survivor, ack);
        Ok(())
    }
    pub fn ready(&self) -> Result<bool, DepartureError> {
        self.target()?;
        Ok(self.acks.len() == self.survivors.len())
    }
    fn compatible(&self, cfg: &SessionConfig, local: PlayerSlot) -> bool {
        cfg.local_slot == local
            && cfg.player_count == self.config.player_count
            && cfg.input_delay == self.config.input_delay
            && cfg.seed == self.config.seed
            && cfg.tick_rate == self.config.tick_rate
            && cfg.build_id == self.config.build_id
            && !cfg.relay
    }
    /// Checks common session settings (player count, delay, seed, tick rate,
    /// build ID and P2P mode), not every SessionConfig field or game/source identity.
    /// Only the designated owner's current Session may commit. Failed checks never
    /// mark committed. The application must invalidate on restore/replacement;
    /// identical replacements cannot be distinguished by this additive helper.
    pub fn commit<G: Game, S: InputSource<G>>(
        &mut self,
        session: &mut Session<G, S>,
        revision: u64,
        survivors: &[PlayerSlot],
    ) -> Result<(), DepartureError> {
        self.check_membership(revision, survivors)?;
        if !self.ready()? {
            return Err(DepartureError::MissingAcknowledgments);
        }
        if !self.compatible(session.config(), self.owner) {
            return Err(DepartureError::SessionMismatch);
        }
        let current = self.acknowledgment(session)?;
        if self.acks.get(&self.owner) != Some(&current) {
            return self.poison(DepartureError::Contradiction);
        }
        session
            .mark_slot_vacant(self.departed, current.target.cutoff)
            .map_err(DepartureError::Vacate)?;
        self.committed = true;
        Ok(())
    }
}
