//! Opt-in bounded, locally verified recovery evidence. This is not consensus,
//! a network protocol, or authorization to repair a peer or choose a cutoff.
use std::collections::BTreeMap;

use orr_sim::{Game, PlayerSlot, SimCommand, SimInput};

use crate::{InputSource, LocallyVerifiedTick, RemoteInput};

/// Retention limits. Player metadata is fixed by the constructor's player count.
#[derive(Clone, Copy, Debug)]
pub struct VerifiedHistoryLimits {
    /// Maximum number of (tick, slot) records, including empty-command records.
    pub max_records: usize,
    /// Sum of input byte lengths and command payload lengths plus eight bytes
    /// of length framing per command. This also bounds zero-byte command count.
    /// Container overhead, export copies, and temporary encoder allocations are
    /// outside this retained-encoded-byte bound.
    pub max_encoded_bytes: usize,
}

/// An exact retained sample from one local Session generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocallyVerifiedRecord<I> {
    pub tick: u64,
    pub slot: PlayerSlot,
    pub input: I,
    /// Exact simulation-time bytes, in submission order (including duplicates).
    pub commands: Vec<Vec<u8>>,
    pub disconnected: bool,
}

impl<I: SimInput> LocallyVerifiedRecord<I> {
    fn encoded_bytes(&self) -> usize {
        std::mem::size_of::<I>() + self.commands.iter().map(|c| 8 + c.len()).sum::<usize>()
    }
}

/// Recovery is unavailable; normal gameplay is unaffected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifiedHistoryError {
    InvalidPlayerCount,
    InvalidRange,
    Unavailable,
    Invalidated,
}

impl std::fmt::Display for VerifiedHistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "locally verified history: {self:?}")
    }
}
impl std::error::Error for VerifiedHistoryError {}

/// Fallible opt-in command encoder. Successful output must be the canonical
/// `SimCommand::encode` representation; an error makes that tick unavailable.
/// It runs synchronously during local verification. Its CPU work and temporary
/// allocations are not covered by retention limits. Implementations must not
/// panic; `SimCommand`'s ordinary infallible encoding contract still applies.
pub type VerifiedHistoryEncoder<C> = fn(&C, &mut Vec<u8>) -> Result<(), VerifiedHistoryEncodeError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifiedHistoryEncodeError;

fn encode_command<C: SimCommand>(command: &C, out: &mut Vec<u8>) -> Result<(), VerifiedHistoryEncodeError> {
    command.encode(out);
    Ok(())
}

/// Bounded archive populated only from local verification callbacks, never
/// from ingress. The observer must belong to exactly one Session generation.
/// Before `Session::restore_confirmed` or replacing a Session, call
/// [`Self::invalidate`] or install a fresh archive with a fresh generation ID.
/// Forward restores cannot be detected automatically. IDs are caller-assigned,
/// local labels, not a distributed membership generation or consensus proof.
///
/// Eviction retires whole oldest ticks. Any missing/conflicting evidence or
/// encoder/size failure retires that tick and all older ticks. A monotonic
/// retirement floor plus a high-water tick and invalidation bit provide bounded
/// poison state: no tombstone map grows with rejected input. Non-increasing
/// callbacks invalidate the archive rather than resurrecting old records.
/// This intentionally favors safety over recovery availability.
pub struct LocallyVerifiedHistory<G: Game> {
    generation: u64,
    player_count: u8,
    limits: VerifiedHistoryLimits,
    records: BTreeMap<(u64, PlayerSlot), LocallyVerifiedRecord<G::Input>>,
    encoded_bytes: usize,
    retired_through: u64,
    observed_through: u64,
    invalidated: bool,
    encoder: VerifiedHistoryEncoder<G::Command>,
}

impl<G: Game> LocallyVerifiedHistory<G> {
    pub fn new(generation: u64, player_count: u8, limits: VerifiedHistoryLimits) -> Result<Self, VerifiedHistoryError> {
        Self::with_encoder(generation, player_count, limits, encode_command::<G::Command>)
    }

    pub fn with_encoder(
        generation: u64,
        player_count: u8,
        limits: VerifiedHistoryLimits,
        encoder: VerifiedHistoryEncoder<G::Command>,
    ) -> Result<Self, VerifiedHistoryError> {
        if player_count == 0 {
            return Err(VerifiedHistoryError::InvalidPlayerCount);
        }
        Ok(Self {
            generation,
            player_count,
            limits,
            records: BTreeMap::new(),
            encoded_bytes: 0,
            retired_through: 0,
            observed_through: 0,
            invalidated: false,
            encoder,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn len(&self) -> usize {
        self.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    pub fn retained_encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
    pub fn retired_through(&self) -> u64 {
        self.retired_through
    }

    /// Permanently disable this generation. Use a new archive for a new Session.
    pub fn invalidate(&mut self) {
        self.invalidated = true;
        self.records.clear();
        self.encoded_bytes = 0;
        self.retired_through = self.observed_through;
    }

    fn retire(&mut self, through: u64) {
        self.retired_through = self.retired_through.max(through);
        while self
            .records
            .first_key_value()
            .is_some_and(|((tick, _), _)| *tick <= self.retired_through)
        {
            let (_, record) = self.records.pop_first().unwrap();
            self.encoded_bytes -= record.encoded_bytes();
        }
    }

    /// Exact inclusive range, or an explicit error. No prediction, default
    /// input, partial range, or ingress packet can substitute for a missing
    /// record. This copies retained records only when explicitly requested;
    /// range length is checked against bounded storage before allocating.
    pub fn export_range(
        &self,
        slot: PlayerSlot,
        first: u64,
        through: u64,
    ) -> Result<Vec<LocallyVerifiedRecord<G::Input>>, VerifiedHistoryError> {
        if self.invalidated {
            return Err(VerifiedHistoryError::Invalidated);
        }
        if slot.0 >= self.player_count || first == 0 || first > through {
            return Err(VerifiedHistoryError::InvalidRange);
        }
        let count = through
            .checked_sub(first)
            .and_then(|n| n.checked_add(1))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or(VerifiedHistoryError::Unavailable)?;
        if first <= self.retired_through || count > self.records.len() {
            return Err(VerifiedHistoryError::Unavailable);
        }
        // Validate completeness first, so a failed export returns no partial data.
        for tick in first..=through {
            if !self.records.contains_key(&(tick, slot)) {
                return Err(VerifiedHistoryError::Unavailable);
            }
        }
        Ok((first..=through)
            .map(|tick| self.records[&(tick, slot)].clone())
            .collect())
    }

    fn observe(&mut self, view: &LocallyVerifiedTick<'_, G>) {
        if self.invalidated {
            return;
        }
        let tick = view.simulated.tick();
        if tick <= self.observed_through {
            self.invalidate();
            return;
        }
        self.observed_through = tick;
        if tick <= self.retired_through {
            return;
        }
        let Some(records) = self.agreed_records(view) else {
            self.retire(tick);
            return;
        };
        let bytes: usize = records.iter().map(LocallyVerifiedRecord::encoded_bytes).sum();
        // Admission is atomic per tick; a too-large tick must not erase and then
        // repopulate only a subset of its slots.
        if records.len() > self.limits.max_records || bytes > self.limits.max_encoded_bytes {
            self.retire(tick);
            return;
        }
        while self.records.len() > self.limits.max_records - records.len()
            || self.encoded_bytes > self.limits.max_encoded_bytes - bytes
        {
            let oldest = self.records.first_key_value().unwrap().0 .0;
            self.retire(oldest);
        }
        self.encoded_bytes += bytes;
        for record in records {
            self.records.insert((tick, record.slot), record);
        }
    }

    fn agreed_records(&mut self, view: &LocallyVerifiedTick<'_, G>) -> Option<Vec<LocallyVerifiedRecord<G::Input>>> {
        if view.simulated.player_count() != self.player_count
            || view.simulated.commands().len() != view.simulated_commands.len()
            || view
                .simulated
                .commands()
                .iter()
                .zip(view.simulated_commands)
                .any(|((slot, _), (encoded_slot, _))| slot != encoded_slot)
            || view.confirmed_inputs.len() != usize::from(self.player_count)
            || view
                .confirmed_commands
                .is_some_and(|m| m.keys().any(|s| s.0 >= self.player_count))
            || usize::from(self.player_count) > self.limits.max_records
        {
            return None;
        }
        let tick = view.simulated.tick();
        let mut used = view.simulated_commands.iter();
        let mut bytes = 0usize;
        let mut records = Vec::new();
        for slot in (0..self.player_count).map(PlayerSlot) {
            let input = *view.simulated.input(slot);
            let disconnected = view.simulated.flags(slot).disconnected;
            if view.confirmed_inputs.get(&slot) != Some(&input)
                || disconnected != view.confirmed_absent.contains(&(tick, slot.0))
            {
                return None;
            }
            bytes = bytes.checked_add(std::mem::size_of::<G::Input>())?;
            if bytes > self.limits.max_encoded_bytes {
                return None;
            }
            let mut commands = Vec::new();
            for confirmed in view.confirmed_commands.and_then(|m| m.get(&slot)).into_iter().flatten() {
                let (used_slot, used_bytes) = used.next()?;
                if *used_slot != slot {
                    return None;
                }
                bytes = bytes.checked_add(8)?.checked_add(used_bytes.len())?;
                if bytes > self.limits.max_encoded_bytes {
                    return None;
                }
                let mut encoded = Vec::new();
                (self.encoder)(confirmed, &mut encoded).ok()?;
                if &encoded != used_bytes {
                    return None;
                }
                // Retain only the bytes recorded by simulation, never raw ingress.
                commands.push(used_bytes.clone());
            }
            records.push(LocallyVerifiedRecord {
                tick,
                slot,
                input,
                commands,
                disconnected,
            });
        }
        if used.next().is_some() {
            return None;
        }
        Some(records)
    }
}

/// Input source decorator that opts into synchronous encoding and bounded
/// locally verified history. Network behavior is delegated unchanged.
pub struct VerifiedHistorySource<G: Game, S> {
    inner: S,
    history: LocallyVerifiedHistory<G>,
}

impl<G: Game, S> VerifiedHistorySource<G, S> {
    pub fn new(inner: S, history: LocallyVerifiedHistory<G>) -> Self {
        Self { inner, history }
    }
    pub fn inner(&self) -> &S {
        &self.inner
    }
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }
    pub fn history(&self) -> &LocallyVerifiedHistory<G> {
        &self.history
    }
    pub fn history_mut(&mut self) -> &mut LocallyVerifiedHistory<G> {
        &mut self.history
    }
}

impl<G: Game, S: InputSource<G>> InputSource<G> for VerifiedHistorySource<G, S> {
    fn send_local(&mut self, tick: u64, slot: PlayerSlot, input: G::Input, commands: Vec<G::Command>) {
        self.inner.send_local(tick, slot, input, commands);
    }
    fn poll_remote(&mut self) -> Vec<RemoteInput<G>> {
        self.inner.poll_remote()
    }
    fn on_locally_verified(&mut self, tick: LocallyVerifiedTick<'_, G>) {
        self.history.observe(&tick);
        self.inner.on_locally_verified(tick);
    }
}
