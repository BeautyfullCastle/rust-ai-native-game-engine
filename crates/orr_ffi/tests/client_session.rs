//! A multiplayer game through the C ABI: two `OrrHost*` client handles (`orr_client_open`) and one
//! Rust bridge client play together on an in-process `orr_server` over QUIC, with simulated latency
//! and loss. The views see what a Rust view sees: rollbacks (the `rolled_back` flag with a range),
//! events as predicted, then verified or canceled, and session status; all peers agree on the
//! confirmed state.
//!
//! Waits are progress based (the run ends when the ticks have been played), with generous limits.
#![allow(clippy::disallowed_types)] // tests wait on the wall clock

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use orr_bridge::{Bridge, RelayMetrics};
use orr_ffi::*;
use orr_relay_net::{format_fingerprint, listen, ListenOptions, SimConditions, TransportKind};
use orr_sample::net_client::{physics_bridge, NetArgs};
use orr_sample::physics_game::{PhysInput, SHOOT};
use orr_sample::physics_host::SimMetrics;
use orr_sample::physics_stream::PhysKinds;
use orr_sample::physics_view::PhysExtractor;
use orr_server::presets::{self, Game, PhysicsScene};
use orr_server::serve::run_wall_clock;
use orr_server::{RelayServer, RoomStats};
use orr_sim::PlayerSlot;
use orr_view::InterpMode;
use orr_viewstream::{
    EventBatch, FrameEncoder, FrameMeta, ViewFrame, FLAG_ROLLED_BACK, STATE_CANCELED, STATE_PREDICTED, STATE_VERIFIED,
};

mod common;

/// Ticks to play.
const TICKS: u64 = 600;
/// From this tick on every player holds a neutral input, so predictions equal confirmed inputs.
const QUIET_FROM: u64 = 440;
/// Frames of ticks from here are compared with the Rust bridge's verified frames.
const COMPARE_FROM: u64 = QUIET_FROM + 70;
const PLAYERS: u8 = 3;

fn last_error() -> String {
    unsafe { CStr::from_ptr(orr_last_error()) }.to_string_lossy().into_owned()
}

/// A scripted player: a pure function of `(slot, tick)` that changes direction every 9 ticks and
/// shoots in bursts (so a remote player's prediction, "the last input again", is often wrong).
fn script(slot: u8, tick: u64) -> PhysInput {
    if tick >= QUIET_FROM {
        return PhysInput::default();
    }
    let k = (tick / 9 + u64::from(slot) * 5) % 7;
    let shoot = (tick / 12 + u64::from(slot)) % 3 == 0;
    PhysInput::new((k % 3) as i32 - 1, ((k / 2) % 3) as i32 - 1, (k % 2) as i32, shoot)
}

struct Server {
    addr: SocketAddr,
    fingerprint: String,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<RoomStats>>,
}

impl Server {
    fn start(players: u8) -> Server {
        let ep = listen(&ListenOptions::new("127.0.0.1:0".parse().unwrap(), TransportKind::Quic)).expect("listen");
        let (addr, fp) = (ep.local_addr(), ep.cert_sha256().expect("self-signed certificate"));
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
        Server { addr, fingerprint: format_fingerprint(&fp), stop, join: Some(join) }
    }

    fn finish(mut self) -> RoomStats {
        self.stop.store(true, Ordering::Relaxed);
        self.join.take().unwrap().join().expect("server thread")
    }
}

/// One `OrrHost*` client and what its view saw.
struct FfiPeer {
    name: &'static str,
    host: *mut OrrHost,
    slot: Option<u8>,
    frames: u64,
    rolled_back: u64,
    max_depth: u64,
    last_tick: u64,
    last_input_tick: u64,
    /// Entity records (id, kind, cur) of frames at or after `COMPARE_FROM`, by tick.
    late_frames: BTreeMap<u64, Vec<(u64, u16, [f32; 3])>>,
    /// Event states by key, in the order they arrived.
    events: BTreeMap<(u64, u32, u32), Vec<u8>>,
    /// A verified or canceled record whose key had not been announced as predicted.
    orphan: Vec<(u64, u32, u32, u8)>,
    buf: Vec<u8>,
    status: OrrSessionStatus,
}

impl FfiPeer {
    fn open(name: &'static str, server: &Server, latency_ms: u32, jitter_ms: u32, loss_permille: u32, seed: u64) -> FfiPeer {
        let addr = CString::new(server.addr.to_string()).unwrap();
        let fp = CString::new(server.fingerprint.clone()).unwrap();
        let cfg = OrrClientConfig {
            struct_size: std::mem::size_of::<OrrClientConfig>() as u32,
            flags: 0,
            transport: ORR_TRANSPORT_QUIC,
            slot: -1,
            room: 0,
            sim_seed: seed,
            sim_latency_ms: latency_ms,
            sim_jitter_ms: jitter_ms,
            sim_loss_permille: loss_permille,
            connect_timeout_ms: 60_000,
            server: addr.as_ptr(),
            fingerprint: fp.as_ptr(),
            desync_dir: std::ptr::null(),
        };
        let host = unsafe { orr_client_open(&cfg) };
        assert!(!host.is_null(), "{name}: orr_client_open: {}", last_error());
        FfiPeer {
            name,
            host,
            slot: None,
            frames: 0,
            rolled_back: 0,
            max_depth: 0,
            last_tick: 0,
            last_input_tick: 0,
            late_frames: BTreeMap::new(),
            events: BTreeMap::new(),
            orphan: Vec::new(),
            buf: vec![0; 1 << 17],
            status: OrrSessionStatus::default(),
        }
    }

    fn refresh_status(&mut self) {
        let mut s = OrrSessionStatus { struct_size: std::mem::size_of::<OrrSessionStatus>() as u32, ..OrrSessionStatus::default() };
        assert_eq!(unsafe { orr_session_status(self.host, &mut s) }, ORR_OK, "{}: {}", self.name, last_error());
        assert_eq!(s.struct_size as usize, std::mem::size_of::<OrrSessionStatus>());
        assert_eq!(s.mode, ORR_MODE_CLIENT);
        assert_ne!(s.state, ORR_STATE_FAILED, "{}: joining failed: {}", self.name, last_error());
        if s.state == ORR_STATE_PLAYING && self.slot.is_none() {
            self.slot = Some(s.slot as u8);
        }
        self.status = s;
    }

    /// Reads everything the handle has, sets the input for new ticks.
    fn step(&mut self) {
        self.refresh_status();
        let Some(slot) = self.slot else { return };
        loop {
            let mut written = 0usize;
            let rc = unsafe { orr_view_poll(self.host, self.buf.as_mut_ptr(), self.buf.len(), &mut written) };
            match rc {
                ORR_OK => {}
                ORR_NO_FRAME => break,
                ORR_ERR_BUFFER => {
                    self.buf.resize(written, 0);
                    continue;
                }
                other => panic!("{}: orr_view_poll {other}: {}", self.name, last_error()),
            }
            let f = ViewFrame::decode(&self.buf[..written]).unwrap();
            self.frames += 1;
            self.last_tick = f.tick;
            if f.flags & FLAG_ROLLED_BACK != 0 {
                self.rolled_back += 1;
                let (from, to) = f.rollback.expect("the rollback flag comes with a range");
                assert!(from >= 1 && from <= to && to <= f.tick, "{}: rollback range {from}..{to} at tick {}", self.name, f.tick);
                assert!(to - from < 64, "{}: rollback of {} ticks", self.name, to - from + 1);
                self.max_depth = self.max_depth.max(to - from + 1);
            } else {
                assert!(f.rollback.is_none());
            }
            assert!(f.verified_tick <= f.tick);
            if f.tick >= COMPARE_FROM {
                self.late_frames.insert(f.tick, f.entities.iter().map(|e| (e.id, e.kind, e.cur)).collect());
            }
        }
        loop {
            let mut written = 0usize;
            let rc = unsafe { orr_events_poll(self.host, self.buf.as_mut_ptr(), self.buf.len(), &mut written) };
            match rc {
                ORR_OK => {}
                ORR_NO_FRAME => break,
                ORR_ERR_BUFFER => {
                    self.buf.resize(written, 0);
                    continue;
                }
                other => panic!("{}: orr_events_poll {other}: {}", self.name, last_error()),
            }
            for e in EventBatch::decode(&self.buf[..written]).unwrap().events {
                let key = (e.tick, e.system, e.seq);
                let seen = self.events.entry(key).or_default();
                if e.state != STATE_PREDICTED && seen.is_empty() {
                    self.orphan.push((e.tick, e.system, e.seq, e.state));
                }
                seen.push(e.state);
            }
        }
        if self.last_tick > self.last_input_tick || self.last_input_tick == 0 {
            self.last_input_tick = self.last_tick;
            let input = script(slot, self.last_tick);
            let rc = unsafe { orr_set_input(self.host, slot, bytemuck::bytes_of(&input).as_ptr(), std::mem::size_of::<PhysInput>()) };
            assert_eq!(rc, ORR_OK, "{}: orr_set_input: {}", self.name, last_error());
        }
    }

    fn count_events(&self) -> (usize, usize, usize) {
        let predicted_then = |last: u8| self.events.values().filter(|v| v.first() == Some(&STATE_PREDICTED) && v.last() == Some(&last)).count();
        (self.events.len(), predicted_then(STATE_VERIFIED), predicted_then(STATE_CANCELED))
    }
}

impl Drop for FfiPeer {
    fn drop(&mut self) {
        unsafe { orr_host_close(self.host) };
    }
}

#[test]
fn two_c_abi_clients_and_a_rust_client_play_with_prediction_and_rollback() {
    let server = Server::start(PLAYERS);

    // Two views through the C ABI: ~80 ms and ~100 ms round trip, one of them with loss.
    let mut a = FfiPeer::open("ffi-a", &server, 35, 5, 0, 5);
    let mut b = FfiPeer::open("ffi-b", &server, 45, 5, 20, 7);

    // Before the room starts the handle reports it is joining, and the view calls say "not yet".
    a.refresh_status();
    assert_eq!(a.status.state, ORR_STATE_CONNECTING);
    let mut written = 0usize;
    let mut one = [0u8; 8];
    assert_eq!(unsafe { orr_view_poll(a.host, one.as_mut_ptr(), one.len(), &mut written) }, ORR_NO_FRAME);
    assert_eq!(unsafe { orr_schema_json(a.host, std::ptr::null_mut(), 0) }, 0);
    assert!(last_error().contains("joining"), "{}", last_error());
    // Timeline controls and ERP calls are for a local host.
    assert_eq!(unsafe { orr_control(a.host, ORR_CTL_PLAY, 0) }, ORR_ERR_ARG);
    assert!(last_error().contains("client session"), "{}", last_error());

    // The third player is a Rust bridge client (the sample's `--connect` path); it completes the room.
    let mut args = NetArgs {
        connect: Some(server.addr.to_string()),
        fingerprint: Some(orr_relay_net::parse_fingerprint(&server.fingerprint).unwrap()),
        name: "rust".to_string(),
        quiet: true,
        desync_dir: std::env::temp_dir().join("orr_ffi_client_test_desync"),
        ..NetArgs::default()
    };
    args.sim = SimConditions { latency_ms: 40, jitter_ms: 5, loss: 0.0, seed: 9 };
    args.sim_seed = Some(9);
    let metrics = RelayMetrics::new();
    let (mut rust, _scene) = physics_bridge(&args, metrics.clone(), SimMetrics::new()).expect("the Rust client joins");
    let rust_slot = rust.local_slot();

    // Play, progress based: until every peer has played TICKS and the confirmations caught up.
    let mut rust_verified: BTreeMap<u64, Vec<(u64, u16, [f32; 3])>> = BTreeMap::new();
    let mut encoder = FrameEncoder::new(PhysExtractor { remote_mode: InterpMode::Snapshot, local_slot: rust_slot.0 }, PhysKinds);
    let (mut rust_input_tick, mut rust_head) = (0u64, 0u64);
    let started = Instant::now();
    let mut last_progress = (Instant::now(), 0u64);
    let mut desync_seen = false;
    loop {
        a.step();
        b.step();
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
        desync_seen |= a.status.desyncs > 0 || b.status.desyncs > 0 || metrics.status().desyncs > 0;
        let slowest_verified = a.status.verified_tick.min(b.status.verified_tick).min(metrics.status().verified_tick);
        let heads = a.status.head_tick.min(b.status.head_tick).min(rust_head);
        if heads >= TICKS && slowest_verified >= TICKS - 20 {
            break;
        }
        // Progress based limits: the slowest head must keep moving, and the whole run has a ceiling.
        if heads > last_progress.1 {
            last_progress = (Instant::now(), heads);
        }
        assert!(last_progress.0.elapsed() < Duration::from_secs(40), "no progress for 40 s: heads at {heads}");
        assert!(started.elapsed() < Duration::from_secs(240), "the run took more than 240 s");
        assert!(!desync_seen, "a desync was detected");
        thread::sleep(Duration::from_millis(3));
    }
    a.step();
    b.step();
    let rs = metrics.status();

    // Everyone joined different slots and is playing.
    let mut slots = vec![a.slot.unwrap(), b.slot.unwrap(), rust_slot.0];
    slots.sort_unstable();
    assert_eq!(slots, vec![0, 1, 2]);
    for p in [&a, &b] {
        assert_eq!(p.status.state, ORR_STATE_PLAYING, "{}", p.name);
        assert_eq!(p.status.player_count, u32::from(PLAYERS));
        assert_eq!(p.status.desyncs, 0, "{}: desyncs", p.name);
        assert_eq!(p.status.flags & ORR_STATUS_DESYNC, 0);
        assert!(p.status.rtt_ms >= 30 && p.status.rtt_ms < 600, "{}: rtt {} ms", p.name, p.status.rtt_ms);
        assert!(p.status.input_delay >= 1, "{}: input delay {}", p.name, p.status.input_delay);
        assert!(p.status.verified_tick > 0 && p.status.verified_tick <= p.status.head_tick);
        // Rollbacks reached the view: flag and range on a frame, counted by the status, and
        // every resimulated tick is counted (the status counts at least one tick per rollback).
        assert!(p.rolled_back > 0, "{}: no frame had rolled_back set in {} frames", p.name, p.frames);
        assert!(p.status.rollbacks > 0 && p.status.resim_ticks >= p.status.rollbacks, "{}: {:?}", p.name, p.status);
        assert!(p.status.last_rollback_to >= p.status.last_rollback_from && p.status.last_rollback_from > 0);
        // Events: predicted, then verified (final) or canceled (taken back by a rollback).
        let (total, verified, canceled) = p.count_events();
        assert!(p.orphan.is_empty(), "{}: verified/canceled without a predicted record: {:?}", p.name, &p.orphan[..p.orphan.len().min(5)]);
        assert!(verified > 0, "{}: no event went predicted -> verified ({total} events)", p.name);
        assert!(canceled > 0, "{}: no event went predicted -> canceled ({total} events)", p.name);
        // A canceled key is never verified afterwards (and vice versa).
        for (key, states) in &p.events {
            let finals = states.iter().filter(|s| **s != STATE_PREDICTED).count();
            assert!(finals <= 1 || states.iter().filter(|s| **s == STATE_VERIFIED).count() == 0, "{}: {key:?} {states:?}", p.name);
        }
        println!(
            "{}: slot {} rtt {} ms delay {} | {} frames, {} with rolled_back, deepest rollback {} ticks, {} rollbacks / {} resimulated ticks, {} stalls | {total} events: {verified} predicted->verified, {canceled} predicted->canceled",
            p.name, p.status.slot, p.status.rtt_ms, p.status.input_delay, p.frames, p.rolled_back, p.max_depth, p.status.rollbacks, p.status.resim_ticks, p.status.stall_episodes
        );
    }
    assert!(rs.rollbacks > 0 && rs.resim_ticks >= rs.rollbacks, "the Rust client: {rs:?}");

    // The frames the views got at a tick (late in the run, inputs steady) are the records the Rust
    // bridge has for the verified frame of that tick.
    let mut compared = 0;
    for (name, peer) in [("ffi-a", &a), ("ffi-b", &b)] {
        for (tick, records) in &peer.late_frames {
            if let Some(want) = rust_verified.get(tick) {
                assert_eq!(records, want, "{name}: the view frame of tick {tick} differs from the Rust bridge's verified frame");
                compared += 1;
            }
        }
    }
    println!("compared {compared} view frames with the Rust bridge's verified frames (ticks {COMPARE_FROM}..)");
    assert!(compared >= 20, "only {compared} frames could be compared");

    // The confirmed checksums agree: the same state at every checkpoint tick the peers recorded.
    let mut agreed = 0;
    for tick in (30..=TICKS - 30).step_by(30) {
        let sums: Vec<u64> = [&a, &b]
            .iter()
            .map(|p| {
                let (mut found, mut sum) = (0u64, 0u64);
                assert_eq!(unsafe { orr_confirmed_checksum(p.host, tick, &mut found, &mut sum) }, ORR_OK, "{}: no checksum at {tick}", p.name);
                assert_eq!(found, tick);
                sum
            })
            .chain(std::iter::once(metrics.checksum_at(tick).expect("the Rust client's checksum")))
            .collect();
        assert!(sums.iter().all(|s| *s == sums[0]), "confirmed checksums differ at tick {tick}: {sums:x?}");
        agreed += 1;
    }
    println!("confirmed checksums agree at {agreed} checkpoints (ticks 30..{})", TICKS - 30);
    let (mut found, mut sum) = (0u64, 0u64);
    assert_eq!(unsafe { orr_confirmed_checksum(a.host, 0, &mut found, &mut sum) }, ORR_OK);
    assert!(found >= TICKS - 60 && found % 30 == 0);
    assert_eq!(unsafe { orr_confirmed_checksum(a.host, 31, &mut found, &mut sum) }, ORR_NO_FRAME);

    // Close the clients, then the server: nobody desynced.
    drop(a);
    drop(b);
    drop(rust);
    let room = server.finish();
    assert_eq!(room.desyncs, 0, "the server found different checksums");
    assert!(room.finalized >= TICKS, "the room confirmed {} ticks", room.finalized);
    let _ = PlayerSlot(0);
    let _ = SHOOT;
}

#[test]
fn a_client_that_cannot_join_reports_failed() {
    unsafe {
        // Config errors.
        let bad = OrrClientConfig {
            struct_size: 8,
            flags: 0,
            transport: ORR_TRANSPORT_QUIC,
            slot: -1,
            room: 0,
            sim_seed: 0,
            sim_latency_ms: 0,
            sim_jitter_ms: 0,
            sim_loss_permille: 0,
            connect_timeout_ms: 0,
            server: std::ptr::null(),
            fingerprint: std::ptr::null(),
            desync_dir: std::ptr::null(),
        };
        assert!(orr_client_open(&bad).is_null());
        assert!(last_error().contains("struct_size"));
        assert!(orr_client_open(std::ptr::null()).is_null());
        let full = OrrClientConfig { struct_size: std::mem::size_of::<OrrClientConfig>() as u32, ..bad };
        assert!(orr_client_open(&full).is_null());
        assert!(last_error().contains("server"), "{}", last_error());
        // QUIC needs a fingerprint or the insecure flag.
        let addr = CString::new("127.0.0.1:9").unwrap();
        let no_trust = OrrClientConfig { server: addr.as_ptr(), connect_timeout_ms: 3000, flags: ORR_CLIENT_WAIT, ..full };
        assert!(orr_client_open(&no_trust).is_null());
        assert!(last_error().contains("fingerprint") || last_error().contains("trust"), "{}", last_error());
        // Nobody listens there: with ORR_CLIENT_WAIT the open fails (and says why) instead of hanging.
        let nobody = OrrClientConfig { flags: ORR_CLIENT_INSECURE | ORR_CLIENT_WAIT, ..no_trust };
        assert!(orr_client_open(&nobody).is_null(), "a failed join with ORR_CLIENT_WAIT returns null");
        assert!(!last_error().is_empty());
        // Without it the handle comes back at once, joining, and then reports FAILED.
        let h = orr_client_open(&OrrClientConfig { flags: ORR_CLIENT_INSECURE, ..no_trust });
        assert!(!h.is_null(), "{}", last_error());
        let end = Instant::now() + Duration::from_secs(30);
        let mut s = OrrSessionStatus { struct_size: std::mem::size_of::<OrrSessionStatus>() as u32, ..OrrSessionStatus::default() };
        loop {
            assert_eq!(orr_session_status(h, &mut s), ORR_OK);
            if s.state == ORR_STATE_FAILED {
                break;
            }
            assert!(Instant::now() < end, "the join neither succeeded nor failed");
            thread::sleep(Duration::from_millis(20));
        }
        let mut written = 0usize;
        let mut buf = [0u8; 16];
        assert_eq!(orr_view_poll(h, buf.as_mut_ptr(), buf.len(), &mut written), ORR_ERR_HOST);
        assert!(last_error().contains("joining the server failed"), "{}", last_error());
        orr_host_close(h);
    }
}

/// The C client (`tests/c/relay_client.c`) joins a two-player room with a second client that is
/// driven through the C ABI from Rust. Both must print/see the same confirmed state.
#[test]
fn a_c_program_and_a_rust_driven_client_play_together() {
    let Some(lib) = common::ensure_lib() else {
        return common::skip_or_fail("the orr_ffi shared library is not there and `cargo build -p orr_ffi` did not make it");
    };
    let dir = std::env::temp_dir().join(format!("orr_ffi_relay_c_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = match common::compile_c(&dir, &lib, "relay_client") {
        Ok(e) => e,
        Err(why) if why.starts_with("no C compiler") || why.starts_with("cannot run the C compiler") => return common::skip_or_fail(&why),
        Err(why) => panic!("{why}"),
    };
    let server = Server::start(2);
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![lib.dir.clone()];
    paths.extend(std::env::split_paths(&path_var));
    let mut child = Command::new(&exe)
        .args([server.addr.to_string(), server.fingerprint.clone(), "40".to_string(), "10".to_string()])
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("LD_LIBRARY_PATH", &lib.dir)
        .env("DYLD_LIBRARY_PATH", &lib.dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the C client");

    // The other player, through the same C functions, in this process.
    let mut peer = FfiPeer::open("ffi-rust-driven", &server, 30, 5, 0, 5);
    let started = Instant::now();
    let mut last_progress = (Instant::now(), 0u64);
    let c_done = loop {
        peer.step();
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if peer.last_tick > last_progress.1 {
            last_progress = (Instant::now(), peer.last_tick);
        }
        // Joining may take a while (the C process starts and connects); once playing, ticks must keep coming.
        let allowed = if peer.last_tick == 0 { Duration::from_secs(120) } else { Duration::from_secs(60) };
        assert!(last_progress.0.elapsed() < allowed, "no progress for {allowed:?} (tick {})", peer.last_tick);
        assert!(started.elapsed() < Duration::from_secs(240), "the C client did not finish");
        thread::sleep(Duration::from_millis(3));
    };
    let out = child.wait_with_output().unwrap();
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string());
    assert!(c_done.success(), "the C client failed ({c_done:?}):\n{stdout}\n{stderr}");
    println!("{stdout}");
    let line = stdout.lines().find(|l| l.starts_with("RESULT relay")).unwrap_or_else(|| panic!("no RESULT line in:\n{stdout}")).to_string();
    let field = |name: &str| -> String { line.split_whitespace().find_map(|w| w.strip_prefix(&format!("{name}="))).unwrap().to_string() };
    let num = |name: &str| -> u64 { field(name).parse().unwrap() };
    assert!(num("rolled_back") > 0 && num("max_depth") >= 1, "{line}");
    assert!(num("predicted") > 0 && num("verified") > 0, "{line}");
    assert!(stdout.contains("joined: state=1") && stdout.contains("desyncs=0"), "{stdout}");

    // The Rust-driven peer: same state at the same verified tick (it keeps playing until it has it).
    let end = Instant::now() + Duration::from_secs(60);
    let tick: u64 = field("checkpoint").parse().unwrap();
    let sum = loop {
        peer.step();
        let (mut found, mut sum) = (0u64, 0u64);
        if unsafe { orr_confirmed_checksum(peer.host, tick, &mut found, &mut sum) } == ORR_OK {
            break sum;
        }
        assert!(Instant::now() < end, "the Rust-driven peer never confirmed tick {tick}");
        thread::sleep(Duration::from_millis(3));
    };
    assert_eq!(format!("0x{sum:016x}"), field("checksum"), "the C client and the Rust-driven client disagree on tick {tick}");
    assert_eq!(peer.status.desyncs, 0);
    println!("C client and Rust-driven client agree: tick {tick} checksum 0x{sum:016x}");
    drop(peer);
    let room = server.finish();
    assert_eq!(room.desyncs, 0);
    let _ = std::fs::remove_dir_all(&dir);
}
