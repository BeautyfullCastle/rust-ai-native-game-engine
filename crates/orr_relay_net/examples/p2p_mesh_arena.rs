//! Fixed three-peer Arena over three real, pinned direct QUIC connections.
//! `cargo run -p orr_relay_net --release --example p2p_mesh_arena -- smoke [live_ticks]`
//! `cargo run -p orr_relay_net --release --example p2p_mesh_arena -- rolling [live_ticks] [prelude_ticks]`
//! Local orchestration supplies the trusted identities. There is no discovery,
//! production client authentication, relay forwarding, reconnection or roster shrink.

#![allow(clippy::disallowed_types)] // Wall clocks and sockets stay outside simulation.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::thread;
use std::time::{Duration, Instant};

use orr_net::SendError;
use orr_proto::{Channel, ConnId, Endpoint, Link, LinkEvent, ServerEvent};
use orr_relay_net::{
    connect, listen, ConnectOptions, ListenOptions, NetConfig, NetEndpoint, NetLink, P2pInputCodec,
    TransportKind, Trust,
};
use orr_session::{
    CheckedJoinContext, JoinBootstrap, JoinBootstrapStatus, JoinRoster, P2pAttempt, P2pMembership,
    P2pRoutedEvent, PlayerSlot, Session, SessionConfig,
};
use orr_sim::{Simulation, TickInputs, FP};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

// Driver imports are deliberately separate from the unchanged two-peer API.
use orr_relay_net::p2p_mesh_input::{
    P2pMeshDelivery, P2pMeshEdge, P2pMeshInputDriver, P2pMeshInputLimits, P2pMeshInputSource,
    P2pMeshRollingWindow,
};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
type Driver = P2pMeshInputDriver<Arena, ArenaCodec>;
type Source = P2pMeshInputSource<Arena, ArenaCodec>;
type Peer = Session<Arena, Source>;
type Bootstrap = JoinBootstrap<Arena, Source>;
const DONOR: PlayerSlot = PlayerSlot(0);
const EXISTING: PlayerSlot = PlayerSlot(1);
const JOINER: PlayerSlot = PlayerSlot(2);
const SLOTS: [PlayerSlot; 3] = [DONOR, EXISTING, JOINER];
const GENERATION: u64 = 0x4d45534800000001;
const BUILD_ID: u64 = 0x5032504d45534831;
const PRELUDE: u64 = 24;
const LIVE_TICKS: u64 = 120;
const END_TICK: u64 = 513; // Exclusive finite-mode limit.
const ROLLING_PRELUDE: u64 = 640;
const ROLLING_LIVE_TICKS: u64 = 2048;
const RECENT_TICKS: u64 = 4;
const FUTURE_TICKS: u64 = 32;
const AUTHOR_AHEAD: u64 = 8;
const SNAPSHOT_LIMIT: usize = 64 * 1024;
const NOTICE_LIMIT: usize = 4096;
const CONTROL_QUEUE_LIMIT: usize = 2 * SNAPSHOT_LIMIT + 2 * NOTICE_LIMIT;
const PHASE_TIMEOUT: Duration = Duration::from_secs(30);
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Finite,
    Rolling,
}

/// Arena mesh schema v1: i64 LE raw Q48.16 x/y, then u32 LE buttons.
/// Input is exactly 20 bytes; axes in [-1,1], FIRE is the only allowed bit.
/// Padding is reconstructed as zero. Commands are u32 LE owners in 0..=2.
struct ArenaCodec;
impl P2pInputCodec<Arena> for ArenaCodec {
    const SCHEMA: u64 = 0x4152454e41330001;
    fn encode_input(input: &ArenaInput, out: &mut Vec<u8>) {
        out.extend_from_slice(&input.axis_x.0.to_le_bytes());
        out.extend_from_slice(&input.axis_y.0.to_le_bytes());
        out.extend_from_slice(&input.buttons.to_le_bytes());
    }
    fn decode_input(bytes: &[u8]) -> Option<ArenaInput> {
        if bytes.len() != 20 {
            return None;
        }
        let x = i64::from_le_bytes(bytes[..8].try_into().ok()?);
        let y = i64::from_le_bytes(bytes[8..16].try_into().ok()?);
        let buttons = u32::from_le_bytes(bytes[16..].try_into().ok()?);
        if !(-65536..=65536).contains(&x) || !(-65536..=65536).contains(&y) || buttons > 1 {
            return None;
        }
        Some(ArenaInput {
            axis_x: FP::from_raw(x),
            axis_y: FP::from_raw(y),
            buttons,
            _pad: 0,
        })
    }
    fn encode_command(command: &SpawnBulletCmd, out: &mut Vec<u8>) {
        out.extend_from_slice(&command.owner.to_le_bytes());
    }
    fn decode_command(bytes: &[u8]) -> Option<SpawnBulletCmd> {
        let owner = u32::from_le_bytes(bytes.try_into().ok()?);
        (owner < 3).then_some(SpawnBulletCmd { owner })
    }
}
fn context() -> Result<CheckedJoinContext> {
    Ok(CheckedJoinContext::new(
        GENERATION,
        1,
        JoinRoster::completed(3, JOINER, DONOR, vec![DONOR, EXISTING], &[])?,
    )?)
}
fn game() -> ArenaConfig {
    ArenaConfig { player_count: 3 }
}
fn config(slot: PlayerSlot) -> SessionConfig {
    let mut cfg = SessionConfig::new(3, slot, 42, 60);
    cfg.join_id = GENERATION;
    cfg.build_id = BUILD_ID;
    cfg.checksum_interval = 1;
    cfg.max_prediction = 16;
    cfg.input_log_ticks = 512;
    if slot == DONOR {
        cfg.vacant_slots.push(JOINER);
    }
    cfg
}
fn limits(mode: Mode) -> P2pMeshInputLimits {
    let mut limits = P2pMeshInputLimits::default();
    if mode == Mode::Rolling {
        // Fixed encoded-storage caps, independent of requested run duration.
        limits.max_logical_records = 96;
        limits.max_incoming_records = 96;
        limits.max_destination_records = 128;
        limits.max_edge_pending_records = 64;
        limits.max_logical_encoded_bytes = 96 * 256;
        limits.max_incoming_encoded_bytes = 96 * 256;
        limits.max_pending_encoded_bytes = 128 * 256;
        limits.max_edge_pending_encoded_bytes = 64 * 256;
    }
    limits
}
fn net_config() -> NetConfig {
    NetConfig {
        max_message_size: SNAPSHOT_LIMIT,
        max_connections: 1,
        event_queue: 128,
        max_queued_send_bytes: 256 * 1024,
        worker_threads: 1,
        ..NetConfig::default()
    }
}
fn input(slot: PlayerSlot, tick: u64) -> ArenaInput {
    ArenaInput::new(
        FP::from_int(((tick % 3 + u64::from(slot.0)) % 3) as i32 - 1),
        FP::from_int(i32::from(slot.0) - 1),
        true,
    )
}
fn commands(slot: PlayerSlot) -> Vec<SpawnBulletCmd> {
    // Repetition and distinct owners make both command multiplicity and order observable.
    [slot.0, (slot.0 + 1) % 3, slot.0]
        .map(|owner| SpawnBulletCmd {
            owner: u32::from(owner),
        })
        .to_vec()
}

/// Each adapter owns a different namespace, even when every raw ID equals 1.
/// Mapping an ingress event requires BOTH the actual adapter and its raw ID.
#[derive(Clone, Copy, Debug)]
struct Route {
    owner: PlayerSlot,
    remote: PlayerSlot,
    adapter: u64,
    raw: ConnId,
    unique: ConnId,
}
enum Socket {
    Server(NetEndpoint),
    Client(NetLink),
}
struct Port {
    route: Route,
    socket: Socket,
}
impl Port {
    fn poll(&mut self) -> Option<ServerEvent> {
        match &mut self.socket {
            Socket::Server(endpoint) => endpoint.poll(),
            Socket::Client(link) => link.poll().map(|event| match event {
                LinkEvent::Connected => ServerEvent::Connected(self.route.raw),
                LinkEvent::Disconnected => ServerEvent::Disconnected(self.route.raw),
                LinkEvent::Message { channel, data } => ServerEvent::Message {
                    conn: self.route.raw,
                    channel,
                    data,
                },
            }),
        }
    }
    fn send(&mut self, bytes: &[u8]) -> std::result::Result<(), SendError> {
        match &mut self.socket {
            Socket::Server(endpoint) => endpoint.try_send_reliable(self.route.raw, bytes),
            Socket::Client(link) => link.try_send_reliable(bytes),
        }
    }
    fn close(&mut self) {
        match &mut self.socket {
            Socket::Server(endpoint) => endpoint.disconnect(self.route.raw),
            Socket::Client(link) => link.close(),
        }
    }
}
#[derive(Default)]
struct Network {
    ports: Vec<Port>,
    registry: BTreeMap<(u64, ConnId), ConnId>,
}
impl Network {
    fn add_pair(&mut self, server: PlayerSlot, client: PlayerSlot) -> Result<()> {
        let mut listen_opts = ListenOptions::new("127.0.0.1:0".parse()?, TransportKind::Quic);
        listen_opts.net = net_config();
        let mut endpoint = listen(&listen_opts)?;
        let address = endpoint.local_addr();
        let fingerprint = endpoint.cert_sha256().ok_or("listener has no pin")?;
        let mut connect_opts = ConnectOptions::new(
            address.to_string(),
            TransportKind::Quic,
            Trust::Fingerprint(fingerprint),
        );
        connect_opts.net = net_config();
        let mut link = connect(&connect_opts)?;
        let started = Instant::now();
        let mut accepted = None;
        let mut connected = false;
        while accepted.is_none() || !connected {
            if started.elapsed() > PHASE_TIMEOUT {
                return Err("pinned QUIC connection timed out".into());
            }
            if let Some(event) = endpoint.poll() {
                match event {
                    ServerEvent::Connected(raw) if accepted.is_none() => accepted = Some(raw),
                    _ => return Err(format!("unexpected connection event: {event:?}").into()),
                }
            }
            if let Some(event) = link.poll() {
                match event {
                    LinkEvent::Connected if !connected => connected = true,
                    _ => return Err(format!("unexpected link event: {event:?}").into()),
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        let server_raw = accepted.ok_or("missing accepted connection")?;
        let client_raw = link.connection_id(); // Always 1. Never a mesh-global identity.
        for (owner, remote, raw, socket) in [
            (server, client, server_raw, Socket::Server(endpoint)),
            (client, server, client_raw, Socket::Client(link)),
        ] {
            let adapter = self.ports.len() as u64 + 1;
            let unique = ConnId(u32::try_from(adapter + 100)?);
            if self.registry.insert((adapter, raw), unique).is_some()
                || self.ports.iter().any(|p| p.route.unique == unique)
            {
                return Err("non-injective adapter registry".into());
            }
            self.ports.push(Port {
                route: Route {
                    owner,
                    remote,
                    adapter,
                    raw,
                    unique,
                },
                socket,
            });
        }
        Ok(())
    }
    fn route(&self, owner: PlayerSlot, remote: PlayerSlot) -> Result<Route> {
        self.ports
            .iter()
            .find(|p| p.route.owner == owner && p.route.remote == remote)
            .map(|p| p.route)
            .ok_or_else(|| "missing fixed edge".into())
    }
    fn resolve(&self, adapter: u64, raw: ConnId) -> Result<ConnId> {
        self.registry
            .get(&(adapter, raw))
            .copied()
            .ok_or_else(|| "unknown adapter/raw connection pair".into())
    }
    fn send(
        &mut self,
        owner: PlayerSlot,
        conn: ConnId,
        bytes: &[u8],
    ) -> std::result::Result<(), SendError> {
        self.ports
            .iter_mut()
            .find(|p| p.route.owner == owner && p.route.unique == conn)
            .ok_or(SendError::UnknownConnection)?
            .send(bytes)
    }
}
impl Drop for Network {
    fn drop(&mut self) {
        for port in &mut self.ports {
            port.close();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Report {
    slot: PlayerSlot,
    tick: u64,
    checksum: u64,
}
// Example-only report: ORRF, version u8=1, author u8, generation u64 LE,
// attempt u32 LE, build ID u64 LE, verified tick u64 LE, checksum u64 LE. Exactly 42 bytes.
// Every peer requires reports from the same fixed three slots, including itself.
fn report_bytes(report: Report) -> Vec<u8> {
    let mut bytes = b"ORRF\x01".to_vec();
    bytes.push(report.slot.0);
    bytes.extend_from_slice(&GENERATION.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&BUILD_ID.to_le_bytes());
    bytes.extend_from_slice(&report.tick.to_le_bytes());
    bytes.extend_from_slice(&report.checksum.to_le_bytes());
    bytes
}
fn read_report(bytes: &[u8], sender: PlayerSlot) -> Result<Report> {
    if bytes.len() != 42
        || &bytes[..5] != b"ORRF\x01"
        || bytes[5] != sender.0
        || !SLOTS.contains(&sender)
        || u64::from_le_bytes(bytes[6..14].try_into()?) != GENERATION
        || u32::from_le_bytes(bytes[14..18].try_into()?) != 1
        || u64::from_le_bytes(bytes[18..26].try_into()?) != BUILD_ID
    {
        return Err("invalid fixed-mesh report".into());
    }
    Ok(Report {
        slot: sender,
        tick: u64::from_le_bytes(bytes[26..34].try_into()?),
        checksum: u64::from_le_bytes(bytes[34..42].try_into()?),
    })
}
fn require_reports(
    reports: &BTreeMap<PlayerSlot, Report>,
    target: u64,
    expected: u64,
) -> Result<()> {
    for slot in SLOTS {
        let report = reports.get(&slot).ok_or("missing fixed-peer report")?;
        if report.slot != slot || report.tick != target || report.checksum != expected {
            return Err("fixed-peer verified report mismatch".into());
        }
    }
    if reports.len() != 3 {
        return Err("unexpected report author".into());
    }
    Ok(())
}
fn reference(first_input: u64, target: u64) -> BTreeMap<u64, u64> {
    // Independently scripted from tick zero, never cloned from donor/joiner frames.
    let mut sim = Simulation::<Arena>::with_build_id(game(), 60, 42, BUILD_ID);
    let mut checksums = BTreeMap::from([(0, sim.frame().checksum())]);
    for tick in 1..=target {
        let mut inputs = TickInputs::new(tick, 3);
        for slot in SLOTS {
            if tick > 2 && (slot != JOINER || tick >= first_input) {
                inputs.set_input(slot, input(slot, tick));
                for command in commands(slot) {
                    inputs.push_command(slot, command);
                }
            }
        }
        sim.step(&inputs);
        checksums.insert(tick, sim.frame().checksum());
    }
    checksums
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Grant {
    snapshot: u64,
    first: u64,
    target: u64,
}
impl Grant {
    fn new(snapshot: u64, first: u64, live_ticks: u64, mode: Mode) -> Result<Self> {
        let target = first
            .checked_add(live_ticks.checked_sub(1).ok_or("empty live run")?)
            .ok_or("target overflow")?;
        // Reserve the input-delay tail, the Session send-cursor increment and
        // the rolling driver's checked future horizon before simulation work.
        target
            .checked_add(FUTURE_TICKS + 2)
            .ok_or("tick horizon overflow")?;
        // Session authors input_delay ticks ahead of its head, even on a stalled
        // advance. Check before advancing; never rely only on max_prediction.
        if mode == Mode::Finite && target.checked_add(2).is_none_or(|tick| tick >= END_TICK) {
            return Err("requested live run exceeds finite 512-tick input window".into());
        }
        Ok(Self {
            snapshot,
            first,
            target,
        })
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct DriverStats {
    evidence: (usize, usize),
    incoming: (usize, usize),
    pending: (usize, usize),
    edge_pending: (usize, usize),
    destinations: usize,
    floor: u64,
    floor_advances: u64,
}
fn high_water(high: &mut (usize, usize), now: (usize, usize)) {
    high.0 = high.0.max(now.0);
    high.1 = high.1.max(now.1);
}
fn check_usage(now: (usize, usize), records: usize, bytes: usize) -> Result<()> {
    if now.0 > records || now.1 > bytes {
        return Err("mesh driver exceeded its record/encoded-byte cap".into());
    }
    Ok(())
}
struct Node {
    slot: PlayerSlot,
    mode: Mode,
    input_connections: Vec<ConnId>,
    stats: DriverStats,
    driver: Driver,
    members: P2pMembership,
    attempt: Option<P2pAttempt>,
    peer: Option<Peer>,
    bootstrap: Option<Bootstrap>,
    source: Option<Source>,
    grant: Option<Grant>,
    ready: bool,
    checked: usize,
    reports: BTreeMap<PlayerSlot, Report>,
    sent_reports: BTreeSet<PlayerSlot>,
    direct_inputs: BTreeMap<PlayerSlot, usize>,
}
impl Node {
    fn new(slot: PlayerSlot, mode: Mode) -> Result<Self> {
        let (driver, source) = match mode {
            Mode::Finite => Driver::new(slot, context()?, limits(mode))?,
            Mode::Rolling => Driver::new_rolling(
                slot,
                context()?,
                limits(mode),
                P2pMeshRollingWindow {
                    recent_ticks: RECENT_TICKS,
                    future_ticks: FUTURE_TICKS,
                },
            )?,
        };
        let mut members = P2pMembership::new(3, slot, CONTROL_QUEUE_LIMIT)?;
        let (peer, bootstrap, source) = if slot == JOINER {
            assert!(members.commit_vacant(JOINER)?.is_empty());
            (
                None,
                Some(Bootstrap::new(
                    config(slot),
                    context()?.roster().clone(),
                    NOTICE_LIMIT,
                    SNAPSHOT_LIMIT,
                )?),
                Some(source),
            )
        } else {
            assert!(members.admit_local()?.is_empty());
            assert!(members.commit_vacant(JOINER)?.is_empty());
            driver.check()?;
            let peer = Session::new(game(), config(slot), source);
            driver.check()?;
            (Some(peer), None, None)
        };
        Ok(Self {
            slot,
            mode,
            input_connections: Vec::new(),
            stats: DriverStats::default(),
            driver,
            members,
            attempt: None,
            peer,
            bootstrap,
            source,
            grant: None,
            ready: false,
            checked: 0,
            reports: BTreeMap::new(),
            sent_reports: BTreeSet::new(),
            direct_inputs: BTreeMap::new(),
        })
    }
    fn session(&self) -> Option<&Peer> {
        self.peer
            .as_ref()
            .or_else(|| self.bootstrap.as_ref().and_then(Bootstrap::session))
    }
    fn admit_input(&mut self, route: Route) -> Result<()> {
        if route.owner != self.slot {
            return Err("edge belongs to another local adapter".into());
        }
        // Both endpoints derive this edge fence from the trusted local topology.
        let low = route.owner.0.min(route.remote.0);
        let high = route.owner.0.max(route.remote.0);
        self.driver.admit_edge(P2pMeshEdge {
            connection: route.unique,
            adapter_id: route.adapter,
            raw_connection: route.raw,
            remote: route.remote,
            generation: GENERATION + u64::from(low) * 3 + u64::from(high),
        })?;
        self.input_connections.push(route.unique);
        self.observe_driver()?;
        Ok(())
    }
    fn observe_driver(&mut self) -> Result<()> {
        self.driver.check()?;
        let cap = limits(self.mode);
        let evidence = self.driver.retained_evidence();
        let incoming = self.driver.incoming();
        let pending = self.driver.pending();
        let destinations = self.driver.destination_records();
        check_usage(
            evidence,
            cap.max_logical_records,
            cap.max_logical_encoded_bytes,
        )?;
        check_usage(
            incoming,
            cap.max_incoming_records,
            cap.max_incoming_encoded_bytes,
        )?;
        check_usage(
            pending,
            cap.max_destination_records,
            cap.max_pending_encoded_bytes,
        )?;
        if destinations > cap.max_destination_records {
            return Err("mesh driver exceeded destination cap".into());
        }
        high_water(&mut self.stats.evidence, evidence);
        high_water(&mut self.stats.incoming, incoming);
        high_water(&mut self.stats.pending, pending);
        self.stats.destinations = self.stats.destinations.max(destinations);
        for &conn in &self.input_connections {
            let edge = self.driver.edge_pending(conn)?;
            check_usage(
                edge,
                cap.max_edge_pending_records,
                cap.max_edge_pending_encoded_bytes,
            )?;
            high_water(&mut self.stats.edge_pending, edge);
        }
        if let Some((progress, floor)) = self.driver.rolling_progress() {
            if floor < self.stats.floor
                || floor > progress
                || self
                    .session()
                    .is_some_and(|peer| progress > peer.verified_tick())
            {
                return Err("rolling progress/floor is not local verified progress".into());
            }
            if floor > self.stats.floor {
                self.stats.floor_advances += 1;
                self.stats.floor = floor;
            }
        }
        Ok(())
    }
    fn poll_confirmed(&mut self) -> Result<()> {
        self.driver.check()?;
        if let Some(peer) = &mut self.peer {
            peer.poll_confirmed();
        } else if let Some(bootstrap) = &mut self.bootstrap {
            bootstrap.poll_confirmed();
        }
        self.observe_driver()?;
        Ok(())
    }
    fn advance_to(&mut self, target: u64) -> Result<()> {
        self.driver.check()?;
        let Some(peer) = self.session() else {
            return Ok(());
        };
        // Do not build a hidden bootstrap held-input queue while notices are absent.
        if self.slot == JOINER && !self.ready {
            return Ok(());
        }
        let tick = peer.next_send_tick();
        let author_limit = peer
            .verified_tick()
            .checked_add(AUTHOR_AHEAD)
            .ok_or("author-ahead bound overflow")?;
        if peer.head_tick() >= target || tick > author_limit {
            return Ok(());
        }
        if self.mode == Mode::Finite && tick >= END_TICK {
            return Err("finite input window exhausted before completion".into());
        }
        if let Some(peer) = &mut self.peer {
            peer.advance(input(self.slot, tick), commands(self.slot));
        } else if let Some(bootstrap) = &mut self.bootstrap {
            if bootstrap
                .advance(input(self.slot, tick), commands(self.slot))
                .is_none()
            {
                return Err("bootstrap cannot advance".into());
            }
        }
        self.observe_driver()?;
        Ok(())
    }
    fn check_reference(&mut self, reference: &BTreeMap<u64, u64>) -> Result<()> {
        self.driver.check()?;
        let Some(peer) = self.session() else {
            return Ok(());
        };
        let mut checked = 0;
        for &(tick, checksum) in peer.checksums() {
            if let Some(expected) = reference.get(&tick) {
                if checksum != *expected {
                    return Err(format!(
                        "peer {} reference checksum mismatch at tick {tick}",
                        self.slot.0
                    )
                    .into());
                }
                checked += 1;
            }
        }
        self.checked = checked;
        Ok(())
    }
    fn make_report(&mut self) -> Result<()> {
        let Some(grant) = self.grant else {
            return Ok(());
        };
        let peer = self.session().ok_or("missing report session")?;
        if peer.verified_tick() < grant.target || self.reports.contains_key(&self.slot) {
            return Ok(());
        }
        let checksum = peer
            .checksums()
            .iter()
            .find(|&&(tick, _)| tick == grant.target)
            .ok_or("missing verified target checksum")?
            .1;
        let report = Report {
            slot: self.slot,
            tick: grant.target,
            checksum,
        };
        self.reports.insert(self.slot, report);
        println!(
            "MESH_VERIFIED peer={} tick={} checksum={:016x} reference_checkpoints={}",
            self.slot.0, report.tick, report.checksum, self.checked
        );
        Ok(())
    }
}

fn admit_membership(nodes: &mut [Node], network: &Network, only_join_edges: bool) -> Result<()> {
    for port in &network.ports {
        let r = port.route;
        if (r.owner == JOINER || r.remote == JOINER) != only_join_edges {
            continue;
        }
        let node = &mut nodes[usize::from(r.owner.0)];
        let cleanup = if r.remote == JOINER {
            node.members.admit_joiner(r.remote, r.unique)?
        } else {
            node.members.admit_active(r.remote, r.unique)?
        };
        if !cleanup.is_empty() {
            return Err("unexpected membership invalidation during fixed admission".into());
        }
        if !only_join_edges {
            node.admit_input(r)?;
        }
    }
    Ok(())
}
fn begin_join(nodes: &mut [Node], network: &Network) -> Result<()> {
    admit_membership(nodes, network, true)?;
    for node in nodes.iter_mut() {
        let attempt = node.members.begin_attempt(JOINER, DONOR, GENERATION, 1)?;
        if attempt.context() != &context()? {
            return Err("fixed membership context mismatch".into());
        }
        node.attempt = Some(attempt);
    }
    let node = &mut nodes[2];
    node.driver.check()?;
    let request = node
        .bootstrap
        .as_mut()
        .ok_or("missing join bootstrap")?
        .next_request()?;
    node.driver.check()?;
    node.members.queue_control(
        node.attempt.as_ref().ok_or("missing attempt")?,
        DONOR,
        &request,
    )?;
    Ok(())
}
fn receive_control(
    node: &mut Node,
    conn: ConnId,
    data: &[u8],
    network: &Network,
    live_ticks: u64,
) -> Result<()> {
    node.driver.check()?;
    let attempt = node
        .attempt
        .as_ref()
        .ok_or("control outside checked attempt")?
        .clone();
    match &data[..4] {
        b"ORRQ" if node.slot == DONOR && data.len() <= NOTICE_LIMIT => {
            let peer = node.peer.as_mut().ok_or("donor session missing")?;
            let (snapshot, ticket) = node.members.serve_request(&attempt, conn, peer, data)?;
            node.driver.check()?;
            if snapshot.len() > SNAPSHOT_LIMIT {
                return Err("snapshot exceeds separate wire limit".into());
            }
            let before_ticket = node.driver.rolling_progress();
            node.driver.install_ticket(&ticket)?;
            if node.driver.rolling_progress() != before_ticket {
                return Err("checked ticket improperly advanced existing-peer progress".into());
            }
            let raw = ticket.ticket();
            node.grant = Some(Grant::new(
                raw.snapshot_tick,
                raw.first_input_tick,
                live_ticks,
                node.mode,
            )?);
            let notice = node.members.backlog_notice(&attempt, peer, &ticket)?;
            node.driver.check()?;
            node.admit_input(network.route(DONOR, JOINER)?)?;
            node.members.queue_control(&attempt, EXISTING, &snapshot)?;
            node.members.queue_control(&attempt, JOINER, &snapshot)?;
            node.members.queue_control(&attempt, JOINER, &notice)?;
            println!(
                "MESH_SNAPSHOT tick={} first_input={} active_peers=0,1 joiner=2",
                raw.snapshot_tick, raw.first_input_tick
            );
        }
        b"ORRJ" if node.slot == EXISTING && data.len() <= SNAPSHOT_LIMIT => {
            if node.grant.is_some() {
                return Err("duplicate donor snapshot ticket".into());
            }
            let peer = node.peer.as_mut().ok_or("existing session missing")?;
            let ticket = node
                .members
                .import_ticket(&attempt, conn, peer, data, SNAPSHOT_LIMIT)?;
            let before_ticket = node.driver.rolling_progress();
            node.driver.install_ticket(&ticket)?;
            if node.driver.rolling_progress() != before_ticket {
                return Err("checked ticket improperly advanced existing-peer progress".into());
            }
            let raw = ticket.ticket();
            node.grant = Some(Grant::new(
                raw.snapshot_tick,
                raw.first_input_tick,
                live_ticks,
                node.mode,
            )?);
            let notice = node.members.backlog_notice(&attempt, peer, &ticket)?;
            node.driver.check()?;
            node.admit_input(network.route(EXISTING, JOINER)?)?;
            node.members.queue_control(&attempt, JOINER, &notice)?;
        }
        b"ORRJ" if node.slot == JOINER && data.len() <= SNAPSHOT_LIMIT => {
            let bootstrap = node.bootstrap.as_mut().ok_or("missing join bootstrap")?;
            node.members.receive_snapshot(
                &attempt,
                conn,
                bootstrap,
                game(),
                node.source.take().ok_or("duplicate join snapshot")?,
                data,
            )?;
            node.driver.check()?;
            // Binding must be derived from this exact accepted bootstrap/source,
            // before any advance, source polling, or non-donor edge delivery.
            node.driver.install_joiner_snapshot(bootstrap)?;
            let peer = bootstrap.session().ok_or("missing accepted snapshot")?;
            if node
                .driver
                .rolling_progress()
                .is_some_and(|(progress, _)| progress != peer.verified_tick())
            {
                return Err("joiner rolling progress did not bind to its accepted snapshot".into());
            }
            node.grant = Some(Grant::new(
                peer.verified_tick(),
                peer.next_send_tick(),
                live_ticks,
                node.mode,
            )?);
            node.admit_input(network.route(JOINER, DONOR)?)?;
            node.admit_input(network.route(JOINER, EXISTING)?)?;
        }
        b"ORRB" if node.slot == JOINER && data.len() <= NOTICE_LIMIT => {
            let bootstrap = node.bootstrap.as_mut().ok_or("missing join bootstrap")?;
            node.members
                .receive_notice(&attempt, conn, bootstrap, data)?;
        }
        _ => return Err("unexpected or oversized checked mesh control".into()),
    }
    node.observe_driver()?;
    if let Some(bootstrap) = &node.bootstrap {
        let status = bootstrap.status()?;
        if matches!(status, JoinBootstrapStatus::InputGap { .. }) {
            return Err("checked join backlog has an input gap".into());
        }
        if status == JoinBootstrapStatus::Ready && !node.ready {
            if bootstrap.notice_usage().0 != 2 {
                return Err("Ready without both fixed peer notices".into());
            }
            let target = node.grant.ok_or("Ready without snapshot")?.target;
            if bootstrap.caught_up_to(target)? {
                return Err("unexpected already-caught-up bootstrap".into());
            }
            node.ready = true;
            println!("MESH_JOIN_READY notices=2 caught_up=false target={target}");
        }
    }
    Ok(())
}

fn receive_events(nodes: &mut [Node], network: &mut Network, live_ticks: u64) -> Result<()> {
    for index in 0..network.ports.len() {
        let route = network.ports[index].route;
        // No unbounded staging Vec. Until donor snapshot acceptance, the 1->2
        // edge stays in its bounded native transport event queue.
        if route.owner == JOINER && route.remote != DONOR && nodes[2].grant.is_none() {
            continue;
        }
        for _ in 0..128 {
            let Some(event) = network.ports[index].poll() else {
                break;
            };
            let node = &mut nodes[usize::from(route.owner.0)];
            node.driver.check()?;
            let event = match event {
                ServerEvent::Connected(_) => return Err("unexpected replacement connection".into()),
                ServerEvent::Disconnected(raw) => {
                    let conn = network.resolve(route.adapter, raw)?;
                    // Fixed membership failure is terminal. There is no recovery,
                    // survivor-count reduction, default takeover, or new admission.
                    let _ = node.driver.disconnected(conn);
                    return Err(format!(
                        "fixed mesh edge {}-{} disconnected",
                        route.owner.0, route.remote.0
                    )
                    .into());
                }
                ServerEvent::Message {
                    conn: raw,
                    channel,
                    data,
                } => {
                    let conn = network.resolve(route.adapter, raw)?;
                    if conn != route.unique || node.members.sender(conn)? != route.remote {
                        return Err("transport identity does not match admitted peer".into());
                    }
                    ServerEvent::Message {
                        conn,
                        channel,
                        data,
                    }
                }
            };
            match node.members.route_event(event) {
                P2pRoutedEvent::Control { conn, data } => {
                    receive_control(node, conn, &data, network, live_ticks)?;
                }
                P2pRoutedEvent::Forward(ServerEvent::Message {
                    conn,
                    channel,
                    data,
                }) => {
                    if channel != Channel::Reliable {
                        return Err("unreliable mesh traffic".into());
                    }
                    if data.starts_with(b"ORRM") {
                        let resolved = node.driver.resolve_connection(route.adapter, route.raw)?;
                        if resolved != conn {
                            return Err("input adapter mapping mismatch".into());
                        }
                        node.driver.receive(resolved, channel, &data)?;
                        *node.direct_inputs.entry(route.remote).or_default() += 1;
                    } else {
                        let report = read_report(&data, route.remote)?;
                        if let Some(old) = node.reports.insert(route.remote, report) {
                            if old != report {
                                return Err("conflicting fixed-peer report".into());
                            }
                        }
                    }
                }
                other => return Err(format!("unexpected membership event: {other:?}").into()),
            }
            node.observe_driver()?;
        }
    }
    Ok(())
}

#[derive(Clone)]
struct Options {
    mode: Mode,
    prelude_ticks: u64,
    live_ticks: u64,
    hold_peer_one_backlog: bool,
    drop_live_edge: bool,
    missing_report: Option<PlayerSlot>,
    report_timeout: Duration,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Finite,
            prelude_ticks: PRELUDE,
            live_ticks: LIVE_TICKS,
            hold_peer_one_backlog: false,
            drop_live_edge: false,
            missing_report: None,
            report_timeout: Duration::from_secs(5),
        }
    }
}
impl Options {
    fn rolling() -> Self {
        Self {
            mode: Mode::Rolling,
            prelude_ticks: ROLLING_PRELUDE,
            live_ticks: ROLLING_LIVE_TICKS,
            hold_peer_one_backlog: true,
            ..Self::default()
        }
    }
    fn validate(&self) -> Result<()> {
        match self.mode {
            Mode::Finite
                if !(LIVE_TICKS..=484).contains(&self.live_ticks)
                    || self.prelude_ticks != PRELUDE =>
            {
                return Err(
                    "live_ticks must be in 120..=484 within the finite 512-tick run".into(),
                );
            }
            Mode::Rolling
                if self.prelude_ticks < END_TICK || self.live_ticks < ROLLING_LIVE_TICKS =>
            {
                return Err("rolling requires a prelude >512 and at least 2048 live ticks".into());
            }
            _ => {}
        }
        let first = self
            .prelude_ticks
            .checked_add(3)
            .ok_or("prelude overflow")?;
        Grant::new(self.prelude_ticks, first, self.live_ticks, self.mode)?;
        usize::try_from(self.live_ticks)
            .map_err(|_| "live tick count cannot fit input counters")?;
        Ok(())
    }
}
/// A healthy long run has no total-duration deadline. Each phase must either
/// make local verified progress or finish its bounded join/report exchange.
struct ProgressWatch {
    ticks: [u64; 3],
    changed: Instant,
}
impl ProgressWatch {
    fn new() -> Self {
        Self {
            ticks: [0; 3],
            changed: Instant::now(),
        }
    }
    fn observe(&mut self, nodes: &[Node], phase: &str) -> Result<()> {
        let ticks = SLOTS.map(|slot| {
            nodes[usize::from(slot.0)]
                .session()
                .map_or(0, Peer::verified_tick)
        });
        if ticks != self.ticks {
            self.ticks = ticks;
            self.changed = Instant::now();
        } else if self.changed.elapsed() > STALL_TIMEOUT {
            return Err(format!("{phase} made no verified progress before stall timeout").into());
        }
        Ok(())
    }
}
#[derive(Debug)]
struct Outcome {
    reports: [Report; 3],
    first_input: u64,
    direct_one_to_two: usize,
    direct_two_to_one: usize,
    delayed_ready: bool,
    delayed_retirement: bool,
    hold_cleanups: usize,
    stats: [DriverStats; 3],
}
fn flush_nodes(nodes: &mut [Node], network: &mut Network, hold_peer_one: bool) -> Result<()> {
    for node in nodes {
        node.driver.check()?;
        let owner = node.slot;
        let progress = node.members.flush(|conn, channel, bytes| {
            assert_eq!(channel, Channel::Reliable);
            network.send(owner, conn, bytes)
        });
        for (_, error) in progress.blocked {
            if error != SendError::Backpressure {
                return Err(error.into());
            }
        }
        // Preserve checked snapshot/notice-before-input FIFO on each sender.
        if progress.remaining_bytes == 0 {
            let delayed = if owner == EXISTING && hold_peer_one {
                Some(network.route(EXISTING, JOINER)?.unique)
            } else {
                None
            };
            node.driver.flush(|conn, bytes| {
                if Some(conn) == delayed {
                    Err(SendError::Backpressure)
                } else {
                    network.send(owner, conn, bytes)
                }
            })?;
        }
        node.observe_driver()?;
    }
    Ok(())
}
fn send_reports(nodes: &mut [Node], network: &mut Network, options: &Options) -> Result<()> {
    for node in nodes {
        if node.driver.pending().0 != 0 || options.missing_report == Some(node.slot) {
            continue;
        }
        let Some(&report) = node.reports.get(&node.slot) else {
            continue;
        };
        for remote in SLOTS {
            if remote == node.slot || node.sent_reports.contains(&remote) {
                continue;
            }
            let conn = network.route(node.slot, remote)?.unique;
            match network.send(node.slot, conn, &report_bytes(report)) {
                Ok(()) => {
                    node.sent_reports.insert(remote);
                }
                Err(SendError::Backpressure) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}
fn release_and_promote(nodes: &mut [Node]) -> Result<usize> {
    let mut released = 0;
    for node in nodes {
        node.driver.check()?;
        if node.members.outbound_usage().0 != 0 || node.driver.pending().0 != 0 {
            return Err("promotion with unsent obligations".into());
        }
        // These three transport edges are room-shared from admission. They were
        // never claimed as exclusive join-owned endpoints; cleanup must preserve them.
        for cleanup in node.members.promote_joiner(JOINER)? {
            if !cleanup.exclusive_connections().is_empty() {
                return Err("room-shared edge unexpectedly join-owned".into());
            }
            if let Some(peer) = &mut node.peer {
                let _ = cleanup.release_local_hold(peer);
                released += 1;
                if peer.join_hold_lease(JOINER).is_some() {
                    return Err("exact checked join hold survived promotion".into());
                }
            }
        }
        node.attempt = None;
        node.driver.check()?;
        for remote in SLOTS {
            if remote != node.slot && node.members.connection(remote)?.is_none() {
                return Err("promotion removed a room-shared edge".into());
            }
        }
    }
    Ok(released)
}
fn run(options: Options) -> Result<Outcome> {
    options.validate()?;
    let mut nodes: Vec<Node> = SLOTS
        .into_iter()
        .map(|slot| Node::new(slot, options.mode))
        .collect::<Result<_>>()?;
    let mut network = Network::default();
    network.add_pair(DONOR, EXISTING)?;
    admit_membership(&mut nodes, &network, false)?;
    let mut progress = ProgressWatch::new();
    while nodes[..2]
        .iter()
        .any(|node| node.session().unwrap().verified_tick() < options.prelude_ticks)
    {
        receive_events(&mut nodes, &mut network, options.live_ticks)?;
        flush_nodes(&mut nodes, &mut network, false)?;
        for node in &mut nodes[..2] {
            node.poll_confirmed()?;
            node.advance_to(options.prelude_ticks)?;
        }
        progress.observe(&nodes, "two-active-peer prelude")?;
        thread::sleep(Duration::from_millis(1));
    }
    let prelude_reference = reference(u64::MAX, options.prelude_ticks);
    for node in &mut nodes[..2] {
        node.check_reference(&prelude_reference)?;
        if node.session().unwrap().head_tick() != options.prelude_ticks {
            return Err("prelude head was not bounded".into());
        }
        if options.mode == Mode::Rolling && node.stats.floor == 0 {
            return Err("rolling prelude did not retire any old evidence".into());
        }
    }
    println!(
        "MESH_PRELUDE active=0,1 vacant=2 verified={} mode={:?}",
        options.prelude_ticks, options.mode
    );
    network.add_pair(DONOR, JOINER)?;
    network.add_pair(EXISTING, JOINER)?;
    if network.ports.len() != 6 || network.registry.len() != 6 {
        return Err("expected exactly three direct QUIC pairs".into());
    }
    begin_join(&mut nodes, &network)?;
    let join_started = Instant::now();
    let mut progress = ProgressWatch::new();
    let mut delayed = options.hold_peer_one_backlog;
    let mut delayed_ready = false;
    let mut delayed_retirement = false;
    let mut dropped = false;
    let mut report_start = None;
    let mut baseline = None;
    loop {
        if !nodes[2].ready && join_started.elapsed() > PHASE_TIMEOUT {
            return Err("checked join phase timed out before both peer notices".into());
        }
        receive_events(&mut nodes, &mut network, options.live_ticks)?;
        flush_nodes(&mut nodes, &mut network, delayed)?;
        for node in &mut nodes {
            node.poll_confirmed()?;
            let target = node
                .grant
                .map_or(options.prelude_ticks, |grant| grant.target);
            node.advance_to(target)?;
        }
        progress.observe(&nodes, "fixed mesh live run")?;
        if baseline.is_none() {
            if let Some(grant) = nodes[0].grant {
                baseline = Some(reference(grant.first, grant.target));
            }
        }
        if let Some(reference) = &baseline {
            for node in &mut nodes {
                node.check_reference(reference)?;
                node.make_report()?;
            }
        }
        if delayed && nodes[2].ready {
            let first = nodes[2].grant.ok_or("Ready without grant")?.first;
            let donor_verified = nodes[0].session().unwrap().verified_tick();
            let joiner_verified = nodes[2].session().unwrap().verified_tick();
            let release_tick = first.checked_add(4).ok_or("delay probe overflow")?;
            let grant = nodes[2].grant.ok_or("Ready without grant")?;
            let backlog_tick = grant
                .snapshot
                .checked_add(1)
                .ok_or("backlog tick overflow")?;
            let crossed_retirement = nodes[1]
                .driver
                .rolling_progress()
                .is_some_and(|(_, floor)| floor >= backlog_tick);
            if donor_verified >= release_tick
                && joiner_verified < first
                && (options.mode == Mode::Finite || crossed_retirement)
            {
                // Ready only advertises coverage. Peer1's direct stream must
                // actually arrive before peer2 can verify these same ticks.
                if nodes[2].direct_inputs.get(&EXISTING).copied().unwrap_or(0) != 0 {
                    return Err("delayed edge leaked a peer1 input".into());
                }
                if options.mode == Mode::Rolling {
                    let direct = network.route(EXISTING, JOINER)?.unique;
                    let other = network.route(EXISTING, DONOR)?.unique;
                    if nodes[1].driver.delivery(direct, backlog_tick, EXISTING)
                        != Some(P2pMeshDelivery::Pending)
                        || nodes[1]
                            .driver
                            .delivery(other, backlog_tick, EXISTING)
                            .is_some()
                    {
                        return Err(
                            "retirement lost delayed backlog or retained old accepted metadata"
                                .into(),
                        );
                    }
                    delayed_retirement = true;
                    println!("MESH_RETIREMENT_DELAY_PROVED backlog_tick={backlog_tick} retired_through={} pending_1_to_2=true accepted_1_to_0_retired=true",
                        nodes[1].stats.floor);
                }
                delayed_ready = true;
                delayed = false;
                println!("MESH_DELAY_PROVED ready=true joiner_verified={joiner_verified} donor_verified={donor_verified}");
            }
        }
        if options.drop_live_edge
            && !dropped
            && nodes[2]
                .grant
                .and_then(|grant| grant.first.checked_add(4))
                .is_some_and(|probe_tick| {
                    nodes.iter().all(|node| {
                        node.session()
                            .is_some_and(|peer| peer.verified_tick() >= probe_tick)
                    })
                })
        {
            network
                .ports
                .iter_mut()
                .find(|p| p.route.owner == EXISTING && p.route.remote == JOINER)
                .ok_or("missing 1-2 edge")?
                .close();
            dropped = true;
        }
        send_reports(&mut nodes, &mut network, &options)?;
        let own_reports_ready = nodes
            .iter()
            .all(|node| node.reports.contains_key(&node.slot));
        if own_reports_ready {
            let since = report_start.get_or_insert_with(Instant::now);
            if since.elapsed() > options.report_timeout {
                return Err(
                    "missing fixed-peer report; expected all three slots, refusing roster shrink"
                        .into(),
                );
            }
        }
        if nodes
            .iter()
            .all(|node| node.reports.len() == 3 && node.sent_reports.len() == 2)
            && nodes.iter().all(|node| node.driver.pending().0 == 0)
        {
            let grant = nodes[0].grant.ok_or("missing donor grant")?;
            let expected = *baseline
                .as_ref()
                .and_then(|r| r.get(&grant.target))
                .ok_or("missing independent final checksum")?;
            for node in &nodes {
                if node.grant != Some(grant) {
                    return Err("peer cutoff disagreement".into());
                }
                require_reports(&node.reports, grant.target, expected)?;
            }
            if !nodes[2]
                .bootstrap
                .as_ref()
                .ok_or("missing bootstrap")?
                .caught_up_to(grant.target)?
            {
                return Err("Ready without verified catch-up".into());
            }
            let one_to_two = nodes[2].direct_inputs.get(&EXISTING).copied().unwrap_or(0);
            let two_to_one = nodes[1].direct_inputs.get(&JOINER).copied().unwrap_or(0);
            if one_to_two < options.live_ticks as usize || two_to_one < options.live_ticks as usize
            {
                return Err("live 1-2 direct input was not demonstrated".into());
            }
            if options.hold_peer_one_backlog && !delayed_ready {
                return Err("delay probe did not distinguish Ready from catch-up".into());
            }
            let reports = SLOTS.map(|slot| nodes[usize::from(slot.0)].reports[&slot]);
            let hold_cleanups = release_and_promote(&mut nodes)?;
            if hold_cleanups != 2 {
                return Err("missing existing-peer hold cleanup".into());
            }
            let stats = SLOTS.map(|slot| nodes[usize::from(slot.0)].stats);
            if options.mode == Mode::Rolling {
                if !delayed_retirement
                    || stats
                        .iter()
                        .any(|s| s.floor_advances < 100 || s.floor <= options.prelude_ticks)
                {
                    return Err(
                        "rolling run did not prove repeated retirement and delayed obligations"
                            .into(),
                    );
                }
                let cap = limits(Mode::Rolling);
                println!("MESH_DRIVER_CAPS logical_records={} logical_bytes={} incoming_records={} incoming_bytes={} destination_records={} pending_bytes={} edge_pending_records={} edge_pending_bytes={}",
                    cap.max_logical_records, cap.max_logical_encoded_bytes, cap.max_incoming_records,
                    cap.max_incoming_encoded_bytes, cap.max_destination_records, cap.max_pending_encoded_bytes,
                    cap.max_edge_pending_records, cap.max_edge_pending_encoded_bytes);
                for (slot, stats) in SLOTS.into_iter().zip(stats) {
                    println!("MESH_DRIVER_STATS peer={} retired_through={} floor_advances={} max_logical_records={} max_logical_bytes={} max_incoming_records={} max_incoming_bytes={} max_destination_records={} max_pending_records={} max_pending_bytes={} max_edge_pending_records={} max_edge_pending_bytes={}",
                        slot.0, stats.floor, stats.floor_advances, stats.evidence.0, stats.evidence.1,
                        stats.incoming.0, stats.incoming.1, stats.destinations, stats.pending.0,
                        stats.pending.1, stats.edge_pending.0, stats.edge_pending.1);
                }
            }
            return Ok(Outcome {
                reports,
                first_input: grant.first,
                direct_one_to_two: one_to_two,
                direct_two_to_one: two_to_one,
                delayed_ready,
                delayed_retirement,
                hold_cleanups,
                stats,
            });
        }
        thread::sleep(Duration::from_millis(1));
    }
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut options = match args.first().map(String::as_str) {
        None | Some("smoke") if args.len() <= 2 => Options::default(),
        Some("rolling") if args.len() <= 3 => Options::rolling(),
        _ => return Err("usage: p2p_mesh_arena [smoke [live_ticks (120..=484)] | rolling [live_ticks (>=2048)] [prelude_ticks (>512)]]".into()),
    };
    if let Some(ticks) = args.get(1) {
        options.live_ticks = ticks.parse()?;
    }
    if let Some(ticks) = args.get(2) {
        options.prelude_ticks = ticks.parse()?;
    }
    let result = run(options.clone())?;
    let label = if options.mode == Mode::Rolling {
        "MESH_ROLLING_OK"
    } else {
        "MESH_SMOKE_OK"
    };
    let min_retired = result
        .stats
        .iter()
        .map(|stats| stats.floor)
        .min()
        .unwrap_or(0);
    println!("{label} peers=3 direct_edges=3 tick={} checksum={:016x} live_ticks={} first_input={} input_1_to_2={} input_2_to_1={} hold_cleanups={} delayed_ready={} delayed_retirement={} min_retired_through={}",
        result.reports[0].tick, result.reports[0].checksum, options.live_ticks, result.first_input,
        result.direct_one_to_two, result.direct_two_to_one, result.hold_cleanups, result.delayed_ready,
        result.delayed_retirement, min_retired);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Keep actual socket scenarios independent of test-harness thread scheduling.
    static SOCKET_TEST: Mutex<()> = Mutex::new(());

    #[test]
    fn pinned_three_edge_quic_matches_independent_arena() {
        let _lock = SOCKET_TEST.lock().unwrap();
        let result = run(Options::default()).unwrap();
        assert_eq!(result.reports[0].tick, result.first_input + LIVE_TICKS - 1);
        assert!(result
            .reports
            .iter()
            .all(|r| r.tick == result.reports[0].tick && r.checksum == result.reports[0].checksum));
        assert!(result.direct_one_to_two >= LIVE_TICKS as usize);
        assert!(result.direct_two_to_one >= LIVE_TICKS as usize);
        assert_eq!(result.hold_cleanups, 2);
    }
    #[test]
    fn ready_waits_for_delayed_direct_peer_backlog() {
        let _lock = SOCKET_TEST.lock().unwrap();
        let result = run(Options {
            hold_peer_one_backlog: true,
            ..Options::default()
        })
        .unwrap();
        assert!(result.delayed_ready);
        assert_eq!(result.hold_cleanups, 2);
    }
    #[test]
    fn losing_one_live_edge_fails_the_fixed_mesh() {
        let _lock = SOCKET_TEST.lock().unwrap();
        let error = run(Options {
            drop_live_edge: true,
            ..Options::default()
        })
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("disconnected") || error.contains("UnknownConnection"),
            "{error}"
        );
    }
    #[test]
    fn missing_peer_report_cannot_shrink_completion_roster() {
        let _lock = SOCKET_TEST.lock().unwrap();
        let error = run(Options {
            missing_report: Some(JOINER),
            report_timeout: Duration::from_millis(150),
            ..Options::default()
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("missing fixed-peer report"), "{error}");
    }
    #[test]
    fn rolling_late_join_retains_delayed_backlog_across_retirement() {
        let _lock = SOCKET_TEST.lock().unwrap();
        let result = run(Options::rolling()).unwrap();
        assert!(result.first_input > END_TICK);
        assert_eq!(
            result.reports[0].tick,
            result.first_input + ROLLING_LIVE_TICKS - 1
        );
        assert!(result
            .reports
            .iter()
            .all(|report| report.tick == result.reports[0].tick
                && report.checksum == result.reports[0].checksum));
        assert!(result.direct_one_to_two >= ROLLING_LIVE_TICKS as usize);
        assert!(result.direct_two_to_one >= ROLLING_LIVE_TICKS as usize);
        assert!(result.delayed_ready && result.delayed_retirement);
        assert_eq!(result.hold_cleanups, 2);
        let caps = limits(Mode::Rolling);
        for stats in result.stats {
            assert!(stats.floor > ROLLING_PRELUDE && stats.floor_advances >= 100);
            assert!(stats.evidence.0 <= caps.max_logical_records);
            assert!(stats.destinations <= caps.max_destination_records);
            assert!(stats.edge_pending.0 <= caps.max_edge_pending_records);
        }
    }
    #[test]
    fn rolling_losing_one_live_edge_fails_the_fixed_mesh() {
        let _lock = SOCKET_TEST.lock().unwrap();
        let error = run(Options {
            drop_live_edge: true,
            ..Options::rolling()
        })
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("disconnected") || error.contains("UnknownConnection"),
            "{error}"
        );
    }
    #[test]
    fn rolling_missing_report_cannot_shrink_completion_roster() {
        let _lock = SOCKET_TEST.lock().unwrap();
        let error = run(Options {
            missing_report: Some(JOINER),
            report_timeout: Duration::from_millis(150),
            ..Options::rolling()
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("missing fixed-peer report"), "{error}");
    }
    #[test]
    fn mode_windows_and_tick_math_are_checked_before_starting() {
        assert!(Options::default().validate().is_ok());
        assert!(Options::rolling().validate().is_ok());
        assert!(Options {
            live_ticks: 485,
            ..Options::default()
        }
        .validate()
        .is_err());
        assert!(Options {
            prelude_ticks: 512,
            ..Options::rolling()
        }
        .validate()
        .is_err());
        assert!(Options {
            live_ticks: 2047,
            ..Options::rolling()
        }
        .validate()
        .is_err());
        assert!(Options {
            live_ticks: u64::MAX,
            ..Options::rolling()
        }
        .validate()
        .is_err());
        assert!(Options {
            prelude_ticks: u64::MAX,
            ..Options::rolling()
        }
        .validate()
        .is_err());
        assert!(Grant::new(24, 27, 0, Mode::Finite).is_err());
        assert!(Grant::new(u64::MAX - 10, u64::MAX - 7, 3, Mode::Rolling).is_err());
        assert!(Grant::new(u64::MAX - 39, u64::MAX - 36, 3, Mode::Rolling).is_ok());
        assert!(Grant::new(u64::MAX - 38, u64::MAX - 35, 3, Mode::Rolling).is_err());
        assert!(Options {
            prelude_ticks: 513,
            ..Options::rolling()
        }
        .validate()
        .is_ok());
    }
    #[test]
    fn report_identity_context_and_exact_length_are_checked() {
        let report = Report {
            slot: EXISTING,
            tick: 146,
            checksum: 7,
        };
        let bytes = report_bytes(report);
        assert_eq!(read_report(&bytes, EXISTING).unwrap(), report);
        assert!(read_report(&bytes, DONOR).is_err());
        for offset in [4, 5, 6, 14, 18] {
            let mut bad = bytes.clone();
            bad[offset] ^= 1;
            assert!(read_report(&bad, EXISTING).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(read_report(&trailing, EXISTING).is_err());
        assert!(read_report(&bytes[..bytes.len() - 1], EXISTING).is_err());
        let incomplete = BTreeMap::from([(EXISTING, report)]);
        assert!(require_reports(&incomplete, report.tick, report.checksum).is_err());
    }
    #[test]
    fn arena_codec_preserves_order_and_rejects_invalid_fields() {
        for slot in SLOTS {
            let original = input(slot, 17);
            let mut bytes = Vec::new();
            ArenaCodec::encode_input(&original, &mut bytes);
            assert_eq!(ArenaCodec::decode_input(&bytes), Some(original));
            bytes[16] = 2;
            assert!(ArenaCodec::decode_input(&bytes).is_none());
            let owners: Vec<_> = commands(slot)
                .iter()
                .map(|c| {
                    let mut bytes = Vec::new();
                    ArenaCodec::encode_command(c, &mut bytes);
                    ArenaCodec::decode_command(&bytes).unwrap().owner
                })
                .collect();
            assert_eq!(
                owners,
                vec![
                    u32::from(slot.0),
                    u32::from((slot.0 + 1) % 3),
                    u32::from(slot.0)
                ]
            );
        }
        assert!(ArenaCodec::decode_command(&3u32.to_le_bytes()).is_none());
    }
}
