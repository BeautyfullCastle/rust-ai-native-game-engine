//! Relay mode, client side: [`RelayClient`] drives a relay-mode
//! [`Session`] against a relay server (`orr_server`) over an abstract
//! [`Link`].
//!
//! What it does:
//!
//! - **Handshake**: `Hello` (build hash, slot wish, rejoin token) ->
//!   `Welcome` (slot, tick rate, seed, config). A build-hash mismatch is
//!   rejected by the server.
//! - **Clock**: pings measure round-trip time (smoothed, with variation)
//!   and the offset to the server clock. The server tick is the reference.
//! - **Inputs**: the local input for tick `h` is sampled when the client's
//!   *send clock* reaches `h` and sent to the server (redundantly: the last
//!   `input_redundancy` unconfirmed inputs in every packet). The session
//!   simulates tick `h - delay` at that moment, using its own input as a
//!   prediction. Only the server's `Confirmed` bundle verifies a tick.
//! - **Input delay** (auto): `delay` is the number of ticks between
//!   sampling and simulating an input. See [`RelayClientConfig`].
//! - **Time sync**: the server reports how early this client's inputs
//!   arrived; the client scales the rate of its send clock by up to +-2% to
//!   keep the smallest arrival slack near a target.
//! - **Desync**: on the server's `Desync` notice the client writes a
//!   `.orrd` dump ([`crate::DesyncDump`]) into a [`DumpSink`].
//! - **Late join**: a client that joins a running room gets a snapshot the
//!   server asked another client for, then the confirmed bundles since; it
//!   also serves as donor when asked.
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

use orr_proto::{
    Bundle, Channel, ClientMsg, Hello, InputEntry, Link, LinkEvent, RejectReason, ServerMsg, TimeSync, Welcome,
    FLAG_REPEATED, NO_SLOT,
};
use orr_sim::{Game, PlayerSlot, SimCommand};

use crate::dump::DesyncDump;
use crate::events::EventBatch;
use crate::input_source::{InputSource, RemoteInput};
use crate::session::{AdvanceResult, Anchor, RollbackInfo, Session, SessionConfig};

/// Where diagnostic dumps go. No paths are hardcoded: the caller decides
/// (a directory, a crash reporter, memory).
pub trait DumpSink {
    fn write_dump(&mut self, name: &str, bytes: Vec<u8>);
}

/// A [`DumpSink`] that keeps dumps in memory; clones share the storage.
#[derive(Clone, Default)]
pub struct DumpCollector(Rc<RefCell<Vec<(String, Vec<u8>)>>>);

impl DumpCollector {
    pub fn new() -> Self {
        Self::default()
    }
    /// Takes every dump written so far.
    pub fn take(&self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.0.borrow_mut())
    }
    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl DumpSink for DumpCollector {
    fn write_dump(&mut self, name: &str, bytes: Vec<u8>) {
        self.0.borrow_mut().push((name.to_string(), bytes));
    }
}

/// Counters of one relay input source (the network side).
#[derive(Clone, Copy, Debug, Default)]
pub struct SourceStats {
    pub bundles_received: u64,
    pub duplicate_bundles: u64,
    /// Bundles or messages that did not decode or did not fit the room.
    pub decode_errors: u64,
    pub bad_commands: u64,
    /// The server repeated this client's input for a tick (it arrived late
    /// or was lost).
    pub own_repeated: u64,
    /// The confirmed input or commands of this client's slot differed from
    /// what it had sent for the tick (so the session rolled back).
    pub own_overridden: u64,
    pub packets_sent: u64,
}

/// The [`InputSource`] of a relay session: sends the local inputs to the
/// server and delivers the server's confirmed bundles as remote inputs (for
/// every slot, the local one included). Also collects the other server
/// messages for [`RelayClient`].
pub struct RelaySource<G: Game, L: Link> {
    link: L,
    input_size: u32,
    slot: u8,
    player_count: u8,
    redundancy: usize,
    inbox: Vec<RemoteInput<G>>,
    others: VecDeque<ServerMsg>,
    /// Inputs sent and not yet known to be confirmed, by tick.
    out: BTreeMap<u64, (Vec<u8>, Vec<Vec<u8>>)>,
    dirty: bool,
    /// Every bundle up to this tick is in.
    ack: u64,
    /// Bundles received above `ack`.
    seen: BTreeSet<u64>,
    /// Confirmed bundles, kept for desync dumps.
    log: BTreeMap<u64, Bundle>,
    connected: bool,
    disconnected: bool,
    stats: SourceStats,
}

impl<G: Game, L: Link> RelaySource<G, L> {
    fn new(link: L, redundancy: usize) -> Self {
        Self {
            link,
            input_size: std::mem::size_of::<G::Input>() as u32,
            slot: 0,
            player_count: 0,
            redundancy: redundancy.max(1),
            inbox: Vec::new(),
            others: VecDeque::new(),
            out: BTreeMap::new(),
            dirty: false,
            ack: 0,
            seen: BTreeSet::new(),
            log: BTreeMap::new(),
            connected: false,
            disconnected: false,
            stats: SourceStats::default(),
        }
    }

    fn configure(&mut self, slot: u8, player_count: u8) {
        self.slot = slot;
        self.player_count = player_count;
    }

    /// Everything up to `tick` is known (a snapshot covers it).
    fn set_ack_floor(&mut self, tick: u64) {
        if tick > self.ack {
            self.ack = tick;
            self.seen = self.seen.split_off(&(tick + 1));
            self.advance_ack();
        }
    }

    fn advance_ack(&mut self) {
        while self.seen.remove(&(self.ack + 1)) {
            self.ack += 1;
        }
        self.out = self.out.split_off(&(self.ack + 1));
    }

    pub fn stats(&self) -> SourceStats {
        self.stats
    }

    /// The highest tick up to which every confirmed bundle has arrived.
    pub fn acked_tick(&self) -> u64 {
        self.ack
    }

    fn send(&mut self, channel: Channel, msg: &ClientMsg) {
        self.link.send(channel, &msg.encode());
    }

    fn pump(&mut self) {
        while let Some(ev) = self.link.poll() {
            match ev {
                LinkEvent::Connected => self.connected = true,
                LinkEvent::Disconnected => self.disconnected = true,
                LinkEvent::Message { data, .. } => match ServerMsg::decode(&data) {
                    Ok(ServerMsg::Confirmed { input_size, bundles }) => {
                        if input_size != self.input_size {
                            self.stats.decode_errors += 1;
                            continue;
                        }
                        for b in bundles {
                            self.on_bundle(b);
                        }
                    }
                    Ok(other) => self.others.push_back(other),
                    Err(_) => self.stats.decode_errors += 1,
                },
            }
        }
    }

    fn on_bundle(&mut self, b: Bundle) {
        if b.tick == 0 || b.slots.len() != self.player_count as usize {
            self.stats.decode_errors += 1;
            return;
        }
        if b.tick <= self.ack || self.seen.contains(&b.tick) {
            self.stats.duplicate_bundles += 1;
            return;
        }
        let mut decoded = Vec::with_capacity(b.slots.len());
        for (i, sc) in b.slots.iter().enumerate() {
            let Ok(input) = bytemuck::try_pod_read_unaligned::<G::Input>(&sc.input) else {
                self.stats.decode_errors += 1;
                return;
            };
            let mut commands = Vec::with_capacity(sc.commands.len());
            for raw in &sc.commands {
                match <G::Command as SimCommand>::decode(raw) {
                    Some(c) => commands.push(c),
                    None => self.stats.bad_commands += 1,
                }
            }
            decoded.push(RemoteInput { tick: b.tick, slot: PlayerSlot(i as u8), input, commands });
        }
        let own = &b.slots[self.slot as usize];
        if own.flags & FLAG_REPEATED != 0 {
            self.stats.own_repeated += 1;
        }
        if let Some((input, commands)) = self.out.get(&b.tick) {
            if *input != own.input || *commands != own.commands {
                self.stats.own_overridden += 1;
            }
        }
        self.stats.bundles_received += 1;
        self.seen.insert(b.tick);
        self.inbox.extend(decoded);
        self.log.insert(b.tick, b);
        self.advance_ack();
    }

    /// Sends the newest unconfirmed inputs (with the ack of the bundles).
    fn flush(&mut self) {
        if !self.dirty || self.out.is_empty() {
            self.dirty = false;
            return;
        }
        self.dirty = false;
        let mut entries: Vec<InputEntry> = Vec::new();
        let mut bytes = 0usize;
        for (&tick, (input, commands)) in self.out.iter().rev().take(self.redundancy) {
            let len = 12 + input.len() + commands.iter().map(|c| 4 + c.len()).sum::<usize>();
            if bytes + len > 1000 && !entries.is_empty() {
                break;
            }
            bytes += len;
            entries.push(InputEntry { tick, input: input.clone(), commands: commands.clone() });
        }
        entries.reverse();
        let msg = ClientMsg::Input { slot: self.slot, input_size: self.input_size, ack_tick: self.ack, entries };
        self.stats.packets_sent += 1;
        self.send(Channel::Unreliable, &msg);
    }
}

impl<G: Game, L: Link> InputSource<G> for RelaySource<G, L> {
    fn send_local(&mut self, tick: u64, _slot: PlayerSlot, input: G::Input, commands: Vec<G::Command>) {
        let input_bytes = bytemuck::bytes_of(&input).to_vec();
        let cmd_bytes = commands
            .iter()
            .map(|c| {
                let mut b = Vec::new();
                c.encode(&mut b);
                b
            })
            .collect();
        self.out.insert(tick, (input_bytes, cmd_bytes));
        self.dirty = true;
    }

    fn poll_remote(&mut self) -> Vec<RemoteInput<G>> {
        self.pump();
        self.flush();
        std::mem::take(&mut self.inbox)
    }
}

/// Settings of a [`RelayClient`].
#[derive(Clone, Debug)]
pub struct RelayClientConfig {
    /// Build identity (see `Simulation::with_build_id`); the server rejects
    /// a client whose build hash differs from the room's.
    pub build_id: u64,
    pub room: u64,
    /// Slot wanted, or any.
    pub want_slot: Option<PlayerSlot>,
    /// Token of an earlier `Welcome` to take the same slot back (`0` = new).
    pub token: u64,
    /// Bounds of the input delay in ticks. Default `2..=10`.
    pub min_delay: u32,
    pub max_delay: u32,
    /// Prediction limit of the session (design default 8).
    pub max_prediction: u32,
    /// The auto delay keeps the expected prediction depth this many ticks
    /// below `max_prediction`. Default 1.
    pub prediction_margin: u32,
    /// Target of the smallest input arrival slack at the server, in
    /// thousandths of a tick. Default 1500.
    pub target_slack_milliticks: u32,
    /// Largest deviation of the send-clock rate from 1, in parts per
    /// million. Default 20_000 (2%).
    pub max_rate_dev_ppm: u32,
    /// Rate change per tick of slack error, in ppm. Default 15_000.
    pub rate_gain_ppm_per_tick: u32,
    /// A slack error beyond this many ticks in two time syncs in a row makes
    /// the client jump its clock instead of slewing it. Default 8.
    pub hard_resync_ticks: u32,
    /// Ping period while playing, and while syncing (microseconds).
    pub ping_interval_us: u64,
    pub sync_ping_interval_us: u64,
    /// Pongs needed before the client reports itself ready.
    pub sync_pings: u32,
    /// Unconfirmed inputs in every input packet. Default 4.
    pub input_redundancy: u32,
    /// Most ticks simulated in one `update`. Default 32.
    pub max_steps_per_update: u32,
    /// A lower delay must hold this many ticks before it is applied.
    pub delay_decrease_ticks: u32,
    /// Verified frames kept for desync dumps.
    pub keep_anchors: u32,
}

impl RelayClientConfig {
    pub fn new(room: u64, build_id: u64) -> Self {
        Self {
            build_id,
            room,
            want_slot: None,
            token: 0,
            min_delay: 2,
            max_delay: 10,
            max_prediction: 8,
            prediction_margin: 1,
            target_slack_milliticks: 1500,
            max_rate_dev_ppm: 20_000,
            rate_gain_ppm_per_tick: 15_000,
            hard_resync_ticks: 8,
            ping_interval_us: 250_000,
            sync_ping_interval_us: 40_000,
            sync_pings: 5,
            input_redundancy: 4,
            max_steps_per_update: 32,
            delay_decrease_ticks: 120,
            keep_anchors: 3,
        }
    }
}

/// Where a [`RelayClient`] is in its life.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientState {
    /// Waiting for the link to come up.
    Connecting,
    /// `Hello` sent, waiting for `Welcome`.
    Handshake,
    /// Welcomed into a room that has not started: syncing the clock.
    Syncing,
    /// Welcomed into a running room: waiting for the relayed snapshot.
    AwaitSnapshot,
    /// Late join: the snapshot is loaded, the session runs to catch up.
    CatchingUp,
    Playing,
    Rejected(RejectReason),
    Disconnected,
    Failed(String),
}

/// Things worth telling the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientEvent {
    Welcomed { slot: u8 },
    Started,
    /// A late join finished loading the snapshot.
    SnapshotLoaded { tick: u64 },
    Presence { slot: u8, present: bool, from_tick: u64 },
    /// The server found different checksums at `tick`; a dump was written.
    Desync { tick: u64, dump_name: String },
    DelayChanged { delay: u32 },
    HardResync { error_us: i64 },
}

/// Counters of a client.
#[derive(Clone, Debug, Default)]
pub struct ClientStats {
    /// Ticks simulated (predicted) by `step`, resimulation not counted.
    pub steps: u64,
    /// Ticks resimulated by rollbacks (only those of steps that advanced).
    pub resim_ticks: u64,
    /// Times the prediction limit stopped the simulation, counted once per
    /// stretch of consecutive updates.
    pub stall_episodes: u64,
    /// Local time spent stalled, microseconds.
    pub stalled_us: u64,
    /// Largest `head - verified` seen.
    pub max_prediction_depth: u64,
    pub delay_changes: u32,
    pub hard_resyncs: u32,
    pub time_syncs: u64,
    pub desyncs: u64,
    pub dumps_written: u64,
    pub snapshots_served: u64,
    pub pongs: u64,
}

/// What one [`RelayClient::update`] produced.
pub struct RelayUpdate<G: Game> {
    /// Ticks simulated in this update.
    pub steps: u32,
    pub rollbacks: Vec<RollbackInfo>,
    /// Event batches for the view layer, in order.
    pub events: Vec<EventBatch<G::Event>>,
}

impl<G: Game> Default for RelayUpdate<G> {
    fn default() -> Self {
        Self { steps: 0, rollbacks: Vec::new(), events: Vec::new() }
    }
}

struct PendingSnapshot {
    tick: u64,
    checksum: u64,
    frame_bytes: Vec<u8>,
}

const MICRO: u64 = 1_000_000;

/// See the module docs.
pub struct RelayClient<G: Game, L: Link> {
    cfg: RelayClientConfig,
    make_config: Box<dyn FnMut(&Welcome) -> G::Config>,
    dumps: Box<dyn DumpSink>,
    source: Option<RelaySource<G, L>>,
    session: Option<Session<G, RelaySource<G, L>>>,
    state: ClientState,
    welcome: Option<Welcome>,
    events: Vec<ClientEvent>,
    pending_snapshot: Option<PendingSnapshot>,

    now_us: u64,
    offset_us: i64,
    samples: VecDeque<(u64, i64)>,
    pongs: u32,
    srtt_us: u64,
    rttvar_us: u64,
    ping_seq: u32,
    next_ping_us: u64,
    t0_us: Option<u64>,
    sent_ready: bool,

    /// Send clock in millionths of a tick; `floor / 1e6` is the tick whose
    /// input is sampled now.
    send_clock: u64,
    rate_ppm: u32,
    delay: u32,
    next_submit: u64,
    submitting: bool,
    below_since: Option<u64>,
    hard_streak: u8,

    stalled_prev: bool,
    reported: usize,
    base_anchor: Option<Anchor>,
    stats: ClientStats,
}

impl<G: Game, L: Link> RelayClient<G, L> {
    /// `make_config` builds the game's `Config` from the server's `Welcome`
    /// (player count, seed, the room's opaque config blob).
    pub fn new(
        cfg: RelayClientConfig,
        link: L,
        make_config: impl FnMut(&Welcome) -> G::Config + 'static,
        dumps: impl DumpSink + 'static,
    ) -> Self {
        let redundancy = cfg.input_redundancy as usize;
        Self {
            delay: cfg.min_delay,
            cfg,
            make_config: Box::new(make_config),
            dumps: Box::new(dumps),
            source: Some(RelaySource::new(link, redundancy)),
            session: None,
            state: ClientState::Connecting,
            welcome: None,
            events: Vec::new(),
            pending_snapshot: None,
            now_us: 0,
            offset_us: 0,
            samples: VecDeque::new(),
            pongs: 0,
            srtt_us: 0,
            rttvar_us: 0,
            ping_seq: 0,
            next_ping_us: 0,
            t0_us: None,
            sent_ready: false,
            send_clock: 0,
            rate_ppm: MICRO as u32,
            next_submit: 0,
            submitting: false,
            below_since: None,
            hard_streak: 0,
            stalled_prev: false,
            reported: 0,
            base_anchor: None,
            stats: ClientStats::default(),
        }
    }

    pub fn state(&self) -> &ClientState {
        &self.state
    }

    pub fn session(&self) -> Option<&Session<G, RelaySource<G, L>>> {
        self.session.as_ref()
    }

    pub fn session_mut(&mut self) -> Option<&mut Session<G, RelaySource<G, L>>> {
        self.session.as_mut()
    }

    pub fn welcome(&self) -> Option<&Welcome> {
        self.welcome.as_ref()
    }

    /// The token to pass as `RelayClientConfig::token` to take the slot back
    /// after a disconnect.
    pub fn token(&self) -> Option<u64> {
        self.welcome.as_ref().map(|w| w.token)
    }

    pub fn stats(&self) -> &ClientStats {
        &self.stats
    }

    pub fn source_stats(&self) -> SourceStats {
        self.src().stats
    }

    /// Current input delay in ticks.
    pub fn delay(&self) -> u32 {
        self.delay
    }

    /// Current send-clock rate in ppm (1_000_000 = real time).
    pub fn rate_ppm(&self) -> u32 {
        self.rate_ppm
    }

    pub fn srtt_us(&self) -> u64 {
        self.srtt_us
    }

    pub fn drain_events(&mut self) -> Vec<ClientEvent> {
        std::mem::take(&mut self.events)
    }

    /// Tells the server this client leaves, and closes the link.
    pub fn leave(&mut self) {
        self.src_mut().send(Channel::Reliable, &ClientMsg::Leave);
        self.src_mut().link.close();
        self.state = ClientState::Disconnected;
    }

    fn src(&self) -> &RelaySource<G, L> {
        match (&self.source, &self.session) {
            (Some(s), _) => s,
            (None, Some(sess)) => sess.source(),
            (None, None) => unreachable!("source is in the client or the session"),
        }
    }

    fn src_mut(&mut self) -> &mut RelaySource<G, L> {
        match (&mut self.source, &mut self.session) {
            (Some(s), _) => s,
            (None, Some(sess)) => sess.source_mut(),
            (None, None) => unreachable!("source is in the client or the session"),
        }
    }

    fn tick_rate(&self) -> u64 {
        u64::from(self.welcome.as_ref().map_or(60, |w| w.tick_rate))
    }

    fn tick_us(&self) -> u64 {
        MICRO / self.tick_rate()
    }

    fn target_slack_us(&self) -> u64 {
        u64::from(self.cfg.target_slack_milliticks) * MICRO / self.tick_rate() / 1000
    }

    /// One step of the client. `now_us` is this client's own clock
    /// (any monotonic microsecond counter); `input(tick)` returns the local
    /// player's input and commands for `tick`, and is called once per tick
    /// in order while playing.
    pub fn update(
        &mut self,
        now_us: u64,
        input: &mut dyn FnMut(u64) -> (G::Input, Vec<G::Command>),
    ) -> RelayUpdate<G> {
        let mut up = RelayUpdate::default();
        let dt = now_us.saturating_sub(self.now_us);
        self.now_us = now_us;
        self.src_mut().pump();
        while let Some(msg) = self.src_mut().others.pop_front() {
            self.handle(msg);
        }
        let (connected, disconnected) = {
            let s = self.src_mut();
            (std::mem::take(&mut s.connected), s.disconnected)
        };
        if connected && self.state == ClientState::Connecting {
            let hello = Hello {
                build_hash: orr_sim::build_hash_of(self.cfg.build_id, 0),
                room: self.cfg.room,
                input_size: std::mem::size_of::<G::Input>() as u32,
                want_slot: self.cfg.want_slot.map_or(NO_SLOT, |s| s.0),
                token: self.cfg.token,
            };
            self.src_mut().send(Channel::Reliable, &ClientMsg::Hello(hello));
            self.state = ClientState::Handshake;
        }
        if disconnected && !matches!(self.state, ClientState::Rejected(_) | ClientState::Failed(_)) {
            self.state = ClientState::Disconnected;
        }
        self.maintain_pings();
        self.try_start_join();
        if matches!(self.state, ClientState::Playing | ClientState::CatchingUp) {
            self.run_clock(dt, input, &mut up);
        }
        up
    }

    // ---- messages -----------------------------------------------------

    fn handle(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Welcome(w) => {
                if self.state != ClientState::Handshake {
                    return;
                }
                let (slot, count) = (w.slot, w.player_count);
                self.src_mut().configure(slot, count);
                self.state = if w.running { ClientState::AwaitSnapshot } else { ClientState::Syncing };
                if w.running {
                    self.t0_us = Some(w.t0_us);
                }
                self.events.push(ClientEvent::Welcomed { slot });
                self.welcome = Some(w);
            }
            ServerMsg::Reject(reason) => {
                self.state = ClientState::Rejected(reason);
                self.src_mut().link.close();
            }
            ServerMsg::Start { t0_us, server_time_us } => {
                if self.state == ClientState::Syncing {
                    self.t0_us = Some(t0_us);
                    let rtt = self.srtt_us;
                    self.add_offset_sample(rtt, server_time_us);
                    self.start_session(None);
                }
            }
            ServerMsg::Pong { client_time_us, server_time_us, t0_us, .. } => {
                if self.now_us >= client_time_us {
                    let rtt = self.now_us - client_time_us;
                    self.on_pong(rtt, server_time_us, t0_us);
                }
            }
            ServerMsg::TimeSync(ts) => {
                if self.state == ClientState::Playing {
                    self.on_time_sync(ts);
                }
            }
            ServerMsg::Desync { tick, reports } => self.on_desync(tick, reports),
            ServerMsg::SnapshotRequest { request_id } => self.serve_snapshot(request_id),
            ServerMsg::JoinSnapshot { tick, checksum, data } => {
                if self.state == ClientState::AwaitSnapshot && self.pending_snapshot.is_none() {
                    match crate::wire::decompress_bounded(&data) {
                        Ok(frame_bytes) => self.pending_snapshot = Some(PendingSnapshot { tick, checksum, frame_bytes }),
                        Err(e) => self.fail(format!("snapshot: {e}")),
                    }
                }
            }
            ServerMsg::Presence { slot, present, from_tick } => {
                self.events.push(ClientEvent::Presence { slot, present, from_tick });
                let mine = self.welcome.as_ref().map(|w| w.slot);
                if present && Some(slot) == mine && self.state == ClientState::CatchingUp {
                    self.state = ClientState::Playing;
                    self.submitting = true;
                    self.next_submit = self.send_clock / MICRO + 1;
                    self.events.push(ClientEvent::Started);
                }
            }
            ServerMsg::Bye { .. } => self.state = ClientState::Disconnected,
            ServerMsg::Confirmed { .. } => {}
        }
    }

    fn fail(&mut self, why: String) {
        self.src_mut().send(Channel::Reliable, &ClientMsg::Leave);
        self.src_mut().link.close();
        self.state = ClientState::Failed(why);
    }

    // ---- clock ----------------------------------------------------------

    fn add_offset_sample(&mut self, rtt_us: u64, server_time_us: u64) {
        let offset = server_time_us as i64 + (rtt_us / 2) as i64 - self.now_us as i64;
        self.samples.push_back((rtt_us, offset));
        while self.samples.len() > 8 {
            self.samples.pop_front();
        }
        // The sample with the smallest round trip is the least distorted.
        if let Some(&(_, off)) = self.samples.iter().min_by_key(|(r, _)| *r) {
            self.offset_us = off;
        }
    }

    fn on_pong(&mut self, rtt: u64, server_time_us: u64, t0_us: u64) {
        self.stats.pongs += 1;
        self.pongs += 1;
        if self.pongs == 1 {
            self.srtt_us = rtt;
            self.rttvar_us = rtt / 2;
        } else {
            let dev = self.srtt_us.abs_diff(rtt);
            self.rttvar_us = (self.rttvar_us * 3 + dev) / 4;
            self.srtt_us = (self.srtt_us * 7 + rtt) / 8;
        }
        self.add_offset_sample(rtt, server_time_us);
        if t0_us != u64::MAX && self.t0_us.is_none() {
            self.t0_us = Some(t0_us);
        }
        if self.state == ClientState::Playing {
            self.adapt_delay();
        }
    }

    fn maintain_pings(&mut self) {
        if matches!(self.state, ClientState::Connecting | ClientState::Handshake | ClientState::Rejected(_))
            || matches!(self.state, ClientState::Disconnected | ClientState::Failed(_))
        {
            return;
        }
        if self.now_us < self.next_ping_us {
            return;
        }
        let interval = if self.pongs < self.cfg.sync_pings { self.cfg.sync_ping_interval_us } else { self.cfg.ping_interval_us };
        self.next_ping_us = self.now_us + interval;
        self.ping_seq += 1;
        let msg = ClientMsg::Ping {
            seq: self.ping_seq,
            client_time_us: self.now_us,
            rtt_hint_us: self.srtt_us.min(u64::from(u32::MAX)) as u32,
        };
        self.src_mut().send(Channel::Unreliable, &msg);
        if self.state == ClientState::Syncing && self.pongs >= self.cfg.sync_pings && !self.sent_ready {
            self.sent_ready = true;
            self.src_mut().send(Channel::Reliable, &ClientMsg::Ready);
        }
    }

    /// Delay from the round trip (see the config docs): the smallest delay
    /// that keeps `2 * owd + slack - delay` within the prediction limit.
    fn desired_delay(&self) -> u32 {
        let tick_us = self.tick_us().max(1) as i64;
        let rtt_eff = (self.srtt_us + 2 * self.rttvar_us) as i64;
        let need = (rtt_eff + self.target_slack_us() as i64 + tick_us - 1) / tick_us
            - i64::from(self.cfg.max_prediction.saturating_sub(self.cfg.prediction_margin));
        need.clamp(i64::from(self.cfg.min_delay), i64::from(self.cfg.max_delay)) as u32
    }

    fn adapt_delay(&mut self) {
        let desired = self.desired_delay();
        let head = self.send_clock / MICRO;
        if desired > self.delay {
            self.set_delay(desired);
            self.below_since = None;
        } else if desired < self.delay {
            let since = *self.below_since.get_or_insert(head);
            if head.saturating_sub(since) >= u64::from(self.cfg.delay_decrease_ticks) {
                self.set_delay(desired);
                self.below_since = None;
            }
        } else {
            self.below_since = None;
        }
    }

    fn set_delay(&mut self, delay: u32) {
        self.delay = delay;
        self.stats.delay_changes += 1;
        self.events.push(ClientEvent::DelayChanged { delay });
    }

    fn on_time_sync(&mut self, ts: TimeSync) {
        if ts.samples == 0 {
            return;
        }
        self.stats.time_syncs += 1;
        let tick_us = self.tick_us().max(1) as i64;
        let err = i64::from(ts.min_slack_us) - self.target_slack_us() as i64;
        let limit = i64::from(self.cfg.max_rate_dev_ppm);
        let dev = (err * i64::from(self.cfg.rate_gain_ppm_per_tick) / tick_us).clamp(-limit, limit);
        self.rate_ppm = (MICRO as i64 - dev) as u32;
        // Far off (after a long stall or a clock jump): slewing at 2% would
        // take seconds, so jump once the error persists.
        if err.abs() > i64::from(self.cfg.hard_resync_ticks) * tick_us {
            self.hard_streak += 1;
            if self.hard_streak >= 2 {
                let shift = err * self.tick_rate() as i64; // microticks
                self.send_clock = (self.send_clock as i64 - shift).max(0) as u64;
                self.next_submit = self.next_submit.max(self.send_clock / MICRO + 1);
                self.hard_streak = 0;
                self.stats.hard_resyncs += 1;
                self.events.push(ClientEvent::HardResync { error_us: err });
            }
        } else {
            self.hard_streak = 0;
        }
    }

    // ---- session ------------------------------------------------------------

    fn session_config(&self, w: &Welcome) -> SessionConfig {
        let mut cfg = SessionConfig::new(w.player_count, PlayerSlot(w.slot), w.seed, w.tick_rate);
        cfg.build_id = self.cfg.build_id;
        cfg.max_prediction = self.cfg.max_prediction;
        cfg.checksum_interval = w.checksum_interval.max(1);
        cfg.input_delay = self.cfg.min_delay;
        cfg.relay = true;
        cfg.keep_anchors = self.cfg.keep_anchors;
        cfg
    }

    /// Starts the send clock at the server's tick plus one way trip plus the
    /// target slack, so inputs arrive with the slack to spare.
    fn init_clock(&mut self) {
        let w = self.welcome.as_ref().expect("welcomed");
        let tick_rate = u128::from(w.tick_rate);
        let server_time = (self.now_us as i64 + self.offset_us).max(0) as u64;
        let t0 = self.t0_us.unwrap_or(server_time);
        let server_micro = u128::from(server_time.saturating_sub(t0)) * tick_rate;
        let lead_us = self.srtt_us / 2 + self.target_slack_us();
        self.send_clock = (server_micro + u128::from(lead_us) * tick_rate) as u64;
        self.rate_ppm = MICRO as u32;
        self.delay = self.desired_delay();
        self.next_submit = self.send_clock / MICRO + 1;
    }

    fn start_session(&mut self, snapshot: Option<PendingSnapshot>) {
        let Some(w) = self.welcome.clone() else { return };
        let config = (self.make_config)(&w);
        let cfg = self.session_config(&w);
        let source = self.source.take().expect("source not yet in a session");
        match snapshot {
            None => {
                self.session = Some(Session::new(config, cfg, source));
                self.init_clock();
                self.submitting = true;
                self.state = ClientState::Playing;
                self.events.push(ClientEvent::Started);
            }
            Some(snap) => {
                let base = Anchor { tick: snap.tick, checksum: snap.checksum, frame_bytes: snap.frame_bytes.clone() };
                match Session::from_relay_snapshot(config, cfg, source, snap.tick, snap.checksum, &snap.frame_bytes) {
                    Ok(session) => {
                        self.session = Some(session);
                        self.src_mut().set_ack_floor(snap.tick);
                        self.base_anchor = Some(base);
                        self.init_clock();
                        self.submitting = false;
                        self.state = ClientState::CatchingUp;
                        self.events.push(ClientEvent::SnapshotLoaded { tick: snap.tick });
                    }
                    Err((e, source)) => {
                        self.source = Some(source);
                        self.fail(format!("snapshot: {e}"));
                    }
                }
            }
        }
    }

    /// A relayed snapshot waits for the first clock sample, so the send
    /// clock starts at the right tick.
    fn try_start_join(&mut self) {
        if self.state == ClientState::AwaitSnapshot && self.pongs >= 1 {
            if let Some(snap) = self.pending_snapshot.take() {
                self.start_session(Some(snap));
            }
        }
    }

    fn run_clock(
        &mut self,
        dt: u64,
        input: &mut dyn FnMut(u64) -> (G::Input, Vec<G::Command>),
        up: &mut RelayUpdate<G>,
    ) {
        let tick_rate = u128::from(self.tick_rate());
        let adv = (u128::from(dt) * tick_rate * u128::from(self.rate_ppm) / u128::from(MICRO)) as u64;
        self.send_clock += adv;
        let horizon = self.send_clock / MICRO;

        // Sample and send the inputs that are due.
        if self.submitting {
            while self.next_submit <= horizon {
                let (i, c) = input(self.next_submit);
                let tick = self.next_submit;
                self.next_submit += 1;
                self.session.as_mut().expect("session").submit_local(tick, i, c);
            }
            self.src_mut().flush();
        }

        // Simulate up to the tick the delay allows.
        let target = self.send_clock.saturating_sub(u64::from(self.delay) * MICRO) / MICRO;
        let cap = self.cfg.max_steps_per_update;
        let session = self.session.as_mut().expect("session");
        let (batch, rb) = session.poll_confirmed();
        if !batch.is_empty() {
            up.events.push(batch);
        }
        up.rollbacks.extend(rb);
        let mut stalled = false;
        let mut steps = 0u32;
        while session.head_tick() < target && steps < cap {
            match session.step() {
                AdvanceResult::Advanced { events, rollback, .. } => {
                    steps += 1;
                    if !events.is_empty() {
                        up.events.push(events);
                    }
                    if let Some(r) = rollback {
                        self.stats.resim_ticks += u64::from(r.resim_count);
                        up.rollbacks.push(r);
                    }
                }
                AdvanceResult::Stalled { events } => {
                    stalled = true;
                    if !events.is_empty() {
                        up.events.push(events);
                    }
                    break;
                }
            }
        }
        up.steps += steps;
        self.stats.steps += u64::from(steps);
        let depth = session.head_tick().saturating_sub(session.verified_tick());
        self.stats.max_prediction_depth = self.stats.max_prediction_depth.max(depth);
        if stalled {
            self.stats.stalled_us += dt;
            if !self.stalled_prev {
                self.stats.stall_episodes += 1;
            }
        }
        self.stalled_prev = stalled;
        let head = session.head_tick();

        // Late join: report ready once the session has caught up.
        if self.state == ClientState::CatchingUp && !self.sent_ready && head + 1 >= target {
            self.sent_ready = true;
            self.src_mut().send(Channel::Reliable, &ClientMsg::Ready);
        }

        // Checksums of newly verified ticks, for the server to compare.
        let session = self.session.as_ref().expect("session");
        let fresh: Vec<(u64, u64)> = session.checksums()[self.reported..].to_vec();
        self.reported += fresh.len();
        for (tick, checksum) in fresh {
            self.src_mut().send(Channel::Reliable, &ClientMsg::Checksum { tick, checksum });
        }

        // Keep the dump log as short as the anchors allow.
        let session = self.session.as_ref().expect("session");
        let floor = session
            .anchors()
            .front()
            .map(|a| a.tick)
            .unwrap_or_else(|| self.src().ack.saturating_sub(4 * u64::from(session.config().checksum_interval)));
        let floor = self.base_anchor.as_ref().map_or(floor, |b| floor.min(b.tick));
        let log = &mut self.src_mut().log;
        *log = log.split_off(&(floor + 1));
    }

    // ---- desync dump and snapshot donation ---------------------------------

    fn on_desync(&mut self, tick: u64, reports: Vec<(u8, u64)>) {
        self.stats.desyncs += 1;
        let (Some(session), Some(w)) = (self.session.as_ref(), self.welcome.as_ref()) else { return };
        // The newest verified frame from before the mismatching tick.
        let anchor = session
            .anchors()
            .iter()
            .rev()
            .chain(self.base_anchor.iter())
            .find(|a| a.tick < tick)
            .cloned();
        let (anchor_tick, anchor_checksum, anchor_frame) = match anchor {
            Some(a) => (a.tick, a.checksum, a.frame_bytes),
            None => (0, 0, Vec::new()),
        };
        let mut ticks = Vec::new();
        let mut expect = anchor_tick + 1;
        for (&t, b) in self.src().log.range(anchor_tick + 1..) {
            if t != expect {
                break;
            }
            ticks.push(b.clone());
            expect += 1;
        }
        let local_checksum = session.checksums().iter().find(|(t, _)| *t == tick).map_or(0, |&(_, c)| c);
        let dump = DesyncDump {
            build_hash: session.build_hash(),
            seed: w.seed,
            tick_rate: w.tick_rate,
            player_count: w.player_count,
            input_size: w.input_size,
            checksum_interval: w.checksum_interval,
            local_slot: w.slot,
            desync_tick: tick,
            local_checksum,
            reports,
            anchor_tick,
            anchor_checksum,
            anchor_frame,
            ticks,
        };
        let name = format!("desync_tick{tick}_slot{}.orrd", w.slot);
        self.dumps.write_dump(&name, dump.to_bytes());
        self.stats.dumps_written += 1;
        self.events.push(ClientEvent::Desync { tick, dump_name: name });
    }

    fn serve_snapshot(&mut self, request_id: u32) {
        let reply = match (&self.state, self.session.as_ref().and_then(|s| s.verified_frame().map(|f| (s.verified_tick(), f)))) {
            (ClientState::Playing, Some((tick, frame))) => {
                let data = lz4_flex::block::compress_prepend_size(&frame.to_bytes());
                Some(ClientMsg::SnapshotUpload { request_id, tick, checksum: frame.checksum(), data })
            }
            _ => None,
        };
        let msg = match reply {
            Some(m) => {
                self.stats.snapshots_served += 1;
                m
            }
            None => ClientMsg::SnapshotDecline { request_id },
        };
        self.src_mut().send(Channel::Reliable, &msg);
    }
}
