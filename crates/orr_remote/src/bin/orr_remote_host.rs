//! `orr_remote_host`: a headless Orrery host that serves ERP.
//!
//! It loads a scene, keeps it in an `EditorDoc` (edit mode, undo history)
//! and serves the Engine Remote Protocol, so AI agents (through an MCP
//! adapter), scripts and `RemoteBridge` views can look at and change it.
//! `sim.start` begins a play session of `PhysGame`; while it is playing the
//! host ticks it in real time.
//!
//! ```text
//! orr_remote_host [--scene PATH] [--bind ADDR] [--token name:token:caps]... [--dev-no-auth]
//!                 [--seed N] [--players N] [--tick-rate N] [--max-step N] [--build-id N]
//! ```
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
use orr_remote::{default_build_id, Auth, ErpServer, GameHooks, Host, ServerConfig, TokenEntry, ViewStreamHook};
use orr_sample::physics_game::{bot_input, register_reflect, PhysGame, PhysMetrics};
use orr_sample::physics_stream::phys_stream_source;
use orr_sim::{PlayerSlot, Simulation};

const USAGE: &str = "usage: orr_remote_host [--scene PATH] [--bind ADDR] [--token name:token:caps]... [--dev-no-auth]\n\
                     \x20                       [--seed N] [--players N] [--tick-rate N] [--max-step N] [--build-id N]\n\
                     caps: read, scene_edit, sim_control, approve (comma separated) or all\n\
                     default scene: scenes/physics_demo.scene.yaml, default bind: 127.0.0.1:7777";

struct Args {
    scene: PathBuf,
    bind: SocketAddr,
    tokens: Vec<TokenEntry>,
    dev: bool,
    seed: u64,
    players: u8,
    tick_rate: u32,
    max_step: u32,
    build_id: Option<u64>,
}

fn default_scene() -> PathBuf {
    let rel = PathBuf::from("scenes/physics_demo.scene.yaml");
    if rel.exists() {
        return rel;
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes/physics_demo.scene.yaml"))
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        scene: default_scene(),
        bind: SocketAddr::from(([127, 0, 0, 1], 7777)),
        tokens: Vec::new(),
        dev: false,
        seed: 7,
        players: 2,
        tick_rate: 60,
        max_step: 20_000,
        build_id: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--scene" => a.scene = PathBuf::from(value("--scene")?),
            "--bind" => a.bind = value("--bind")?.parse().map_err(|e| format!("--bind: {e}"))?,
            "--token" => a.tokens.push(TokenEntry::parse(&value("--token")?).map_err(|e| format!("--token: {e}"))?),
            "--dev-no-auth" => a.dev = true,
            "--seed" => a.seed = value("--seed")?.parse().map_err(|e| format!("--seed: {e}"))?,
            "--players" => a.players = value("--players")?.parse().map_err(|e| format!("--players: {e}"))?,
            "--tick-rate" => a.tick_rate = value("--tick-rate")?.parse().map_err(|e| format!("--tick-rate: {e}"))?,
            "--max-step" => a.max_step = value("--max-step")?.parse().map_err(|e| format!("--max-step: {e}"))?,
            "--build-id" => a.build_id = Some(value("--build-id")?.parse().map_err(|e| format!("--build-id: {e}"))?),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown flag '{other}'")),
        }
    }
    if a.dev && !a.tokens.is_empty() {
        return Err("--dev-no-auth and --token cannot be combined".into());
    }
    if !a.dev && a.tokens.is_empty() {
        return Err("no --token given: pass at least one --token name:token:caps, or --dev-no-auth for local development".into());
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
    let text = match std::fs::read_to_string(&args.scene) {
        Ok(t) => t,
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
