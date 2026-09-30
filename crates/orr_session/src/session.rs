use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::Bound;

use orr_ecs::{Frame, FrameRing};
use orr_sim::{EventKey, Game, PlayerSlot, SimCommand, SimEvent, Simulation, TickInputs};

use crate::events::{EventBatch, EventStatus};
use crate::input_source::{InputSource, RemoteInput};
use crate::join::{self, JoinError, JoinTicket};

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
    /// Slots nobody plays yet. This peer authors default input for them on
    /// every `advance`, so the other peers can verify ticks without waiting
    /// for a player who is not there. Exactly one peer of a session must
    /// list a given slot. [`Session::serve_join`] hands such a slot to a
    /// late joiner. Defaults to none.
    pub vacant_slots: Vec<PlayerSlot>,
    /// How many ticks behind `verified_tick` the peer keeps the inputs it
    /// authored, for [`Session::authored_since`] (a late joiner needs every
    /// peer's inputs after the snapshot tick).
    pub input_log_ticks: u32,
    /// Joiner only: a nonce, chosen once per join, that lets the host tell a
    /// retry of the same joiner from a different one. `0` = anonymous
    /// one-shot join (never retried). See [`crate::JoinAttempts`].
    pub join_id: u64,
    /// Joiner only: number of the current join attempt (1, 2, ...). Set by
    /// [`crate::JoinAttempts`]; `from_join_snapshot` refuses a snapshot of
    /// another attempt.
    pub join_attempt: u32,
    /// Joiner only: how many existing peers will send a
    /// [`Session::backlog_notice`]. When above `0`, the joiner checks the
    /// notices for input gaps and sends no input until they are all in and
    /// complete. `0` = no check (the joiner may stall on a gap).
    pub join_backlog_peers: u32,
    /// Joiner only: the most join requests [`crate::JoinAttempts`] makes.
    pub max_join_attempts: u32,
    /// Relay mode: the session's own input is not confirmed by itself. It
    /// is predicted until the server's confirmed bundle for the tick
    /// arrives, and the bundle (all slots, including this one) is the truth.
    /// Inputs are submitted with [`Session::submit_local`], one per tick,
    /// independently of simulating. See [`crate::RelayClient`].
    pub relay: bool,
    /// Keep the verified frame (as `Frame::to_bytes`) of the last this many
    /// checksum ticks, for desync diagnostics. `0` = none.
    pub keep_anchors: u32,
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
            vacant_slots: Vec::new(),
            input_log_ticks: 256,
            join_id: 0,
            join_attempt: 0,
            join_backlog_peers: 0,
            max_join_attempts: 3,
            relay: false,
            keep_anchors: 0,
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

/// One input this peer authored: slot, input and the commands sent with it.
type AuthoredInput<G> = (PlayerSlot, <G as Game>::Input, Vec<<G as Game>::Command>);

/// The ticks a vacant slot's default input is authored for by this peer:
/// `from <= tick < until` (`until` is `u64::MAX` while the slot is open).
/// Only the latest interval is kept; earlier ones are done and only live on
/// in the `authored` log.
#[derive(Clone, Copy)]
struct VacantSpan {
    from: u64,
    until: u64,
}

/// A join this host served: who asked, and the ticket the peers hold for.
struct Grant {
    joiner_id: u64,
    ticket: JoinTicket,
    /// The joiner's first input arrived: the transfer is over.
    confirmed: bool,
}

/// Where a joiner stands on the input check of its backlogs.
enum JoinState {
    /// Backlog notices are still missing.
    Syncing,
    /// Every notice is in and the inputs cover the whole range needed.
    Ready,
    /// Proven hole (see `JoinError::InputGap`).
    Gap { slot: PlayerSlot, needed_from: u64, available_from: u64 },
}

/// A joiner's view of the join: the notices received so far, and the
/// inputs it authored meanwhile but has not sent (see `Session::emit`).
struct JoinProgress<G: Game> {
    attempt: u32,
    snapshot_tick: u64,
    first_input_tick: u64,
    expected: u32,
    notices: BTreeMap<PlayerSlot, Vec<join::Span>>,
    state: JoinState,
    held: Vec<(u64, AuthoredInput<G>)>,
}

/// How far a joiner's input check has come, see [`Session::join_status`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinStatus {
    /// This session does not check backlogs (not a joiner, or
    /// `join_backlog_peers` is `0`).
    Unchecked,
    /// `received` of the `expected` backlog notices are in.
    Syncing { received: u32, expected: u32 },
    /// All notices are in and no input is missing. The session sends its
    /// own inputs.
    Ready,
}

struct TickRecord<G: Game> {
    inputs: TickInputs<G::Input, G::Command>,
    predicted: Vec<bool>,
    events: Vec<SimEvent<G::Event>>,
    /// Encoded commands the tick was simulated with (relay mode only).
    cmds: Vec<(PlayerSlot, Vec<u8>)>,
}

/// A verified frame kept for desync diagnostics (see
/// `SessionConfig::keep_anchors`).
#[derive(Clone, Debug)]
pub struct Anchor {
    pub tick: u64,
    pub checksum: u64,
    /// `Frame::to_bytes` of the verified frame at `tick`.
    pub frame_bytes: Vec<u8>,
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
    /// Vacant slots this peer authors default input for.
    vacant: BTreeMap<PlayerSlot, VacantSpan>,
    /// Inputs this peer authored (local slot and vacant slots) by tick, kept
    /// for `input_log_ticks` behind `verified_tick` (and longer while a join
    /// holds them, see `holds`).
    authored: BTreeMap<u64, Vec<AuthoredInput<G>>>,
    /// Joins this host served, by slot.
    grants: BTreeMap<PlayerSlot, Grant>,
    /// Joins in transfer, by slot: `authored` keeps every tick after the
    /// ticket's `snapshot_tick` until the join is confirmed or dropped.
    holds: BTreeMap<PlayerSlot, JoinTicket>,
    /// Highest tick of an input received from a remote peer, by slot.
    last_remote_tick: BTreeMap<PlayerSlot, u64>,
    /// Set on a joiner that checks its backlogs.
    join: Option<JoinProgress<G>>,
    /// How many rollbacks this session has done, and the latest one.
    rollback_count: u64,
    last_rollback: Option<RollbackInfo>,
    /// Relay mode: this peer's own inputs (and commands) by tick, used as
    /// its prediction until the server's bundle confirms the tick.
    local_pending: BTreeMap<u64, (G::Input, Vec<G::Command>)>,
    anchors: VecDeque<Anchor>,
}

fn event_bytes<E: bytemuck::Pod>(e: &E) -> Vec<u8> {
    bytemuck::bytes_of(e).to_vec()
}

impl<G: Game, S: InputSource<G>> Session<G, S> {
    pub fn new(config: G::Config, cfg: SessionConfig, source: S) -> Self {
        let sim = Simulation::<G>::with_build_id(config, cfg.tick_rate, cfg.seed, cfg.build_id);
        let next_send_tick = 1 + cfg.input_delay as u64;
        Self::from_sim(sim, cfg, source, next_send_tick)
    }

    /// Wraps `sim`, whose current tick becomes the verified tick.
    fn from_sim(sim: Simulation<G>, cfg: SessionConfig, source: S, next_send_tick: u64) -> Self {
        let verified_tick = sim.tick();
        let mut ring = FrameRing::new(cfg.max_prediction + 2, sim.registry().clone());
        ring.store(sim.frame());
        let last_input = vec![G::Input::default(); cfg.player_count as usize];

        // Bootstrap ticks `1..=input_delay` never get a real local/remote
        // send (the first real send targets `1 + input_delay`), so without
        // this they'd never accumulate a full confirmed set and
        // `verified_tick` could never leave 0. Pre-confirm them with
        // default input for every player — before any real input exists,
        // "nothing pressed" *is* the true input.
        // (Relay mode has none: the server confirms every tick from 1.)
        let mut confirmed_input: BTreeMap<u64, BTreeMap<PlayerSlot, G::Input>> = BTreeMap::new();
        let bootstrap_until = if cfg.relay { verified_tick } else { cfg.input_delay as u64 };
        for tick in verified_tick + 1..=bootstrap_until {
            let mut per_player = BTreeMap::new();
            for slot in 0..cfg.player_count {
                per_player.insert(PlayerSlot(slot), G::Input::default());
            }
            confirmed_input.insert(tick, per_player);
        }

        let vacant = cfg.vacant_slots.iter().map(|&slot| (slot, VacantSpan { from: 0, until: u64::MAX })).collect();

        Self {
            sim,
            ring,
            verified_tick,
            cfg,
            source,
            history: BTreeMap::new(),
            confirmed_input,
            confirmed_commands: BTreeMap::new(),
            last_input,
            next_send_tick,
            announced: BTreeMap::new(),
            checksums: Vec::new(),
            vacant,
            authored: BTreeMap::new(),
            grants: BTreeMap::new(),
            holds: BTreeMap::new(),
            last_remote_tick: BTreeMap::new(),
            join: None,
            rollback_count: 0,
            last_rollback: None,
            local_pending: BTreeMap::new(),
            anchors: VecDeque::new(),
        }
    }

    /// Joins a running session from a snapshot message made by the host's
    /// [`serve_join`](Self::serve_join). `cfg.local_slot` must be the slot
    /// requested in [`join_request`], and `config`/`cfg` must match the
    /// host's. The returned session starts at the snapshot's verified tick
    /// and needs the other peers' inputs after it (see
    /// [`authored_since`](Self::authored_since)) to move on.
    ///
    /// Rejects the message, before anything is simulated, when the build
    /// hash or a session setting differs, or the snapshot does not decode,
    /// or its tick or checksum differs from the message header, or it
    /// answers another attempt than `cfg.join_attempt`.
    ///
    /// With `cfg.join_backlog_peers > 0` the session then checks the peers'
    /// [`backlog_notice`](Self::backlog_notice)s (see
    /// [`receive_backlog`](Self::receive_backlog)) and keeps its own inputs
    /// back until they prove nothing is missing. Until then it never
    /// authors a tick anywhere but locally, so dropping a session that
    /// found a gap leaves no input of its slot with any peer.
    pub fn from_join_snapshot(
        config: G::Config,
        cfg: SessionConfig,
        source: S,
        message: &[u8],
    ) -> Result<Self, JoinError> {
        let snap = join::decode_snapshot(message)?;
        let mut sim = Simulation::<G>::with_build_id(config, cfg.tick_rate, cfg.seed, cfg.build_id);
        snap.header.check_against(&cfg, sim.build_hash())?;
        if snap.header.slot != cfg.local_slot {
            return Err(JoinError::ConfigMismatch("slot"));
        }
        let frame = Frame::from_bytes(sim.registry().clone(), &snap.frame_bytes).map_err(JoinError::BadSnapshot)?;
        if frame.tick() != snap.snapshot_tick {
            return Err(JoinError::SnapshotMismatch { expected: snap.snapshot_tick, actual: frame.tick() });
        }
        if frame.checksum() != snap.checksum {
            return Err(JoinError::SnapshotMismatch { expected: snap.checksum, actual: frame.checksum() });
        }
        let attempt = snap.attempt?;
        if attempt != cfg.join_attempt {
            return Err(JoinError::StaleAttempt { current: cfg.join_attempt, got: attempt });
        }
        sim.restore(&frame);
        let expected = cfg.join_backlog_peers;
        let mut session = Self::from_sim(sim, cfg, source, snap.first_input_tick);
        if expected > 0 {
            session.join = Some(JoinProgress {
                attempt,
                snapshot_tick: snap.snapshot_tick,
                first_input_tick: snap.first_input_tick,
                expected,
                notices: BTreeMap::new(),
                state: JoinState::Syncing,
                held: Vec::new(),
            });
        }
        Ok(session)
    }

    /// Relay mode: starts a session from a snapshot a donor client
    /// uploaded and the server relayed. `frame_bytes` is the donor's
    /// `Frame::to_bytes` of its verified frame at `tick` (already
    /// decompressed), `checksum` its checksum. `cfg.relay` must be set.
    ///
    /// The session starts with `tick` as its verified tick; it needs the
    /// server's confirmed bundles after `tick` to move on. On error the
    /// input source is handed back.
    pub fn from_relay_snapshot(
        config: G::Config,
        cfg: SessionConfig,
        source: S,
        tick: u64,
        checksum: u64,
        frame_bytes: &[u8],
    ) -> Result<Self, (JoinError, S)> {
        let mut sim = Simulation::<G>::with_build_id(config, cfg.tick_rate, cfg.seed, cfg.build_id);
        let frame = match Frame::from_bytes(sim.registry().clone(), frame_bytes) {
            Ok(f) => f,
            Err(e) => return Err((JoinError::BadSnapshot(e), source)),
        };
        if frame.tick() != tick {
            return Err((JoinError::SnapshotMismatch { expected: tick, actual: frame.tick() }, source));
        }
        if frame.checksum() != checksum {
            return Err((JoinError::SnapshotMismatch { expected: checksum, actual: frame.checksum() }, source));
        }
        sim.restore(&frame);
        Ok(Self::from_sim(sim, cfg, source, tick + 1))
    }

    /// Answers a joiner's [`join_request`] with a snapshot message for
    /// [`from_join_snapshot`](Self::from_join_snapshot). Only the peer that
    /// lists the slot in `SessionConfig::vacant_slots` can serve it.
    ///
    /// The snapshot is this peer's frame at its `verified_tick` (never a
    /// predicted one). From the `next_send_tick` at this call on, this peer
    /// stops authoring default input for the slot: the joiner supplies it.
    /// The caller must also connect the joiner to every other peer, give
    /// each of them the [`pending_join`](Self::pending_join) ticket for
    /// [`hold_inputs_for_join`](Self::hold_inputs_for_join), and have each
    /// send its `authored_since(snapshot_tick)` and
    /// [`backlog_notice`](Self::backlog_notice) over the new link.
    ///
    /// The slot is handed out once. While a join is pending, a request of
    /// the same `joiner_id` with a higher `attempt` (a retry) supersedes it:
    /// this peer takes the slot back, authoring the default input for every
    /// tick since the old `first_input_tick`, and serves a fresh snapshot.
    /// That is safe because a joiner that checks its backlogs
    /// (`join_backlog_peers > 0`, required for a retry) sends no input
    /// before it is `Ready`. A request with a lower or equal `attempt` gets
    /// `StaleAttempt`; any other second request `SlotNotVacant`.
    ///
    /// Until the joiner's first input arrives, this peer holds its inputs
    /// after `snapshot_tick`, whatever `input_log_ticks` says.
    pub fn serve_join(&mut self, request: &[u8]) -> Result<Vec<u8>, JoinError> {
        let req = join::decode_request(request)?;
        req.header.check_against(&self.cfg, self.build_hash())?;
        let slot = req.header.slot;
        let open = self.vacant.get(&slot).is_some_and(|v| v.until == u64::MAX);
        let takeover_from = match self.grants.get(&slot) {
            None if open => None,
            None => return Err(JoinError::SlotNotVacant(slot)),
            Some(g) => {
                let retry = req.joiner_id != 0 && req.backlog_peers > 0 && g.joiner_id == req.joiner_id;
                if g.confirmed || !retry {
                    return Err(JoinError::SlotNotVacant(slot));
                }
                if req.attempt <= g.ticket.attempt {
                    return Err(JoinError::StaleAttempt { current: g.ticket.attempt, got: req.attempt });
                }
                // The old joiner never sent an input, so the slot's ticks
                // from its `first_input_tick` on have no author yet.
                Some(g.ticket.first_input_tick)
            }
        };
        let frame = self.ring.get(self.verified_tick).ok_or(JoinError::NoVerifiedFrame)?;
        let header = join::JoinHeader { build_hash: self.build_hash(), ..req.header };
        let ticket = JoinTicket {
            slot,
            attempt: req.attempt,
            snapshot_tick: self.verified_tick,
            first_input_tick: self.next_send_tick,
        };
        let message = join::encode_snapshot(
            &header,
            ticket.snapshot_tick,
            ticket.first_input_tick,
            frame.checksum(),
            ticket.attempt,
            &frame.to_bytes(),
        );
        if let Some(from) = takeover_from {
            self.author_defaults(slot, from, ticket.first_input_tick);
        }
        if let Some(span) = self.vacant.get_mut(&slot) {
            span.until = ticket.first_input_tick;
        }
        self.grants.insert(slot, Grant { joiner_id: req.joiner_id, ticket, confirmed: false });
        self.holds.insert(slot, ticket);
        Ok(message)
    }

    /// The ticket of the join this host is serving for `slot`, until the
    /// joiner's first input arrives.
    pub fn pending_join(&self, slot: PlayerSlot) -> Option<JoinTicket> {
        self.grants.get(&slot).filter(|g| !g.confirmed).map(|g| g.ticket)
    }

    /// Keeps every input this peer authors after `ticket.snapshot_tick`
    /// (whatever `input_log_ticks` says) so the joiner of `ticket.slot` can
    /// catch up from it. Call it on every existing peer when a join starts;
    /// a ticket of a retry replaces the old one. The hold ends when an input
    /// of the slot at `first_input_tick` or later arrives (the joiner is
    /// caught up), or on [`release_join_hold`](Self::release_join_hold).
    pub fn hold_inputs_for_join(&mut self, ticket: JoinTicket) {
        self.holds.insert(ticket.slot, ticket);
    }

    /// Ends the hold of `slot` (the join was given up), and drops this
    /// host's pending grant for it. Does not vacate the slot; see
    /// [`mark_slot_vacant`](Self::mark_slot_vacant).
    pub fn release_join_hold(&mut self, slot: PlayerSlot) {
        self.holds.remove(&slot);
        self.grants.remove(&slot);
    }

    /// Makes `slot`, played by a peer that is gone, open again: this peer
    /// authors default input for it from `from_tick` on, so
    /// [`serve_join`](Self::serve_join) can hand it to a new player.
    ///
    /// Exactly one author per slot per tick holds because the old author
    /// stops for good at `from_tick` (see below) and this peer starts
    /// exactly there: it authors `from_tick..next_send_tick` at once (also
    /// sending them) and every later tick in `advance`. `from_tick` must
    /// be after `verified_tick` (a verified tick cannot change), not after
    /// `next_send_tick`, and after every tick of the slot received from a
    /// remote peer. The caller must make sure of the rest: the old author
    /// sends nothing at or after `from_tick` (its link is closed), and
    /// every peer holds its inputs up to `from_tick - 1`. Pick
    /// `from_tick` as the highest [`last_remote_tick`](Self::last_remote_tick)
    /// of the slot over all peers, plus one.
    ///
    /// Cancels a pending join of the slot (its hold and grant).
    pub fn mark_slot_vacant(&mut self, slot: PlayerSlot, from_tick: u64) -> Result<(), JoinError> {
        if slot == self.cfg.local_slot || slot.0 >= self.cfg.player_count {
            return Err(JoinError::InvalidVacate("not a remote slot"));
        }
        let prev = self.vacant.get(&slot).copied();
        if prev.is_some_and(|v| v.until == u64::MAX) {
            return Err(JoinError::InvalidVacate("slot is already vacant"));
        }
        if from_tick <= self.verified_tick {
            return Err(JoinError::InvalidVacate("from tick is already verified"));
        }
        if from_tick > self.next_send_tick {
            return Err(JoinError::InvalidVacate("from tick is after the next send tick"));
        }
        if prev.is_some_and(|v| from_tick < v.until) {
            return Err(JoinError::InvalidVacate("from tick overlaps the previous default input"));
        }
        if self.last_remote_tick.get(&slot).is_some_and(|&t| t >= from_tick) {
            return Err(JoinError::InvalidVacate("an input at or after from tick was already received"));
        }
        self.vacant.insert(slot, VacantSpan { from: from_tick, until: u64::MAX });
        self.holds.remove(&slot);
        self.grants.remove(&slot);
        self.author_defaults(slot, from_tick, self.next_send_tick);
        Ok(())
    }

    /// The highest tick of an input of `slot` received from a remote peer.
    /// Peers compare these to pick `from_tick` for `mark_slot_vacant`.
    pub fn last_remote_tick(&self, slot: PlayerSlot) -> Option<u64> {
        self.last_remote_tick.get(&slot).copied()
    }

    /// Describes what this peer's `authored_since` backlog holds, for the
    /// joiner of `slot` (see [`hold_inputs_for_join`](Self::hold_inputs_for_join)).
    /// `None` when this peer holds no join of that slot. Send it over the
    /// same link as the backlog; the joiner reads it with
    /// [`receive_backlog`](Self::receive_backlog).
    pub fn backlog_notice(&self, slot: PlayerSlot) -> Option<Vec<u8>> {
        let ticket = self.holds.get(&slot)?;
        // Ticks are visited in order, so each slot's ticks form runs.
        let mut runs: BTreeMap<PlayerSlot, (u64, u64)> = BTreeMap::new();
        let mut spans = Vec::new();
        for (&tick, list) in &self.authored {
            for (s, ..) in list {
                match runs.get_mut(s) {
                    Some((_, last)) if *last + 1 == tick => *last = tick,
                    Some(run) => {
                        spans.push(join::Span { slot: *s, from: run.0, until: run.1 + 1 });
                        *run = (tick, tick);
                    }
                    None => {
                        runs.insert(*s, (tick, tick));
                    }
                }
            }
        }
        for (s, (from, last)) in runs {
            let open = s == self.cfg.local_slot || self.vacant.get(&s).is_some_and(|v| v.until == u64::MAX);
            spans.push(join::Span { slot: s, from, until: if open { u64::MAX } else { last + 1 } });
        }
        Some(join::encode_backlog(ticket.attempt, self.cfg.local_slot, &spans))
    }

    /// Joiner: takes in one existing peer's [`backlog_notice`](Self::backlog_notice).
    /// Once all `join_backlog_peers` notices are in, the spans of all peers
    /// must cover, from the tick after the snapshot on, the input of every
    /// remote slot (and of the joiner's own slot up to `first_input_tick`,
    /// which the host authored). Then the session is `Ready` and sends the
    /// inputs it held back. If a tick is offered by nobody, the gap can
    /// never be filled and this returns `InputGap`: drop the session and
    /// join again. Whether a missing input is late or gone is settled by
    /// the notices, not by time: until all are in the status is `Syncing`.
    ///
    /// A notice of another attempt returns `StaleAttempt` and changes
    /// nothing; a repeated notice of the same peer replaces the earlier one.
    pub fn receive_backlog(&mut self, message: &[u8]) -> Result<JoinStatus, JoinError> {
        let notice = join::decode_backlog(message)?;
        let Some(progress) = self.join.as_mut() else { return Ok(JoinStatus::Unchecked) };
        if notice.attempt != progress.attempt {
            return Err(JoinError::StaleAttempt { current: progress.attempt, got: notice.attempt });
        }
        let player_count = self.cfg.player_count;
        if notice.sender.0 >= player_count || notice.sender == self.cfg.local_slot {
            return Err(JoinError::Corrupt("notice sender is not another peer"));
        }
        if notice.spans.iter().any(|s| s.slot.0 >= player_count) {
            return Err(JoinError::Corrupt("notice span for an unknown slot"));
        }
        if matches!(progress.state, JoinState::Syncing) {
            progress.notices.insert(notice.sender, notice.spans);
            if progress.notices.len() as u64 >= u64::from(progress.expected) {
                self.settle_join();
            }
        }
        self.join_status()
    }

    /// All notices are in: decide between `Ready` and `Gap`.
    fn settle_join(&mut self) {
        let Some(progress) = self.join.as_mut() else { return };
        // Ticks up to `input_delay` are pre-confirmed, not sent by anyone.
        let need_from = (progress.snapshot_tick + 1).max(self.cfg.input_delay as u64 + 1);
        for slot in 0..self.cfg.player_count {
            let slot = PlayerSlot(slot);
            let need_until = if slot == self.cfg.local_slot { progress.first_input_tick } else { u64::MAX };
            let mut ranges: Vec<(u64, u64)> = progress
                .notices
                .values()
                .flatten()
                .filter(|s| s.slot == slot)
                .map(|s| (s.from, s.until))
                .collect();
            if let Some((needed_from, available_from)) = join::first_gap(&mut ranges, need_from, need_until) {
                progress.state = JoinState::Gap { slot, needed_from, available_from };
                return;
            }
        }
        progress.state = JoinState::Ready;
        let held = std::mem::take(&mut progress.held);
        for (tick, (slot, input, commands)) in held {
            self.source.send_local(tick, slot, input, commands);
        }
    }

    /// Joiner: how far the backlog check has come. `Err(InputGap)` once a
    /// hole is proven.
    pub fn join_status(&self) -> Result<JoinStatus, JoinError> {
        let Some(progress) = &self.join else { return Ok(JoinStatus::Unchecked) };
        match progress.state {
            JoinState::Syncing => Ok(JoinStatus::Syncing {
                received: progress.notices.len() as u32,
                expected: progress.expected,
            }),
            JoinState::Ready => Ok(JoinStatus::Ready),
            JoinState::Gap { slot, needed_from, available_from } => {
                Err(JoinError::InputGap { slot, needed_from, available_from })
            }
        }
    }

    /// Authors default input for `slot` at ticks `from..to`, sending it as
    /// this peer's own.
    fn author_defaults(&mut self, slot: PlayerSlot, from: u64, to: u64) {
        for tick in from..to {
            let idle = G::Input::default();
            self.confirmed_input.entry(tick).or_default().insert(slot, idle);
            self.authored.entry(tick).or_default().push((slot, idle, Vec::new()));
            self.emit(tick, slot, idle, Vec::new());
        }
    }

    /// Sends an input this peer authored, unless it is a joiner that has
    /// not yet proven its backlogs complete: then the input waits.
    fn emit(&mut self, tick: u64, slot: PlayerSlot, input: G::Input, commands: Vec<G::Command>) {
        match &mut self.join {
            Some(p) if !matches!(p.state, JoinState::Ready) => p.held.push((tick, (slot, input, commands))),
            _ => self.source.send_local(tick, slot, input, commands),
        }
    }

    /// Inputs (and commands) this peer authored for ticks after `tick`:
    /// its own slot and any vacant slot it still or previously filled.
    /// Kept `input_log_ticks` behind `verified_tick`, or since the snapshot
    /// tick of a join in transfer.
    pub fn authored_since(&self, tick: u64) -> Vec<RemoteInput<G>> {
        self.authored
            .range((Bound::Excluded(tick), Bound::Unbounded))
            .flat_map(|(&tick, list)| {
                list.iter().map(move |(slot, input, commands)| RemoteInput {
                    tick,
                    slot: *slot,
                    input: *input,
                    commands: commands.clone(),
                })
            })
            .collect()
    }

    /// The tick the next `advance` stamps the local input for.
    pub fn next_send_tick(&self) -> u64 {
        self.next_send_tick
    }

    pub fn source(&self) -> &S {
        &self.source
    }

    /// The input source, e.g. to attach a new link for a late joiner.
    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
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
    /// The stored frame of `tick` (predicted or verified), while it is
    /// still in the snapshot ring (about `max_prediction + 2` ticks back
    /// from the head).
    pub fn frame_at(&self, tick: u64) -> Option<&orr_ecs::Frame> {
        self.ring.get(tick)
    }
    /// How many rollbacks this session has done so far. Also counts the
    /// ones of an `advance` that returned `Stalled`, which carries no
    /// `RollbackInfo`.
    pub fn rollback_count(&self) -> u64 {
        self.rollback_count
    }
    /// The latest rollback, if any.
    pub fn last_rollback(&self) -> Option<RollbackInfo> {
        self.last_rollback
    }
    pub fn checksums(&self) -> &[(u64, u64)] {
        &self.checksums
    }
    /// Verified frames kept for desync diagnostics, oldest first (see
    /// `SessionConfig::keep_anchors`).
    pub fn anchors(&self) -> &VecDeque<Anchor> {
        &self.anchors
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
        let mut local_predicted = false;
        for slot in 0..n {
            let ps = PlayerSlot(slot);
            if let Some(v) = confirmed.and_then(|m| m.get(&ps)) {
                ti.set_input(ps, *v);
            } else if self.cfg.relay && ps == self.cfg.local_slot {
                // Relay: the own input is a prediction until the bundle.
                let own = self.local_pending.get(&tick).map(|p| p.0).unwrap_or(self.last_input[slot as usize]);
                ti.set_input(ps, own);
                predicted[slot as usize] = true;
                local_predicted = true;
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
        if local_predicted {
            if let Some((_, own)) = self.local_pending.get(&tick) {
                for c in own {
                    cmds.push((self.cfg.local_slot, c.clone()));
                }
                // Stable: keeps each slot's commands in submission order.
                cmds.sort_by_key(|(slot, _)| slot.0);
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
        let cmds = if self.cfg.relay {
            inputs
                .commands()
                .iter()
                .map(|(slot, c)| {
                    let mut bytes = Vec::new();
                    c.encode(&mut bytes);
                    (*slot, bytes)
                })
                .collect()
        } else {
            Vec::new()
        };
        self.history.insert(tick, TickRecord { inputs, predicted, events, cmds });
    }

    /// Relay mode: whether the confirmed bundle of `tick` differs from what
    /// the tick was simulated with, in any slot that was only predicted:
    /// its input, or its commands (a confirmed slot that had commands the
    /// prediction lacked, or the reverse).
    fn relay_tick_mismatch(&self, tick: u64, rec: &TickRecord<G>) -> bool {
        let Some(confirmed) = self.confirmed_input.get(&tick) else { return false };
        let commands = self.confirmed_commands.get(&tick);
        for (&slot, &val) in confirmed {
            if !rec.predicted[slot.0 as usize] {
                continue;
            }
            if *rec.inputs.input(slot) != val {
                return true;
            }
            let used = rec.cmds.iter().filter(|(s, _)| *s == slot).map(|(_, b)| b);
            let want = commands.and_then(|m| m.get(&slot)).into_iter().flatten().map(|c| {
                let mut bytes = Vec::new();
                c.encode(&mut bytes);
                bytes
            });
            if !used.eq(want.collect::<Vec<_>>().iter()) {
                return true;
            }
        }
        false
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

        let info = RollbackInfo { from_tick: from, to_tick: head, resim_count };
        self.rollback_count += 1;
        self.last_rollback = Some(info);
        info
    }

    /// Finds the earliest tick in `(verified_tick, head]` whose used input
    /// (recorded as `predicted`) disagrees with an input that has since
    /// been confirmed.
    fn earliest_mismatch(&self) -> Option<u64> {
        let mut earliest = None;
        for (&tick, rec) in self.history.range((self.verified_tick + 1)..=self.sim.tick()) {
            let Some(confirmed) = self.confirmed_input.get(&tick) else { continue };
            if self.cfg.relay {
                if self.relay_tick_mismatch(tick, rec) {
                    earliest = Some(earliest.map_or(tick, |e: u64| e.min(tick)));
                }
                continue;
            }
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
            }) && !(self.cfg.relay && self.relay_tick_mismatch(t, rec));
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
                    let checksum = f.checksum();
                    self.checksums.push((self.verified_tick, checksum));
                    if self.cfg.keep_anchors > 0 {
                        self.anchors.push_back(Anchor {
                            tick: self.verified_tick,
                            checksum,
                            frame_bytes: f.to_bytes(),
                        });
                        while self.anchors.len() > self.cfg.keep_anchors as usize {
                            self.anchors.pop_front();
                        }
                    }
                }
            }
        }
    }

    /// Advances the session by one tick: records the local player's input,
    /// ingests newly-arrived remote inputs, corrects any misprediction via
    /// rollback + resimulation, advances `verified_tick` through any newly
    /// fully-confirmed ticks, and — unless prediction is already at the
    /// cap — simulates one new tick.
    ///
    /// In relay mode this submits the input for the next send tick (see
    /// [`submit_local`](Self::submit_local)) and then calls
    /// [`step`](Self::step); a relay client that wants to submit inputs and
    /// simulate on separate schedules calls those two directly.
    pub fn advance(&mut self, local_input: G::Input, local_commands: Vec<G::Command>) -> AdvanceResult<G> {
        let send_tick = self.next_send_tick;
        self.next_send_tick += 1;
        if self.cfg.relay {
            self.submit_local(send_tick, local_input, local_commands);
            return self.step();
        }
        self.confirmed_input.entry(send_tick).or_default().insert(self.cfg.local_slot, local_input);
        if !local_commands.is_empty() {
            self.confirmed_commands.entry(send_tick).or_default().insert(self.cfg.local_slot, local_commands.clone());
        }
        self.authored.entry(send_tick).or_default().push((self.cfg.local_slot, local_input, local_commands.clone()));
        self.emit(send_tick, self.cfg.local_slot, local_input, local_commands);

        // Vacant slots this peer still covers get default input, sent like
        // any other peer's so everyone confirms the same thing.
        let covered: Vec<PlayerSlot> = self
            .vacant
            .iter()
            .filter(|&(_, span)| span.from <= send_tick && send_tick < span.until)
            .map(|(&slot, _)| slot)
            .collect();
        for slot in covered {
            self.author_defaults(slot, send_tick, send_tick + 1);
        }
        self.step()
    }

    /// Relay mode: records this peer's input for `tick` as its own
    /// prediction and sends it to the server through the input source. The
    /// input is *not* confirmed: only the server's bundle for the tick
    /// confirms it (possibly with a different value, if the server had to
    /// repeat or override it, which then rolls the session back).
    ///
    /// Call it once per tick, `input delay` ticks ahead of the tick being
    /// simulated. Ticks that were already verified are ignored.
    pub fn submit_local(&mut self, tick: u64, input: G::Input, commands: Vec<G::Command>) {
        debug_assert!(self.cfg.relay, "submit_local is for relay sessions");
        if tick <= self.verified_tick {
            return;
        }
        self.local_pending.insert(tick, (input, commands.clone()));
        self.source.send_local(tick, self.cfg.local_slot, input, commands);
    }

    /// Everything of [`advance`](Self::advance) after the local input is
    /// recorded: takes in arrived confirmed inputs, rolls back on a
    /// misprediction, verifies, and simulates one tick unless the
    /// prediction limit is reached.
    pub fn step(&mut self) -> AdvanceResult<G> {
        let (mut batch, rollback) = self.poll_confirmed();
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

    /// Takes in the inputs that arrived since the last call, rolls back and
    /// resimulates on a misprediction, and verifies every tick that is now
    /// fully confirmed, without simulating a new tick. [`step`](Self::step)
    /// is this plus one predicted tick.
    pub fn poll_confirmed(&mut self) -> (EventBatch<G::Event>, Option<RollbackInfo>) {
        for remote in self.source.poll_remote() {
            let seen = self.last_remote_tick.entry(remote.slot).or_insert(0);
            *seen = (*seen).max(remote.tick);
            // The joiner is caught up once it sends input of its own: the
            // hold on our inputs is over.
            if self.holds.get(&remote.slot).is_some_and(|t| remote.tick >= t.first_input_tick) {
                self.holds.remove(&remote.slot);
                if let Some(g) = self.grants.get_mut(&remote.slot) {
                    g.confirmed = true;
                }
            }
            // Already verified (a late joiner is sent the whole input log).
            if remote.tick <= self.verified_tick {
                continue;
            }
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
        if self.cfg.relay {
            self.local_pending = self.local_pending.split_off(&(self.verified_tick + 1));
        }
        let mut keep_from = self.verified_tick.saturating_sub(self.cfg.input_log_ticks as u64);
        for ticket in self.holds.values() {
            keep_from = keep_from.min(ticket.snapshot_tick + 1);
        }
        self.authored = self.authored.split_off(&keep_from);
        (batch, rollback)
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
