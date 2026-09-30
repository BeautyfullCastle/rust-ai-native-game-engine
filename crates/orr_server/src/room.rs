//! One room: player slots, the tick clock, input collection, tick
//! finalization, confirmed-tick delivery, time-sync feedback, checksum
//! comparison and late join.
//!
//! # Tick finalize rule
//!
//! A running room has a clock origin `t0` (server time of tick 0). Tick `T`
//! is *finalized* when server time reaches its deadline
//! `t0 + ceil(T * 1_000_000 / tick_rate)` microseconds, in tick order, no
//! earlier and no later than the caller's `update(now)` calls allow. At that
//! moment, for each slot:
//!
//! - the slot has a player and an accepted input for `T` is buffered: the
//!   input (and its commands) goes into the bundle as is;
//! - otherwise the slot's previous input is used, without commands, and the
//!   bundle marks the slot `FLAG_REPEATED` (and `FLAG_ABSENT` when nobody
//!   plays the slot).
//!
//! The bundle is then final. Inputs for `T` that arrive later are dropped
//! (counted, and reported to the sender as negative slack in its
//! `TimeSync`); the sender learns the truth from the bundle.
use std::collections::{BTreeMap, BTreeSet};

use orr_proto::{
    Bundle, Channel, ConnId, Endpoint, Hello, InputEntry, RejectReason, ServerMsg, SlotConfirmed, TimeSync, Welcome,
    FLAG_ABSENT, FLAG_REPEATED, NO_SLOT,
};

use crate::validate::{InputCtx, InputValidator, Verdict};

/// What a slot's input is while nobody plays it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VacantPolicy {
    /// Keep repeating the last input the slot had (the design's rule).
    #[default]
    RepeatLast,
    /// Use the room's default input.
    Default,
}

/// Room settings. Every client learns them from `Welcome`.
#[derive(Clone, Debug)]
pub struct RoomConfig {
    pub player_count: u8,
    pub tick_rate: u32,
    pub seed: u64,
    /// Build hash every client must have; `0` accepts any.
    pub build_hash: u64,
    /// Byte size of one input.
    pub input_size: u32,
    /// Input of a slot before its player sent one (`input_size` bytes).
    pub default_input: Vec<u8>,
    /// Opaque game configuration, handed to clients in `Welcome`.
    pub config_blob: Vec<u8>,
    /// Clients report a checksum every this many verified ticks.
    pub checksum_interval: u32,
    /// The room starts once this many clients are ready and no connected
    /// client is still syncing.
    pub min_players_to_start: u8,
    /// Confirmed ticks kept for late joiners and resends.
    pub retain_ticks: u32,
    /// A `TimeSync` goes to each client every this many finalized ticks.
    pub sync_interval_ticks: u32,
    /// Each confirmed bundle is sent in this many consecutive packets to a
    /// client that has not acked it (loss cover), then again after a
    /// resend timeout.
    pub bundle_redundancy: u8,
    /// Round trip assumed for a client before its first ping.
    pub default_rtt_us: u32,
    /// Payload budget of one unreliable message.
    pub max_unreliable_payload: u32,
    /// A donor that does not upload a snapshot within this many ticks is
    /// replaced.
    pub join_timeout_ticks: u32,
    /// A departed player's slot is kept for its token this many ticks.
    pub reserve_ticks: u32,
    /// A running room with nobody in it for this many ticks is closed.
    pub idle_close_ticks: u32,
    /// Inputs for ticks further than this ahead of the clock are ignored.
    pub max_ticks_ahead: u32,
    pub vacant_policy: VacantPolicy,
    /// Keep every confirmed bundle (for replays and tests) in
    /// [`Room::recorded`](crate::RelayServer::recorded).
    pub record_all: bool,
}

impl RoomConfig {
    pub fn new(player_count: u8, tick_rate: u32, seed: u64, input_size: u32) -> Self {
        Self {
            player_count,
            tick_rate,
            seed,
            build_hash: 0,
            input_size,
            default_input: vec![0; input_size as usize],
            config_blob: Vec::new(),
            checksum_interval: 30,
            min_players_to_start: player_count,
            retain_ticks: 900,
            sync_interval_ticks: 6,
            bundle_redundancy: 3,
            default_rtt_us: 100_000,
            max_unreliable_payload: 1100,
            join_timeout_ticks: 300,
            reserve_ticks: 600,
            idle_close_ticks: 1800,
            max_ticks_ahead: 120,
            vacant_policy: VacantPolicy::RepeatLast,
            record_all: false,
        }
    }
}

/// Things that happened, for logs and tests. Drain with
/// [`RelayServer::drain_notes`](crate::RelayServer::drain_notes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerNote {
    RoomStarted { room: u64, t0_us: u64 },
    PlayerJoined { room: u64, slot: u8, from_tick: u64 },
    PlayerLeft { room: u64, slot: u8, from_tick: u64 },
    Rejected { conn: ConnId, reason: RejectReason },
    /// `finalized` is the server tick when the mismatch was found.
    Desync { room: u64, tick: u64, finalized: u64, reports: Vec<(u8, u64)> },
    SnapshotRequested { room: u64, donor: u8, joiner: u8 },
    SnapshotRelayed { room: u64, donor: u8, joiner: u8, tick: u64, backlog_ticks: u32 },
    JoinFailed { room: u64, joiner: u8, reason: RejectReason },
    BadMessage { conn: ConnId },
    RoomClosed { room: u64 },
}

/// Per-slot counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotStats {
    /// Ticks confirmed with this slot's own input.
    pub delivered: u64,
    /// Ticks confirmed with a repeated input while a player was present.
    pub repeated: u64,
    /// Input entries that arrived after their tick was final and had not
    /// been seen before.
    pub late_dropped: u64,
    pub rejected_by_validator: u64,
    pub bundles_sent: u64,
}

/// Room counters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomStats {
    pub finalized: u64,
    pub slots: Vec<SlotStats>,
    pub desyncs: u64,
    pub snapshots_relayed: u64,
}

pub(crate) struct Out<'a, E: Endpoint> {
    pub ep: &'a mut E,
    pub notes: &'a mut Vec<ServerNote>,
    pub now_us: u64,
}

impl<E: Endpoint> Out<'_, E> {
    pub fn send(&mut self, conn: ConnId, channel: Channel, msg: &ServerMsg) {
        self.ep.send(conn, channel, &msg.encode());
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Welcome sent, still syncing its clock.
    Lobby,
    /// Synced, waiting for the room to start.
    LobbyReady,
    /// Late join: waiting for a donor's snapshot.
    AwaitSnapshot,
    /// Late join: snapshot and backlog sent, waiting for `Ready`.
    CatchUp,
    Playing,
}

struct Player {
    conn: ConnId,
    token: u64,
    phase: Phase,
}

#[derive(Clone, Copy)]
struct SendRec {
    count: u8,
    last_us: u64,
}

#[derive(Default)]
struct Window {
    from: u64,
    min: i32,
    sum: i64,
    n: u32,
    late: u32,
}

impl Window {
    fn reset(&mut self, from: u64) {
        *self = Window { from, min: i32::MAX, ..Window::default() };
    }
    fn sample(&mut self, slack_us: i64) {
        let s = slack_us.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        self.min = self.min.min(s);
        self.sum += i64::from(s);
        self.n += 1;
    }
}

struct Slot {
    player: Option<Player>,
    /// `(token, until finalized tick)`: the slot is kept for a returning
    /// player.
    reserved: Option<(u64, u64)>,
    last_input: Vec<u8>,
    pending: BTreeMap<u64, (Vec<u8>, Vec<Vec<u8>>)>,
    /// Ticks whose input this slot already delivered (or that were judged).
    seen: BTreeSet<u64>,
    acked: u64,
    sent: BTreeMap<u64, SendRec>,
    rtt_hint_us: u32,
    win: Window,
    stats: SlotStats,
}

struct PendingJoin {
    joiner: u8,
    donor: u8,
    request_id: u32,
    started_tick: u64,
    tried: BTreeSet<u8>,
}

pub(crate) struct Room {
    pub id: u64,
    cfg: RoomConfig,
    running: bool,
    t0_us: u64,
    finalized: u64,
    slots: Vec<Slot>,
    log: BTreeMap<u64, Bundle>,
    recorded: Vec<Bundle>,
    checksums: BTreeMap<u64, BTreeMap<u8, u64>>,
    notified: BTreeSet<u64>,
    join: Option<PendingJoin>,
    next_request: u32,
    empty_since: Option<u64>,
    last_sync_tick: u64,
    desyncs: u64,
    snapshots_relayed: u64,
}

fn require_same_build_hash(a: u64, b: u64) -> bool {
    a == 0 || b == 0 || a == b
}

impl Room {
    pub fn new(id: u64, cfg: RoomConfig) -> Self {
        assert!(cfg.player_count > 0 && cfg.player_count <= orr_proto::msg::MAX_SLOTS, "player_count out of range");
        assert!(cfg.tick_rate > 0, "tick_rate must be positive");
        assert_eq!(cfg.default_input.len() as u32, cfg.input_size, "default_input must have input_size bytes");
        let slots = (0..cfg.player_count)
            .map(|_| Slot {
                player: None,
                reserved: None,
                last_input: cfg.default_input.clone(),
                pending: BTreeMap::new(),
                seen: BTreeSet::new(),
                acked: 0,
                sent: BTreeMap::new(),
                rtt_hint_us: cfg.default_rtt_us,
                win: Window { min: i32::MAX, ..Window::default() },
                stats: SlotStats::default(),
            })
            .collect();
        Self {
            id,
            cfg,
            running: false,
            t0_us: 0,
            finalized: 0,
            slots,
            log: BTreeMap::new(),
            recorded: Vec::new(),
            checksums: BTreeMap::new(),
            notified: BTreeSet::new(),
            join: None,
            next_request: 1,
            empty_since: None,
            last_sync_tick: 0,
            desyncs: 0,
            snapshots_relayed: 0,
        }
    }

    pub fn stats(&self) -> RoomStats {
        RoomStats {
            finalized: self.finalized,
            slots: self.slots.iter().map(|s| s.stats).collect(),
            desyncs: self.desyncs,
            snapshots_relayed: self.snapshots_relayed,
        }
    }

    pub fn recorded(&self) -> &[Bundle] {
        &self.recorded
    }

    pub fn finalized(&self) -> u64 {
        self.finalized
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn connected_count(&self) -> usize {
        self.slots.iter().filter(|s| s.player.is_some()).count()
    }

    pub fn slot_of(&self, conn: ConnId) -> Option<u8> {
        self.slots.iter().position(|s| s.player.as_ref().is_some_and(|p| p.conn == conn)).map(|i| i as u8)
    }

    fn tick_us_ceil(&self, tick: u64) -> u64 {
        let n = u128::from(tick) * 1_000_000 + u128::from(self.cfg.tick_rate) - 1;
        (n / u128::from(self.cfg.tick_rate)) as u64
    }

    /// Server time at which `tick` is finalized.
    pub fn deadline(&self, tick: u64) -> u64 {
        self.t0_us.saturating_add(self.tick_us_ceil(tick))
    }

    // ---- joining and leaving -------------------------------------------

    /// Picks the slot for a `Hello`. Returns the slot, the token to hand
    /// out, and a stale connection of the same player that must be closed.
    fn pick_slot(&mut self, h: &Hello, new_token: u64) -> Result<(u8, u64, Option<ConnId>), RejectReason> {
        let n = self.slots.len();
        let now_tick = self.finalized;
        let live_reservation = |s: &Slot| s.reserved.is_some_and(|(_, until)| until > now_tick);
        // Reclaim by token: a vacant slot reserved for it, or a still
        // "connected" slot of the same token (the old connection is stale).
        if h.token != 0 {
            for (i, s) in self.slots.iter().enumerate() {
                if let Some(p) = &s.player {
                    if p.token == h.token {
                        return Ok((i as u8, h.token, Some(p.conn)));
                    }
                } else if s.reserved.is_some_and(|(t, _)| t == h.token) {
                    return Ok((i as u8, h.token, None));
                }
            }
        }
        if h.want_slot != NO_SLOT {
            let i = h.want_slot as usize;
            if i >= n {
                return Err(RejectReason::BadRequest);
            }
            let s = &self.slots[i];
            if s.player.is_some() || live_reservation(s) {
                return Err(RejectReason::SlotTaken);
            }
            return Ok((h.want_slot, new_token, None));
        }
        for (i, s) in self.slots.iter().enumerate() {
            if s.player.is_none() && !live_reservation(s) {
                return Ok((i as u8, new_token, None));
            }
        }
        Err(RejectReason::RoomFull)
    }

    /// Handles an accepted-or-refused `Hello`. On success returns the slot
    /// and the stale connection to close, if any.
    pub fn hello<E: Endpoint>(
        &mut self,
        out: &mut Out<'_, E>,
        conn: ConnId,
        h: &Hello,
        new_token: u64,
    ) -> Result<(u8, Option<ConnId>), RejectReason> {
        if !require_same_build_hash(self.cfg.build_hash, h.build_hash) {
            return Err(RejectReason::BuildHashMismatch { server: self.cfg.build_hash, client: h.build_hash });
        }
        if h.input_size != self.cfg.input_size {
            return Err(RejectReason::InputSizeMismatch { server: self.cfg.input_size, client: h.input_size });
        }
        let (slot, token, stale) = self.pick_slot(h, new_token)?;
        if stale.is_some() {
            self.vacate(out, slot, false);
        }
        let phase = if self.running { Phase::AwaitSnapshot } else { Phase::Lobby };
        let s = &mut self.slots[slot as usize];
        s.player = Some(Player { conn, token, phase });
        s.reserved = None;
        s.pending.clear();
        s.seen.clear();
        s.sent.clear();
        s.acked = self.finalized;
        s.win.reset(self.finalized + 1);
        self.empty_since = None;
        let welcome = Welcome {
            room: self.id,
            slot,
            player_count: self.cfg.player_count,
            tick_rate: self.cfg.tick_rate,
            seed: self.cfg.seed,
            build_hash: self.cfg.build_hash,
            input_size: self.cfg.input_size,
            checksum_interval: self.cfg.checksum_interval,
            token,
            running: self.running,
            t0_us: if self.running { self.t0_us } else { 0 },
            server_time_us: out.now_us,
            finalized_tick: self.finalized,
            config: self.cfg.config_blob.clone(),
        };
        out.send(conn, Channel::Reliable, &ServerMsg::Welcome(welcome));
        if self.running {
            self.pump_join(out);
        }
        Ok((slot, stale))
    }

    /// Removes the player of `slot` (left, disconnected or replaced).
    /// `announce` tells the others.
    pub fn vacate<E: Endpoint>(&mut self, out: &mut Out<'_, E>, slot: u8, announce: bool) {
        let running = self.running;
        let finalized = self.finalized;
        let reserve = u64::from(self.cfg.reserve_ticks);
        let s = &mut self.slots[slot as usize];
        let Some(p) = s.player.take() else { return };
        s.reserved = if running { Some((p.token, finalized + reserve)) } else { None };
        s.pending.clear();
        s.sent.clear();
        let was_playing = matches!(p.phase, Phase::Playing | Phase::CatchUp);
        if self.cfg.vacant_policy == VacantPolicy::Default {
            self.slots[slot as usize].last_input = self.cfg.default_input.clone();
        }
        if announce && was_playing {
            let from_tick = finalized + 1;
            out.notes.push(ServerNote::PlayerLeft { room: self.id, slot, from_tick });
            self.broadcast(out, &ServerMsg::Presence { slot, present: false, from_tick });
        }
        // A join that depended on this slot.
        if let Some(j) = &self.join {
            if j.joiner == slot {
                self.join = None;
            } else if j.donor == slot {
                self.donor_failed(out);
            }
        }
        if self.connected_count() == 0 {
            self.empty_since = Some(finalized);
        }
        if running {
            self.pump_join(out);
        } else {
            self.maybe_start(out);
        }
    }

    fn broadcast<E: Endpoint>(&self, out: &mut Out<'_, E>, msg: &ServerMsg) {
        for s in &self.slots {
            if let Some(p) = &s.player {
                if matches!(p.phase, Phase::Playing | Phase::CatchUp) {
                    out.send(p.conn, Channel::Reliable, msg);
                }
            }
        }
    }

    // ---- start ----------------------------------------------------------

    pub fn ready<E: Endpoint>(&mut self, out: &mut Out<'_, E>, slot: u8) {
        let finalized = self.finalized;
        let Some(p) = self.slots[slot as usize].player.as_mut() else { return };
        match p.phase {
            Phase::Lobby => {
                p.phase = Phase::LobbyReady;
                self.maybe_start(out);
            }
            Phase::CatchUp => {
                p.phase = Phase::Playing;
                let from_tick = finalized + 1;
                self.slots[slot as usize].win.reset(from_tick);
                out.notes.push(ServerNote::PlayerJoined { room: self.id, slot, from_tick });
                self.broadcast(out, &ServerMsg::Presence { slot, present: true, from_tick });
            }
            _ => {}
        }
    }

    fn maybe_start<E: Endpoint>(&mut self, out: &mut Out<'_, E>) {
        if self.running {
            return;
        }
        let ready = self.slots.iter().filter(|s| s.player.as_ref().is_some_and(|p| p.phase == Phase::LobbyReady)).count();
        let syncing = self.slots.iter().any(|s| s.player.as_ref().is_some_and(|p| p.phase == Phase::Lobby));
        if ready < usize::from(self.cfg.min_players_to_start) || syncing {
            return;
        }
        self.running = true;
        self.t0_us = out.now_us;
        self.finalized = 0;
        self.last_sync_tick = 0;
        out.notes.push(ServerNote::RoomStarted { room: self.id, t0_us: self.t0_us });
        for i in 0..self.slots.len() {
            let t0 = self.t0_us;
            let now = out.now_us;
            let s = &mut self.slots[i];
            s.win.reset(1);
            if let Some(p) = s.player.as_mut() {
                p.phase = Phase::Playing;
                let conn = p.conn;
                out.send(conn, Channel::Reliable, &ServerMsg::Start { t0_us: t0, server_time_us: now });
                out.notes.push(ServerNote::PlayerJoined { room: self.id, slot: i as u8, from_tick: 1 });
            }
        }
    }

    // ---- pings, inputs, checksums ----------------------------------------

    pub fn ping<E: Endpoint>(&mut self, out: &mut Out<'_, E>, slot: u8, seq: u32, client_time_us: u64, rtt_hint_us: u32) {
        let Some(p) = &self.slots[slot as usize].player else { return };
        let conn = p.conn;
        if rtt_hint_us > 0 {
            self.slots[slot as usize].rtt_hint_us = rtt_hint_us;
        }
        let t0 = if self.running { self.t0_us } else { u64::MAX };
        out.send(
            conn,
            Channel::Unreliable,
            &ServerMsg::Pong { seq, client_time_us, server_time_us: out.now_us, t0_us: t0, finalized_tick: self.finalized },
        );
    }

    pub fn input<V: InputValidator, E: Endpoint>(
        &mut self,
        out: &mut Out<'_, E>,
        validator: &mut V,
        slot: u8,
        claimed_slot: u8,
        ack_tick: u64,
        entries: Vec<InputEntry>,
    ) {
        if claimed_slot != slot || !self.running {
            return;
        }
        let finalized = self.finalized;
        let max_tick = finalized + u64::from(self.cfg.max_ticks_ahead);
        let now = out.now_us;
        let room = self.id;
        let s = &mut self.slots[slot as usize];
        if s.player.as_ref().is_none_or(|p| p.phase != Phase::Playing) {
            return;
        }
        s.acked = s.acked.max(ack_tick.min(finalized));
        for e in entries {
            if e.tick == 0 || e.tick > max_tick || e.input.len() as u32 != self.cfg.input_size {
                continue;
            }
            if !s.seen.insert(e.tick) {
                continue;
            }
            let deadline = self.t0_us.saturating_add(tick_us(e.tick, self.cfg.tick_rate));
            let slack = deadline as i64 - now as i64;
            s.win.sample(slack);
            if e.tick <= finalized {
                s.stats.late_dropped += 1;
                continue;
            }
            let ctx = InputCtx { room, slot, tick: e.tick, finalized, last_input: &s.last_input, now_us: now };
            match validator.validate(&ctx, &e.input, &e.commands) {
                Verdict::Accept => {
                    s.pending.insert(e.tick, (e.input, e.commands));
                }
                Verdict::DropCommands => {
                    s.pending.insert(e.tick, (e.input, Vec::new()));
                }
                Verdict::Reject => s.stats.rejected_by_validator += 1,
            }
        }
    }

    pub fn checksum<E: Endpoint>(&mut self, out: &mut Out<'_, E>, slot: u8, tick: u64, checksum: u64) {
        if !self.running || tick > self.finalized {
            return;
        }
        let reports = self.checksums.entry(tick).or_default();
        reports.insert(slot, checksum);
        let mismatch = reports.values().any(|&c| c != checksum);
        if mismatch && self.notified.insert(tick) {
            let list: Vec<(u8, u64)> = reports.iter().map(|(&s, &c)| (s, c)).collect();
            self.desyncs += 1;
            out.notes.push(ServerNote::Desync { room: self.id, tick, finalized: self.finalized, reports: list.clone() });
            self.broadcast(out, &ServerMsg::Desync { tick, reports: list });
        }
    }

    // ---- clock ------------------------------------------------------------

    /// Finalizes every tick whose deadline has passed and sends out what
    /// that produced.
    pub fn advance<E: Endpoint>(&mut self, out: &mut Out<'_, E>) {
        if !self.running {
            return;
        }
        let mut any = false;
        while self.deadline(self.finalized + 1) <= out.now_us {
            self.finalize_next();
            any = true;
        }
        if any {
            self.send_confirmed(out);
            self.send_time_sync(out);
            self.prune();
        }
        if let Some(j) = &self.join {
            if self.finalized.saturating_sub(j.started_tick) > u64::from(self.cfg.join_timeout_ticks) {
                self.donor_failed(out);
                self.pump_join(out);
            }
        }
    }

    /// Whether the room has been empty long enough to close.
    pub fn should_close(&self) -> bool {
        self.running
            && self.empty_since.is_some_and(|t| self.finalized.saturating_sub(t) >= u64::from(self.cfg.idle_close_ticks))
    }

    fn finalize_next(&mut self) {
        let tick = self.finalized + 1;
        let policy = self.cfg.vacant_policy;
        let mut slots = Vec::with_capacity(self.slots.len());
        for s in &mut self.slots {
            let playing = s.player.as_ref().is_some_and(|p| p.phase == Phase::Playing);
            let got = if playing { s.pending.remove(&tick) } else { None };
            let (input, commands, flags) = match got {
                Some((input, commands)) => {
                    s.stats.delivered += 1;
                    s.last_input = input.clone();
                    (input, commands, 0)
                }
                None => {
                    let mut flags = FLAG_REPEATED;
                    if playing {
                        s.stats.repeated += 1;
                        s.win.late += 1;
                    } else {
                        flags |= FLAG_ABSENT;
                        if policy == VacantPolicy::Default {
                            s.last_input = self.cfg.default_input.clone();
                        }
                    }
                    (s.last_input.clone(), Vec::new(), flags)
                }
            };
            s.pending.retain(|&t, _| t > tick);
            slots.push(SlotConfirmed { input, commands, flags });
        }
        let bundle = Bundle { tick, slots };
        if self.cfg.record_all {
            self.recorded.push(bundle.clone());
        }
        self.log.insert(tick, bundle);
        self.finalized = tick;
    }

    fn prune(&mut self) {
        let finalized = self.finalized;
        for s in &mut self.slots {
            let floor = finalized.saturating_sub(64);
            s.seen = s.seen.split_off(&floor);
        }
        let keep = finalized.saturating_sub(u64::from(self.cfg.checksum_interval) * 16);
        self.checksums = self.checksums.split_off(&keep);
        self.notified = self.notified.split_off(&keep);
        // Never prune while a join is being arranged: its snapshot may be
        // older than the usual window.
        if self.join.is_none() {
            let floor = finalized.saturating_sub(u64::from(self.cfg.retain_ticks));
            self.log = self.log.split_off(&(floor + 1));
        }
    }

    fn resend_timeout_us(&self, rtt_hint_us: u32) -> u64 {
        let tick = 1_000_000 / u64::from(self.cfg.tick_rate);
        (u64::from(rtt_hint_us) * 3 / 2 + tick).max(2 * tick)
    }

    fn send_confirmed<E: Endpoint>(&mut self, out: &mut Out<'_, E>) {
        let now = out.now_us;
        let input_size = self.cfg.input_size;
        let budget = self.cfg.max_unreliable_payload as usize - 32;
        let redundancy = self.cfg.bundle_redundancy;
        let floor = self.log.keys().next().copied().unwrap_or(self.finalized + 1);
        let mut dropped = Vec::new();
        for i in 0..self.slots.len() {
            let timeout = self.resend_timeout_us(self.slots[i].rtt_hint_us);
            let s = &mut self.slots[i];
            let Some(p) = &s.player else { continue };
            if !matches!(p.phase, Phase::Playing | Phase::CatchUp) {
                continue;
            }
            let conn = p.conn;
            if s.acked + 1 < floor {
                // The client is further behind than the server keeps.
                dropped.push(i as u8);
                continue;
            }
            let acked = s.acked;
            s.sent = s.sent.split_off(&(acked + 1));
            let mut chosen: Vec<Bundle> = Vec::new();
            let mut used = 0usize;
            for (&t, b) in self.log.range(acked + 1..).take_while(|(t, _)| **t <= self.finalized) {
                let due = match s.sent.get(&t) {
                    None => true,
                    Some(r) => r.count < redundancy || now.saturating_sub(r.last_us) >= timeout,
                };
                if !due {
                    continue;
                }
                let len = b.encoded_len(input_size);
                if len > budget {
                    // Too big for a datagram: send it reliably, once.
                    if s.sent.get(&t).is_none_or(|r| r.last_us != u64::MAX) {
                        out.send(conn, Channel::Reliable, &ServerMsg::Confirmed { input_size, bundles: vec![b.clone()] });
                        s.sent.insert(t, SendRec { count: u8::MAX, last_us: u64::MAX });
                        s.stats.bundles_sent += 1;
                    }
                    continue;
                }
                if used + len > budget {
                    break;
                }
                used += len;
                chosen.push(b.clone());
            }
            if chosen.is_empty() {
                continue;
            }
            for b in &chosen {
                let r = s.sent.entry(b.tick).or_insert(SendRec { count: 0, last_us: now });
                r.count = r.count.saturating_add(1);
                r.last_us = now;
                s.stats.bundles_sent += 1;
            }
            out.send(conn, Channel::Unreliable, &ServerMsg::Confirmed { input_size, bundles: chosen });
        }
        for slot in dropped {
            if let Some(conn) = self.slots[slot as usize].player.as_ref().map(|p| p.conn) {
                out.send(conn, Channel::Reliable, &ServerMsg::Bye { code: 1 });
                out.ep.disconnect(conn);
            }
            self.vacate(out, slot, true);
        }
    }

    fn send_time_sync<E: Endpoint>(&mut self, out: &mut Out<'_, E>) {
        if self.finalized < self.last_sync_tick + u64::from(self.cfg.sync_interval_ticks) {
            return;
        }
        let finalized = self.finalized;
        self.last_sync_tick = finalized;
        for s in &mut self.slots {
            let Some(p) = &s.player else { continue };
            if p.phase != Phase::Playing {
                continue;
            }
            let w = &s.win;
            if w.n > 0 {
                let avg = (w.sum / i64::from(w.n)) as i32;
                let msg = ServerMsg::TimeSync(TimeSync {
                    window_from: w.from,
                    finalized,
                    min_slack_us: w.min,
                    avg_slack_us: avg,
                    samples: w.n,
                    late: w.late,
                });
                out.send(p.conn, Channel::Unreliable, &msg);
            }
            s.win.reset(finalized + 1);
        }
    }

    // ---- late join --------------------------------------------------------

    /// Starts the next join if none is running: asks a donor for a snapshot.
    fn pump_join<E: Endpoint>(&mut self, out: &mut Out<'_, E>) {
        if self.join.is_some() {
            return;
        }
        let Some(joiner) = self
            .slots
            .iter()
            .position(|s| s.player.as_ref().is_some_and(|p| p.phase == Phase::AwaitSnapshot))
        else {
            return;
        };
        self.start_join(out, joiner as u8, BTreeSet::new());
    }

    fn start_join<E: Endpoint>(&mut self, out: &mut Out<'_, E>, joiner: u8, tried: BTreeSet<u8>) {
        let donor = self
            .slots
            .iter()
            .enumerate()
            .find(|(i, s)| {
                !tried.contains(&(*i as u8)) && s.player.as_ref().is_some_and(|p| p.phase == Phase::Playing)
            })
            .map(|(i, _)| i as u8);
        let Some(donor) = donor else {
            self.fail_join(out, joiner, RejectReason::NoDonor);
            return;
        };
        let request_id = self.next_request;
        self.next_request += 1;
        let conn = self.slots[donor as usize].player.as_ref().unwrap().conn;
        out.send(conn, Channel::Reliable, &ServerMsg::SnapshotRequest { request_id });
        out.notes.push(ServerNote::SnapshotRequested { room: self.id, donor, joiner });
        self.join = Some(PendingJoin { joiner, donor, request_id, started_tick: self.finalized, tried });
    }

    /// The donor failed (declined, left, too slow, bad snapshot): try the
    /// next one.
    fn donor_failed<E: Endpoint>(&mut self, out: &mut Out<'_, E>) {
        let Some(mut j) = self.join.take() else { return };
        j.tried.insert(j.donor);
        self.start_join(out, j.joiner, j.tried);
    }

    fn fail_join<E: Endpoint>(&mut self, out: &mut Out<'_, E>, joiner: u8, reason: RejectReason) {
        self.join = None;
        out.notes.push(ServerNote::JoinFailed { room: self.id, joiner, reason });
        if let Some(conn) = self.slots[joiner as usize].player.as_ref().map(|p| p.conn) {
            out.send(conn, Channel::Reliable, &ServerMsg::Reject(reason));
        }
        // The client closes the connection after the reject; the slot is
        // freed by its disconnect. Free it now so the next join can go on.
        self.slots[joiner as usize].player = None;
        self.pump_join(out);
    }

    pub fn snapshot_declined<E: Endpoint>(&mut self, out: &mut Out<'_, E>, slot: u8, request_id: u32) {
        if self.join.as_ref().is_some_and(|j| j.donor == slot && j.request_id == request_id) {
            self.donor_failed(out);
        }
    }

    pub fn snapshot_uploaded<E: Endpoint>(
        &mut self,
        out: &mut Out<'_, E>,
        slot: u8,
        request_id: u32,
        tick: u64,
        checksum: u64,
        data: Vec<u8>,
    ) {
        let Some(j) = &self.join else { return };
        if j.donor != slot || j.request_id != request_id {
            return;
        }
        let joiner = j.joiner;
        // The server must hold every confirmed tick after the snapshot.
        let covered = tick <= self.finalized
            && (tick == self.finalized || self.log.contains_key(&(tick + 1)))
            && (tick + 1..=self.finalized).all(|t| self.log.contains_key(&t));
        if !covered {
            self.donor_failed(out);
            return;
        }
        let Some(conn) = self.slots[joiner as usize].player.as_ref().map(|p| p.conn) else {
            self.join = None;
            return;
        };
        out.send(conn, Channel::Reliable, &ServerMsg::JoinSnapshot { tick, checksum, data });
        let input_size = self.cfg.input_size;
        let mut backlog = 0u32;
        let mut chunk: Vec<Bundle> = Vec::new();
        let mut chunk_bytes = 0usize;
        for (_, b) in self.log.range(tick + 1..).take_while(|(t, _)| **t <= self.finalized) {
            let len = b.encoded_len(input_size);
            if chunk_bytes + len > 60_000 && !chunk.is_empty() {
                out.send(conn, Channel::Reliable, &ServerMsg::Confirmed { input_size, bundles: std::mem::take(&mut chunk) });
                chunk_bytes = 0;
            }
            chunk.push(b.clone());
            chunk_bytes += len;
            backlog += 1;
        }
        if !chunk.is_empty() {
            out.send(conn, Channel::Reliable, &ServerMsg::Confirmed { input_size, bundles: chunk });
        }
        let finalized = self.finalized;
        let s = &mut self.slots[joiner as usize];
        if let Some(p) = s.player.as_mut() {
            p.phase = Phase::CatchUp;
        }
        s.acked = finalized;
        s.sent.clear();
        self.join = None;
        self.snapshots_relayed += 1;
        out.notes.push(ServerNote::SnapshotRelayed { room: self.id, donor: slot, joiner, tick, backlog_ticks: backlog });
        self.pump_join(out);
    }
}

fn tick_us(tick: u64, tick_rate: u32) -> u64 {
    let n = u128::from(tick) * 1_000_000 + u128::from(tick_rate) - 1;
    (n / u128::from(tick_rate)) as u64
}
