//! Relay play of the sample games over real loopback sockets, through the
//! same code the `arena` and `physics` programs use (`net_client`): the real
//! server core with the server's room presets, and headless bots.
//!
//! Real time, a few seconds each. The 4-player physics test runs on QUIC
//! with 75 ms one way (150 ms round trip), jitter and 2% loss.
#![allow(clippy::disallowed_types)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use orr_relay_net::{listen, ClientReport, ListenOptions, SimConditions, TransportKind};
use orr_sample::net_client::{run_arena_bot, run_physics_bot, NetArgs, ARENA_BUILD_ID, PHYSICS_BUILD_ID};
use orr_sample::physics_game::{PhysConfig, PhysInput, SceneMode, TICK_RATE};
use orr_server::presets::{self, Game, PhysicsScene};
use orr_server::serve::run_wall_clock;
use orr_server::RelayServer;
use orr_session::ClientState;
use orr_testgame::ArenaInput;

struct Live {
    addr: SocketAddr,
    fingerprint: Option<[u8; 32]>,
    kind: TransportKind,
    stop: Arc<AtomicBool>,
    join: JoinHandle<(u64, u64)>,
}

impl Live {
    fn start(kind: TransportKind, game: Game, players: u8, scene: PhysicsScene) -> Live {
        let ep = listen(&ListenOptions::new("127.0.0.1:0".parse().unwrap(), kind)).expect("listen");
        let (addr, fingerprint) = (ep.local_addr(), ep.cert_sha256());
        let mut server = RelayServer::new(ep, 9);
        server.create_room(1, presets::room_config(game, players, TICK_RATE, presets::SAMPLE_SEED, scene));
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = thread::spawn(move || {
            run_wall_clock(&mut server, &flag, Duration::from_millis(1), |s, _| drop(s.drain_notes()));
            let st = server.room_stats(1).expect("room");
            (st.finalized, st.desyncs)
        });
        Live { addr, fingerprint, kind, stop, join }
    }

    fn args(&self, name: &str, sim: Option<SimConditions>) -> NetArgs {
        let mut a = NetArgs { connect: Some(self.addr.to_string()), kind: self.kind, name: name.to_string(), ..NetArgs::default() };
        a.fingerprint = self.fingerprint;
        a.connect_timeout = Duration::from_secs(20);
        if let Some(s) = sim {
            a.sim = s;
        }
        a
    }

    /// Stops the server; returns `(finalized ticks, desyncs)`.
    fn finish(self) -> (u64, u64) {
        self.stop.store(true, Ordering::Relaxed);
        self.join.join().expect("server thread")
    }
}

fn agree(reports: &[ClientReport]) -> usize {
    let mut by_tick: BTreeMap<u64, u64> = BTreeMap::new();
    let mut compared = 0;
    for (i, r) in reports.iter().enumerate() {
        for &(tick, cs) in &r.checksums {
            if let Some(&want) = by_tick.get(&tick) {
                assert_eq!(cs, want, "client {i} differs at tick {tick}");
                compared += 1;
            } else {
                by_tick.insert(tick, cs);
            }
        }
    }
    compared
}

#[test]
fn presets_match_the_sample_games() {
    assert_eq!(presets::ARENA_BUILD_ID, ARENA_BUILD_ID);
    assert_eq!(presets::PHYSICS_BUILD_ID, PHYSICS_BUILD_ID);
    assert_eq!(presets::SAMPLE_INPUT_SIZE as usize, std::mem::size_of::<ArenaInput>());
    assert_eq!(presets::SAMPLE_INPUT_SIZE as usize, std::mem::size_of::<PhysInput>());
    let scene = PhysicsScene { bodies: 777, mode: 2, spawn_rate: 5, max_entities: 4000, layout_seed: 0xABCDEF };
    let cfg = PhysConfig::from_blob(&scene.to_blob(), 4).expect("blob decodes");
    assert_eq!((cfg.bodies, cfg.mode, cfg.spawn_rate, cfg.max_entities, cfg.layout_seed), (777, SceneMode::Mixer, 5, 4000, 0xABCDEF));
    assert_eq!(cfg.paddles, 4);
    assert!(PhysConfig::from_blob(b"nope", 2).is_none());
    assert!(PhysConfig::from_blob(&scene.to_blob()[..20], 2).is_none());
}

/// The arena preset with the sample's own client code, over WebSocket.
#[test]
fn arena_preset_bots_over_websocket() {
    let live = Live::start(TransportKind::Ws, Game::Arena, 2, PhysicsScene::default());
    let handles: Vec<_> = (0..2)
        .map(|i| {
            let args = live.args(&format!("arena{i}"), None);
            thread::spawn(move || run_arena_bot(&args, 3.0).expect("bot"))
        })
        .collect();
    let reports: Vec<ClientReport> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let (finalized, desyncs) = live.finish();
    for r in &reports {
        eprintln!("arena bot: {}", r.summary());
        assert_eq!(r.state, ClientState::Playing);
        assert_eq!(r.desyncs, 0);
        assert!(r.verified_tick > 90);
    }
    assert!(agree(&reports) >= 4);
    assert_eq!((desyncs, finalized > 100), (0, true));
}

/// 4 physics players (60 bodies) at 150 ms round trip with jitter and 2% loss over QUIC.
#[test]
fn four_physics_bots_over_quic_at_150ms_rtt() {
    let scene = PhysicsScene { bodies: 60, ..PhysicsScene::default() };
    let live = Live::start(TransportKind::Quic, Game::Physics, 4, scene);
    let handles: Vec<_> = (0..4u64)
        .map(|i| {
            let sim = SimConditions { latency_ms: 75, jitter_ms: 10, loss: 0.02, seed: 40 + i };
            let args = live.args(&format!("phys{i}"), Some(sim));
            thread::spawn(move || run_physics_bot(&args, 6.0).expect("bot"))
        })
        .collect();
    let reports: Vec<ClientReport> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let (finalized, desyncs) = live.finish();
    for (i, r) in reports.iter().enumerate() {
        eprintln!("physics client {i}: {}", r.summary());
        assert_eq!(r.state, ClientState::Playing, "client {i}");
        assert_eq!(r.desyncs, 0, "client {i}");
        assert!(r.verified_tick >= 150, "client {i} verified {}", r.verified_tick);
    }
    let compared = agree(&reports);
    eprintln!("server finalized {finalized} ticks, desyncs {desyncs}, {compared} checkpoints compared");
    assert!(compared >= 3 * 5, "compared {compared}");
    assert_eq!(desyncs, 0);
}
