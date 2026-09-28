use std::collections::{BTreeMap, BTreeSet};

use orr_ecs::FrameRing;
use orr_sim::{EventKey, Game, PlayerSlot, SimEvent, Simulation, TickInputs};

use crate::events::{EventBatch, EventStatus};
use crate::input_source::InputSource;

/// `Session` configuration.
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// How many ticks ahead of "now" a locally produced input is stamped
    /// for (absorbs one-way latency without every remote input starting
    /// out mispredicted).
    pub input_delay: u32,
    /// Maximum ticks the predicted head is allowed to run ahead of the
    /// last verified tick before `advance` reports [`AdvanceResult::Stalled`].
    pub max_prediction: u32,
    /// Take (and, if a writer is attached, record) a checksum every this
    /// many *verified* ticks.
    pub checksum_interval: u32,
    pub player_count: u8,
    pub local_slot: PlayerSlot,
    pub seed: u64,
    pub tick_rate: u32,
    /// This peer's build id (see [`orr_sim::Simulation::build_hash`]).
    /// Defaults to `0` ("not tracking a build id"), matching
    /// `Simulation::new`'s default — set it explicitly for a real
    /// networked session so [`Session::build_hash`] is meaningful and
    /// [`require_same_build_hash`] can actually catch a mismatched peer
    /// before trusting its confirmed inputs.
    pub build_id: u64,
}

impl SessionConfig {
    pub fn new(player_count: u8, local_slot: PlayerSlot, seed: u64, tick_rate: u32) -> Self {
        Self {
            input_delay: 2,
            max_prediction: 8,
            checksum_interval: 30,
            player_count,
            local_slot,
            seed,
            tick_rate,
            build_id: 0,
        }
    }
}

/// Two peers (or a peer and a replay) disagree on which exact build/patch
/// generation produced their state — see
/// [`orr_sim::Simulation::build_hash`]. Comparing `Frame` state (or
/// resimulating a replay) across a build-hash mismatch is meaningless: the
/// two sides may be running different code for the same tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildHashMismatch {
    pub local: u64,
    pub remote: u64,
}

impl core::fmt::Display for BuildHashMismatch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "build hash mismatch: local {:#x} != remote {:#x}", self.local, self.remote)
    }
}
impl std::error::Error for BuildHashMismatch {}

/// Refuses to proceed when two build hashes disagree. `0` is treated as
/// "not tracking a build id" on *either* side and never triggers a
/// mismatch (matches `Simulation::build_hash`'s `build_id == 0` wildcard —
/// see its docs), so this is a no-op for sessions/tools that never opted
/// into build-id tracking.
pub fn require_same_build_hash(local: u64, remote: u64) -> Result<(), BuildHashMismatch> {
    if local == 0 || remote == 0 || local == remote {
        Ok(())
    } else {
        Err(BuildHashMismatch { local, remote })
    }
}

/// What [`Session::advance`] had to do to correct a misprediction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RollbackInfo {
    pub from_tick: u64,
    pub to_tick: u64,
    pub resim_count: u32,
}

/// The outcome of one [`Session::advance`] call.
pub enum AdvanceResult<G: Game> {
    /// The predicted head advanced by one tick (unless prediction was
    /// already stalled at the cap and this call just processed
    /// confirmations/rollback without moving further ahead — see
    /// `Stalled`, which is reported instead in that case).
    Advanced { tick: u64, events: EventBatch<G::Event>, rollback: Option<RollbackInfo> },
    /// The predicted head is `max_prediction` ticks ahead of the last
    /// verified tick; the session did not simulate a new tick this call
    /// (it still processed any newly confirmed inputs/rollback/events).
    Stalled { events: EventBatch<G::Event> },
}

/// A checksum mismatch between two peers (or a peer and a recorded
/// baseline) at the same verified tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desync {
    pub tick: u64,
    pub local: u64,
    pub remote: u64,
}

/// Compares two `(tick, checksum)` lists (as produced by
/// [`Session::checksums`]) and returns every tick present in both where the
/// checksums differ.
pub fn compare_checksums(local: &[(u64, u64)], remote: &[(u64, u64)]) -> Vec<Desync> {
    let remote_map: BTreeMap<u64, u64> = remote.iter().copied().collect();
    local
        .iter()
        .filter_map(|&(tick, local_cs)| {
            remote_map.get(&tick).and_then(|&remote_cs| {
                if remote_cs != local_cs {
                    Some(Desync { tick, local: local_cs, remote: remote_cs })
                } else {
                    None
                }
            })
        })
        .collect()
}

struct TickRecord<G: Game> {
    inputs: TickInputs<G::Input, G::Command>,
    predicted: Vec<bool>,
    events: Vec<SimEvent<G::Event>>,
}

/// Owns a rollback-capable [`Simulation<G>`] plus everything needed to
/// predict ahead of confirmed input, detect mispredictions, resimulate, and
/// reconcile events for the view layer. See the crate docs for the overall
/// design.
pub struct Session<G: Game, S: InputSource<G>> {
    sim: Simulation<G>,
    ring: FrameRing,
    verified_tick: u64,
    cfg: SessionConfig,
    source: S,
    history: BTreeMap<u64, TickRecord<G>>,
    confirmed_input: BTreeMap<u64, BTreeMap<PlayerSlot, G::Input>>,
    confirmed_commands: BTreeMap<u64, BTreeMap<PlayerSlot, Vec<G::Command>>>,
    last_input: Vec<G::Input>,
    next_send_tick: u64,
    /// Events currently "live" as `Predicted` (announced, not yet verified
    /// or canceled), keyed by their `EventKey` with the payload bytes they
    /// were announced with (for change detection across a rollback).
    announced: BTreeMap<EventKey, Vec<u8>>,
    checksums: Vec<(u64, u64)>,
}

fn event_bytes<E: bytemuck::Pod>(e: &E) -> Vec<u8> {
    bytemuck::bytes_of(e).to_vec()
}

impl<G: Game, S: InputSource<G>> Session<G, S> {
    pub fn new(config: G::Config, cfg: SessionConfig, source: S) -> Self {
        let sim = Simulation::<G>::with_build_id(config, cfg.tick_rate, cfg.seed, cfg.build_id);
        let mut ring = FrameRing::new(cfg.max_prediction + 2, sim.registry().clone());
        ring.store(sim.frame());
        let last_input = vec![G::Input::default(); cfg.player_count as usize];
        let next_send_tick = 1 + cfg.input_delay as u64;

        // Bootstrap ticks `1..=input_delay` never get a real local/remote
        // send (the first real send targets `1 + input_delay`), so without
        // this they'd never accumulate a full confirmed set and
        // `verified_tick` could never leave 0. Pre-confirm them with
        // default input for every player — before any real input exists,
        // "nothing pressed" *is* the true input.
        let mut confirmed_input: BTreeMap<u64, BTreeMap<PlayerSlot, G::Input>> = BTreeMap::new();
        for tick in 1..=cfg.input_delay as u64 {
            let mut per_player = BTreeMap::new();
            for slot in 0..cfg.player_count {
                per_player.insert(PlayerSlot(slot), G::Input::default());
            }
            confirmed_input.insert(tick, per_player);
        }

        Self {
            sim,
            ring,
            verified_tick: 0,
            cfg,
            source,
            history: BTreeMap::new(),
            confirmed_input,
            confirmed_commands: BTreeMap::new(),
            last_input,
            next_send_tick,
            announced: BTreeMap::new(),
            checksums: Vec::new(),
        }
    }

    pub fn verified_tick(&self) -> u64 {
        self.verified_tick
    }
    pub fn head_tick(&self) -> u64 {
        self.sim.tick()
    }
    pub fn predicted_frame(&self) -> &orr_ecs::Frame {
        self.sim.frame()
    }
    pub fn verified_frame(&self) -> Option<&orr_ecs::Frame> {
        self.ring.get(self.verified_tick)
    }
    pub fn checksums(&self) -> &[(u64, u64)] {
        &self.checksums
    }
    pub fn config(&self) -> &SessionConfig {
        &self.cfg
    }
    /// This session's current build hash (see
    /// [`orr_sim::Simulation::build_hash`]). Compare against a remote
    /// peer's (out of band, e.g. in a connection handshake) with
    /// [`require_same_build_hash`] before trusting its confirmed inputs.
    pub fn build_hash(&self) -> u64 {
        self.sim.build_hash()
    }

    fn best_effort_inputs(&self, tick: u64) -> (TickInputs<G::Input, G::Command>, Vec<bool>) {
        let n = self.cfg.player_count;
        let mut ti = TickInputs::<G::Input, G::Command>::new(tick, n);
        let mut predicted = vec![false; n as usize];
        let confirmed = self.confirmed_input.get(&tick);
        let mut cmds = Vec::new();
        for slot in 0..n {
            let ps = PlayerSlot(slot);
            if let Some(v) = confirmed.and_then(|m| m.get(&ps)) {
                ti.set_input(ps, *v);
            } else {
                ti.set_input(ps, self.last_input[slot as usize]);
                predicted[slot as usize] = true;
            }
        }
        if let Some(cmap) = self.confirmed_commands.get(&tick) {
            for (&slot, list) in cmap {
                for c in list {
                    cmds.push((slot, c.clone()));
                }
            }
        }
        ti.set_commands(cmds);
        (ti, predicted)
    }

    fn simulate_tick(&mut self, tick: u64) {
        debug_assert_eq!(self.sim.tick() + 1, tick);
        let (inputs, predicted) = self.best_effort_inputs(tick);
        let events = self.sim.step(&inputs);
        self.ring.store(self.sim.frame());
        for (slot, was_predicted) in predicted.iter().enumerate() {
            if !was_predicted {
                self.last_input[slot] = *inputs.input(PlayerSlot(slot as u8));
            }
        }
        self.history.insert(tick, TickRecord { inputs, predicted, events });
    }

    /// Restores the ring snapshot just before `from` and resimulates
    /// `from..=to_tick` (the current head at call time) with the best
    /// currently-available inputs, diffing each resimulated tick's events
    /// against what was previously recorded to produce `Predicted`/
    /// `Canceled` notifications for `batch`.
    fn rollback_and_resim(&mut self, from: u64, batch: &mut EventBatch<G::Event>) -> RollbackInfo {
        let head = self.sim.tick();
        let restored = self.ring.get(from - 1).cloned();
        if let Some(snapshot) = restored {
            self.sim.restore(&snapshot);
        }
        // ring.get returned by reference above requires a clone since we
        // then need &mut self; if unavailable (shouldn't happen given ring
        // capacity), we simply resimulate forward from whatever the sim's
        // current frame is (best effort).

        let mut resim_count = 0u32;
        for tick in from..=head {
            let old_events = self.history.remove(&tick).map(|r| r.events).unwrap_or_default();
            self.simulate_tick(tick);
            resim_count += 1;
            let new_events = &self.history[&tick].events;
            diff_events(&old_events, new_events, &mut self.announced, batch);
        }

        RollbackInfo { from_tick: from, to_tick: head, resim_count }
    }

    /// Finds the earliest tick in `(verified_tick, head]` whose used input
    /// (recorded as `predicted`) disagrees with an input that has since
    /// been confirmed.
    fn earliest_mismatch(&self) -> Option<u64> {
        let mut earliest = None;
        for (&tick, rec) in self.history.range((self.verified_tick + 1)..=self.sim.tick()) {
            let Some(confirmed) = self.confirmed_input.get(&tick) else { continue };
            for (&slot, &val) in confirmed {
                if rec.predicted[slot.0 as usize] && *rec.inputs.input(slot) != val {
                    earliest = Some(earliest.map_or(tick, |e: u64| e.min(tick)));
                }
            }
        }
        earliest
    }

    /// Advances `verified_tick` through every contiguous, fully-confirmed
    /// tick whose recorded simulated input matches the confirmed input
    /// exactly, announcing their events as `Verified` and garbage
    /// collecting their bookkeeping.
    fn advance_verified(&mut self, batch: &mut EventBatch<G::Event>) {
        loop {
            let t = self.verified_tick + 1;
            if t > self.sim.tick() {
                break;
            }
            let Some(confirmed) = self.confirmed_input.get(&t) else { break };
            if confirmed.len() != self.cfg.player_count as usize {
                break;
            }
            let Some(rec) = self.history.get(&t) else { break };
            let matches = (0..self.cfg.player_count).all(|slot| {
                let ps = PlayerSlot(slot);
                confirmed.get(&ps) == Some(rec.inputs.input(ps))
            });
            if !matches {
                // A mismatch here means `earliest_mismatch`/resim above
                // should already have corrected it earlier this call; if
                // it somehow didn't (e.g. commands-only difference), stop
                // rather than falsely verifying wrong state.
                break;
            }
            let rec = self.history.remove(&t).unwrap();
            for e in &rec.events {
                let bytes = event_bytes(&e.payload);
                self.announced.remove(&e.key);
                let _ = bytes;
                batch.push(e.key, EventStatus::Verified(e.payload));
            }
            self.verified_tick = t;
            self.confirmed_input.remove(&t);
            self.confirmed_commands.remove(&t);

            if self.verified_tick % self.cfg.checksum_interval as u64 == 0 {
                if let Some(f) = self.ring.get(self.verified_tick) {
                    self.checksums.push((self.verified_tick, f.checksum()));
                }
            }
        }
    }

    /// Advances the session by one tick: records the local player's input,
    /// ingests newly-arrived remote inputs, corrects any misprediction via
    /// rollback + resimulation, advances `verified_tick` through any newly
    /// fully-confirmed ticks, and — unless prediction is already at the
    /// cap — simulates one new tick.
    pub fn advance(&mut self, local_input: G::Input, local_commands: Vec<G::Command>) -> AdvanceResult<G> {
        let send_tick = self.next_send_tick;
        self.next_send_tick += 1;
        self.confirmed_input.entry(send_tick).or_default().insert(self.cfg.local_slot, local_input);
        if !local_commands.is_empty() {
            self.confirmed_commands.entry(send_tick).or_default().insert(self.cfg.local_slot, local_commands.clone());
        }
        self.source.send_local(send_tick, self.cfg.local_slot, local_input, local_commands);

        for remote in self.source.poll_remote() {
            self.confirmed_input.entry(remote.tick).or_default().insert(remote.slot, remote.input);
            if !remote.commands.is_empty() {
                self.confirmed_commands.entry(remote.tick).or_default().insert(remote.slot, remote.commands);
            }
        }

        let mut batch = EventBatch::new();
        let mut rollback = None;
        if let Some(from) = self.earliest_mismatch() {
            rollback = Some(self.rollback_and_resim(from, &mut batch));
        }

        self.advance_verified(&mut batch);

        if self.sim.tick() - self.verified_tick >= self.cfg.max_prediction as u64 {
            return AdvanceResult::Stalled { events: batch };
        }

        let next = self.sim.tick() + 1;
        self.simulate_tick(next);
        for e in &self.history[&next].events {
            let bytes = event_bytes(&e.payload);
            self.announced.insert(e.key, bytes);
            batch.push(e.key, EventStatus::Predicted(e.payload));
        }

        AdvanceResult::Advanced { tick: next, events: batch, rollback }
    }
}

fn diff_events<E: bytemuck::Pod + PartialEq>(
    old: &[SimEvent<E>],
    new: &[SimEvent<E>],
    announced: &mut BTreeMap<EventKey, Vec<u8>>,
    batch: &mut EventBatch<E>,
) {
    let new_keys: BTreeSet<EventKey> = new.iter().map(|e| e.key).collect();
    // Cancel anything that was live and either disappeared or changed payload.
    for e in old {
        let was_live = announced.contains_key(&e.key);
        if !was_live {
            continue;
        }
        let still_same = new.iter().any(|n| n.key == e.key && n.payload == e.payload);
        if !still_same {
            announced.remove(&e.key);
            batch.push(e.key, EventStatus::Canceled);
        }
    }
    // Announce anything new that wasn't already live with the same payload.
    for e in new {
        let bytes = event_bytes(&e.payload);
        let already_live_same = announced.get(&e.key).map(|b| *b == bytes).unwrap_or(false);
        if !already_live_same {
            announced.insert(e.key, bytes);
            batch.push(e.key, EventStatus::Predicted(e.payload));
        }
    }
    let _ = new_keys;
}
