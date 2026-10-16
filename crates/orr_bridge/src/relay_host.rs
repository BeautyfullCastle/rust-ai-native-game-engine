//! A [`SimHost`] that plays through a relay server: it owns a
//! [`RelayClient`] (relay-mode `Session` plus the server protocol) over any
//! `orr_proto::Link`, so the `Threaded` bridge and the view interpolation
//! work the same as with the local loopback hosts.
//!
//! The sim thread calls [`SimHost::advance`] at the tick rate. The host
//! passes the wall clock to [`RelayClient::update`], which decides how many
//! ticks to simulate now (zero, one, or a few when catching up) from its own
//! send clock, and reports them back as one `AdvanceResult`. The input the
//! bridge holds is sampled for the ticks the client submits; commands queued
//! with `send_command` go out with the next submitted tick. Commands derived
//! from the input (a fire bit that spawns a bullet) must be derived here,
//! once per submitted tick, so use [`RelayHostOptions::commands_from_input`]
//! and leave the bridge's own derive off.
//!
//! `Instant` is fine here: this crate is not a sim crate.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use orr_ecs::Frame;
use orr_proto::{Link, RejectReason};
use orr_session::{AdvanceResult, ClientEvent, ClientState, EventBatch, RelayClient, RollbackInfo};
use orr_sim::{Game, PlayerSlot};

use crate::event::Lifecycle;
use crate::host::SimHost;

/// Live numbers of a relay host, readable from any thread (the window title).
#[derive(Debug, Default)]
pub struct RelayMetrics {
    srtt_us: AtomicU64,
    delay: AtomicU64,
    rate_ppm: AtomicU64,
    stall_episodes: AtomicU64,
    stalled_us: AtomicU64,
    own_repeated: AtomicU64,
    own_overridden: AtomicU64,
    desyncs: AtomicU64,
    hard_resyncs: AtomicU64,
    rollbacks: AtomicU64,
    resim_ticks: AtomicU64,
    head: AtomicU64,
    verified: AtomicU64,
    connected: AtomicU64,
    /// Confirmed checkpoints `(tick, frame checksum)` of the session, oldest first (capped).
    checksums: Mutex<Vec<(u64, u64)>>,
}

/// Checkpoints [`RelayMetrics`] keeps (the oldest are dropped above this).
const MAX_CHECKSUMS: usize = 4096;

/// A copy of [`RelayMetrics`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RelayStatus {
    pub rtt_ms: u32,
    /// Input delay in ticks.
    pub delay: u32,
    /// Send-clock rate in ppm of real time (1_000_000 = exact).
    pub rate_ppm: u32,
    pub stall_episodes: u64,
    pub stalled_ms: u64,
    /// Ticks the server confirmed with a repeated input of this client.
    pub repeats: u64,
    pub overridden: u64,
    pub desyncs: u64,
    pub hard_resyncs: u32,
    pub rollbacks: u64,
    pub resim_ticks: u64,
    pub head_tick: u64,
    pub verified_tick: u64,
    pub connected: bool,
}

impl RelayMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The frame checksum of the confirmed (verified) state at `tick`, if the session recorded one
    /// (every `checksum_interval` verified ticks, the same ones it reports to the server).
    pub fn checksum_at(&self, tick: u64) -> Option<u64> {
        let all = self.checksums.lock().unwrap_or_else(PoisonError::into_inner);
        all.binary_search_by_key(&tick, |(t, _)| *t).ok().map(|i| all[i].1)
    }

    /// The newest confirmed checkpoint `(tick, checksum)`.
    pub fn last_checksum(&self) -> Option<(u64, u64)> {
        self.checksums.lock().unwrap_or_else(PoisonError::into_inner).last().copied()
    }

    pub fn status(&self) -> RelayStatus {
        RelayStatus {
            rtt_ms: (self.srtt_us.load(Relaxed) / 1000) as u32,
            delay: self.delay.load(Relaxed) as u32,
            rate_ppm: self.rate_ppm.load(Relaxed) as u32,
            stall_episodes: self.stall_episodes.load(Relaxed),
            stalled_ms: self.stalled_us.load(Relaxed) / 1000,
            repeats: self.own_repeated.load(Relaxed),
            overridden: self.own_overridden.load(Relaxed),
            desyncs: self.desyncs.load(Relaxed),
            hard_resyncs: self.hard_resyncs.load(Relaxed) as u32,
            rollbacks: self.rollbacks.load(Relaxed),
            resim_ticks: self.resim_ticks.load(Relaxed),
            head_tick: self.head.load(Relaxed),
            verified_tick: self.verified.load(Relaxed),
            connected: self.connected.load(Relaxed) != 0,
        }
    }
}

/// Why [`RelayHost::connect`] gave up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectError {
    Rejected(RejectReason),
    /// The link went down before the game started.
    Disconnected,
    Failed(String),
    /// The room did not start in time (not enough players).
    Timeout,
}

impl core::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ConnectError::Rejected(r) => write!(f, "the server rejected the join: {r:?}"),
            ConnectError::Disconnected => write!(f, "could not reach the server (connection closed before the game started)"),
            ConnectError::Failed(e) => write!(f, "the join failed: {e}"),
            ConnectError::Timeout => write!(f, "timed out waiting for the room to start"),
        }
    }
}
impl std::error::Error for ConnectError {}

/// Called with each status line while connecting.
pub type StatusFn = Box<dyn FnMut(&str)>;

type Derive<G> = Box<dyn Fn(PlayerSlot, &<G as Game>::Input) -> Vec<<G as Game>::Command>>;

/// Settings of [`RelayHost::connect`].
pub struct RelayHostOptions<G: Game> {
    /// How long to wait for the handshake and for the room to start.
    pub connect_timeout: Duration,
    /// Called for status lines while connecting.
    pub on_status: Option<StatusFn>,
    /// Commands made from the input of every submitted tick (given this
    /// client's slot).
    pub commands_from_input: Option<Derive<G>>,
    /// Shared numbers for a window title or a log.
    pub metrics: Option<Arc<RelayMetrics>>,
}

impl<G: Game> Default for RelayHostOptions<G> {
    fn default() -> Self {
        Self { connect_timeout: Duration::from_secs(60), on_status: None, commands_from_input: None, metrics: None }
    }
}

/// See the module docs.
pub struct RelayHost<G: Game, L: Link> {
    client: RelayClient<G, L>,
    start: Instant,
    input: G::Input,
    pending: Vec<G::Command>,
    derive: Option<Derive<G>>,
    metrics: Arc<RelayMetrics>,
    lifecycle: Vec<Lifecycle>,
    tick_rate: u32,
    slot: PlayerSlot,
    players: u8,
    seen_stalled_us: u64,
    reported_disconnect: bool,
    /// How many of the session's checkpoints are in the metrics already.
    checksums_seen: usize,
}

impl<G: Game, L: Link> RelayHost<G, L> {
    /// Runs `client` until its game starts (the handshake, the clock sync,
    /// and the wait for the other players). Blocks; call it on the sim
    /// thread (`Threaded::try_spawn`).
    pub fn connect(mut client: RelayClient<G, L>, mut opts: RelayHostOptions<G>) -> Result<Self, ConnectError> {
        let start = Instant::now();
        let mut last_state = String::new();
        loop {
            let now = start.elapsed().as_micros() as u64;
            client.update(now, &mut |_| (G::Input::default(), Vec::new()));
            let state = format!("{:?}", client.state());
            if state != last_state {
                if let Some(cb) = opts.on_status.as_mut() {
                    cb(&state);
                }
                last_state = state;
            }
            match client.state() {
                ClientState::Playing => break,
                ClientState::Rejected(r) => return Err(ConnectError::Rejected(*r)),
                ClientState::Disconnected => return Err(ConnectError::Disconnected),
                ClientState::Failed(e) => return Err(ConnectError::Failed(e.clone())),
                _ => {}
            }
            if start.elapsed() > opts.connect_timeout {
                return Err(ConnectError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let w = client.welcome().expect("playing implies welcomed");
        let (tick_rate, slot, players) = (w.tick_rate, PlayerSlot(w.slot), w.player_count);
        let mut host = Self {
            client,
            start,
            input: G::Input::default(),
            pending: Vec::new(),
            derive: opts.commands_from_input,
            metrics: opts.metrics.unwrap_or_default(),
            lifecycle: Vec::new(),
            tick_rate,
            slot,
            players,
            seen_stalled_us: 0,
            reported_disconnect: false,
            checksums_seen: 0,
        };
        host.publish_metrics();
        Ok(host)
    }

    pub fn client(&self) -> &RelayClient<G, L> {
        &self.client
    }

    pub fn metrics(&self) -> Arc<RelayMetrics> {
        self.metrics.clone()
    }

    fn session(&self) -> &orr_session::Session<G, orr_session::RelaySource<G, L>> {
        self.client.session().expect("a playing client has a session")
    }

    fn publish_metrics(&mut self) {
        let m = &self.metrics;
        let (cs, ss) = (self.client.stats(), self.client.source_stats());
        m.srtt_us.store(self.client.srtt_us(), Relaxed);
        m.delay.store(u64::from(self.client.delay()), Relaxed);
        m.rate_ppm.store(u64::from(self.client.rate_ppm()), Relaxed);
        m.stall_episodes.store(cs.stall_episodes, Relaxed);
        m.stalled_us.store(cs.stalled_us, Relaxed);
        m.own_repeated.store(ss.own_repeated, Relaxed);
        m.own_overridden.store(ss.own_overridden, Relaxed);
        m.desyncs.store(cs.desyncs, Relaxed);
        m.hard_resyncs.store(u64::from(cs.hard_resyncs), Relaxed);
        m.resim_ticks.store(cs.resim_ticks, Relaxed);
        if let Some(s) = self.client.session() {
            m.rollbacks.store(s.rollback_count(), Relaxed);
            m.head.store(s.head_tick(), Relaxed);
            m.verified.store(s.verified_tick(), Relaxed);
            let checkpoints = s.checksums();
            if checkpoints.len() > self.checksums_seen {
                let mut all = m.checksums.lock().unwrap_or_else(PoisonError::into_inner);
                all.extend_from_slice(&checkpoints[self.checksums_seen..]);
                if all.len() > MAX_CHECKSUMS {
                    let excess = all.len() - MAX_CHECKSUMS;
                    all.drain(..excess);
                }
                self.checksums_seen = checkpoints.len();
            }
        }
        m.connected.store(u64::from(*self.client.state() == ClientState::Playing), Relaxed);
    }
}

impl<G: Game, L: Link> SimHost<G> for RelayHost<G, L> {
    fn tick_rate(&self) -> u32 {
        self.tick_rate
    }
    fn local_slot(&self) -> PlayerSlot {
        self.slot
    }
    fn player_count(&self) -> u8 {
        self.players
    }

    fn advance(&mut self, input: G::Input, commands: Vec<G::Command>) -> AdvanceResult<G> {
        self.input = input;
        let accepting_commands = self.accepts_commands();
        if accepting_commands {
            self.pending.extend(commands);
        }
        let now = self.start.elapsed().as_micros() as u64;
        let (pending, derive, current, slot) = (&mut self.pending, &self.derive, self.input, self.slot);
        let up = self.client.update(now, &mut |_tick| {
            let mut cmds = std::mem::take(pending);
            if accepting_commands {
                if let Some(d) = derive {
                    cmds.extend(d(slot, &current));
                }
            }
            (current, cmds)
        });
        let mut events = EventBatch::empty();
        for batch in up.events {
            events.append(batch);
        }
        for ev in self.client.drain_events() {
            match ev {
                ClientEvent::Desync { tick, .. } => self.lifecycle.push(Lifecycle::Desync { tick }),
                ClientEvent::DelayChanged { delay } => self.lifecycle.push(Lifecycle::DelayChanged { delay }),
                _ => {}
            }
        }
        let down = matches!(self.client.state(), ClientState::Disconnected | ClientState::Failed(_) | ClientState::Rejected(_));
        if down && !self.reported_disconnect {
            self.reported_disconnect = true;
            self.lifecycle.push(Lifecycle::Disconnected);
        }
        self.publish_metrics();
        let stalled_us = self.client.stats().stalled_us;
        let stalled = stalled_us > self.seen_stalled_us || down;
        self.seen_stalled_us = stalled_us;
        if stalled {
            AdvanceResult::Stalled { events }
        } else {
            AdvanceResult::Advanced { tick: self.session().head_tick(), events, rollback: up.rollbacks.last().copied() }
        }
    }

    fn head_tick(&self) -> u64 {
        self.session().head_tick()
    }
    fn verified_tick(&self) -> u64 {
        self.session().verified_tick()
    }
    fn predicted_frame(&self) -> &Frame {
        self.session().predicted_frame()
    }
    fn verified_frame(&self) -> Option<&Frame> {
        self.session().verified_frame()
    }
    fn frame_at(&self, tick: u64) -> Option<&Frame> {
        self.session().frame_at(tick)
    }
    fn rollback_count(&self) -> u64 {
        self.session().rollback_count()
    }
    fn last_rollback(&self) -> Option<RollbackInfo> {
        self.session().last_rollback()
    }
    fn pending_command_count(&self) -> usize {
        self.pending.len()
    }
    fn accepts_commands(&self) -> bool {
        !matches!(
            self.client.state(),
            ClientState::Disconnected | ClientState::Failed(_) | ClientState::Rejected(_)
        )
    }
    fn take_lifecycle(&mut self) -> Vec<Lifecycle> {
        std::mem::take(&mut self.lifecycle)
    }
}

#[cfg(test)]
#[path = "relay_host_tests.rs"]
mod tests;
