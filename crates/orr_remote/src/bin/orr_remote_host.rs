//! `orr_remote_host`: a headless Orrery host that serves ERP.
//!
//! It loads a scene, keeps it in an `EditorDoc` (edit mode, undo history)
//! and serves the Engine Remote Protocol, so AI agents (through an MCP
//! adapter), scripts and `RemoteBridge` views can look at and change it.
//! `sim.start` begins a play session of the selected game (`PhysGame` by
//! default, or Arena with `--game arena`); while it is playing the
//! host ticks it in real time.
//!
//! ```text
//! orr_remote_host [--game physics|arena|terrain-yard3d] [--scene PATH] [--bind ADDR] [--token name:token:caps]... [--dev-no-auth]
//!                 [--seed N] [--players N] [--tick-rate N] [--max-step N] [--build-id N]
//!                 [--join HOST:PORT [--fingerprint HEX | --insecure] [--ws] [--room N] [--slot N]
//!                  [--sim-latency MS] [--sim-jitter MS] [--sim-loss P] [--sim-seed N] [--connect-timeout S]]
//! ```
//!
//! # Client mode (`--join`)
//!
//! Instead of a play session of its own the host is a relay CLIENT of an `orr_server --game physics`
//! room: it predicts, rolls back and reconciles like the C ABI's `orr_client_open` (the same
//! session code), and its `viewstream` topic streams the frames and events of that session. ERP
//! methods: `sim.state` / `session.status` (state, slot, RTT, delay, rollbacks, confirmed
//! checksum), `sim.checksum` (confirmed checksum of a checkpoint tick), `sim.input` and
//! `sim.command` (the joined slot only), `watch.subscribe` of `viewstream` and `activity`,
//! `activity.list`, `rpc.discover`. Scene edit, proposal and timeline methods answer
//! `not_in_client_mode`. The host starts serving ERP once the room has started.
//!
//! Capabilities: `read`, `scene_edit`, `sim_control`, `approve` (comma
//! separated) or `all`. `approve` is what accepts an agent's proposal into
//! the scene: give an agent `read,scene_edit` to let it propose and verify
//! while a person decides.
//! Without `--token` or `--dev-no-auth` the host refuses to start.
//! `--dev-no-auth` is only allowed on a loopback address.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use orr_edit::EditorDoc;
use orr_reflect::TypeRegistry;
use orr_relay_net::{parse_fingerprint, TransportKind};
use orr_remote::sample::join_phys_client;
use orr_remote::{default_build_id, Auth, ErpServer, GameHooks, Host, ServerConfig, TokenEntry, ViewStreamHook};
use orr_sample::net_client::NetArgs;
use orr_sample::physics_game::{bot_input, register_reflect, PhysGame, PhysMetrics};
use orr_sample::physics_stream::phys_stream_source;
use orr_sim::{PlayerSlot, Simulation};

const USAGE: &str = "usage: orr_remote_host [--game physics|arena|terrain-yard3d] [--scene PATH] [--bind ADDR] [--token name:token:caps]... [--dev-no-auth]\n\
                     \x20                       [--seed N] [--players N] [--tick-rate N] [--max-step N] [--build-id N]\n\
                     \x20                       [--join HOST:PORT [--fingerprint HEX | --insecure] [--ws] [--room N] [--slot N]\n\
                     \x20                        [--sim-latency MS] [--sim-jitter MS] [--sim-loss P] [--sim-seed N] [--connect-timeout S]]\n\
                     --join: be a relay CLIENT of `orr_server --game physics` instead of hosting a play session. The ERP viewstream\n\
                     \x20       topic then streams the client session (rollbacks, predicted/verified/canceled events); `session.status` gives\n\
                     \x20       slot, RTT, delay, rollbacks, confirmed checksum; `sim.input`/`sim.command` act for the joined slot; scene edit\n\
                     \x20       and timeline methods are not available. QUIC needs --fingerprint HEX (the server prints it) or --insecure\n\
                     \x20       (development only); --ws uses WebSocket; --sim-* simulate latency/jitter/loss of this client's network.\n\
                     caps: read, scene_edit, sim_control, approve (comma separated) or all\n\
                     default scene: scenes/physics_demo.scene.yaml (arena: scenes/arena_blank.scene.yaml), default bind: 127.0.0.1:7777";

struct Args {
    game: String,
    scene: PathBuf,
    bind: SocketAddr,
    tokens: Vec<TokenEntry>,
    dev: bool,
    seed: u64,
    players: u8,
    tick_rate: u32,
    max_step: u32,
    build_id: Option<u64>,
    /// Client mode: the options of the relay client (`connect` is the server address).
    join: NetArgs,
}

fn default_scene(game: &str) -> PathBuf {
    let file = match game { "arena" => "arena_blank.scene.yaml", "terrain-yard3d" => "terrain_sphere.scene.yaml", _ => "physics_demo.scene.yaml" };
    let rel = PathBuf::from("scenes").join(file);
    if rel.exists() {
        return rel;
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes")).join(file)
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        game: "physics".into(),
        scene: default_scene("physics"),
        bind: SocketAddr::from(([127, 0, 0, 1], 7777)),
        tokens: Vec::new(),
        dev: false,
        seed: 7,
        players: 2,
        tick_rate: 60,
        max_step: 20_000,
        build_id: None,
        join: NetArgs { name: "host".to_string(), quiet: true, ..NetArgs::default() },
    };
    let mut scene_explicit = false;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--game" => a.game = value("--game")?,
            "--scene" => { a.scene = PathBuf::from(value("--scene")?); scene_explicit = true; },
            "--bind" => a.bind = value("--bind")?.parse().map_err(|e| format!("--bind: {e}"))?,
            "--token" => a.tokens.push(TokenEntry::parse(&value("--token")?).map_err(|e| format!("--token: {e}"))?),
            "--dev-no-auth" => a.dev = true,
            "--seed" => a.seed = value("--seed")?.parse().map_err(|e| format!("--seed: {e}"))?,
            "--players" => a.players = value("--players")?.parse().map_err(|e| format!("--players: {e}"))?,
            "--tick-rate" => a.tick_rate = value("--tick-rate")?.parse().map_err(|e| format!("--tick-rate: {e}"))?,
            "--max-step" => a.max_step = value("--max-step")?.parse().map_err(|e| format!("--max-step: {e}"))?,
            "--build-id" => a.build_id = Some(value("--build-id")?.parse().map_err(|e| format!("--build-id: {e}"))?),
            "--join" => a.join.connect = Some(value("--join")?),
            "--fingerprint" => a.join.fingerprint = Some(parse_fingerprint(&value("--fingerprint")?).map_err(|e| format!("--fingerprint: {e}"))?),
            "--insecure" => a.join.insecure_dev = true,
            "--ws" => a.join.kind = TransportKind::Ws,
            "--room" => a.join.room = value("--room")?.parse().map_err(|e| format!("--room: {e}"))?,
            "--slot" => a.join.slot = Some(value("--slot")?.parse().map_err(|e| format!("--slot: {e}"))?),
            "--sim-latency" => a.join.sim.latency_ms = value("--sim-latency")?.parse().map_err(|e| format!("--sim-latency: {e}"))?,
            "--sim-jitter" => a.join.sim.jitter_ms = value("--sim-jitter")?.parse().map_err(|e| format!("--sim-jitter: {e}"))?,
            "--sim-loss" => a.join.sim.loss = value("--sim-loss")?.parse().map_err(|e| format!("--sim-loss: {e}"))?,
            "--sim-seed" => a.join.sim_seed = Some(value("--sim-seed")?.parse().map_err(|e| format!("--sim-seed: {e}"))?),
            "--connect-timeout" => {
                a.join.connect_timeout = Duration::from_secs_f32(value("--connect-timeout")?.parse().map_err(|e| format!("--connect-timeout: {e}"))?)
            }
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown flag '{other}'")),
        }
    }
    if !matches!(a.game.as_str(), "physics" | "arena") && !(cfg!(feature = "terrain-physics") && a.game == "terrain-yard3d") {
        return Err("--game must be physics or arena, or terrain-yard3d with the terrain-physics feature".into());
    }
    if a.game == "terrain-yard3d" && (a.join.connect.is_some() || a.tick_rate != 60) {
        return Err("terrain-yard3d requires local 60 Hz authoring; network join is unsupported".into());
    }
    if a.game == "arena" && a.join.connect.is_some() {
        return Err("--game arena is local authoring only; --join remains physics-only".into());
    }
    if a.game == "arena" && !(1..=8).contains(&a.players) {
        return Err("Arena --players must be 1..=8".into());
    }
    if !scene_explicit { a.scene = default_scene(&a.game); }
    if a.dev && !a.tokens.is_empty() {
        return Err("--dev-no-auth and --token cannot be combined".into());
    }
    if !a.dev && a.tokens.is_empty() {
        return Err("no --token given: pass at least one --token name:token:caps, or --dev-no-auth for local development".into());
    }
    if a.join.connect.is_none() {
        let client_only = a.join.fingerprint.is_some() || a.join.insecure_dev || a.join.kind != TransportKind::Quic || a.join.slot.is_some() || a.join.sim.is_active();
        if client_only {
            return Err("--fingerprint, --insecure, --ws, --slot and --sim-* belong to client mode: add --join HOST:PORT".into());
        }
    }
    Ok(a)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.game == "arena" { return run_arena(args); }
    #[cfg(feature = "terrain-physics")]
    if args.game == "terrain-yard3d" { return run_terrain_yard3d(args); }
    let client_mode = args.join.connect.is_some();
    let text = match std::fs::read_to_string(&args.scene) {
        Ok(t) => t,
        // A client has no scene of its own (the document only exists to keep the host's type complete).
        Err(_) if client_mode => String::from("schema: orr.scene/1\nentities: {}\n"),
        Err(e) => {
            eprintln!("error: cannot read scene {}: {e}", args.scene.display());
            return ExitCode::FAILURE;
        }
    };
    let mut types = TypeRegistry::new();
    register_reflect(&mut types);
    let doc = match EditorDoc::from_yaml(&text, types, Simulation::<PhysGame>::build_registry(), args.seed) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {}: {e}", args.scene.display());
            return ExitCode::FAILURE;
        }
    };
    let entities = doc.scene().entities.len();

    let auth = if args.dev { Auth::DevNoAuth } else { Auth::Tokens(args.tokens.clone()) };
    let mut cfg = ServerConfig::new(auth);
    cfg.bind = args.bind;
    cfg.limits.player_count = args.players;
    cfg.limits.tick_rate = args.tick_rate;
    cfg.limits.max_step_per_call = args.max_step;
    cfg.limits.scene_path = Some(args.scene.clone());
    cfg.limits.build_id = args.build_id.unwrap_or_else(|| default_build_id("PhysGame"));
    cfg.limits.game = GameHooks::new("PhysGame")
        .with_metrics(PhysMetrics)
        .with_bot(|seed, tick, slot| bot_input(seed, tick, PlayerSlot(slot)));
    // The `viewstream` topic (docs/view-stream.md): views that are not Rust, like `orr_tui`, read this.
    cfg.limits.view_stream = Some(ViewStreamHook::new(phys_stream_source(cfg.limits.build_id, args.players)));
    if client_mode {
        let room = args.join.room;
        println!("orr_remote_host: joining room {room} on {} ...", args.join.connect.as_deref().unwrap_or(""));
        let mut join = args.join.clone();
        if args.build_id.is_some() {
            join.build_id = args.build_id;
        }
        if let Err(e) = join_phys_client(&mut cfg.limits, join) {
            eprintln!("error: joining the server failed: {e}");
            return ExitCode::FAILURE;
        }
        println!("orr_remote_host: joined room {room}: client mode ({} players, {} Hz); ERP view stream is the client session", cfg.limits.player_count, cfg.limits.tick_rate);
    }
    let server = match ErpServer::start(cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("orr_remote_host: scene {} ({entities} entities)", args.scene.display());
    println!("orr_remote_host: ERP listening on {}", server.url());
    if args.dev {
        println!("orr_remote_host: DEV MODE, no authentication (loopback only)");
    }
    for t in &args.tokens {
        println!("orr_remote_host: client '{}' may {}", t.client, t.caps);
    }
    let mut host = Host::<PhysGame>::new(doc, server);
    let stop = AtomicBool::new(false);
    host.run(&stop, Duration::from_millis(1));
    ExitCode::SUCCESS
}


fn run_arena(args: Args) -> ExitCode {
    use orr_testgame::{Arena, ArenaMetrics};
    let text = match std::fs::read_to_string(&args.scene) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("error: cannot read scene {}: {e}", args.scene.display());
            return ExitCode::FAILURE;
        }
    };
    let mut types = TypeRegistry::new();
    orr_testgame::register_reflect(&mut types);
    let doc = match EditorDoc::from_yaml(&text, types, Simulation::<Arena>::build_registry(), args.seed) {
        Ok(doc) => doc,
        Err(e) => {
            eprintln!("error: {}: {e}", args.scene.display());
            return ExitCode::FAILURE;
        }
    };
    let entities = doc.scene().entities.len();
    let auth = if args.dev { Auth::DevNoAuth } else { Auth::Tokens(args.tokens.clone()) };
    let mut cfg = ServerConfig::new(auth);
    cfg.bind = args.bind;
    cfg.limits.player_count = args.players;
    cfg.limits.tick_rate = args.tick_rate;
    cfg.limits.max_step_per_call = args.max_step;
    cfg.limits.scene_path = Some(args.scene.clone());
    cfg.limits.build_id = args.build_id.unwrap_or_else(|| default_build_id("Arena"));
    // Scripted verification cannot derive commands yet. Advertise no bot;
    // normal live input and its recorded commands support exact replay.
    cfg.limits.game = GameHooks::new("Arena").with_metrics(ArenaMetrics);
    let mut server = match ErpServer::start(cfg) {
        Ok(server) => server,
        Err(e) => { eprintln!("error: {e}"); return ExitCode::FAILURE; }
    };
    server.set_structured_input::<Arena>("ArenaInput", 8, |slot, input| {
        orr_sample::arena_view::arena_fire_commands(u32::from(slot.0), input)
    });
    server.enable_managed_input();
    println!("orr_remote_host: Arena scene {} ({entities} entities)", args.scene.display());
    println!("orr_remote_host: ERP listening on {}", server.url());
    if args.dev { println!("orr_remote_host: DEV MODE, no authentication (loopback only)"); }
    for token in &args.tokens { println!("orr_remote_host: client '{}' may {}", token.client, token.caps); }
    let mut host = Host::<Arena>::new(doc, server);
    host.run(&AtomicBool::new(false), Duration::from_millis(1));
    ExitCode::SUCCESS
}

#[cfg(feature = "terrain-physics")]
fn run_terrain_yard3d(args: Args) -> ExitCode {
    use orr_remote::terrain_yard3d::{configure_terrain_yard3d, terrain_yard3d_doc_from_path, TerrainYard3D};
    let doc = match terrain_yard3d_doc_from_path(&args.scene) {
        Ok(doc) => doc,
        Err(error) => { eprintln!("error: {error}"); return ExitCode::FAILURE; }
    };
    let auth = if args.dev { Auth::DevNoAuth } else { Auth::Tokens(args.tokens) };
    let mut config = ServerConfig::new(auth);
    config.bind = args.bind;
    config.limits.scene_path = Some(args.scene.clone());
    config.limits.max_step_per_call = args.max_step;
    configure_terrain_yard3d(&mut config.limits);
    let server = match ErpServer::start(config) {
        Ok(server) => server,
        Err(error) => { eprintln!("error: {error}"); return ExitCode::FAILURE; }
    };
    println!("orr_remote_host: TerrainYard3D admitted {} (sphere-only, 60 Hz, seed 42)", args.scene.display());
    println!("orr_remote_host: ERP listening on {}", server.url());
    let mut host = Host::<TerrainYard3D>::new(doc, server);
    host.run(&AtomicBool::new(false), Duration::from_millis(1));
    ExitCode::SUCCESS
}
