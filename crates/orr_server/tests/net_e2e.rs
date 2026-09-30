//! End-to-end tests of the relay over real loopback sockets: the real server
//! core (`RelayServer` on `orr_relay_net::NetEndpoint`, driven by the wall
//! clock) and headless relay clients on QUIC or WebSocket, with the network
//! conditioner adding latency, jitter and loss to the client side.
//!
//! These run in real time (a few seconds each). The longer soak is `#[ignore]`:
//! `cargo test -p orr_server --release --test net_e2e -- --ignored --nocapture`.
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)]

mod common;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use common::{arena_room, arena_script};
use orr_proto::RejectReason;
use orr_relay_net::{
    connect, drive, listen, ClientReport, ConnectOptions, DriveOptions, ListenOptions, SimConditions, TransportKind, Trust,
};
use orr_server::serve::run_wall_clock;
use orr_server::{RelayServer, RoomStats, ServerNote};
use orr_session::{ClientState, DumpCollector, RelayClient, RelayClientConfig};
use orr_sim::PlayerSlot;
use orr_testgame::{Arena, ArenaConfig};

const ROOM: u64 = 1;

struct ServerResult {
    room: RoomStats,
    notes: Vec<ServerNote>,
    bad_messages: u64,
}

struct Live {
    kind: TransportKind,
    addr: SocketAddr,
    fingerprint: Option<[u8; 32]>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<ServerResult>>,
}

impl Live {
    fn start(kind: TransportKind, players: u8) -> Live {
        let ep = listen(&ListenOptions::new("127.0.0.1:0".parse().unwrap(), kind)).expect("listen");
        let (addr, fingerprint) = (ep.local_addr(), ep.cert_sha256());
        let mut server = RelayServer::new(ep, 7);
        server.create_room(ROOM, arena_room(players));
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let join = thread::spawn(move || {
            let mut notes = Vec::new();
            run_wall_clock(&mut server, &stop_flag, Duration::from_millis(1), |s, _| notes.extend(s.drain_notes()));
            let room = server.room_stats(ROOM).expect("room");
            ServerResult { room, notes, bad_messages: server.bad_messages() }
        });
        Live { kind, addr, fingerprint, stop, join: Some(join) }
    }

    fn options(&self, sim: Option<SimConditions>) -> ConnectOptions {
        let trust = self.fingerprint.map_or(Trust::InsecureDev, Trust::Fingerprint);
        let mut o = ConnectOptions::new(self.addr.to_string(), self.kind, trust);
        o.sim = sim;
        o
    }

    fn finish(mut self) -> ServerResult {
        self.stop.store(true, Ordering::Relaxed);
        self.join.take().unwrap().join().expect("server thread")
    }
}

#[derive(Clone)]
struct ClientSetup {
    build_id: u64,
    want_slot: Option<u8>,
    token: u64,
    sim: Option<SimConditions>,
    play_for: Option<Duration>,
    leave_at_end: bool,
}

impl ClientSetup {
    fn new(play_secs: f32) -> Self {
        Self {
            build_id: 1,
            want_slot: None,
            token: 0,
            sim: None,
            play_for: Some(Duration::from_secs_f32(play_secs)),
            leave_at_end: true,
        }
    }
}

fn run_client(live: &Live, setup: ClientSetup, stop: Option<Arc<AtomicBool>>, tag: &str) -> ClientReport {
    let link = connect(&live.options(setup.sim)).expect("connect");
    let mut cfg = RelayClientConfig::new(ROOM, setup.build_id);
    cfg.want_slot = setup.want_slot.map(PlayerSlot);
    cfg.token = setup.token;
    let mut client: RelayClient<Arena, _> =
        RelayClient::new(cfg, link, |w| ArenaConfig { player_count: w.player_count }, DumpCollector::new());
    let opts = DriveOptions {
        play_for: setup.play_for,
        connect_timeout: Duration::from_secs(20),
        stop,
        tag: tag.to_string(),
        leave_at_end: setup.leave_at_end,
        ..DriveOptions::default()
    };
    drive(&mut client, &mut |slot, tick| arena_script(usize::from(slot), tick), &opts)
}

/// Checks that every report's verified checksums agree with the others on
/// the ticks they share. Returns how many checkpoints were compared.
fn assert_checksums_agree(reports: &[ClientReport]) -> usize {
    let mut by_tick: BTreeMap<u64, (u64, usize)> = BTreeMap::new();
    let mut compared = 0;
    for (i, r) in reports.iter().enumerate() {
        for &(tick, cs) in &r.checksums {
            match by_tick.get(&tick) {
                Some(&(want, first)) => {
                    assert_eq!(cs, want, "client {i} disagrees with client {first} at tick {tick}");
                    compared += 1;
                }
                None => {
                    by_tick.insert(tick, (cs, i));
                }
            }
        }
    }
    compared
}

fn four_clients(kind: TransportKind, sim: SimConditions, secs: f32) {
    let live = Live::start(kind, 4);
    let handles: Vec<_> = (0..4u64)
        .map(|i| {
            let mut setup = ClientSetup::new(secs);
            setup.sim = Some(SimConditions { seed: 100 + i, ..sim });
            let addr_live = Live { kind: live.kind, addr: live.addr, fingerprint: live.fingerprint, stop: live.stop.clone(), join: None };
            thread::spawn(move || run_client(&addr_live, setup, None, &format!("c{i}")))
        })
        .collect();
    let reports: Vec<ClientReport> = handles.into_iter().map(|h| h.join().expect("client thread")).collect();
    let server = live.finish();

    for (i, r) in reports.iter().enumerate() {
        eprintln!("client {i}: {}", r.summary());
        assert_eq!(r.state, ClientState::Playing, "client {i} is not playing at the end");
        assert_eq!(r.desyncs, 0, "client {i} saw a desync");
        assert_eq!(r.decode_errors, 0);
        assert!(r.verified_tick + 90 >= r.head_tick.saturating_sub(30), "client {i} verification is far behind");
        assert!(r.verified_tick as f64 >= f64::from(secs) * 60.0 * 0.5, "client {i} verified only {} ticks", r.verified_tick);
        assert!(r.rtt_ms > 100.0 && r.rtt_ms < 400.0, "client {i} rtt {} ms", r.rtt_ms);
    }
    let compared = assert_checksums_agree(&reports);
    let min_compared = 3 * (f64::from(secs) * 60.0 / 30.0 * 0.4) as usize;
    assert!(compared >= min_compared, "only {compared} checkpoints compared (want {min_compared})");
    eprintln!(
        "server: finalized {}, desyncs {}, repeated {:?}, late {:?}, bad messages {}, {compared} checkpoints compared",
        server.room.finalized,
        server.room.desyncs,
        server.room.slots.iter().map(|s| s.repeated).collect::<Vec<_>>(),
        server.room.slots.iter().map(|s| s.late_dropped).collect::<Vec<_>>(),
        server.bad_messages
    );
    assert_eq!(server.room.desyncs, 0);
    assert_eq!(server.bad_messages, 0);
    assert!(!server.notes.iter().any(|n| matches!(n, ServerNote::Desync { .. })));
}

/// 4 players at 150 ms round trip (75 ms each way), jitter and 2% loss over real QUIC.
#[test]
fn four_clients_over_quic_at_150ms_rtt() {
    four_clients(TransportKind::Quic, SimConditions { latency_ms: 75, jitter_ms: 10, loss: 0.02, seed: 0 }, 7.0);
}

#[test]
#[ignore = "60 s soak"]
fn soak_four_clients_over_quic_at_150ms_rtt() {
    four_clients(TransportKind::Quic, SimConditions { latency_ms: 75, jitter_ms: 15, loss: 0.02, seed: 0 }, 60.0);
}

/// The same relay over WebSocket (no conditioner): two players.
#[test]
fn two_clients_over_websocket() {
    let live = Live::start(TransportKind::Ws, 2);
    let handles: Vec<_> = (0..2)
        .map(|i| {
            let l = Live { kind: live.kind, addr: live.addr, fingerprint: None, stop: live.stop.clone(), join: None };
            thread::spawn(move || run_client(&l, ClientSetup::new(3.0), None, &format!("ws{i}")))
        })
        .collect();
    let reports: Vec<ClientReport> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let server = live.finish();
    for r in &reports {
        eprintln!("ws client: {}", r.summary());
        assert_eq!(r.state, ClientState::Playing);
        assert_eq!(r.desyncs, 0);
        assert!(r.verified_tick > 90, "verified {}", r.verified_tick);
    }
    assert!(assert_checksums_agree(&reports) >= 4);
    assert_eq!(server.room.desyncs, 0);
}

/// A client with another build hash is rejected over the real transport, and
/// its slot stays free for a matching client.
#[test]
fn build_hash_mismatch_is_rejected() {
    for kind in [TransportKind::Quic, TransportKind::Ws] {
        let live = Live::start(kind, 2);
        let mut bad = ClientSetup::new(1.0);
        bad.build_id = 2;
        let r = run_client(&live, bad, None, "bad-build");
        let want = RejectReason::BuildHashMismatch { server: orr_sim::build_hash_of(1, 0), client: orr_sim::build_hash_of(2, 0) };
        assert_eq!(r.state, ClientState::Rejected(want), "{kind:?}");
        let server = live.finish();
        assert!(server.notes.iter().any(|n| matches!(n, ServerNote::Rejected { reason, .. } if *reason == want)), "{kind:?}");
        assert!(!server.notes.iter().any(|n| matches!(n, ServerNote::PlayerJoined { .. })), "{kind:?}");
    }
}

/// A client drops out (no goodbye, as after a crash), then comes back with
/// its token: same slot, state from the other client's snapshot, no desync.
#[test]
fn disconnect_and_rejoin_over_quic() {
    let live = Live::start(TransportKind::Quic, 2);
    let stop = Arc::new(AtomicBool::new(false));

    let stay = {
        let l = Live { kind: live.kind, addr: live.addr, fingerprint: live.fingerprint, stop: live.stop.clone(), join: None };
        let mut setup = ClientSetup::new(0.0);
        setup.play_for = None;
        setup.want_slot = Some(0);
        let stop = stop.clone();
        thread::spawn(move || run_client(&l, setup, Some(stop), "stayer"))
    };
    let mut first = ClientSetup::new(2.5);
    first.want_slot = Some(1);
    first.leave_at_end = false; // just drop the connection
    let before = run_client(&live, first, None, "leaver");
    assert_eq!(before.state, ClientState::Playing);
    let token = before.token.expect("token");
    eprintln!("leaver before: {}", before.summary());
    thread::sleep(Duration::from_millis(500));

    let mut second = ClientSetup::new(4.0);
    second.want_slot = Some(1);
    second.token = token;
    let after = run_client(&live, second, None, "rejoiner");
    eprintln!("rejoiner after: {}", after.summary());
    assert_eq!(after.state, ClientState::Playing);
    assert_eq!(after.slot, Some(1));
    stop.store(true, Ordering::Relaxed);
    let stayer = stay.join().unwrap();
    eprintln!("stayer: {}", stayer.summary());
    let server = live.finish();

    assert_eq!(stayer.desyncs, 0);
    assert_eq!(after.desyncs, 0);
    assert!(after.verified_tick > before.verified_tick, "the rejoiner did not advance past its earlier tick");
    let compared = assert_checksums_agree(&[stayer, after]);
    assert!(compared >= 3, "compared {compared}");
    let left = server.notes.iter().filter(|n| matches!(n, ServerNote::PlayerLeft { slot: 1, .. })).count();
    let joined = server.notes.iter().filter(|n| matches!(n, ServerNote::PlayerJoined { slot: 1, .. })).count();
    // Left once when the connection dropped, once more when the rejoiner said goodbye.
    assert!(left >= 1 && joined == 2, "left {left}, joined {joined}: {:?}", server.notes);
    assert_eq!(server.room.desyncs, 0);
}
