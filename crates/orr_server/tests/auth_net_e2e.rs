//! Authoritative mode over real loopback QUIC: the real server core on
//! `orr_relay_net::NetEndpoint` (wall clock, own thread) with a server-side
//! sim, and headless relay clients with the network conditioner.
//!
//! (a) a clean run: the server's checksums equal every client's, no
//! corrections; (b) one client is corrupted: the server detects it, sends a
//! correction, the client recovers and agrees again, a `.orrd` is written.
//! These run in real time (about 8 s each).
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)]

mod common;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use bytemuck::{Pod, Zeroable};
use common::{arena_room, arena_script, SEED, TICK_RATE};
use orr_proto::SERVER_SLOT;
use orr_relay_net::{
    connect, drive, listen, ClientReport, ConnectOptions, DriveOptions, ListenOptions, SimConditions, TransportKind, Trust,
};
use orr_server::presets::arena_sim;
use orr_server::serve::run_wall_clock;
use orr_server::{AuthoritativeConfig, GameSim, RelayServer, RoomConfig, RoomStats, ServerNote, ServerSim, SharedDumps};
use orr_session::{ClientState, DesyncDump, DumpCollector, RelayClient, RelayClientConfig};
use orr_sim::{decode_pod, encode_pod, Game, PlayerSlot, SimCommand, SimContext, System};
use orr_testgame::{Arena, ArenaConfig};

const ROOM: u64 = 1;

type Script<G> = fn(u8, u64) -> (<G as Game>::Input, Vec<<G as Game>::Command>);

struct ServerResult {
    room: RoomStats,
    notes: Vec<ServerNote>,
    checksums: BTreeMap<u64, u64>,
    bad_messages: u64,
}

struct Live {
    addr: SocketAddr,
    fingerprint: Option<[u8; 32]>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<ServerResult>>,
    dumps: SharedDumps,
}

impl Live {
    fn start(cfg: RoomConfig, auth: AuthoritativeConfig, sim: Box<dyn ServerSim>) -> Live {
        let ep = listen(&ListenOptions::new("127.0.0.1:0".parse().unwrap(), TransportKind::Quic)).expect("listen");
        let (addr, fingerprint) = (ep.local_addr(), ep.cert_sha256());
        let mut server = RelayServer::new(ep, 7);
        let dumps = SharedDumps::new();
        server.set_dump_sink(dumps.clone());
        server.create_authoritative_room(ROOM, cfg, auth, sim);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let join = thread::spawn(move || {
            let mut notes = Vec::new();
            run_wall_clock(&mut server, &stop_flag, Duration::from_millis(1), |s, _| notes.extend(s.drain_notes()));
            ServerResult {
                room: server.room_stats(ROOM).expect("room"),
                notes,
                checksums: server.server_checksums(ROOM).iter().copied().collect(),
                bad_messages: server.bad_messages(),
            }
        });
        Live { addr, fingerprint, stop, join: Some(join), dumps }
    }

    fn options(&self, i: u64) -> ConnectOptions {
        let trust = self.fingerprint.map_or(Trust::InsecureDev, Trust::Fingerprint);
        let mut o = ConnectOptions::new(self.addr.to_string(), TransportKind::Quic, trust);
        o.sim = Some(SimConditions { latency_ms: 40, jitter_ms: 5, loss: 0.01, seed: 100 + i });
        o
    }

    fn finish(&mut self) -> ServerResult {
        self.stop.store(true, Ordering::Relaxed);
        self.join.take().unwrap().join().expect("server thread")
    }
}

fn run_clients<G: Game>(
    live: &Live,
    n: u64,
    secs: f32,
    make_config: fn(&orr_proto::Welcome) -> G::Config,
    script: Script<G>,
    before: fn(u64),
) -> Vec<ClientReport> {
    let handles: Vec<_> = (0..n)
        .map(|i| {
            let opts = live.options(i);
            thread::spawn(move || {
                before(i);
                let link = connect(&opts).expect("connect");
                let cfg = RelayClientConfig::new(ROOM, 1);
                let mut client: RelayClient<G, _> = RelayClient::new(cfg, link, make_config, DumpCollector::new());
                let drive_opts = DriveOptions {
                    play_for: Some(Duration::from_secs_f32(secs)),
                    connect_timeout: Duration::from_secs(20),
                    tag: format!("c{i}"),
                    leave_at_end: true,
                    ..DriveOptions::default()
                };
                drive(&mut client, &mut |slot, tick| script(slot, tick), &drive_opts)
            })
        })
        .collect();
    handles.into_iter().map(|h| h.join().expect("client thread")).collect()
}

/// Checks each report's checksums against the server's at ticks `>= from`.
fn compare_with_server(reports: &[ClientReport], server: &ServerResult, from_per_client: &[u64]) -> usize {
    let mut compared = 0;
    for (i, r) in reports.iter().enumerate() {
        for &(tick, cs) in &r.checksums {
            if tick < from_per_client[i] {
                continue;
            }
            let want = server.checksums.get(&tick).unwrap_or_else(|| panic!("server has no checksum at {tick}"));
            assert_eq!(cs, *want, "client {i} differs from the server at tick {tick}");
            compared += 1;
        }
    }
    compared
}

#[test]
fn authoritative_arena_over_quic_matches_the_server() {
    let mut cfg = arena_room(3);
    cfg.min_players_to_start = 3;
    let mut live = Live::start(cfg, AuthoritativeConfig::default(), Box::new(arena_sim(3, TICK_RATE, SEED, 1, &[])));
    let reports = run_clients::<Arena>(
        &live,
        3,
        7.0,
        |w| ArenaConfig { player_count: w.player_count },
        |slot, tick| arena_script(usize::from(slot), tick),
        |_| {},
    );
    let server = live.finish();
    for (i, r) in reports.iter().enumerate() {
        eprintln!("client {i}: {}", r.summary());
        assert_eq!(r.state, ClientState::Playing);
        assert_eq!((r.desyncs, r.corrections), (0, 0));
        assert!(r.verified_tick >= 150, "verified {}", r.verified_tick);
    }
    let compared = compare_with_server(&reports, &server, &[0; 3]);
    assert!(compared >= 24, "only {compared} checkpoints compared");
    assert_eq!((server.room.desyncs, server.room.corrections, server.room.kicks, server.room.violations), (0, 0, 0, 0));
    assert!(live.dumps.is_empty());
    assert_eq!(server.bad_messages, 0);
    eprintln!("quic (a): {compared} client checkpoints equal the server's over {} server ticks", server.room.server_ticks);
}

// ---- a game one client can corrupt (the thread-local is per client thread) --------

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Pod, Zeroable)]
struct CInput {
    add: u32,
    _pad: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Pod, Zeroable)]
struct NoCmd {
    _pad: u32,
}
impl SimCommand for NoCmd {
    fn encode(&self, out: &mut Vec<u8>) {
        encode_pod(self, out)
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        decode_pod(bytes)
    }
}
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct NoEvent {
    _pad: u32,
}
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Acc {
    sums: [u64; 4],
}

thread_local! {
    static CORRUPT_AT: Cell<Option<u64>> = const { Cell::new(None) };
}

struct Counter;
struct CountSystem;
impl System<Counter> for CountSystem {
    fn name(&self) -> &'static str {
        "CountSystem"
    }
    fn run(&mut self, ctx: &mut SimContext<Counter>) {
        for s in 0..ctx.inputs.player_count().min(4) {
            let add = u64::from(ctx.inputs.input(PlayerSlot(s)).add);
            ctx.frame.singleton_mut::<Acc>().sums[s as usize] += add;
        }
        if CORRUPT_AT.with(Cell::get) == Some(ctx.tick) {
            ctx.frame.singleton_mut::<Acc>().sums[0] ^= 0x8000;
        }
    }
}
impl Game for Counter {
    type Input = CInput;
    type Command = NoCmd;
    type Event = NoEvent;
    type Config = ();
    fn register(b: &mut orr_ecs::ComponentRegistryBuilder) {
        b.register_singleton::<Acc>("Acc");
    }
    fn setup(frame: &mut orr_ecs::Frame, _: &()) {
        frame.set_singleton(Acc { sums: [0; 4] });
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(CountSystem)]
    }
}

#[test]
fn a_corrupted_client_recovers_through_a_server_correction_over_quic() {
    let mut cfg = RoomConfig::new(3, TICK_RATE, SEED, 8);
    cfg.record_all = true;
    let mut live = Live::start(
        cfg,
        AuthoritativeConfig::default(),
        Box::new(GameSim::<Counter>::new((), TICK_RATE, SEED, 1, 3)),
    );
    // Client 1 (the second thread) corrupts its state at tick 150.
    let reports = run_clients::<Counter>(
        &live,
        3,
        8.0,
        |_| (),
        |slot, tick| (CInput { add: ((tick / 5 + u64::from(slot) * 7) % 4) as u32, _pad: 0 }, Vec::new()),
        |i| {
            if i == 1 {
                CORRUPT_AT.with(|c| c.set(Some(150)));
            }
        },
    );
    let server = live.finish();
    for (i, r) in reports.iter().enumerate() {
        eprintln!("client {i}: {}", r.summary());
        assert_eq!(r.state, ClientState::Playing, "client {i}");
        assert!(r.kicked.is_none());
    }
    let (tick, found_at) = server
        .notes
        .iter()
        .find_map(|n| match n {
            ServerNote::Desync { tick, finalized, .. } => Some((*tick, *finalized)),
            _ => None,
        })
        .expect("the server found no desync");
    assert_eq!(tick, 150, "the first checkpoint at or after the corruption");
    let corr = server
        .notes
        .iter()
        .find_map(|n| match n {
            ServerNote::Correction { tick, bytes, .. } => Some((*tick, *bytes)),
            _ => None,
        })
        .expect("no correction");
    assert_eq!(server.room.corrections, 1);
    assert_eq!((reports[1].corrections, reports[0].corrections, reports[2].corrections), (1, 0, 0));
    assert_eq!(reports[1].desyncs, 1, "only the diverged client is told");
    assert_eq!((reports[0].desyncs, reports[2].desyncs), (0, 0));
    // Everyone agrees with the server again; client 1 from after its correction.
    let compared = compare_with_server(&reports, &server, &[0, 0, 0]);
    let after = reports[1].checksums.iter().filter(|(t, _)| *t > corr.0).count();
    assert!(after >= 4, "only {after} checkpoints after the correction");
    assert!(reports[1].checksums.iter().all(|(t, _)| *t < 150 || *t > corr.0));
    // The server's dump replays to its own checksum.
    let dumps = live.dumps.take();
    assert_eq!(dumps.len(), 1);
    let dump = DesyncDump::from_bytes(&dumps[0].1).unwrap();
    assert_eq!((dump.local_slot, dump.desync_tick), (SERVER_SLOT, 150));
    let clean = dump.replay::<Counter>((), 1).unwrap().into_iter().find(|(t, _)| *t == 150).unwrap().1;
    assert_eq!(clean, dump.local_checksum);
    assert_eq!(server.bad_messages, 0);
    eprintln!(
        "quic (b): corrupted at 150, found at server tick {found_at}, correction of tick {} ({} B), {after} checkpoints agree afterwards, {compared} compared",
        corr.0, corr.1
    );
}
