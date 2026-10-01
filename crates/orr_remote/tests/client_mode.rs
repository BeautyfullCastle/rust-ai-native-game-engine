//! Client mode of the ERP host (`orr_remote_host --join`): two in-process hosts and a Rust bridge
//! client play together on an in-process `orr_server` (WebSocket transport) with simulated latency
//! and loss. The views subscribe to the hosts' `viewstream` over WebSocket and drive the game with
//! `sim.input`. They see what the C ABI gives (the same session code): rollbacks (`rolled_back`
//! flag with a range), events as predicted, then verified or canceled, session status, and all
//! peers agree on the confirmed state.
//!
//! Waits are progress based (the run ends when the ticks have been played), with generous limits.
#![allow(clippy::disallowed_types)] // tests wait on the wall clock

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use orr_bridge::{Bridge, RelayMetrics};
use orr_edit::EditorDoc;
use orr_relay_net::{listen, ListenOptions, SimConditions, TransportKind};
use orr_remote::sample::{join_phys_client, phys_types};
use orr_remote::{Auth, ErpClient, ErpServer, Host, ServerConfig};
use orr_sample::net_client::{physics_bridge, NetArgs};
use orr_sample::physics_game::{PhysGame, PhysInput};
use orr_sample::physics_host::SimMetrics;
use orr_sample::physics_stream::PhysKinds;
use orr_sample::physics_view::PhysExtractor;
use orr_server::presets::{self, Game, PhysicsScene};
use orr_server::serve::run_wall_clock;
use orr_server::{RelayServer, RoomStats};
use orr_sim::Simulation;
use orr_view::InterpMode;
use orr_viewstream::{
    message_type, EventBatch, FrameEncoder, FrameMeta, ViewFrame, FLAG_ROLLED_BACK, MSG_EVENTS, MSG_FRAME, STATE_CANCELED, STATE_PREDICTED,
    STATE_VERIFIED,
};
use serde_json::{json, Value as J};

const TICKS: u64 = 600;
const QUIET_FROM: u64 = 440;
const COMPARE_FROM: u64 = QUIET_FROM + 70;
const PLAYERS: u8 = 3;

fn script(slot: u8, tick: u64) -> PhysInput {
    if tick >= QUIET_FROM {
        return PhysInput::default();
    }
    let k = (tick / 9 + u64::from(slot) * 5) % 7;
    let shoot = (tick / 12 + u64::from(slot)) % 3 == 0;
    PhysInput::new((k % 3) as i32 - 1, ((k / 2) % 3) as i32 - 1, (k % 2) as i32, shoot)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct Server {
    addr: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<RoomStats>>,
}

impl Server {
    fn start(players: u8) -> Server {
        let ep = listen(&ListenOptions::new("127.0.0.1:0".parse().unwrap(), TransportKind::Ws)).expect("listen");
        let addr = ep.local_addr();
        let mut server = RelayServer::new(ep, 11);
        let scene = PhysicsScene { bodies: 300, mode: 1, ..PhysicsScene::default() };
        server.create_room(1, presets::room_config(Game::Physics, players, 60, presets::SAMPLE_SEED, scene));
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = thread::spawn(move || {
            run_wall_clock(&mut server, &flag, Duration::from_millis(1), |s, _| {
                let _ = s.drain_notes();
            });
            server.room_stats(1).expect("room")
        });
        Server { addr, stop, join: Some(join) }
    }

    fn finish(mut self) -> RoomStats {
        self.stop.store(true, Ordering::Relaxed);
        self.join.take().unwrap().join().expect("server thread")
    }

    fn args(&self, name: &str, latency_ms: u32, loss: f32, seed: u64) -> NetArgs {
        let mut a = NetArgs {
            connect: Some(self.addr.to_string()),
            kind: TransportKind::Ws,
            name: name.to_string(),
            quiet: true,
            desync_dir: std::env::temp_dir().join("orr_remote_client_mode_desync"),
            ..NetArgs::default()
        };
        a.sim = SimConditions { latency_ms: latency_ms.into(), jitter_ms: 5, loss, seed };
        a.sim_seed = Some(seed);
        a
    }
}

/// What `orr_remote_host --join` does, on a thread: joins, then serves ERP and runs the host loop.
struct JoinedHost {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl JoinedHost {
    fn start(args: NetArgs) -> JoinedHost {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let s = stop.clone();
        let thread = thread::spawn(move || {
            let mut cfg = ServerConfig::new(Auth::DevNoAuth);
            if let Err(e) = join_phys_client(&mut cfg.limits, args) {
                tx.send(Err(e)).unwrap();
                return;
            }
            let doc = EditorDoc::from_yaml("schema: orr.scene/1\nentities: {}\n", phys_types(), Simulation::<PhysGame>::build_registry(), 7).expect("empty scene");
            let server = ErpServer::start(cfg).expect("start server");
            tx.send(Ok(server.url())).unwrap();
            let mut host = Host::<PhysGame>::new(doc, server);
            host.run(&s, Duration::from_micros(500));
        });
        let url = rx.recv_timeout(Duration::from_secs(120)).expect("the host did not join").expect("join failed");
        JoinedHost { url, stop, thread: Some(thread) }
    }
}

impl Drop for JoinedHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A view of one host: what it saw on the stream.
struct View {
    name: &'static str,
    c: ErpClient,
    slot: u8,
    frames: u64,
    rolled_back: u64,
    max_depth: u64,
    last_tick: u64,
    last_input_tick: u64,
    late_frames: BTreeMap<u64, Vec<(u64, u16, [f32; 3])>>,
    events: BTreeMap<(u64, u32, u32), Vec<u8>>,
    orphan: usize,
    status: J,
}

impl View {
    fn open(name: &'static str, url: &str) -> View {
        let mut c = ErpClient::connect_pumped(url, None).unwrap();
        let state = c.call("sim.state", json!({})).unwrap();
        assert_eq!(state["mode"], "client");
        let r = c.call("watch.subscribe", json!({"topics": ["viewstream"], "max_fps": 1000})).unwrap();
        assert_eq!(r["topics"], json!(["viewstream"]));
        let schema = c.wait_notification("watch.viewstream.schema", Duration::from_secs(20)).unwrap().expect("schema first");
        assert_eq!(schema["params"]["game"], "PhysGame");
        assert_eq!(schema["params"]["player_count"], PLAYERS);
        let slot = state["slot"].as_u64().unwrap() as u8;
        View { name, c, slot, frames: 0, rolled_back: 0, max_depth: 0, last_tick: 0, last_input_tick: 0, late_frames: BTreeMap::new(), events: BTreeMap::new(), orphan: 0, status: state }
    }

    fn step(&mut self) {
        self.c.poll().expect("connection");
        while let Some(bytes) = self.c.frames.pop_front() {
            match message_type(&bytes).unwrap() {
                MSG_FRAME => {
                    let f = ViewFrame::decode(&bytes).unwrap();
                    self.frames += 1;
                    self.last_tick = f.tick;
                    if f.flags & FLAG_ROLLED_BACK != 0 {
                        self.rolled_back += 1;
                        let (from, to) = f.rollback.expect("the rollback flag comes with a range");
                        assert!(from >= 1 && from <= to && to <= f.tick, "{}: rollback range {from}..{to} at tick {}", self.name, f.tick);
                        self.max_depth = self.max_depth.max(to - from + 1);
                    } else {
                        assert!(f.rollback.is_none());
                    }
                    assert!(f.verified_tick <= f.tick);
                    if f.tick >= COMPARE_FROM {
                        self.late_frames.insert(f.tick, f.entities.iter().map(|e| (e.id, e.kind, e.cur)).collect());
                    }
                }
                MSG_EVENTS => {
                    for e in EventBatch::decode(&bytes).unwrap().events {
                        let seen = self.events.entry((e.tick, e.system, e.seq)).or_default();
                        if e.state != STATE_PREDICTED && seen.is_empty() {
                            self.orphan += 1;
                        }
                        seen.push(e.state);
                    }
                }
                other => panic!("unexpected message type {other}"),
            }
        }
        if self.last_tick > self.last_input_tick || self.last_input_tick == 0 {
            self.last_input_tick = self.last_tick;
            let input = script(self.slot, self.last_tick);
            let r = self.c.call("sim.input", json!({"player": self.slot, "input": hex(bytemuck::bytes_of(&input))})).unwrap();
            assert_eq!(r["ok"], true);
        }
    }

    fn refresh(&mut self) {
        self.status = self.c.call("session.status", json!({})).unwrap();
    }

    fn n(&self, key: &str) -> u64 {
        self.status[key].as_u64().unwrap_or_else(|| panic!("{key} in {}", self.status))
    }

    fn checksum(&mut self, tick: u64) -> u64 {
        let r = self.c.call("sim.checksum", json!({"tick": tick})).unwrap_or_else(|e| panic!("{}: no checksum at {tick}: {e}", self.name));
        assert_eq!(r["tick"], tick);
        orr_remote::wire::parse_checksum(&r["checksum"]).unwrap()
    }
}

#[test]
fn two_client_mode_hosts_and_a_rust_client_play_with_prediction_and_rollback() {
    let server = Server::start(PLAYERS);
    // Three players join concurrently (the room starts when all are in): two ERP hosts and a Rust bridge client.
    let args_a = server.args("host-a", 35, 0.0, 5);
    let args_b = server.args("host-b", 45, 0.02, 7);
    let args_r = server.args("rust", 40, 0.0, 9);
    let ta = thread::spawn(move || JoinedHost::start(args_a));
    let tb = thread::spawn(move || JoinedHost::start(args_b));
    let metrics = RelayMetrics::new();
    let (mut rust, _scene) = physics_bridge(&args_r, metrics.clone(), SimMetrics::new()).expect("the Rust client joins");
    let (host_a, host_b) = (ta.join().unwrap(), tb.join().unwrap());
    let rust_slot = rust.local_slot();

    let mut a = View::open("a", &host_a.url);
    let mut b = View::open("b", &host_b.url);

    // The edit and timeline methods say they are not for a client; the rest answers.
    for method in ["world.query", "scene.save", "sim.start", "sim.step", "sim.play", "sim.seek", "history.undo", "tx.begin", "proposal.list", "verify.self"] {
        let e = a.c.call_err(method, json!({}));
        assert_eq!(e.kind(), Some("not_in_client_mode"), "{method}: {e}");
        assert!(e.message.contains("client mode"), "{}", e.message);
    }
    let e = a.c.call_err("world.patch", json!({"entity": "x", "component": "y", "value": 1}));
    assert_eq!(e.kind(), Some("not_in_client_mode"));
    let e = a.c.call_err("watch.subscribe", json!({"topics": ["frames"]}));
    assert_eq!(e.kind(), Some("not_in_client_mode"), "{e}");
    let e = a.c.call_err("sim.input", json!({"player": (a.slot + 1) % PLAYERS, "input": hex(&[0u8; 24])}));
    assert!(e.message.contains("only its own input"), "{e}");
    let e = a.c.call_err("sim.input", json!({"player": a.slot, "input": "00"}));
    assert!(e.message.contains("bytes"), "{e}");
    assert!(a.c.call("activity.list", json!({})).is_ok());
    assert!(a.c.call("rpc.discover", json!({})).is_ok());
    assert!(a.c.call_err("session.nope", json!({})).message.contains("unknown"));

    let mut rust_verified: BTreeMap<u64, Vec<(u64, u16, [f32; 3])>> = BTreeMap::new();
    let mut encoder = FrameEncoder::new(PhysExtractor { remote_mode: InterpMode::Snapshot, local_slot: rust_slot.0 }, PhysKinds);
    let (mut rust_input_tick, mut rust_head) = (0u64, 0u64);
    let started = Instant::now();
    let mut progress = (Instant::now(), 0u64);
    loop {
        a.step();
        b.step();
        a.refresh();
        b.refresh();
        if let Some(snap) = rust.snapshot() {
            rust_head = snap.tick();
            if snap.tick() > rust_input_tick || rust_input_tick == 0 {
                rust_input_tick = snap.tick();
                rust.set_input(rust_slot, script(rust_slot.0, snap.tick())).unwrap();
            }
            let vt = snap.verified_tick();
            if vt >= COMPARE_FROM && !rust_verified.contains_key(&vt) {
                if let Some(vf) = snap.verified() {
                    let frame = encoder.encode(vf, None, FrameMeta::default());
                    rust_verified.insert(vt, frame.entities.iter().map(|e| (e.id, e.kind, e.cur)).collect());
                }
            }
        }
        let _ = rust.drain_events();
        assert_eq!(a.n("desyncs") + b.n("desyncs") + metrics.status().desyncs, 0, "a desync was detected");
        let verified = a.n("verified_tick").min(b.n("verified_tick")).min(metrics.status().verified_tick);
        let heads = a.n("head_tick").min(b.n("head_tick")).min(rust_head);
        if heads >= TICKS && verified >= TICKS - 20 {
            break;
        }
        if heads > progress.1 {
            progress = (Instant::now(), heads);
        }
        assert!(progress.0.elapsed() < Duration::from_secs(40), "no progress for 40 s: heads at {heads}");
        assert!(started.elapsed() < Duration::from_secs(240), "the run took more than 240 s");
        thread::sleep(Duration::from_millis(3));
    }
    a.step();
    b.step();
    a.refresh();
    b.refresh();

    let mut slots = vec![a.slot, b.slot, rust_slot.0];
    slots.sort_unstable();
    assert_eq!(slots, vec![0, 1, 2]);
    for v in [&a, &b] {
        assert_eq!(v.status["state"], "playing", "{}", v.name);
        assert_eq!(v.status["mode"], "client");
        assert_eq!(v.n("player_count"), u64::from(PLAYERS));
        assert_eq!(v.n("desyncs"), 0);
        assert!(v.n("rtt_ms") >= 30 && v.n("rtt_ms") < 600, "{}: rtt {}", v.name, v.n("rtt_ms"));
        assert!(v.n("input_delay") >= 1);
        assert!(v.n("rollbacks") > 0 && v.n("resim_ticks") >= v.n("rollbacks"), "{}: {}", v.name, v.status);
        assert!(v.rolled_back > 0, "{}: no frame had rolled_back set in {} frames", v.name, v.frames);
        let predicted_then = |last: u8| v.events.values().filter(|s| s.first() == Some(&STATE_PREDICTED) && s.last() == Some(&last)).count();
        let (verified, canceled) = (predicted_then(STATE_VERIFIED), predicted_then(STATE_CANCELED));
        assert_eq!(v.orphan, 0, "{}: verified/canceled without a predicted record", v.name);
        assert!(verified > 0, "{}: no event went predicted -> verified", v.name);
        assert!(canceled > 0, "{}: no event went predicted -> canceled", v.name);
        println!(
            "{}: slot {} rtt {} ms delay {} | {} frames, {} rolled_back, deepest rollback {}, {} rollbacks / {} resimulated ticks | {} events: {verified} predicted->verified, {canceled} predicted->canceled",
            v.name, v.slot, v.n("rtt_ms"), v.n("input_delay"), v.frames, v.rolled_back, v.max_depth, v.n("rollbacks"), v.n("resim_ticks"), v.events.len()
        );
    }

    // The view frames at a late tick are the records of the Rust bridge's verified frame of that tick.
    let mut compared = 0;
    for peer in [&a, &b] {
        for (tick, records) in &peer.late_frames {
            if let Some(want) = rust_verified.get(tick) {
                assert_eq!(records, want, "{}: the view frame of tick {tick} differs from the Rust bridge's verified frame", peer.name);
                compared += 1;
            }
        }
    }
    println!("compared {compared} frames with the Rust bridge's verified frames");
    assert!(compared >= 20, "only {compared} frames could be compared");

    // Confirmed checksums agree at every checkpoint.
    let mut agreed = 0;
    for tick in (30..=TICKS - 30).step_by(30) {
        let (ca, cb) = (a.checksum(tick), b.checksum(tick));
        let cr = metrics.checksum_at(tick).expect("the Rust client's checksum");
        assert!(ca == cb && ca == cr, "confirmed checksums differ at tick {tick}: {ca:x} {cb:x} {cr:x}");
        agreed += 1;
    }
    println!("confirmed checksums agree at {agreed} checkpoints");
    assert!(a.status["confirmed"]["tick"].as_u64().unwrap() >= TICKS - 60);
    assert_eq!(a.c.call_err("sim.checksum", json!({"tick": 31})).code, orr_remote::NOT_FOUND);

    drop(host_a);
    drop(host_b);
    drop(rust);
    let room = server.finish();
    assert_eq!(room.desyncs, 0);
    // The wait above stops once every peer has verified TICKS - 20, so that is
    // what the server is known to have confirmed (it may not have reached TICKS).
    assert!(room.finalized >= TICKS - 20, "the room confirmed {} ticks", room.finalized);
}

/// The real binary: `orr_remote_host --join` against a one-player room, over its ERP socket.
#[test]
fn the_host_binary_joins_a_room_and_serves_its_stream() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let server = Server::start(1);
    let mut child = Command::new(env!("CARGO_BIN_EXE_orr_remote_host"))
        .args(["--dev-no-auth", "--bind", "127.0.0.1:0", "--join", &server.addr.to_string(), "--ws", "--sim-latency", "10", "--sim-seed", "3"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start orr_remote_host");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut url = None;
    while url.is_none() {
        let line = lines.next().expect("the host exited before serving").unwrap();
        url = line.strip_prefix("orr_remote_host: ERP listening on ").map(str::to_string);
    }
    let mut c = ErpClient::connect_pumped(&url.unwrap(), None).unwrap();
    let st = c.call("session.status", json!({})).unwrap();
    assert_eq!((st["mode"].as_str(), st["slot"].as_u64(), st["player_count"].as_u64()), (Some("client"), Some(0), Some(1)));
    c.call("watch.subscribe", json!({"topics": ["viewstream"], "max_fps": 1000})).unwrap();
    c.wait_notification("watch.viewstream.schema", Duration::from_secs(20)).unwrap().expect("schema");
    let end = Instant::now() + Duration::from_secs(30);
    let mut tick = 0;
    while tick < 120 && Instant::now() < end {
        c.poll().unwrap();
        while let Some(b) = c.frames.pop_front() {
            if message_type(&b).unwrap() == MSG_FRAME {
                tick = ViewFrame::decode(&b).unwrap().tick;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(tick >= 120, "the stream stopped at tick {tick}");
    assert_eq!(c.call_err("scene.load", json!({"text": ""})).kind(), Some("not_in_client_mode"));
    let _ = child.kill();
    let _ = child.wait();
    server.finish();

    // Client options without --join are refused; a server nobody listens on fails with a message.
    let out = Command::new(env!("CARGO_BIN_EXE_orr_remote_host")).args(["--dev-no-auth", "--ws"]).output().unwrap();
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("--join"));
    let out = Command::new(env!("CARGO_BIN_EXE_orr_remote_host"))
        .args(["--dev-no-auth", "--bind", "127.0.0.1:0", "--join", "127.0.0.1:9", "--ws", "--connect-timeout", "1"])
        .output()
        .unwrap();
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("joining the server failed"), "{}", String::from_utf8_lossy(&out.stderr));
}
