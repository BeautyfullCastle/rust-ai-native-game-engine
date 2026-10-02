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
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use common::{arena_room, arena_script};
use orr_proto::RejectReason;
use orr_relay_net::{
    connect, drive, listen, report, ClientReport, ConnectOptions, DriveOptions, ListenOptions, SimConditions, TransportKind, Trust,
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
        Self::start_observed(kind, players, |_| {})
    }

    fn start_observed(kind: TransportKind, players: u8, mut observe: impl FnMut(&ServerNote) + Send + 'static) -> Live {
        let ep = listen(&ListenOptions::new("127.0.0.1:0".parse().unwrap(), kind)).expect("listen");
        let (addr, fingerprint) = (ep.local_addr(), ep.cert_sha256());
        let mut server = RelayServer::new(ep, 7);
        server.create_room(ROOM, arena_room(players));
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let join = thread::spawn(move || {
            let mut notes = Vec::new();
            run_wall_clock(&mut server, &stop_flag, Duration::from_millis(1), |s, _| {
                for note in s.drain_notes() {
                    observe(&note);
                    notes.push(note);
                }
            });
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

#[derive(Clone, Copy, Debug)]
struct ProgressGoal {
    verified_ticks: u64,
    shared_checkpoints: usize,
}

/// Rejoin-only counterpart to `drive`: keep its update/leave behavior, but
/// finish on observed progress instead of a wall-clock play window. A late
/// join's snapshot/catch-up ticks do not count toward its Playing baseline.
fn run_progress_client(
    live: &Live,
    setup: ClientSetup,
    stop: Option<Arc<AtomicBool>>,
    tag: &str,
    goal: Option<ProgressGoal>,
    mut observe: impl FnMut(&ClientReport) -> usize,
) -> ClientReport {
    let link = connect(&live.options(setup.sim)).expect("connect");
    let mut cfg = RelayClientConfig::new(ROOM, setup.build_id);
    cfg.want_slot = setup.want_slot.map(PlayerSlot);
    cfg.token = setup.token;
    let mut client: RelayClient<Arena, _> =
        RelayClient::new(cfg, link, |w| ArenaConfig { player_count: w.player_count }, DumpCollector::new());
    let start = Instant::now();
    let mut playing: Option<(Instant, u64)> = None;
    // Failure guards only: two 20 s client runs plus the 10 s PlayerLeft
    // barrier and cleanup fit within the stayer's 60 s overall watchdog.
    let watchdog = Duration::from_secs(if goal.is_some() { 20 } else { 60 });
    let rep = loop {
        let slot = client.welcome().map_or(0, |w| w.slot);
        client.update(start.elapsed().as_micros() as u64, &mut |tick| arena_script(usize::from(slot), tick));
        let r = report(&client, playing.map_or(0.0, |(p, _)| p.elapsed().as_secs_f64()));
        if playing.is_none() && r.state == ClientState::Playing {
            playing = Some((Instant::now(), r.verified_tick));
        }
        let shared = observe(&r);
        let baseline = playing.map(|(_, tick)| tick);
        let target = baseline.zip(goal).map(|(tick, goal)| tick + goal.verified_ticks);
        if stop.as_ref().is_some_and(|s| s.load(Ordering::Relaxed)) {
            break r;
        }
        assert!(
            !matches!(r.state, ClientState::Rejected(_) | ClientState::Disconnected | ClientState::Failed(_)),
            "{tag} stopped before progress: state {:?}, baseline {baseline:?}, target {target:?}, goal {goal:?}, shared {shared}; {}",
            r.state,
            r.summary()
        );
        assert!(
            start.elapsed() < watchdog,
            "{tag} progress watchdog after {:?}: state {:?}, baseline {baseline:?}, target {target:?}, goal {goal:?}, shared {shared}, checkpoints {}; {}",
            start.elapsed(),
            r.state,
            r.checksums.len(),
            r.summary()
        );
        if r.state == ClientState::Playing
            && target.is_some_and(|tick| r.verified_tick >= tick)
            && goal.is_some_and(|goal| shared >= goal.shared_checkpoints)
        {
            eprintln!("{tag}: baseline {baseline:?}, target {target:?}, shared {shared}; {}", r.summary());
            break r;
        }
        thread::sleep(Duration::from_millis(1));
    };
    if setup.leave_at_end && matches!(client.state(), ClientState::Playing | ClientState::Syncing | ClientState::CatchingUp) {
        client.leave();
        let slot = client.welcome().map_or(0, |w| w.slot);
        // Same reliable Leave flush as `orr_relay_net::drive`.
        for _ in 0..20 {
            client.update(start.elapsed().as_micros() as u64, &mut |tick| arena_script(usize::from(slot), tick));
            thread::sleep(Duration::from_millis(5));
        }
    }
    rep
}

/// Own only this scenario's threads. In particular, address-only `Live`
/// copies elsewhere must never stop or join their owner's server.
struct RejoinRun {
    live: Live,
    stop: Arc<AtomicBool>,
    stayer: Option<JoinHandle<ClientReport>>,
}

impl RejoinRun {
    fn finish(mut self) -> (ClientReport, ServerResult) {
        self.stop.store(true, Ordering::Relaxed);
        let stayer = self.stayer.take().expect("stayer handle").join().expect("stayer thread");
        self.live.stop.store(true, Ordering::Relaxed);
        let server = self.live.join.take().expect("server handle").join().expect("server thread");
        (stayer, server)
    }
}

impl Drop for RejoinRun {
    fn drop(&mut self) {
        // Signal both before joining either, even while unwinding from a
        // connect, progress, disconnect-barrier or assertion failure.
        self.stop.store(true, Ordering::Relaxed);
        self.live.stop.store(true, Ordering::Relaxed);
        if let Some(stayer) = self.stayer.take() {
            match stayer.join() {
                Ok(r) => eprintln!("rejoin cleanup stayer: state {:?}; {}", r.state, r.summary()),
                Err(_) => eprintln!("rejoin cleanup: stayer thread panicked"),
            }
        }
        if let Some(server) = self.live.join.take() {
            match server.join() {
                Ok(s) => eprintln!(
                    "rejoin cleanup server: finalized {}, desyncs {}, bad messages {}, notes {:?}",
                    s.room.finalized, s.room.desyncs, s.bad_messages, s.notes
                ),
                Err(_) => eprintln!("rejoin cleanup: server thread panicked"),
            }
        }
    }
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
    let (left_tx, left_rx) = mpsc::channel();
    let live = Live::start_observed(TransportKind::Quic, 2, move |note| {
        if matches!(note, ServerNote::PlayerLeft { room: ROOM, slot: 1, .. }) {
            let _ = left_tx.send(());
        }
    });
    let mut run = RejoinRun { live, stop: Arc::new(AtomicBool::new(false)), stayer: None };
    let stayer_progress = Arc::new(Mutex::new(None));
    run.stayer = Some({
        let live = &run.live;
        let l = Live { kind: live.kind, addr: live.addr, fingerprint: live.fingerprint, stop: live.stop.clone(), join: None };
        let mut setup = ClientSetup::new(0.0);
        setup.play_for = None;
        setup.want_slot = Some(0);
        let stop = run.stop.clone();
        let progress = stayer_progress.clone();
        thread::spawn(move || {
            run_progress_client(&l, setup, Some(stop), "stayer", None, |r| {
                *progress.lock().expect("stayer progress") = Some(r.clone());
                0
            })
        })
    });
    let mut first = ClientSetup::new(0.0);
    first.want_slot = Some(1);
    first.leave_at_end = false; // just drop the connection
    // Equivalent simulation depth to the old 2.5 s at 60 Hz, irrespective
    // of how much wall time the clients need under load.
    let before = run_progress_client(
        &run.live, first, None, "leaver", Some(ProgressGoal { verified_ticks: 150, shared_checkpoints: 0 }), |_| 0,
    );
    assert_eq!(before.state, ClientState::Playing);
    let token = before.token.expect("token");
    eprintln!("leaver before: {}", before.summary());
    // Reusing the token while the old connection is still registered takes
    // over its slot without announcing PlayerLeft. Observe the disconnect
    // first so this test exercises a real departure and subsequent rejoin.
    left_rx.recv_timeout(Duration::from_secs(10)).expect("slot 1 did not leave before rejoining");

    let mut second = ClientSetup::new(0.0);
    second.want_slot = Some(1);
    second.token = token;
    let after = run_progress_client(
        &run.live,
        second,
        None,
        "rejoiner",
        Some(ProgressGoal { verified_ticks: 240, shared_checkpoints: 3 }),
        |r| {
            let progress = stayer_progress.lock().expect("stayer progress");
            progress.as_ref().map_or(0, |stayer| {
                r.checksums.iter().filter(|(tick, _)| stayer.checksums.binary_search_by_key(tick, |&(t, _)| t).is_ok()).count()
            })
        },
    );
    eprintln!("rejoiner after: {}", after.summary());
    assert_eq!(after.state, ClientState::Playing);
    assert_eq!(after.slot, Some(1));
    assert_eq!(after.token, Some(token));
    let (stayer, server) = run.finish();
    eprintln!("stayer: {}", stayer.summary());

    assert_eq!(before.desyncs, 0);
    assert_eq!(stayer.state, ClientState::Playing);
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
