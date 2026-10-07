//! A finite, centrally coordinated planned departure over pinned loopback QUIC.
//! Input/backlog packets use real sockets; checked bootstrap and departure
//! controls are in-memory coordinator assertions, not a distributed protocol.
#![allow(clippy::disallowed_types)] // Failure deadlines belong to the socket fixture.

use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::thread;
use std::time::{Duration, Instant};

use orr_net::SendError;
use orr_proto::{Channel, ConnId, Endpoint, Link, LinkEvent, ServerEvent};
use orr_relay_net::p2p_mesh_input::{
    P2pMeshDelivery, P2pMeshEdge, P2pMeshInputDriver, P2pMeshInputLimits, P2pMeshInputSource,
};
use orr_relay_net::{
    connect, listen, ConnectOptions, ListenOptions, NetConfig, NetEndpoint, NetLink, P2pInputCodec,
    TransportKind, Trust,
};
use orr_session::{
    checked_backlog_notice, serve_checked_join, CheckedJoinContext, DepartureBarrier,
    DepartureFence, JoinBootstrap, JoinBootstrapStatus, JoinRoster, PlayerSlot, Session,
    SessionConfig,
};
use orr_sim::{Simulation, TickInputs, FP};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
type Driver = P2pMeshInputDriver<Arena, ArenaCodec>;
type Source = P2pMeshInputSource<Arena, ArenaCodec>;
type Peer = Session<Arena, Source>;
type Bootstrap = JoinBootstrap<Arena, Source>;
const PHASE_TIMEOUT: Duration = Duration::from_secs(30);
const OLD_GENERATION: u64 = 77;
const FRESH_GENERATION: u64 = 78;
const FIRST_COMMAND: u64 = 4;

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
fn roster() -> JoinRoster {
    JoinRoster::completed(
        3,
        PlayerSlot(2),
        PlayerSlot(0),
        vec![PlayerSlot(0), PlayerSlot(1)],
        &[],
    )
    .unwrap()
}
fn context() -> CheckedJoinContext {
    CheckedJoinContext::new(17, 1, roster()).unwrap()
}
fn game() -> ArenaConfig {
    ArenaConfig { player_count: 3 }
}
fn config(slot: u8) -> SessionConfig {
    let mut cfg = SessionConfig::new(3, PlayerSlot(slot), 42, 60);
    cfg.join_id = if slot == 2 { 17 } else { 0 };
    cfg.input_delay = 2;
    cfg.input_log_ticks = 512;
    cfg.checksum_interval = 1;
    cfg
}
fn limits() -> P2pMeshInputLimits {
    P2pMeshInputLimits {
        end_tick: 64,
        max_logical_records: 64,
        max_incoming_records: 16,
        max_destination_records: 64,
        max_edge_pending_records: 32,
        max_logical_encoded_bytes: 64 * 256,
        max_incoming_encoded_bytes: 16 * 256,
        max_pending_encoded_bytes: 64 * 256,
        max_edge_pending_encoded_bytes: 32 * 256,
        ..P2pMeshInputLimits::default()
    }
}
fn net_config() -> NetConfig {
    NetConfig {
        max_message_size: 64 * 1024,
        max_connections: 1,
        event_queue: 128,
        max_queued_send_bytes: 256 * 1024,
        worker_threads: 1,
        ..NetConfig::default()
    }
}
fn commands(slot: u8) -> Vec<SpawnBulletCmd> {
    [u32::from(slot), 2, u32::from(slot)]
        .map(|owner| SpawnBulletCmd { owner })
        .to_vec()
}
fn reference(tick: u64, cutoff: u64) -> Simulation<Arena> {
    let mut sim = Simulation::<Arena>::new(game(), 60, 42);
    for t in 1..=tick {
        let mut inputs = TickInputs::new(t, 3);
        if t >= FIRST_COMMAND {
            for slot in 0..3 {
                if slot != 2 || t < cutoff {
                    for command in commands(slot) {
                        inputs.push_command(PlayerSlot(slot), command);
                    }
                }
            }
        }
        sim.step(&inputs);
    }
    sim
}
fn assert_reference(peer: &Peer, cutoff: u64) {
    // Check every reached verified checkpoint, not a predicted head or just final equality.
    for &(tick, checksum) in peer.checksums() {
        assert_eq!(
            checksum,
            reference(tick, cutoff).frame().checksum(),
            "tick {tick}"
        );
    }
    assert_eq!(
        peer.verified_frame().unwrap().to_bytes(),
        reference(peer.verified_tick(), cutoff).frame().to_bytes()
    );
}
fn drain_head(peer: &mut Peer) {
    let target = peer.next_send_tick() - 1;
    assert!(target < 64, "finite fixture guard");
    for _ in 0..64 {
        if peer.head_tick() == target {
            break;
        }
        assert!(peer.head_tick() < target);
        peer.step();
    }
    assert_eq!(peer.head_tick(), target);
    peer.poll_confirmed();
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
    expected: BTreeMap<(PlayerSlot, PlayerSlot), VecDeque<Vec<u8>>>,
    issued: BTreeMap<(PlayerSlot, PlayerSlot), usize>,
    received: BTreeMap<(PlayerSlot, PlayerSlot), usize>,
}
impl Network {
    fn add_pair(&mut self, server: PlayerSlot, client: PlayerSlot) -> Result<()> {
        let mut listen_opts = ListenOptions::new("127.0.0.1:0".parse()?, TransportKind::Quic);
        listen_opts.net = net_config();
        listen_opts.sim = None;
        let mut endpoint = listen(&listen_opts)?;
        let address = endpoint.local_addr();
        let fingerprint = endpoint.cert_sha256().ok_or("listener has no pin")?;
        let mut connect_opts = ConnectOptions::new(
            address.to_string(),
            TransportKind::Quic,
            Trust::Fingerprint(fingerprint),
        );
        connect_opts.net = net_config();
        connect_opts.sim = None;
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
        assert_eq!(client_raw, ConnId(1));
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
        let port = self
            .ports
            .iter_mut()
            .find(|p| p.route.owner == owner && p.route.unique == conn)
            .ok_or(SendError::UnknownConnection)?;
        // No receive polling occurs inside this callback. Record only an actual
        // queue acceptance; Backpressure leaves both this inventory and driver head alone.
        port.send(bytes)?;
        let direction = (owner, port.route.remote);
        self.expected
            .entry(direction)
            .or_default()
            .push_back(bytes.to_vec());
        *self.issued.entry(direction).or_default() += 1;
        Ok(())
    }
}
impl Drop for Network {
    fn drop(&mut self) {
        for port in &mut self.ports {
            port.close();
        }
    }
}

impl Network {
    fn edge(&self, local: u8, remote: u8, generation: u64) -> P2pMeshEdge {
        let r = self.route(PlayerSlot(local), PlayerSlot(remote)).unwrap();
        P2pMeshEdge {
            connection: r.unique,
            adapter_id: r.adapter,
            raw_connection: r.raw,
            remote: r.remote,
            generation,
        }
    }
    fn inventory_empty(&self) -> bool {
        self.expected.values().all(VecDeque::is_empty) && self.issued == self.received
    }
    fn settle(&mut self, drivers: &[Driver], peers: &mut [Peer], cutoff: u64) {
        let started = Instant::now();
        loop {
            assert!(
                started.elapsed() < PHASE_TIMEOUT,
                "incomplete positive receipt inventory: issued={:?}, received={:?}",
                self.issued,
                self.received
            );
            for (slot, driver) in drivers.iter().enumerate() {
                driver.check().unwrap();
                driver
                    .flush(|conn, bytes| self.send(PlayerSlot(slot as u8), conn, bytes))
                    .unwrap();
            }
            // Poll all six handles even after departure. Any late old-route packet
            // fails rather than being silently discarded or routed to a fresh source.
            for i in 0..self.ports.len() {
                let route = self.ports[i].route;
                while let Some(event) = self.ports[i].poll() {
                    let ServerEvent::Message {
                        conn,
                        channel,
                        data,
                    } = event
                    else {
                        panic!("unexpected transport event: {event:?}");
                    };
                    assert_eq!(channel, Channel::Reliable);
                    let logical = self.resolve(route.adapter, conn).unwrap();
                    assert_eq!(logical, route.unique);
                    let driver = drivers
                        .get(usize::from(route.owner.0))
                        .expect("traffic reached departed peer");
                    assert_eq!(
                        driver.resolve_connection(route.adapter, conn).unwrap(),
                        logical
                    );
                    let direction = (route.remote, route.owner);
                    let queue = self.expected.get_mut(&direction).expect("unsent direction");
                    assert_eq!(queue.front().expect("unexpected extra packet"), &data);
                    driver.receive(logical, channel, &data).unwrap();
                    // A byte match plus successful public-driver admission is receipt.
                    queue.pop_front();
                    *self.received.entry(direction).or_default() += 1;
                    peers[usize::from(route.owner.0)].poll_confirmed();
                    driver.check().unwrap();
                    assert_reference(&peers[usize::from(route.owner.0)], cutoff);
                }
            }
            for (driver, peer) in drivers.iter().zip(peers.iter_mut()) {
                peer.poll_confirmed();
                driver.check().unwrap();
                assert_reference(peer, cutoff);
            }
            if self.inventory_empty()
                && drivers
                    .iter()
                    .all(|d| d.pending() == (0, 0) && d.incoming() == (0, 0))
            {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(self.inventory_empty());
        for d in drivers {
            assert_eq!(d.pending(), (0, 0));
            assert_eq!(d.incoming(), (0, 0));
        }
    }
}

#[test]
fn planned_departure_settles_real_quic_and_preserves_delayed_commands() -> Result<()> {
    let mut net = Network::default();
    for (server, client) in [(0, 1), (0, 2), (1, 2)] {
        net.add_pair(PlayerSlot(server), PlayerSlot(client))?;
    }
    assert_eq!(net.ports.len(), 6);
    assert_eq!(net.registry.len(), 6);
    let (a, sa) = Driver::new(PlayerSlot(0), context(), limits())?;
    let (b, sb) = Driver::new(PlayerSlot(1), context(), limits())?;
    let (c, sc) = Driver::new(PlayerSlot(2), context(), limits())?;
    let mut ca = config(0);
    ca.vacant_slots.push(PlayerSlot(2));
    let mut initial = [Peer::new(game(), ca, sa), Peer::new(game(), config(1), sb)];
    a.admit_edge(net.edge(0, 1, OLD_GENERATION))?;
    b.admit_edge(net.edge(1, 0, OLD_GENERATION))?;
    for peer in &mut initial {
        peer.advance(ArenaInput::default(), vec![]);
    }
    net.settle(&[a.clone(), b.clone()], &mut initial, u64::MAX);
    assert_eq!(initial[0].verified_tick(), 1);

    // Only checked setup controls are in-memory. Admission backfills and every
    // mesh input below traverse the same real socket/receipt-inventory path.
    let mut boot = Bootstrap::new(config(2), roster(), 4096, 1 << 20)?;
    let request = boot.next_request()?;
    let (snapshot, ticket) = serve_checked_join(&mut initial[0], &context(), &request)?;
    let notices = [
        checked_backlog_notice(&mut initial[0], &context(), &ticket)?,
        checked_backlog_notice(&mut initial[1], &context(), &ticket)?,
    ];
    a.install_ticket(&ticket)?;
    b.install_ticket(&ticket)?;
    boot.receive_snapshot(PlayerSlot(0), game(), sc, &snapshot)?;
    c.install_joiner_snapshot(&boot)?;
    for (slot, notice) in notices.iter().enumerate() {
        boot.receive_notice(PlayerSlot(slot as u8), notice)?;
    }
    assert_eq!(boot.status()?, JoinBootstrapStatus::Ready);
    let joiner = boot.cancel().ok_or("bootstrap did not yield session")?;
    let old = [a, b, c];
    for (local, driver) in old.iter().enumerate() {
        for remote in 0..3 {
            if usize::from(remote) != local && !(local < 2 && remote < 2) {
                driver.admit_edge(net.edge(local as u8, remote, OLD_GENERATION))?;
            }
        }
    }
    let [pa, pb] = initial;
    let mut peers = [pa, pb, joiner];
    net.settle(&old, &mut peers, u64::MAX);
    for peer in &mut peers {
        drain_head(peer);
        assert_eq!(peer.next_send_tick(), FIRST_COMMAND);
    }
    for _ in 0..4 {
        for (slot, peer) in peers.iter_mut().enumerate() {
            peer.advance(ArenaInput::default(), commands(slot as u8));
        }
        net.settle(&old, &mut peers, u64::MAX);
    }
    // Authoring is now frozen everywhere. Non-authoring steps consume the delay
    // pipeline, then positive byte inventories settle *all* issued frames.
    for peer in &mut peers {
        drain_head(peer);
    }
    net.settle(&old, &mut peers, u64::MAX);
    let target_tick = peers[0].verified_tick();
    let cutoff = target_tick + 1;
    assert_eq!(cutoff, FIRST_COMMAND + 4);
    for peer in &peers {
        assert_eq!(peer.head_tick(), target_tick);
        assert_eq!(peer.verified_tick(), target_tick);
        assert_eq!(peer.next_send_tick(), cutoff);
        assert_reference(peer, cutoff);
    }
    for local in 0..3 {
        for remote in 0..3 {
            if local != remote {
                let direction = (PlayerSlot(local), PlayerSlot(remote));
                assert!(net.issued[&direction] > 0);
                assert_eq!(net.issued[&direction], net.received[&direction]);
            }
        }
    }
    // Routing remains paused until BOTH survivor source swaps finish. All old
    // sources, including slot 2 and retained clones, remain quiesced thereafter.
    let summaries: Vec<_> = old
        .iter()
        .zip(&peers)
        .map(|(d, p)| d.quiesce(p).unwrap())
        .collect();
    let fences = std::array::from_fn(|i| DepartureFence {
        recovery_id: 91,
        revision: 2,
        survivor: summaries[i].local,
        verified_tick: summaries[i].verified_tick,
        departed_max: summaries[i].accepted_remote_max_by_slot[2],
    });
    let mut barrier = DepartureBarrier::new(
        91,
        2,
        PlayerSlot(2),
        PlayerSlot(0),
        vec![PlayerSlot(0), PlayerSlot(1)],
        &[],
        peers[0].config(),
    )?;
    for fence in fences {
        barrier.report_fence(fence)?;
    }
    assert_eq!(barrier.target()?.target, target_tick);
    assert_eq!(barrier.target()?.cutoff, cutoff);
    let acks = [
        barrier.acknowledgment(&peers[0])?,
        barrier.acknowledgment(&peers[1])?,
    ];
    for ack in acks {
        barrier.acknowledge(ack)?;
    }
    assert!(barrier.ready()?);
    let before = peers[0].verified_frame().unwrap().to_bytes();
    let result0 = old[0].continue_planned_departure(
        &mut peers[0],
        fences,
        acks,
        net.edge(0, 1, FRESH_GENERATION),
    )?;
    let result1 = old[1].continue_planned_departure(
        &mut peers[1],
        fences,
        acks,
        net.edge(1, 0, FRESH_GENERATION),
    )?;
    let fresh = [result0.fresh_driver, result1.fresh_driver];
    let _retired_sources = [result0.retired_source, result1.retired_source];
    for peer in &peers {
        assert_eq!(peer.verified_frame().unwrap().to_bytes(), before);
        assert_eq!(peer.head_tick(), target_tick);
        assert_eq!(peer.next_send_tick(), cutoff);
    }
    let departure_counts = net.received.clone();
    for _ in 0..12 {
        for (slot, peer) in peers[..2].iter_mut().enumerate() {
            peer.advance(ArenaInput::default(), commands(slot as u8));
        }
        net.settle(&fresh, &mut peers[..2], cutoff);
    }
    for peer in &mut peers[..2] {
        drain_head(peer);
    }
    net.settle(&fresh, &mut peers[..2], cutoff);
    for peer in &peers[..2] {
        assert_eq!(peer.next_send_tick(), cutoff + 12);
        assert_eq!(peer.head_tick(), cutoff + 11);
        assert_eq!(peer.verified_tick(), cutoff + 11);
        assert_reference(peer, cutoff);
    }
    // Public authored records establish exact command-free defaults, and public
    // destination accounting + matched real receipts establish their delivery.
    // No private ORRM offsets or second mesh parser are used by this fixture.
    let defaults: Vec<_> = peers[0]
        .authored_since(cutoff - 1)
        .into_iter()
        .filter(|r| r.slot == PlayerSlot(2))
        .collect();
    assert_eq!(defaults.len(), 12);
    for (i, record) in defaults.iter().enumerate() {
        assert_eq!(record.tick, cutoff + i as u64);
        assert_eq!(record.input, ArenaInput::default());
        assert!(record.commands.is_empty());
        assert!(!record.disconnected);
        assert_eq!(
            fresh[0].delivery(
                net.edge(0, 1, FRESH_GENERATION).connection,
                record.tick,
                PlayerSlot(2)
            ),
            Some(P2pMeshDelivery::QueueAccepted)
        );
    }
    assert!(peers[1]
        .authored_since(cutoff - 1)
        .iter()
        .all(|r| r.slot == PlayerSlot(1)));
    assert_eq!(
        net.received[&(PlayerSlot(0), PlayerSlot(1))]
            - departure_counts[&(PlayerSlot(0), PlayerSlot(1))],
        24
    );
    assert_eq!(
        net.received[&(PlayerSlot(1), PlayerSlot(0))]
            - departure_counts[&(PlayerSlot(1), PlayerSlot(0))],
        12
    );
    for (&direction, &count) in &departure_counts {
        if direction.0 == PlayerSlot(2) || direction.1 == PlayerSlot(2) {
            assert_eq!(net.received[&direction], count);
        }
    }
    for driver in &old {
        assert!(driver.is_quiesced());
        driver.check()?;
    }
    assert_eq!(peers[2].next_send_tick(), cutoff);
    assert_eq!(peers[2].head_tick(), target_tick);
    assert_eq!(peers[2].verified_frame().unwrap().to_bytes(), before);
    assert!(net.inventory_empty());
    // Drop closes the six handles only as cleanup, never as delivery evidence.
    Ok(())
}
