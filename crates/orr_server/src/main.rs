//! The relay server program: `orr_server`.
//!
//! ```text
//! orr_server [options]
//!   --bind ADDR              listen address (default 0.0.0.0:4433)
//!   --transport quic|ws      (default quic; ws is plain, put TLS in a reverse proxy)
//!   --tls-cert FILE --tls-key FILE   PEM certificate chain and key (QUIC).
//!                            Without them a self-signed development certificate is made
//!                            and its SHA-256 fingerprint is printed for `--trust-fingerprint`.
//!   --tls-name NAME          extra name for the self-signed certificate (repeatable)
//!   --webtransport           QUIC only: the same UDP port also answers browsers over WebTransport
//!                            (ALPN h3). A generated certificate is then ECDSA P-256 valid 13 days,
//!                            the form `serverCertificateHashes` accepts (Chrome, Firefox); Safari needs
//!                            a CA-signed certificate (--tls-cert/--tls-key). See docs/webtransport-trial.md.
//!   --ws-bind ADDR           also accept plain WebSocket clients on this TCP address (browser fallback)
//!   --wss-bind ADDR          also accept wss:// WebSocket clients on this TCP address, with the QUIC certificate
//!                            (browsers on an https:// page; give --tls-cert/--tls-key a CA-signed certificate)
//!
//!   --game arena|physics|custom   what the rooms play (default arena)
//!   --players N              slots per room (default 4)
//!   --min-players N          the room starts when this many are ready (default: players)
//!   --tick-rate HZ           (default 60)
//!   --seed N                 sim seed the clients get (default a fixed sample seed)
//!   --room ID                first room id (default 1)   --rooms N   number of rooms (default 1)
//!   --checksum-interval N    clients report a checksum every N verified ticks (default 30)
//!
//!   physics game:  --bodies N (1000)  --mode rain|pile|mixer  --spawn-rate R  --max-entities N
//!   custom game:   --input-size BYTES  --build-id N (or --any-build)  --config-hex HEX
//!                  (these also override the presets)
//!
//!   --authoritative          authoritative rooms (design 6.1): the server also runs the game's sim on the
//!                            confirmed inputs, referees client checksums (a client with a wrong checksum
//!                            gets a correction snapshot; one that keeps diverging is kicked), serves late
//!                            joiners itself and audits the game state for cheating. Needs a game the
//!                            server knows (--game arena). Relay stays the default. See docs/authoritative.md.
//!   --ai-slot N              (authoritative, repeatable) slot N is played by the server's AI bot, no client
//!                            can take it; its input rides in the confirmed bundles like a human's
//!   --dump-dir DIR           (authoritative) where the server's .orrd desync dumps go (default ./orr_dumps)
//!   --kick-after N           (authoritative) kick a client on its Nth wrong checksum within --kick-window
//!                            (default 3; 0 = never)   --kick-window SECS  (default 60)
//!   --violation-limit N      (authoritative) kick a client after N cheat-check violations (default 5; 0 = log only)
//!
//!   --stats-secs N           print room statistics every N seconds (default 5, 0 = off)
//!   --sim-latency MS --sim-jitter MS --sim-loss P  --sim-seed N   add network conditions on the server side
//!   --run-seconds N          stop by itself (same graceful path as Ctrl+C) after N seconds
//! ```
//!
//! By default the server does not simulate: it collects inputs, confirms each
//! tick at its deadline, and relays. With `--authoritative` it also runs the
//! game's simulation on the confirmed inputs (one tick behind the confirm
//! point) and acts as the checksum referee. What the players must agree on (build hash,
//! input size, seed, game configuration) is the room config it hands out in
//! `Welcome`. Ctrl+C stops it gracefully.
// The core takes its time from the caller; only this shell reads a clock.
#![allow(clippy::disallowed_types)]

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use orr_relay_net::{format_fingerprint, listen, ListenOptions, SimConditions, Tls, TransportKind};
use orr_server::presets::{self, Game, PhysicsScene};
use orr_server::serve::run_wall_clock;
use orr_server::{AuthoritativeConfig, DirDumps, RelayServer, ServerNote};

struct Args {
    bind: std::net::SocketAddr,
    kind: TransportKind,
    tls_cert: Option<std::path::PathBuf>,
    tls_key: Option<std::path::PathBuf>,
    tls_names: Vec<String>,
    webtransport: bool,
    ws_bind: Option<std::net::SocketAddr>,
    wss_bind: Option<std::net::SocketAddr>,
    game: Game,
    players: u8,
    min_players: Option<u8>,
    tick_rate: u32,
    seed: u64,
    room: u64,
    rooms: u64,
    checksum_interval: u32,
    scene: PhysicsScene,
    input_size: Option<u32>,
    build_id: Option<u64>,
    any_build: bool,
    config_hex: Option<String>,
    stats_secs: u64,
    run_seconds: Option<f64>,
    sim: SimConditions,
    authoritative: bool,
    ai_slots: Vec<u8>,
    dump_dir: std::path::PathBuf,
    kick_after: u32,
    kick_window: u32,
    violation_limit: u32,
}

const HELP: &str = include_str!("help.txt");

fn parse<T: std::str::FromStr>(name: &str, v: String) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    v.parse().map_err(|e| format!("{name}: {e}"))
}

fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if s.len() % 2 != 0 {
        return Err("--config-hex needs an even number of hex digits".into());
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|e| format!("--config-hex: {e}"))).collect()
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        bind: "0.0.0.0:4433".parse().unwrap(),
        kind: TransportKind::Quic,
        tls_cert: None,
        tls_key: None,
        tls_names: Vec::new(),
        webtransport: false,
        ws_bind: None,
        wss_bind: None,
        game: Game::Arena,
        players: 4,
        min_players: None,
        tick_rate: 60,
        seed: presets::SAMPLE_SEED,
        room: 1,
        rooms: 1,
        checksum_interval: 30,
        scene: PhysicsScene::default(),
        input_size: None,
        build_id: None,
        any_build: false,
        config_hex: None,
        stats_secs: 5,
        run_seconds: None,
        sim: SimConditions { latency_ms: 0, jitter_ms: 0, loss: 0.0, seed: 0x5EED },
        authoritative: false,
        ai_slots: Vec::new(),
        dump_dir: "orr_dumps".into(),
        kick_after: 3,
        kick_window: 60,
        violation_limit: 5,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--bind" => a.bind = parse("--bind", value("--bind")?)?,
            "--transport" => a.kind = parse("--transport", value("--transport")?)?,
            "--tls-cert" => a.tls_cert = Some(value("--tls-cert")?.into()),
            "--tls-key" => a.tls_key = Some(value("--tls-key")?.into()),
            "--tls-name" => a.tls_names.push(value("--tls-name")?),
            "--webtransport" => a.webtransport = true,
            "--wss-bind" => a.wss_bind = Some(parse("--wss-bind", value("--wss-bind")?)?),
            "--ws-bind" => a.ws_bind = Some(parse("--ws-bind", value("--ws-bind")?)?),
            "--game" => a.game = parse("--game", value("--game")?)?,
            "--players" => a.players = parse("--players", value("--players")?)?,
            "--min-players" => a.min_players = Some(parse("--min-players", value("--min-players")?)?),
            "--tick-rate" => a.tick_rate = parse("--tick-rate", value("--tick-rate")?)?,
            "--seed" => a.seed = parse("--seed", value("--seed")?)?,
            "--room" => a.room = parse("--room", value("--room")?)?,
            "--rooms" => a.rooms = parse("--rooms", value("--rooms")?)?,
            "--checksum-interval" => a.checksum_interval = parse("--checksum-interval", value("--checksum-interval")?)?,
            "--bodies" => a.scene.bodies = parse("--bodies", value("--bodies")?)?,
            "--mode" => {
                a.scene.mode = match value("--mode")?.as_str() {
                    "rain" => 0,
                    "pile" => 1,
                    "mixer" => 2,
                    other => return Err(format!("unknown mode '{other}'")),
                }
            }
            "--spawn-rate" => a.scene.spawn_rate = parse("--spawn-rate", value("--spawn-rate")?)?,
            "--max-entities" => a.scene.max_entities = parse("--max-entities", value("--max-entities")?)?,
            "--input-size" => a.input_size = Some(parse("--input-size", value("--input-size")?)?),
            "--build-id" => a.build_id = Some(parse("--build-id", value("--build-id")?)?),
            "--any-build" => a.any_build = true,
            "--config-hex" => a.config_hex = Some(value("--config-hex")?),
            "--run-seconds" => a.run_seconds = Some(parse("--run-seconds", value("--run-seconds")?)?),
            "--stats-secs" => a.stats_secs = parse("--stats-secs", value("--stats-secs")?)?,
            "--sim-latency" => a.sim.latency_ms = parse("--sim-latency", value("--sim-latency")?)?,
            "--sim-jitter" => a.sim.jitter_ms = parse("--sim-jitter", value("--sim-jitter")?)?,
            "--sim-seed" => a.sim.seed = parse("--sim-seed", value("--sim-seed")?)?,
            "--sim-loss" => a.sim.loss = parse("--sim-loss", value("--sim-loss")?)?,
            "--authoritative" => a.authoritative = true,
            "--ai-slot" => a.ai_slots.push(parse("--ai-slot", value("--ai-slot")?)?),
            "--dump-dir" => a.dump_dir = value("--dump-dir")?.into(),
            "--kick-after" => a.kick_after = parse("--kick-after", value("--kick-after")?)?,
            "--kick-window" => a.kick_window = parse("--kick-window", value("--kick-window")?)?,
            "--violation-limit" => a.violation_limit = parse("--violation-limit", value("--violation-limit")?)?,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown option '{other}' (--help)")),
        }
    }
    if a.tls_cert.is_some() != a.tls_key.is_some() {
        return Err("--tls-cert and --tls-key go together".into());
    }
    if (a.webtransport || a.wss_bind.is_some() || a.ws_bind.is_some()) && a.kind != TransportKind::Quic {
        return Err("--webtransport, --ws-bind and --wss-bind need --transport quic (they add listeners to the QUIC server)".into());
    }
    if a.players == 0 || a.rooms == 0 {
        return Err("--players and --rooms must be at least 1".into());
    }
    if !a.authoritative && !a.ai_slots.is_empty() {
        return Err("--ai-slot needs --authoritative (only the server can play a slot)".into());
    }
    if a.authoritative {
        if a.game != Game::Arena {
            return Err("--authoritative needs a game the server can simulate: only `--game arena` is built in \
                        (a game of your own embeds orr_server and calls RelayServer::create_authoritative_room)"
                .into());
        }
        if a.ai_slots.iter().any(|&s| s >= a.players) || a.ai_slots.len() >= usize::from(a.players) {
            return Err("--ai-slot must be a slot below --players, and at least one slot must stay free for a player".into());
        }
    }
    Ok(a)
}

/// What the log keeps per room.
#[derive(Default)]
struct RoomLog {
    present: Vec<u8>,
    last_repeated: Vec<u64>,
    last_late: Vec<u64>,
}

fn stamp(start: Instant) -> String {
    format!("[{:8.2}s]", start.elapsed().as_secs_f64())
}

fn run() -> Result<(), String> {
    let a = parse_args()?;
    let mut room_cfg = presets::room_config(a.game, a.players, a.tick_rate, a.seed, a.scene);
    room_cfg.checksum_interval = a.checksum_interval;
    room_cfg.min_players_to_start = a.min_players.unwrap_or(a.players).clamp(1, a.players);
    if a.authoritative {
        // The server's own slots never have a client.
        room_cfg.min_players_to_start = room_cfg.min_players_to_start.min(a.players - a.ai_slots.len() as u8).max(1);
    }
    if let Some(n) = a.input_size {
        room_cfg.input_size = n;
        room_cfg.default_input = vec![0; n as usize];
    }
    if let Some(id) = a.build_id {
        room_cfg.build_hash = orr_sim::build_hash_of(id, 0);
    }
    if a.any_build {
        room_cfg.build_hash = 0;
    }
    if let Some(hex) = &a.config_hex {
        room_cfg.config_blob = parse_hex(hex)?;
    }

    let mut lo = ListenOptions::new(a.bind, a.kind);
    lo.tls = match (&a.tls_cert, &a.tls_key) {
        (Some(c), Some(k)) => Tls::Pem { cert_chain: c.clone(), private_key: k.clone() },
        _ => Tls::SelfSigned { extra_names: a.tls_names.clone() },
    };
    lo.webtransport = a.webtransport;
    lo.ws_bind = a.ws_bind;
    lo.wss_bind = a.wss_bind;
    lo.sim = Some(a.sim).filter(SimConditions::is_active);
    let endpoint = listen(&lo)?;
    let addr = endpoint.local_addr();
    let fingerprint = endpoint.cert_sha256();
    let ws_addr = endpoint.ws_addr();
    let wss_addr = endpoint.wss_addr();

    let start = Instant::now();
    println!("{} orr_server listening on {addr} ({})", stamp(start), if a.kind == TransportKind::Quic { "quic" } else { "ws" });
    match fingerprint {
        Some(fp) => println!("{} self-signed certificate, sha256 fingerprint: {}", stamp(start), format_fingerprint(&fp)),
        None if a.kind == TransportKind::Quic => println!("{} certificate from PEM files", stamp(start)),
        None => {}
    }
    if a.sim.is_active() {
        println!("{} server-side network simulation: {:?}", stamp(start), a.sim);
    }

    let mut server = RelayServer::new(endpoint, a.seed ^ 0x0BAD_5EED_0BAD_5EED);
    if a.authoritative {
        server.set_dump_sink(DirDumps(a.dump_dir.clone()));
        println!("{} authoritative rooms: server dumps go to {}", stamp(start), a.dump_dir.display());
    }
    for i in 0..a.rooms {
        let id = a.room + i;
        if a.authoritative {
            let mut cfg = room_cfg.clone();
            cfg.min_players_to_start = cfg.min_players_to_start.min(a.players - a.ai_slots.len() as u8).max(1);
            let auth = AuthoritativeConfig {
                server_slots: a.ai_slots.clone(),
                kick_after_corrections: a.kick_after,
                kick_window_secs: a.kick_window,
                violation_limit: a.violation_limit,
            };
            let sim = presets::arena_sim(a.players, a.tick_rate, a.seed, presets::ARENA_BUILD_ID, &a.ai_slots);
            server.create_authoritative_room(id, cfg, auth, Box::new(sim));
        } else {
            server.create_room(id, room_cfg.clone());
        }
        println!(
            "{} room {id}: game {:?}, {} players (starts at {}), {} Hz, input {} B, build hash {:#x}, seed {:#x}, config {} B",
            stamp(start),
            a.game,
            room_cfg.player_count,
            room_cfg.min_players_to_start,
            room_cfg.tick_rate,
            room_cfg.input_size,
            room_cfg.build_hash,
            room_cfg.seed,
            room_cfg.config_blob.len()
        );
    }
    if a.webtransport {
        println!("{} WebTransport (ALPN h3) shares UDP port {}", stamp(start), addr.port());
    }
    if let Some(ws) = ws_addr {
        println!("{} WebSocket listener on {ws}", stamp(start));
    }
    if let Some(wss) = wss_addr {
        println!("{} secure WebSocket (wss://) listener on {wss}", stamp(start));
    }
    if let Some(fp) = fingerprint {
        println!(
            "{} client: orr_sample --connect {addr} --trust-fingerprint {} (use --insecure-dev on a trusted network)",
            stamp(start),
            format_fingerprint(&fp)
        );
    }
    println!("{} Ctrl+C stops the server", stamp(start));

    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();
    ctrlc::set_handler(move || stop_flag.store(true, Ordering::Relaxed)).map_err(|e| format!("Ctrl+C handler: {e}"))?;

    let mut logs: BTreeMap<u64, RoomLog> = BTreeMap::new();
    let stats_every = (a.stats_secs > 0).then(|| Duration::from_secs(a.stats_secs));
    let mut next_stats = stats_every.map(|d| start + d);
    let (first, last) = (a.room, a.room + a.rooms - 1);
    let hook_stop = stop.clone();
    run_wall_clock(&mut server, &stop, Duration::from_millis(1), |server, _now_us| {
        if a.run_seconds.is_some_and(|s| start.elapsed().as_secs_f64() >= s) {
            hook_stop.store(true, Ordering::Relaxed);
        }
        for note in server.drain_notes() {
            match &note {
                ServerNote::PlayerJoined { room, slot, from_tick } => {
                    let log = logs.entry(*room).or_default();
                    if !log.present.contains(slot) {
                        log.present.push(*slot);
                    }
                    println!(
                        "{} room {room}: slot {slot} joined at tick {from_tick} ({} connected)",
                        stamp(start),
                        log.present.len()
                    );
                }
                ServerNote::PlayerLeft { room, slot, from_tick } => {
                    let log = logs.entry(*room).or_default();
                    log.present.retain(|s| s != slot);
                    println!(
                        "{} room {room}: slot {slot} left at tick {from_tick} ({} connected)",
                        stamp(start),
                        log.present.len()
                    );
                }
                ServerNote::Correction { room, slot, from_tick, tick, bytes } => {
                    println!("{} room {room}: slot {slot} was wrong at tick {from_tick}, sent the server frame of tick {tick} ({bytes} B)", stamp(start));
                }
                ServerNote::Kicked { room, slot, code, reason } => {
                    println!("{} room {room}: KICKED slot {slot} (code {code}): {reason}", stamp(start));
                }
                ServerNote::Violation { room, slot, tick, reason } => {
                    println!("{} room {room}: slot {slot} violated a rule at tick {tick}: {reason}", stamp(start));
                }
                ServerNote::Desync { room, tick, finalized, reports } => {
                    println!("{} room {room}: DESYNC at tick {tick} (found at {finalized}), checksums by slot {reports:x?}", stamp(start));
                }
                other => println!("{} {other:?}", stamp(start)),
            }
        }
        if let Some(t) = next_stats {
            if Instant::now() >= t {
                next_stats = stats_every.map(|d| t + d);
                for id in first..=last {
                    let Some(st) = server.room_stats(id) else { continue };
                    let log = logs.entry(id).or_default();
                    log.last_repeated.resize(st.slots.len(), 0);
                    log.last_late.resize(st.slots.len(), 0);
                    let repeated: Vec<u64> = st.slots.iter().map(|s| s.repeated).collect();
                    let late: Vec<u64> = st.slots.iter().map(|s| s.late_dropped).collect();
                    let d_rep: Vec<u64> = repeated.iter().zip(&log.last_repeated).map(|(n, o)| n - o).collect();
                    let d_late: Vec<u64> = late.iter().zip(&log.last_late).map(|(n, o)| n - o).collect();
                    println!(
                        "{} room {id}: {} tick {} | clients {} | repeated inputs by slot {repeated:?} (+{d_rep:?}) | late inputs {late:?} (+{d_late:?}) | desyncs {}{}",
                        stamp(start),
                        if server.is_running(id) { "running" } else { "waiting" },
                        st.finalized,
                        log.present.len(),
                        st.desyncs,
                        if a.authoritative {
                            format!(" | corrections {} kicks {} violations {} server snapshots {}", st.corrections, st.kicks, st.violations, st.server_snapshots)
                        } else {
                            String::new()
                        }
                    );
                    log.last_repeated = repeated;
                    log.last_late = late;
                }
            }
        }
    });
    println!("{} shutting down", stamp(start));
    for id in first..=last {
        if let Some(st) = server.room_stats(id) {
            println!("{} room {id}: final tick {}, desyncs {}", stamp(start), st.finalized, st.desyncs);
        }
    }
    // Dropping the server drops the endpoint, which closes connections gracefully.
    drop(server);
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.is_empty() => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
