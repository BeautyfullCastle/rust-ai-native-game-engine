//! Application-orchestrated lifecycle coverage, not a production P2P driver.
//! Real NetEndpoint and Session code runs over deterministic paired transports.
//! The Arena-only byte codec below is private test scaffolding, not a wire API.

use super::*;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use orr_session::{
    DepartureBarrier, DepartureError, DepartureFence, InputSource, JoinBootstrap,
    JoinBootstrapStatus as Status, JoinHoldRelease, LocallyVerifiedHistory, LocallyVerifiedRecord,
    LocallyVerifiedTick, P2pAttempt, P2pCleanup, P2pMembership, P2pRoutedEvent, P2pSlotState,
    PlayerSlot, RemoteInput, Session, SessionConfig, VerifiedHistoryLimits, VerifiedHistorySource,
};
use orr_sim::{decode_pod, encode_pod, SimCommand, Simulation, TickInputs, FP};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

const JOINER: PlayerSlot = PlayerSlot(2);
const DONOR: PlayerSlot = PlayerSlot(0);
const WIRE_LIMIT: usize = 1 << 20;
type Record = LocallyVerifiedRecord<ArenaInput>;
type Source = VerifiedHistorySource<Arena, ArenaSource>;
type Peer = Session<Arena, Source>;
type Bootstrap = JoinBootstrap<Arena, Source>;
type NodeRef = Rc<RefCell<Node>>;
type OrderedCommands = Vec<(PlayerSlot, Vec<u8>)>;

#[derive(Default)]
struct Wire {
    next: NetConnId,
    routes: BTreeMap<(usize, NetConnId), (usize, NetConnId)>,
    incoming: [VecDeque<Event>; 3],
    errors: BTreeMap<(usize, NetConnId), SendError>,
    held_routes: BTreeSet<(usize, NetConnId)>,
    held: Vec<(usize, Event)>,
    closed: Vec<(usize, NetConnId)>,
    accepted: Vec<(usize, NetConnId, orr_net::Channel, Vec<u8>)>,
}
struct PairedTransport {
    owner: usize,
    wire: Arc<Mutex<Wire>>,
}
impl Transport for PairedTransport {
    fn poll_event(&mut self) -> Option<Event> {
        self.wire.lock().unwrap().incoming[self.owner].pop_front()
    }
    fn send(
        &mut self,
        conn: NetConnId,
        channel: orr_net::Channel,
        bytes: &[u8],
    ) -> Result<(), SendError> {
        let mut wire = self.wire.lock().unwrap();
        let key = (self.owner, conn);
        let &(target, remote) = wire.routes.get(&key).ok_or(SendError::UnknownConnection)?;
        if let Some(error) = wire.errors.get(&key) {
            return Err(error.clone());
        }
        wire.accepted
            .push((self.owner, conn, channel, bytes.to_vec()));
        let event = Event::Message {
            conn: remote,
            channel,
            bytes: bytes.to_vec(),
        };
        if wire.held_routes.contains(&key) {
            wire.held.push((target, event));
        } else {
            wire.incoming[target].push_back(event);
        }
        Ok(())
    }
    fn close(&mut self, conn: NetConnId) {
        // Intentionally delayed close: the adapter must fence queued and late
        // packets immediately, without waiting for transport acknowledgment.
        self.wire.lock().unwrap().closed.push((self.owner, conn));
    }
    fn stats(&self, conn: NetConnId) -> Option<orr_net::ConnStats> {
        self.wire
            .lock()
            .unwrap()
            .routes
            .contains_key(&(self.owner, conn))
            .then_some(orr_net::ConnStats {
                max_unreliable_size: UNRELIABLE_MTU,
                ..Default::default()
            })
    }
}

struct Node {
    endpoint: NetEndpoint,
    members: P2pMembership,
    input_links: Vec<ConnId>,
    inputs: Vec<(ConnId, RemoteInput<Arena>)>,
    controls: VecDeque<(ConnId, Vec<u8>)>,
}
impl Node {
    fn connection(&self, slot: u8) -> ConnId {
        self.members.connection(PlayerSlot(slot)).unwrap().unwrap()
    }
    fn pump(&mut self) {
        while let Some(event) = self.endpoint.poll() {
            match self.members.route_event(event) {
                P2pRoutedEvent::Control { conn, data } => self.controls.push_back((conn, data)),
                P2pRoutedEvent::Forward(ServerEvent::Message {
                    conn,
                    channel,
                    data,
                }) => {
                    assert_eq!(channel, Channel::Reliable);
                    assert!(
                        self.input_links.contains(&conn),
                        "application-scoped input link"
                    );
                    self.inputs.push((conn, decode_input(&data)));
                }
                P2pRoutedEvent::Disconnected {
                    event: ServerEvent::Disconnected(conn),
                    cleanup,
                } => {
                    assert!(
                        cleanup.is_empty(),
                        "coordinate attempts before closing their links"
                    );
                    self.input_links.retain(|id| *id != conn);
                    self.inputs.retain(|(id, _)| *id != conn);
                    self.controls.retain(|(id, _)| *id != conn);
                }
                other => panic!("unexpected routed event: {other:?}"),
            }
        }
    }
    fn control(&mut self) -> (ConnId, Vec<u8>) {
        self.pump();
        self.controls.pop_front().expect("paired control delivery")
    }
    fn queue(&mut self, attempt: &P2pAttempt, target: u8, bytes: &[u8]) {
        self.members
            .queue_control(attempt, PlayerSlot(target), bytes)
            .unwrap();
    }
    fn flush(&mut self) -> orr_session::P2pFlush<SendError> {
        let Self {
            endpoint, members, ..
        } = self;
        members.flush(|conn, channel, bytes| {
            assert_eq!(channel, Channel::Reliable);
            endpoint.try_send_reliable(conn, bytes)
        })
    }
    fn transmit(&mut self, attempt: &P2pAttempt, target: u8, bytes: &[u8]) {
        self.queue(attempt, target, bytes);
        let result = self.flush();
        assert_eq!(result.queued, 1);
        assert!(result.blocked.is_empty());
        assert_eq!(result.remaining_bytes, 0);
    }
}

struct Rig {
    wire: Arc<Mutex<Wire>>,
    nodes: [NodeRef; 3],
}
impl Rig {
    fn new() -> Self {
        let wire = Arc::new(Mutex::new(Wire {
            next: 100,
            ..Wire::default()
        }));
        let nodes = std::array::from_fn(|owner| {
            Rc::new(RefCell::new(Node {
                endpoint: NetEndpoint::new(
                    Box::new(PairedTransport {
                        owner,
                        wire: wire.clone(),
                    }),
                    "127.0.0.1:0".parse().unwrap(),
                    None,
                ),
                members: P2pMembership::new(3, PlayerSlot(owner as u8), WIRE_LIMIT).unwrap(),
                input_links: Vec::new(),
                inputs: Vec::new(),
                controls: VecDeque::new(),
            }))
        });
        Self { wire, nodes }
    }
    fn connect(&self, left: usize, right: usize) -> [ConnId; 2] {
        let mut wire = self.wire.lock().unwrap();
        let ids = [wire.next, wire.next + 1];
        wire.next += 2;
        wire.routes.insert((left, ids[0]), (right, ids[1]));
        wire.routes.insert((right, ids[1]), (left, ids[0]));
        for (owner, conn) in [(left, ids[0]), (right, ids[1])] {
            wire.incoming[owner].push_back(Event::Connected { conn, peer: None });
        }
        drop(wire);
        [left, right].map(
            |owner| match self.nodes[owner].borrow_mut().endpoint.poll() {
                Some(ServerEvent::Connected(conn)) => conn,
                other => panic!("missing paired connection: {other:?}"),
            },
        )
    }
    fn admit_join_links(&self) {
        for peer in 0..2 {
            let [existing, joining] = self.connect(peer, 2);
            let mut node = self.nodes[peer].borrow_mut();
            assert!(node
                .members
                .admit_joiner(JOINER, existing)
                .unwrap()
                .is_empty());
            node.input_links.push(existing);
            let mut node = self.nodes[2].borrow_mut();
            assert!(node
                .members
                .admit_active(PlayerSlot(peer as u8), joining)
                .unwrap()
                .is_empty());
            node.input_links.push(joining);
        }
    }
    fn attempts(&self, generation: u64) -> [P2pAttempt; 3] {
        std::array::from_fn(|owner| {
            let mut node = self.nodes[owner].borrow_mut();
            let attempt = node
                .members
                .begin_attempt(JOINER, DONOR, generation, 1)
                .unwrap();
            let links: Vec<_> = if owner == 2 {
                node.input_links.clone()
            } else {
                vec![node.connection(2)]
            };
            for conn in links {
                let lease = node.endpoint.claim_exclusive_connection(conn).unwrap();
                node.members
                    .register_exclusive_connection(&attempt, lease)
                    .unwrap();
            }
            attempt
        })
    }
    fn key(&self, owner: usize, conn: ConnId) -> (usize, NetConnId) {
        (owner, self.nodes[owner].borrow().endpoint.to_net[&conn.0])
    }
    fn error(&self, owner: usize, conn: ConnId, error: Option<SendError>) {
        let key = self.key(owner, conn);
        let mut wire = self.wire.lock().unwrap();
        if let Some(error) = error {
            wire.errors.insert(key, error);
        } else {
            wire.errors.remove(&key);
        }
    }
    fn pump(&self) {
        for node in &self.nodes {
            node.borrow_mut().pump();
        }
    }
}

// Fixed-width, Arena-only, trusted-fixture encoding. No generic production input
// codec, authentication, version negotiation, or repair protocol is added here.
fn encode_input(
    tick: u64,
    slot: PlayerSlot,
    input: ArenaInput,
    commands: &[SpawnBulletCmd],
) -> Vec<u8> {
    let mut out = b"TEST".to_vec();
    out.extend_from_slice(&tick.to_le_bytes());
    out.push(slot.0);
    encode_pod(&input, &mut out);
    out.extend_from_slice(&(commands.len() as u32).to_le_bytes());
    for command in commands {
        command.encode(&mut out);
    }
    out
}
fn decode_input(bytes: &[u8]) -> RemoteInput<Arena> {
    assert_eq!(&bytes[..4], b"TEST");
    let tick = u64::from_le_bytes(bytes[4..12].try_into().unwrap());
    let slot = PlayerSlot(bytes[12]);
    let end = 13 + std::mem::size_of::<ArenaInput>();
    let input = decode_pod(&bytes[13..end]).unwrap();
    let count = u32::from_le_bytes(bytes[end..end + 4].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), end + 4 + count * 4);
    let commands = bytes[end + 4..]
        .chunks_exact(4)
        .map(|b| SpawnBulletCmd::decode(b).unwrap())
        .collect();
    RemoteInput {
        tick,
        slot,
        input,
        commands,
        disconnected: false,
    }
}
fn record(tick: u64, slot: PlayerSlot, input: ArenaInput, commands: &[SpawnBulletCmd]) -> Record {
    Record {
        tick,
        slot,
        input,
        commands: commands
            .iter()
            .map(|command| {
                let mut bytes = Vec::new();
                command.encode(&mut bytes);
                bytes
            })
            .collect(),
        disconnected: false,
    }
}
struct ArenaSource {
    node: NodeRef,
    sent: Vec<Record>,
    ordered: BTreeMap<u64, OrderedCommands>,
}
impl InputSource<Arena> for ArenaSource {
    fn send_local(
        &mut self,
        tick: u64,
        slot: PlayerSlot,
        input: ArenaInput,
        commands: Vec<SpawnBulletCmd>,
    ) {
        self.sent.push(record(tick, slot, input, &commands));
        let bytes = encode_input(tick, slot, input, &commands);
        let mut node = self.node.borrow_mut();
        for conn in node.input_links.clone() {
            node.endpoint.try_send_reliable(conn, &bytes).unwrap();
        }
    }
    fn poll_remote(&mut self) -> Vec<RemoteInput<Arena>> {
        let mut node = self.node.borrow_mut();
        node.pump();
        std::mem::take(&mut node.inputs)
            .into_iter()
            .map(|(_, record)| record)
            .collect()
    }
    fn on_locally_verified(&mut self, tick: LocallyVerifiedTick<'_, Arena>) {
        assert!(self
            .ordered
            .insert(tick.simulated.tick(), tick.simulated_commands.to_vec())
            .is_none());
    }
}
fn source(node: &NodeRef, generation: u64) -> Source {
    VerifiedHistorySource::new(
        ArenaSource {
            node: node.clone(),
            sent: Vec::new(),
            ordered: BTreeMap::new(),
        },
        LocallyVerifiedHistory::new(
            generation,
            3,
            VerifiedHistoryLimits {
                max_records: 3000,
                max_encoded_bytes: WIRE_LIMIT,
            },
        )
        .unwrap(),
    )
}
fn config(slot: u8, generation: u64) -> SessionConfig {
    let mut config = SessionConfig::new(3, PlayerSlot(slot), 42, 60);
    config.join_id = generation;
    config.input_delay = 0;
    config.max_prediction = 64;
    config.input_log_ticks = 2;
    config.checksum_interval = 1;
    config
}
fn game() -> ArenaConfig {
    ArenaConfig { player_count: 3 }
}
fn input(slot: u8, tick: u64) -> ArenaInput {
    ArenaInput::new(
        FP::from_int((tick % 3) as i32 - 1),
        FP::from_int(i32::from(slot) - 1),
        tick % 2 == 0,
    )
}
fn commands(slot: u8, tick: u64) -> Vec<SpawnBulletCmd> {
    if tick % 4 != 0 {
        return Vec::new();
    }
    // Order and duplicate preservation are observable, including RNG use.
    [slot, (slot + 1) % 3, slot]
        .map(|owner| SpawnBulletCmd {
            owner: u32::from(owner),
        })
        .to_vec()
}
fn drive(peer: &mut Peer) {
    let tick = peer.next_send_tick();
    let slot = peer.config().local_slot.0;
    peer.advance(input(slot, tick), commands(slot, tick));
}
fn round(a: &mut Peer, b: &mut Peer, join: Option<&mut Bootstrap>) {
    drive(a);
    drive(b);
    if let Some(join) = join {
        let tick = join.session().unwrap().next_send_tick();
        join.advance(input(2, tick), commands(2, tick)).unwrap();
        a.poll_confirmed();
        b.poll_confirmed();
        join.poll_confirmed().unwrap();
    } else {
        a.poll_confirmed();
        b.poll_confirmed();
    }
}
fn bootstrap(node: &NodeRef, generation: u64) -> Bootstrap {
    Bootstrap::new(
        config(2, generation),
        node.borrow().members.roster(JOINER, DONOR).unwrap(),
        4096,
        WIRE_LIMIT,
    )
    .unwrap()
}

struct Exchange {
    snapshot: Vec<u8>,
    notices: [Vec<u8>; 2],
    snapshot_tick: u64,
    first_input_tick: u64,
}
fn exchange(
    rig: &Rig,
    attempts: &[P2pAttempt; 3],
    join: &mut Bootstrap,
    a: &mut Peer,
    b: &mut Peer,
) -> Exchange {
    let request = join.next_request().unwrap();
    rig.nodes[2]
        .borrow_mut()
        .transmit(&attempts[2], 0, &request);
    let (conn, request) = rig.nodes[0].borrow_mut().control();
    let (snapshot, ticket) = rig.nodes[0]
        .borrow_mut()
        .members
        .serve_request(&attempts[0], conn, a, &request)
        .unwrap();
    rig.nodes[0]
        .borrow_mut()
        .transmit(&attempts[0], 1, &snapshot);
    let (conn, peer_snapshot) = rig.nodes[1].borrow_mut().control();
    let peer_ticket = rig.nodes[1]
        .borrow()
        .members
        .import_ticket(&attempts[1], conn, b, &peer_snapshot, WIRE_LIMIT)
        .unwrap();
    let notices = [
        rig.nodes[0]
            .borrow_mut()
            .members
            .backlog_notice(&attempts[0], a, &ticket)
            .unwrap(),
        rig.nodes[1]
            .borrow_mut()
            .members
            .backlog_notice(&attempts[1], b, &peer_ticket)
            .unwrap(),
    ];
    for (owner, peer) in [(0, a), (1, b)] {
        let conn = rig.nodes[owner].borrow().connection(2);
        for packet in peer.authored_since(ticket.ticket().snapshot_tick) {
            rig.nodes[owner]
                .borrow_mut()
                .endpoint
                .try_send_reliable(
                    conn,
                    &encode_input(packet.tick, packet.slot, packet.input, &packet.commands),
                )
                .unwrap();
        }
    }
    rig.nodes[0]
        .borrow_mut()
        .transmit(&attempts[0], 2, &snapshot);
    let (conn, received) = rig.nodes[2].borrow_mut().control();
    assert_eq!(
        rig.nodes[2]
            .borrow()
            .members
            .receive_snapshot(
                &attempts[2],
                conn,
                join,
                game(),
                source(&rig.nodes[2], ticket.context().join_id()),
                &received
            )
            .unwrap(),
        Status::Syncing {
            received: 0,
            expected: 2
        }
    );
    Exchange {
        snapshot,
        notices,
        snapshot_tick: ticket.ticket().snapshot_tick,
        first_input_tick: ticket.ticket().first_input_tick,
    }
}
fn notice(
    rig: &Rig,
    attempts: &[P2pAttempt; 3],
    join: &mut Bootstrap,
    exchange: &Exchange,
    owner: usize,
) -> Status {
    rig.nodes[owner]
        .borrow_mut()
        .transmit(&attempts[owner], 2, &exchange.notices[owner]);
    let (conn, data) = rig.nodes[2].borrow_mut().control();
    rig.nodes[2]
        .borrow()
        .members
        .receive_notice(&attempts[2], conn, join, &data)
        .unwrap()
}

#[test]
fn reliable_enqueue_reports_acceptance_and_preserves_legacy_send() {
    let rig = Rig::new();
    let [conn, remote] = rig.connect(0, 1);
    for error in [
        SendError::Backpressure,
        SendError::TooLarge { max: 3 },
        SendError::UnknownConnection,
    ] {
        rig.error(0, conn, Some(error.clone()));
        assert_eq!(
            rig.nodes[0]
                .borrow_mut()
                .endpoint
                .try_send_reliable(conn, b"complete message"),
            Err(error)
        );
        // The existing trait remains fire-and-forget, without a panic or retry.
        rig.nodes[0]
            .borrow_mut()
            .endpoint
            .send(conn, Channel::Reliable, b"legacy");
        assert!(rig.wire.lock().unwrap().accepted.is_empty());
    }
    rig.error(0, conn, None);
    rig.nodes[0]
        .borrow_mut()
        .endpoint
        .try_send_reliable(conn, b"accepted")
        .unwrap();
    assert!(
        matches!(rig.nodes[1].borrow_mut().endpoint.poll(), Some(ServerEvent::Message { conn, channel: Channel::Reliable, data }) if conn == remote && data == b"accepted")
    );
    let lease = rig.nodes[0]
        .borrow_mut()
        .endpoint
        .claim_exclusive_connection(conn)
        .unwrap();
    assert_eq!(
        rig.nodes[0]
            .borrow_mut()
            .endpoint
            .retire_exclusive_connection(&lease),
        P2pConnectionRetirement::Retired
    );
    assert_eq!(
        rig.nodes[0]
            .borrow_mut()
            .endpoint
            .try_send_reliable(conn, b"retired"),
        Err(SendError::UnknownConnection)
    );
    assert_eq!(
        rig.nodes[0]
            .borrow_mut()
            .endpoint
            .try_send_reliable(ConnId(999), b"unknown"),
        Err(SendError::UnknownConnection)
    );
    assert_eq!(rig.wire.lock().unwrap().accepted.len(), 1);
}

fn cancel(rig: &Rig, attempts: &[P2pAttempt; 3]) -> [P2pCleanup; 3] {
    std::array::from_fn(|owner| {
        rig.nodes[owner]
            .borrow_mut()
            .members
            .cancel_attempt(&attempts[owner])
            .unwrap()
    })
}
fn fence_current_links(rig: &Rig) {
    for owner in 0..3 {
        let mut node = rig.nodes[owner].borrow_mut();
        let links = if owner == 2 {
            node.input_links.clone()
        } else {
            vec![node.connection(2)]
        };
        for conn in links {
            // After coordinated departure these gameplay links are exclusive
            // to the departed participant; this is a new explicit lease.
            let lease = node.endpoint.claim_exclusive_connection(conn).unwrap();
            assert_eq!(
                node.endpoint.retire_exclusive_connection(&lease),
                P2pConnectionRetirement::Retired
            );
            assert_eq!(
                node.endpoint.try_send_reliable(conn, b"fenced"),
                Err(SendError::UnknownConnection)
            );
        }
    }
    rig.pump();
}

fn assert_reference(
    peers: &[(&Peer, u64)],
    joining_from: u64,
    departed_through: u64,
    through: u64,
) {
    let mut reference = Simulation::<Arena>::new(game(), 60, 42);
    let mut checkpoints = BTreeMap::new();
    let mut records = BTreeMap::new();
    let mut ordered = BTreeMap::new();
    for tick in 1..=through {
        let mut inputs = TickInputs::new(tick, 3);
        let mut command_order = Vec::new();
        for slot in 0..3 {
            let active = slot != 2 || (joining_from..=departed_through).contains(&tick);
            let sample = if active {
                input(slot, tick)
            } else {
                ArenaInput::default()
            };
            let commands = if active {
                commands(slot, tick)
            } else {
                Vec::new()
            };
            inputs.set_input(PlayerSlot(slot), sample);
            let expected = record(tick, PlayerSlot(slot), sample, &commands);
            for bytes in &expected.commands {
                command_order.push((PlayerSlot(slot), bytes.clone()));
            }
            for command in commands {
                inputs.push_command(PlayerSlot(slot), command);
            }
            records.insert((tick, PlayerSlot(slot)), expected);
        }
        ordered.insert(tick, command_order);
        reference.step(&inputs);
        checkpoints.insert(tick, reference.checksum());
    }
    for &(peer, first) in peers {
        let last = peer.verified_tick();
        assert!(last >= first);
        // Complete histories, not just a final checksum or aggregate command count.
        for slot in (0..3).map(PlayerSlot) {
            let actual = peer
                .source()
                .history()
                .export_range(slot, first, last)
                .unwrap();
            let expected: Vec<_> = (first..=last)
                .map(|tick| records[&(tick, slot)].clone())
                .collect();
            assert_eq!(
                actual,
                expected,
                "slot {slot:?}, observer {:?}",
                peer.config().local_slot
            );
        }
        assert_eq!(
            peer.source().inner().ordered,
            ordered
                .range(first..=last)
                .map(|(&tick, commands)| (tick, commands.clone()))
                .collect()
        );
        let actual: Vec<_> = peer
            .checksums()
            .iter()
            .copied()
            .filter(|(tick, _)| *tick >= first)
            .collect();
        let expected: Vec<_> = checkpoints
            .range(first..=last)
            .map(|(&tick, &checksum)| (tick, checksum))
            .collect();
        assert_eq!(
            actual,
            expected,
            "every common checkpoint for {:?}",
            peer.config().local_slot
        );
    }
}

fn room() -> (Rig, Peer, Peer) {
    let rig = Rig::new();
    let [ab, ba] = rig.connect(0, 1);
    for (owner, conn) in [(0, ab), (1, ba)] {
        let mut node = rig.nodes[owner].borrow_mut();
        assert!(node.members.admit_local().unwrap().is_empty());
        assert!(node
            .members
            .admit_active(PlayerSlot((1 - owner) as u8), conn)
            .unwrap()
            .is_empty());
        node.input_links.push(conn);
    }
    for node in &rig.nodes {
        assert!(node
            .borrow_mut()
            .members
            .commit_vacant(JOINER)
            .unwrap()
            .is_empty());
    }
    let mut donor_config = config(0, 7);
    donor_config.vacant_slots.push(JOINER);
    let a = Peer::new(game(), donor_config, source(&rig.nodes[0], 1));
    let b = Peer::new(game(), config(1, 7), source(&rig.nodes[1], 1));
    (rig, a, b)
}

#[test]
fn cancelled_join_fresh_generation_promotion_and_departure_repair_match_headless() {
    let (rig, mut a, mut b) = room();
    let ab = rig.nodes[0].borrow().connection(1);
    let ba = rig.nodes[1].borrow().connection(0);
    for _ in 0..12 {
        round(&mut a, &mut b, None);
    }
    assert_eq!((a.verified_tick(), b.verified_tick()), (12, 12));

    // A: application-admitted complete roster, with exclusive joining links.
    rig.admit_join_links();
    let attempts_a = rig.attempts(7);
    let mut join_a = bootstrap(&rig.nodes[2], 7);
    let exchange_a = exchange(&rig, &attempts_a, &mut join_a, &mut a, &mut b);
    assert_eq!(
        notice(&rig, &attempts_a, &mut join_a, &exchange_a, 0),
        Status::Syncing {
            received: 1,
            expected: 2
        }
    );
    let blocked_conn = rig.nodes[1].borrow().connection(2);
    rig.error(1, blocked_conn, Some(SendError::Backpressure));
    {
        let mut node = rig.nodes[1].borrow_mut();
        // Both messages survive a real NetEndpoint enqueue rejection.
        node.queue(&attempts_a[1], 2, &exchange_a.notices[1]);
        node.queue(&attempts_a[1], 2, &exchange_a.notices[1]);
        let usage = node.members.outbound_usage();
        let result = node.flush();
        assert_eq!(result.queued, 0);
        assert_eq!(
            result.blocked,
            vec![(blocked_conn, SendError::Backpressure)]
        );
        assert_eq!(result.remaining_bytes, 2 * exchange_a.notices[1].len());
        assert_eq!(node.members.outbound_usage(), usage);
    }
    rig.error(1, blocked_conn, None); // Never flush A's retained controls.
    for _ in 0..8 {
        round(&mut a, &mut b, Some(&mut join_a));
    }
    assert_eq!(
        join_a.status().unwrap(),
        Status::Syncing {
            received: 1,
            expected: 2
        }
    );
    assert!(join_a.session().unwrap().source().inner().sent.is_empty());
    assert!(a.last_remote_tick(JOINER).is_none());
    assert!(b.last_remote_tick(JOINER).unwrap() < exchange_a.first_input_tick);
    assert_eq!(
        a.authored_since(exchange_a.snapshot_tick)
            .first()
            .unwrap()
            .tick,
        exchange_a.snapshot_tick + 1
    );
    assert_eq!(
        b.authored_since(exchange_a.snapshot_tick)
            .first()
            .unwrap()
            .tick,
        exchange_a.snapshot_tick + 1
    );

    let old_links: [Vec<ConnId>; 3] = std::array::from_fn(|owner| {
        let node = rig.nodes[owner].borrow();
        if owner == 2 {
            node.input_links.clone()
        } else {
            vec![node.connection(2)]
        }
    });
    let old_keys: Vec<_> = old_links
        .iter()
        .enumerate()
        .flat_map(|(owner, links)| links.iter().map(move |&conn| (owner, conn)))
        .map(|(owner, conn)| rig.key(owner, conn))
        .collect();
    let stale_input = encode_input(
        exchange_a.first_input_tick,
        JOINER,
        input(2, exchange_a.first_input_tick),
        &commands(2, exchange_a.first_input_tick),
    );
    // Packets staged before retirement at all three layers must not escape:
    // application input buffers, adapter events, and transport events.
    for (owner, links) in old_links.iter().enumerate() {
        let conn = links[0];
        let mut node = rig.nodes[owner].borrow_mut();
        node.inputs.push((conn, decode_input(&stale_input)));
        node.endpoint.pending.push_back(ServerEvent::Message {
            conn,
            channel: Channel::Reliable,
            data: stale_input.clone(),
        });
        let net = node.endpoint.to_net[&conn.0];
        rig.wire.lock().unwrap().incoming[owner].push_back(Event::Message {
            conn: net,
            channel: orr_net::Channel::Reliable,
            bytes: stale_input.clone(),
        });
    }
    let cleanup_a = cancel(&rig, &attempts_a);
    assert_eq!(
        cleanup_a[1].discarded_outbound_bytes,
        2 * exchange_a.notices[1].len()
    );
    assert_eq!(
        cleanup_a[0].release_local_hold(&mut a),
        JoinHoldRelease::ReleasedAssignmentRetained
    );
    assert_eq!(
        cleanup_a[1].release_local_hold(&mut b),
        JoinHoldRelease::Released
    );
    let abandoned = join_a.cancel().unwrap();
    assert!(abandoned.source().inner().sent.is_empty());
    drop(abandoned);
    assert_eq!(join_a.notice_usage(), (0, 0));
    assert_eq!(join_a.status().unwrap(), Status::Cancelled);
    assert!(join_a.session().is_none());
    for (owner, cleanup) in cleanup_a.iter().enumerate() {
        let mut node = rig.nodes[owner].borrow_mut();
        assert_eq!(
            cleanup.retire_exclusive_connections(&mut node.endpoint),
            old_links[owner].len()
        );
        assert_eq!(cleanup.retire_exclusive_connections(&mut node.endpoint), 0);
        assert_eq!(node.members.outbound_usage(), (0, 0));
        assert_eq!(node.members.pending_count(), 0);
        for &conn in &old_links[owner] {
            assert_eq!(
                node.endpoint.try_send_reliable(conn, b"old link"),
                Err(SendError::UnknownConnection)
            );
        }
    }
    rig.pump();
    for node in &rig.nodes {
        let node = node.borrow();
        assert!(node.inputs.is_empty());
        assert!(node.controls.is_empty());
    }
    assert_eq!(rig.wire.lock().unwrap().closed.len(), 4);
    assert_eq!(rig.nodes[0].borrow().input_links, vec![ab]);
    assert_eq!(rig.nodes[1].borrow().input_links, vec![ba]);
    assert!(rig.nodes[2].borrow().input_links.is_empty());
    // Late traffic and physical-close notifications remain fenced, too.
    for &(owner, net) in &old_keys {
        let mut wire = rig.wire.lock().unwrap();
        wire.incoming[owner].push_back(Event::Message {
            conn: net,
            channel: orr_net::Channel::Reliable,
            bytes: stale_input.clone(),
        });
        wire.incoming[owner].push_back(Event::Disconnected {
            conn: net,
            reason: orr_net::DisconnectReason::RemoteClose,
        });
    }
    rig.pump();

    // Hold release alone never authorizes reuse. Here no joiner authored any
    // input, every old participant link is fenced, and Session's boundary
    // checks permit the sole donor's explicit application-coordinated recovery.
    assert!(a.last_remote_tick(JOINER).is_none());
    assert!(b.last_remote_tick(JOINER).unwrap() < exchange_a.first_input_tick);
    assert!(exchange_a.first_input_tick > a.verified_tick());
    assert!(exchange_a.first_input_tick <= a.next_send_tick());
    a.mark_slot_vacant(JOINER, exchange_a.first_input_tick)
        .unwrap();
    a.poll_confirmed();
    b.poll_confirmed();
    assert!(a.pending_join(JOINER).is_none());
    for peer in [&a, &b] {
        assert!(peer.verified_tick() > exchange_a.snapshot_tick + 2);
        assert!(
            peer.authored_since(exchange_a.snapshot_tick)
                .first()
                .unwrap()
                .tick
                > exchange_a.snapshot_tick + 1
        );
    }
    for _ in 0..6 {
        round(&mut a, &mut b, None);
    }
    assert_eq!(a.verified_tick(), b.verified_tick());
    assert_eq!(
        a.verified_frame().unwrap().checksum(),
        b.verified_frame().unwrap().checksum()
    );

    // B uses genuinely fresh links and a fresh generation after membership
    // changed. It is not a same-id retry that bypasses the checked roster.
    rig.admit_join_links();
    for (owner, previous_links) in old_links.iter().enumerate() {
        for &conn in &rig.nodes[owner].borrow().input_links {
            if (owner == 0 && conn == ab) || (owner == 1 && conn == ba) {
                continue;
            }
            assert!(!previous_links.contains(&conn));
        }
    }
    let attempts_b = rig.attempts(8);
    let mut join_b = bootstrap(&rig.nodes[2], 8);
    let exchange_b = exchange(&rig, &attempts_b, &mut join_b, &mut a, &mut b);
    // Old controls carried even on a current admitted link are rejected by
    // expected generation, leaving B's notice staging and exact holds intact.
    for owner in 0..2 {
        let conn = rig.nodes[owner].borrow().connection(2);
        rig.nodes[owner]
            .borrow_mut()
            .endpoint
            .try_send_reliable(conn, &exchange_a.notices[owner])
            .unwrap();
        let (conn, bytes) = rig.nodes[2].borrow_mut().control();
        assert!(matches!(
            rig.nodes[2]
                .borrow()
                .members
                .receive_notice(&attempts_b[2], conn, &mut join_b, &bytes),
            Err(orr_session::P2pMembershipError::Bootstrap(
                orr_session::JoinBootstrapError::Checked(
                    orr_session::CheckedJoinError::GenerationMismatch {
                        expected: 8,
                        got: 7
                    }
                )
            ))
        ));
    }
    let shared = rig.nodes[0].borrow().connection(1);
    rig.nodes[0]
        .borrow_mut()
        .endpoint
        .try_send_reliable(shared, &exchange_a.snapshot)
        .unwrap();
    let (conn, snapshot) = rig.nodes[1].borrow_mut().control();
    assert!(matches!(
        rig.nodes[1].borrow().members.import_ticket(
            &attempts_b[1],
            conn,
            &b,
            &snapshot,
            WIRE_LIMIT
        ),
        Err(orr_session::P2pMembershipError::Checked(
            orr_session::CheckedJoinError::GenerationMismatch {
                expected: 8,
                got: 7
            }
        ))
    ));
    assert_eq!(join_b.notice_usage(), (0, 0));
    assert_eq!(
        cleanup_a[0].release_local_hold(&mut a),
        JoinHoldRelease::NoMatchingHold
    );
    assert_eq!(
        cleanup_a[1].release_local_hold(&mut b),
        JoinHoldRelease::NoMatchingHold
    );
    for (owner, cleanup) in cleanup_a.iter().enumerate() {
        let mut node = rig.nodes[owner].borrow_mut();
        assert_eq!(cleanup.retire_exclusive_connections(&mut node.endpoint), 0);
        assert_eq!(node.endpoint.connection_count(), 2);
        assert_eq!(node.members.pending_count(), 1);
    }
    assert!(a.backlog_notice(JOINER).is_some());
    assert!(b.backlog_notice(JOINER).is_some());
    assert_eq!(
        notice(&rig, &attempts_b, &mut join_b, &exchange_b, 1),
        Status::Syncing {
            received: 1,
            expected: 2
        }
    );
    assert_eq!(
        notice(&rig, &attempts_b, &mut join_b, &exchange_b, 0),
        Status::Ready
    );
    let catch_up_target = exchange_b.first_input_tick + 8;
    assert!(!join_b.caught_up_to(catch_up_target).unwrap());
    for node in &rig.nodes {
        assert_eq!(
            node.borrow().members.state(JOINER).unwrap(),
            P2pSlotState::Vacant
        );
    }
    let pending_inputs = rig.key(1, rig.nodes[1].borrow().connection(2));
    rig.wire.lock().unwrap().held_routes.insert(pending_inputs);
    for _ in 0..10 {
        round(&mut a, &mut b, Some(&mut join_b));
    }
    assert!(a.verified_tick() >= catch_up_target);
    assert_eq!(join_b.status().unwrap(), Status::Ready);
    assert_eq!(join_b.session().unwrap().head_tick(), a.head_tick());
    assert_eq!(
        join_b.session().unwrap().verified_tick(),
        exchange_b.snapshot_tick
    );
    assert!(!join_b.caught_up_to(catch_up_target).unwrap());
    {
        let mut wire = rig.wire.lock().unwrap();
        assert_eq!(wire.held.len(), 10);
        wire.held_routes.remove(&pending_inputs);
        for (owner, event) in std::mem::take(&mut wire.held) {
            wire.incoming[owner].push_back(event);
        }
    }
    join_b.poll_confirmed().unwrap();
    assert!(join_b.caught_up_to(catch_up_target).unwrap());
    for _ in 0..6 {
        round(&mut a, &mut b, Some(&mut join_b));
    }
    assert!(join_b.caught_up_to(catch_up_target).unwrap());
    assert_eq!(a.verified_tick(), b.verified_tick());
    assert_eq!(a.verified_tick(), join_b.session().unwrap().verified_tick());

    // Application promotion explicitly relinquishes each endpoint lease before
    // registry mutation; delayed cleanup cannot retire retained gameplay links.
    for (owner, attempt) in attempts_b.iter().enumerate() {
        let mut node = rig.nodes[owner].borrow_mut();
        let Node {
            endpoint, members, ..
        } = &mut *node;
        for lease in members.exclusive_connections(attempt).unwrap() {
            assert!(endpoint.relinquish_exclusive_connection(lease));
        }
        let cleanups = members.promote_joiner(JOINER).unwrap();
        assert_eq!(cleanups.len(), 1);
        assert_eq!(cleanups[0].retire_exclusive_connections(endpoint), 0);
        assert_eq!(endpoint.connection_count(), 2);
        assert_eq!(members.state(JOINER).unwrap(), P2pSlotState::Active);
        assert_eq!(members.pending_count(), 0);
    }
    let equal_through = a.verified_tick();
    assert_reference(
        &[
            (&a, 1),
            (&b, 1),
            (join_b.session().unwrap(), exchange_b.snapshot_tick + 1),
        ],
        exchange_b.first_input_tick,
        equal_through,
        equal_through,
    );

    // C -> B's last five inputs are accepted but delayed in the paired
    // transport; A actually verifies them and retains exact recovery evidence.
    let held_conn = rig.nodes[2].borrow().connection(1);
    rig.wire
        .lock()
        .unwrap()
        .held_routes
        .insert(rig.key(2, held_conn));
    for _ in 0..5 {
        round(&mut a, &mut b, Some(&mut join_b));
    }
    let departed_through = a.verified_tick();
    assert_eq!(departed_through, equal_through + 5);
    assert_eq!(b.verified_tick(), equal_through);
    assert_eq!(join_b.session().unwrap().verified_tick(), departed_through);
    assert_eq!(a.last_remote_tick(JOINER), Some(departed_through));
    assert_eq!(b.last_remote_tick(JOINER), Some(equal_through));
    fence_current_links(&rig);
    {
        let mut wire = rig.wire.lock().unwrap();
        assert_eq!(wire.held.len(), 5);
        for (owner, event) in std::mem::take(&mut wire.held) {
            wire.incoming[owner].push_back(event);
        }
    }
    rig.pump();
    b.poll_confirmed();
    assert_eq!(
        b.verified_tick(),
        equal_through,
        "fenced original-author tail cannot leak through"
    );
    assert_eq!(a.next_send_tick(), departed_through + 1);
    assert!(a
        .authored_since(0)
        .iter()
        .all(|record| record.slot != JOINER));

    let survivors = [DONOR, PlayerSlot(1)];
    let revision = rig.nodes[0].borrow().members.revision();
    assert_eq!(rig.nodes[1].borrow().members.revision(), revision);
    let mut barrier = DepartureBarrier::new(
        9,
        revision,
        JOINER,
        DONOR,
        survivors.to_vec(),
        &[],
        a.config(),
    )
    .unwrap();
    for peer in [&a, &b] {
        barrier
            .report_fence(DepartureFence {
                recovery_id: 9,
                revision,
                survivor: peer.config().local_slot,
                verified_tick: peer.verified_tick(),
                departed_max: peer.last_remote_tick(JOINER),
            })
            .unwrap();
    }
    assert_eq!(barrier.repair_range(DONOR).unwrap(), None);
    let (first, last) = barrier.repair_range(PlayerSlot(1)).unwrap().unwrap();
    assert_eq!((first, last), (equal_through + 1, departed_through));
    assert!(matches!(
        barrier.acknowledgment(&b),
        Err(DepartureError::UnverifiedTarget)
    ));
    assert!(matches!(
        barrier.commit(&mut a, revision, &survivors),
        Err(DepartureError::MissingAcknowledgments)
    ));
    let repair = a
        .source()
        .history()
        .export_range(JOINER, first, last)
        .unwrap();
    assert_eq!(repair.len(), 5);
    // Application-approved trusted repair over the surviving real endpoint.
    // This uses the test codec, never claims a production repair protocol.
    for record in repair {
        let commands: Vec<_> = record
            .commands
            .iter()
            .map(|bytes| SpawnBulletCmd::decode(bytes).unwrap())
            .collect();
        rig.nodes[0]
            .borrow_mut()
            .endpoint
            .try_send_reliable(
                ab,
                &encode_input(record.tick, record.slot, record.input, &commands),
            )
            .unwrap();
    }
    b.poll_confirmed();
    assert_eq!(b.verified_tick(), departed_through);
    for peer in [&a, &b] {
        let ack = barrier.acknowledgment(peer).unwrap();
        assert_eq!(ack.checksum, a.verified_frame().unwrap().checksum());
        barrier.acknowledge(ack).unwrap();
    }
    assert!(barrier.ready().unwrap());
    assert!(matches!(
        barrier.commit(&mut b, revision, &survivors),
        Err(DepartureError::SessionMismatch)
    ));
    barrier.commit(&mut a, revision, &survivors).unwrap();
    assert!(matches!(
        barrier.commit(&mut a, revision, &survivors),
        Err(DepartureError::Committed)
    ));
    for owner in 0..2 {
        assert!(rig.nodes[owner]
            .borrow_mut()
            .members
            .commit_vacant(JOINER)
            .unwrap()
            .is_empty());
    }
    for _ in 0..8 {
        round(&mut a, &mut b, None);
    }
    let through = departed_through + 8;
    assert_eq!((a.verified_tick(), b.verified_tick()), (through, through));
    assert_eq!(
        join_b.session().unwrap().next_send_tick(),
        departed_through + 1
    );
    let defaults: Vec<_> = a
        .source()
        .inner()
        .sent
        .iter()
        .filter(|record| record.slot == JOINER && record.tick > departed_through)
        .collect();
    assert_eq!(defaults.len(), 8);
    for (tick, record) in (departed_through + 1..=through).zip(defaults) {
        assert_eq!(record.tick, tick);
        assert_eq!(record.input, ArenaInput::default());
        assert!(record.commands.is_empty());
    }
    assert!(b
        .source()
        .inner()
        .sent
        .iter()
        .all(|record| record.slot != JOINER));
    assert_reference(
        &[
            (&a, 1),
            (&b, 1),
            (join_b.session().unwrap(), exchange_b.snapshot_tick + 1),
        ],
        exchange_b.first_input_tick,
        departed_through,
        through,
    );
    assert_eq!(rig.wire.lock().unwrap().closed.len(), 8);
}

#[test]
fn membership_retry_preserves_distinct_controls_and_unblocked_destinations() {
    let (rig, mut a, mut b) = room();
    rig.admit_join_links();
    let attempts = rig.attempts(7);
    let mut join = bootstrap(&rig.nodes[2], 7);
    let exchange = exchange(&rig, &attempts, &mut join, &mut a, &mut b);
    let conn = rig.nodes[0].borrow().connection(2);
    rig.error(0, conn, Some(SendError::Backpressure));
    {
        let mut node = rig.nodes[0].borrow_mut();
        node.queue(&attempts[0], 2, &exchange.snapshot);
        node.queue(&attempts[0], 2, &exchange.notices[0]);
        node.queue(&attempts[0], 1, &exchange.snapshot);
        let result = node.flush();
        assert_eq!(result.queued, 1);
        assert_eq!(result.blocked, vec![(conn, SendError::Backpressure)]);
        assert_eq!(
            result.remaining_bytes,
            exchange.snapshot.len() + exchange.notices[0].len()
        );
        assert_eq!(node.members.outbound_usage().0, 2);
    }
    assert_eq!(rig.nodes[1].borrow_mut().control().1, exchange.snapshot);
    rig.nodes[2].borrow_mut().pump();
    assert!(rig.nodes[2].borrow().controls.is_empty());
    rig.error(0, conn, None);
    let result = rig.nodes[0].borrow_mut().flush();
    assert_eq!(result.queued, 2);
    assert!(result.blocked.is_empty());
    assert_eq!(result.remaining_bytes, 0);
    assert_eq!(rig.nodes[2].borrow_mut().control().1, exchange.snapshot);
    assert_eq!(rig.nodes[2].borrow_mut().control().1, exchange.notices[0]);
    assert!(rig.nodes[2].borrow().controls.is_empty());
}
