//! The browser proof: a headless Chromium running the wasm client (`orr_web`)
//! joins a relay room with a native Rust client, over WebTransport or over the
//! WebSocket fallback. Both play ~600 ticks with scripted bots; the checksums
//! of verified ticks must agree at every checkpoint, no desync, and the browser
//! must have rolled back (so prediction and rollback ran in wasm).
//!
//! The client runs in a Web Worker (`web/worker.js`) and plays the arena or `PhysGame`
//! (300 bodies); the page draws it on a 2D canvas, with `orr_render` on WebGL2, or on
//! WebGPU (`orr_web_gpu`, needs the GPU package from `tools/build_web.sh`), and the test
//! counts the colored pixels of the canvas screenshot. It prints the page's frame times
//! and the worker's sim times.
//!
//! Needs Node.js with Playwright (`NODE_PATH` or a global install; a Chromium in
//! `PLAYWRIGHT_BROWSERS_PATH`, or `CHROMIUM=/path/to/chrome`) and the wasm package:
//!
//! ```text
//! rustup target add wasm32-unknown-unknown
//! cargo install wasm-bindgen-cli --version <wasm-bindgen of Cargo.lock> --locked
//! tools/build_web.sh
//! cargo test -p orr_server --release --test browser_e2e -- --nocapture --test-threads=1
//! ```
//!
//! Without them each test prints `SKIP` and passes, unless `ORR_REQUIRE_BROWSER=1`
//! is set, which makes a missing piece a failure.
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)]

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use common::{arena_room, arena_script};
use orr_relay_net::{
    connect, drive, format_fingerprint, listen, ClientReport, ConnectOptions, DriveOptions, ListenOptions, SimConditions,
    TransportKind, Trust,
};
use orr_server::serve::run_wall_clock;
use orr_server::RelayServer;
use orr_session::{ClientState, DumpCollector, RelayClient, RelayClientConfig};
use orr_games::physics_game::{bot_input, NoCommand, PhysConfig, PhysGame};
use orr_server::presets::{room_config, Game as PresetGame, PhysicsScene, PHYSICS_BUILD_ID};
use orr_sim::PlayerSlot;
use orr_testgame::{Arena, ArenaConfig};

const ROOM: u64 = 1;
// Intentionally not representable as a JS Number. A successful handshake proves
// the page -> worker -> wasm constructor kept the supplied text's low bits.
const ARENA_TEST_BUILD: u64 = (1 << 53) + 1;

/// One browser run at a time: each starts a Chromium and plays in real time.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
const TICKS: u64 = 600;

/// What the page draws with (`render=` of the page) and the Chromium flags that view needs.
#[derive(Clone, Copy)]
struct View {
    render: &'static str,
    flags: &'static str,
}

/// The 2D canvas.
const CANVAS_2D: View = View { render: "2d", flags: "" };
/// `orr_render` on WebGL2 with an explicit software GPU, including on Linux.
/// These flags apply only to this fresh browser running trusted local fixtures.
const WEBGL2: View = View {
    render: "webgl",
    flags: "--use-gl=angle --use-angle=swiftshader --enable-unsafe-swiftshader",
};
/// `orr_render` on WebGPU: headless Chromium needs these flags and runs it on SwiftShader.
const WEBGPU: View = View {
    render: "webgpu",
    flags: "--enable-unsafe-webgpu --enable-unsafe-swiftshader --use-webgpu-adapter=swiftshader --enable-features=Vulkan --use-vulkan=swiftshader --use-angle=swiftshader",
};

/// The game both clients play.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TestGame {
    Arena,
    Phys,
}

fn require() -> bool {
    ["ORR_REQUIRE_BROWSER", "ORR_REQUIRE_WEBGPU"].iter().any(|key| std::env::var(key).is_ok_and(|v| v == "1"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// What the browser client printed.
#[derive(Debug)]
struct Browser {
    fields: BTreeMap<String, String>,
    checksums: BTreeMap<u64, u64>,
}

impl Browser {
    fn num(&self, k: &str) -> u64 {
        self.fields.get(k).unwrap_or_else(|| panic!("no field {k}")).parse().unwrap_or_else(|_| panic!("field {k} is not a number"))
    }
    fn text(&self, k: &str) -> String {
        self.fields.get(k).cloned().unwrap_or_default().trim_matches('"').to_string()
    }
}

fn parse_browser(stdout: &str) -> Browser {
    let line = stdout.lines().find_map(|l| l.strip_prefix("BROWSER ")).expect("BROWSER line");
    let mut fields = BTreeMap::new();
    // key=value pairs; values may be quoted JSON strings without spaces.
    for part in line.split(' ') {
        if let Some((k, v)) = part.split_once('=') {
            fields.insert(k.to_string(), v.to_string());
        }
    }
    let mut checksums = BTreeMap::new();
    let sums = stdout.lines().find_map(|l| l.strip_prefix("CHECKSUMS ")).unwrap_or("");
    for pair in sums.split(',').filter(|p| !p.is_empty()) {
        let (t, c) = pair.split_once(':').expect("tick:hex");
        checksums.insert(t.parse().unwrap(), u64::from_str_radix(c, 16).unwrap());
    }
    Browser { fields, checksums }
}

struct Run {
    /// The view the page ended up with: `2d`, `webgl` or `webgpu`.
    view_kind: String,
    /// Pixels of the canvas screenshot that are clearly colored (bodies, not background).
    colored_pixels: usize,
    browser: Browser,
    native: ClientReport,
    desyncs_on_server: u64,
}

/// Starts a 2-player room on real sockets (QUIC, optionally WebTransport on the
/// same port, plus WebSocket), joins it with a native bot and the browser.
/// `mode` is the browser's transport mode. Returns `None` when the browser part is skipped.
fn run(game: TestGame, webtransport: bool, mode: &str, wss: bool, view: View) -> Option<Run> {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut lo = ListenOptions::new("127.0.0.1:0".parse().unwrap(), TransportKind::Quic);
    lo.webtransport = webtransport;
    lo.ws_bind = Some("127.0.0.1:0".parse().unwrap());
    lo.wss_bind = wss.then(|| "127.0.0.1:0".parse().unwrap());
    // Some delay and jitter so that the clients really predict and roll back.
    lo.sim = Some(SimConditions { latency_ms: 35, jitter_ms: 8, loss: 0.0, seed: 11 });
    let ep = listen(&lo).expect("listen");
    let (addr, ws_addr, fp) = (ep.local_addr(), ep.ws_addr().expect("ws listener"), ep.cert_sha256().expect("self-signed"));
    let wss_addr = ep.wss_addr();
    let mut server = RelayServer::new(ep, 7);
    server.create_room(
        ROOM,
        match game {
            TestGame::Arena => {
                let mut room = arena_room(2);
                room.build_hash = orr_sim::build_hash_of(ARENA_TEST_BUILD, 0);
                room
            },
            // 300 bodies that keep moving (rotating bars), so both clients mispredict and roll back with
            // physics in the resimulation.
            TestGame::Phys => {
                let scene = PhysicsScene { bodies: 300, mode: 2, ..PhysicsScene::default() };
                room_config(PresetGame::Physics, 2, 60, 0x5EED_0A2E, scene)
            }
        },
    );
    let stop_server = Arc::new(AtomicBool::new(false));
    let flag = stop_server.clone();
    let server_thread = thread::spawn(move || {
        run_wall_clock(&mut server, &flag, Duration::from_millis(1), |_, _| {});
        server.room_stats(ROOM).expect("room")
    });

    // The native client.
    let stop_native = Arc::new(AtomicBool::new(false));
    let native = {
        let stop = stop_native.clone();
        thread::spawn(move || {
            let opts = ConnectOptions::new(addr.to_string(), TransportKind::Quic, Trust::Fingerprint(fp));
            let link = connect(&opts).expect("connect");
            let opts = DriveOptions {
                connect_timeout: Duration::from_secs(60),
                stop: Some(stop),
                tag: "native".into(),
                ..DriveOptions::default()
            };
            match game {
                TestGame::Arena => {
                    let cfg = RelayClientConfig::new(ROOM, ARENA_TEST_BUILD);
                    let mut client: RelayClient<Arena, _> =
                        RelayClient::new(cfg, link, |w| ArenaConfig { player_count: w.player_count }, DumpCollector::new());
                    drive(&mut client, &mut |slot, tick| arena_script(usize::from(slot), tick), &opts)
                }
                TestGame::Phys => {
                    let cfg = RelayClientConfig::new(ROOM, PHYSICS_BUILD_ID);
                    let mut client: RelayClient<PhysGame, _> = RelayClient::new(
                        cfg,
                        link,
                        |w| PhysConfig::from_blob(&w.config, w.player_count).expect("physics scene"),
                        DumpCollector::new(),
                    );
                    // The same scripted player as the browser's bot (`PhysClient`, seed 1234).
                    drive(&mut client, &mut |slot, tick| (bot_input(1234, tick, PlayerSlot(slot)), Vec::<NoCommand>::new()), &opts)
                }
            }
        })
    };

    // The browser, through Node + Playwright.
    // wss: the page connects to wss://127.0.0.1 and Chromium is told to accept the
    // server's self-signed certificate (a real deployment uses a CA certificate).
    let (ws_target, wss_flag) = match wss_addr {
        Some(a) => (format!("wss://127.0.0.1:{}/", a.port()), "--ignore-certificate-errors"),
        None => (ws_addr.port().to_string(), ""),
    };
    let extra_flags = std::env::var("CHROMIUM_FLAGS").unwrap_or_default();
    let flags = format!("{wss_flag} {} {extra_flags}", view.flags);
    let shot = std::env::temp_dir().join(format!("orr_browser_e2e_{}_{}.png", std::process::id(), view.render));
    let mut cmd = Command::new("node");
    if !flags.trim().is_empty() {
        cmd.env("CHROMIUM_FLAGS", flags.trim());
    }
    if game == TestGame::Arena {
        cmd.env("BUILD", if wss { format!("0x{ARENA_TEST_BUILD:x}") } else { ARENA_TEST_BUILD.to_string() });
    }
    let out = cmd
        .arg(repo_root().join("tools/webtransport/browser_e2e.cjs"))
        .env("GAME", if game == TestGame::Arena { "arena" } else { "phys" })
        .env("UDP", addr.port().to_string())
        .env("WS", ws_target)
        .env("HASH", format_fingerprint(&fp))
        .env("MODE", mode)
        .env("RENDER", view.render)
        .env("SCREENSHOT", &shot)
        .env("ROOM", ROOM.to_string())
        .env("TICKS", TICKS.to_string())
        .output();
    stop_native.store(true, Ordering::Relaxed);
    let native = native.join().expect("native client thread");
    stop_server.store(true, Ordering::Relaxed);
    let room = server_thread.join().expect("server thread");

    let out = match out {
        Ok(o) => o,
        Err(e) => {
            assert!(!require(), "cannot run node: {e}");
            eprintln!("SKIP: cannot run node ({e})");
            return None;
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr);
    eprintln!("---- browser output ----\n{stdout}{stderr}------------------------");
    if out.status.code() == Some(2) {
        assert!(!require(), "browser prerequisites are mandatory, but the browser runner skipped:\n{stdout}{stderr}");
        eprintln!("SKIP: the browser part is not available here (set ORR_REQUIRE_BROWSER=1 to fail instead)");
        return None;
    }
    assert!(out.status.success(), "browser client failed ({:?}):\n{stdout}{stderr}", out.status);
    let view_kind = stdout
        .lines()
        .find_map(|l| l.strip_prefix("RENDERER "))
        .and_then(|l| l.split(' ').find_map(|p| p.strip_prefix("kind=")))
        .unwrap_or("\"?\"")
        .trim_matches('"')
        .to_string();
    let colored_pixels = colored_pixels(&shot);
    let _ = std::fs::remove_file(&shot);
    Some(Run { view_kind, colored_pixels, browser: parse_browser(&stdout), native, desyncs_on_server: room.desyncs })
}

/// Pixels of a PNG whose channels differ by more than 60 (the bodies are saturated colors on a dark
/// background; the HUD of the page is not in the canvas).
fn colored_pixels(path: &std::path::Path) -> usize {
    let Ok(file) = std::fs::File::open(path) else { return 0 };
    let mut reader = match png::Decoder::new(std::io::BufReader::new(file)).read_info() {
        Ok(r) => r,
        Err(_) => return 0,
    };
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let Ok(info) = reader.next_frame(&mut buf) else { return 0 };
    let ch = info.color_type.samples();
    buf[..info.buffer_size()]
        .chunks_exact(ch)
        .filter(|p| {
            let rgb = &p[..3.min(ch)];
            rgb.iter().max().copied().unwrap_or(0) as i32 - rgb.iter().min().copied().unwrap_or(0) as i32 > 60
        })
        .count()
}

/// The page used `kind` (`2d`, `webgl`, `webgpu`) and really drew colored bodies into its canvas.
fn check_view(run: &Run, kind: &str) {
    eprintln!("view: {} ({} colored pixels in the canvas)", run.view_kind, run.colored_pixels);
    if kind != "2d" && !repo_root().join("crates/orr_web/web/pkg_gpu/orr_web_gpu.js").exists() && !require() {
        eprintln!("SKIP view check: the GPU view package is not built (tools/build_web.sh without WEB_GPU=0)");
        return;
    }
    if kind == "webgpu" && run.view_kind != "webgpu" && !std::env::var("ORR_REQUIRE_WEBGPU").is_ok_and(|v| v == "1") {
        eprintln!("SKIP view check: this Chromium has no WebGPU adapter (set ORR_REQUIRE_WEBGPU=1 to fail instead)");
        return;
    }
    assert_eq!(run.view_kind, kind, "the page's view");
    assert!(run.colored_pixels >= 200, "the {kind} view drew only {} colored pixels", run.colored_pixels);
}

fn check(run: &Run, transport: &str) {
    let b = &run.browser;
    eprintln!("native : {}", run.native.summary());
    eprintln!(
        "browser: transport {} slot {} rollbacks {} (resim {} ticks, depth {}) verified {} head {} desyncs {} rtt {} us delay {}",
        b.text("transport"),
        b.num("slot"),
        b.num("rollbacks"),
        b.num("resim_ticks"),
        b.num("max_prediction_depth"),
        b.num("verified"),
        b.num("head"),
        b.num("desyncs"),
        b.num("rtt_us"),
        b.num("delay")
    );
    assert_eq!(b.text("transport"), transport, "browser transport");
    assert_eq!(b.text("state"), "Playing");
    assert_eq!(b.num("desyncs"), 0, "browser saw a desync");
    assert_eq!(b.num("decode_errors"), 0);
    assert_eq!(run.native.desyncs, 0, "native client saw a desync");
    assert_eq!(run.desyncs_on_server, 0, "server saw a desync");
    assert!(b.num("verified") >= TICKS, "browser verified only {} ticks", b.num("verified"));
    assert!(b.num("rollbacks") > 0, "no rollback happened in the browser");
    assert_ne!(b.num("slot"), u64::from(run.native.slot.unwrap_or(99)), "both clients got the same slot");

    // Every checkpoint both clients verified must carry the same checksum.
    let native: BTreeMap<u64, u64> = run.native.checksums.iter().copied().collect();
    let mut compared = 0;
    for (tick, sum) in &b.checksums {
        if let Some(n) = native.get(tick) {
            assert_eq!(n, sum, "browser and native checksums differ at tick {tick}");
            compared += 1;
        }
    }
    eprintln!("compared {compared} verified checkpoints (browser has {}, native {}): all equal", b.checksums.len(), native.len());
    assert!(compared >= 15, "only {compared} checkpoints compared");
    assert_eq!(run.native.state, ClientState::Playing);
}

#[test]
fn browser_joins_over_webtransport() {
    if let Some(r) = run(TestGame::Arena, true, "webtransport", false, CANVAS_2D) {
        check(&r, "webtransport");
        check_view(&r, "2d");
    }
}

/// WebTransport is off on the server (QUIC `h3` is refused), the browser starts in
/// `auto` mode, fails to connect over WebTransport and falls back to WebSocket.
#[test]
fn browser_falls_back_to_websocket() {
    if let Some(r) = run(TestGame::Arena, false, "auto", false, WEBGL2) {
        check(&r, "websocket");
        check_view(&r, "webgl");
    }
}

/// `wss://` (TLS on the WebSocket, the certificate of the QUIC server).
#[test]
fn browser_joins_over_secure_websocket() {
    if let Some(r) = run(TestGame::Arena, false, "websocket", true, CANVAS_2D) {
        check(&r, "websocket");
    }
}

/// The physics sample (`PhysGame`, 300 bodies) in the browser, in the Web Worker, against a native client.
#[test]
fn browser_plays_the_physics_game_over_webtransport() {
    if let Some(r) = run(TestGame::Phys, true, "webtransport", false, WEBGL2) {
        check(&r, "webtransport");
        check_view(&r, "webgl");
    }
}

/// Same over the WebSocket fallback.
#[test]
fn browser_plays_the_physics_game_over_websocket() {
    if let Some(r) = run(TestGame::Phys, false, "websocket", false, CANVAS_2D) {
        check(&r, "websocket");
        check_view(&r, "2d");
    }
}

/// The same physics game drawn with `orr_render` on WebGPU (SwiftShader in headless Chromium).
#[test]
fn browser_plays_the_physics_game_drawn_on_webgpu() {
    if let Some(r) = run(TestGame::Phys, true, "webtransport", false, WEBGPU) {
        check(&r, "webtransport");
        check_view(&r, "webgpu");
    }
}
