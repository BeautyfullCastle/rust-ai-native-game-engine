//! Runs the physics stress scene in a window, or headless as a benchmark.
//!
//! ```text
//! cargo run -p orr_sample --release --bin physics -- [options]
//!   --bodies N                   dynamic bodies at the start (default 1000)
//!   --mode rain|pile|mixer       start layout (default rain)
//!   --spawn-rate R               keep adding R bodies per second (default 0)
//!   --max-entities N             stop spawning at this many entities (default 20000)
//!   --bridge threaded|inproc     where the sim runs (default threaded)
//!   --latency TICKS              one-way loopback latency (default 6)
//!   --jitter TICKS               loopback jitter (default 2)
//!   --remote snapshot|prediction|none   how the bot's paddle is shown (default prediction)
//!   --tau SECONDS                rollback smoothing time constant (default 0.12, 0 = off)
//!   --no-vsync                   do not wait for the display (measure raw speed)
//!   --seconds N                  close after N seconds and print a summary
//!   --headless                   no window, no GPU: run the sim as fast as possible
//!   --ticks N                    headless: stop after N sim steps instead of --seconds
//! ```
//! Play on a relay server (`orr_server --game physics`) instead of the local
//! loopback with `--connect HOST:PORT`; the scene comes from the server. With
//! `--headless --bot` a scripted bot plays (for `--seconds`, default 30):
//! ```text
//!   --connect HOST:PORT  --transport quic|ws  --trust-fingerprint HEX | --insecure-dev
//!   --room N  --slot N  --name TEXT  --sim-latency MS  --sim-jitter MS  --sim-loss P  --sim-seed N
//!   --desync-dir DIR  --bot  --connect-timeout SECONDS
//! ```
//! Keys: WASD or arrows move your paddle (blue), Q/E turn it, space shoots
//! balls, Escape quits. The other paddle (orange) is a bot. The window title
//! shows fps, sim cost, ticks and rollbacks; a summary prints on exit.
//! Headless runs default to 8 seconds.
use std::process::ExitCode;

use orr_bridge::{Bridge, InProc, RelayMetrics, Threaded, ThreadedConfig};
use orr_sample::app::Options;
use orr_sample::arena_view::Loopback;
use orr_sample::net_client::{physics_bridge, print_bot_report, print_relay_status, run_physics_bot, NetArgs};
use orr_sample::physics_app::{print_summary, run_headless, run_window, HeadlessLimit, PhysSummary};
use orr_sample::physics_game::{PhysConfig, PhysGame, SceneMode};
use orr_sample::physics_host::{physics_bridge_config, physics_pair, SimMetrics};
use orr_view::{InterpMode, ViewConfig};

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse<T: std::str::FromStr>(name: &str, v: String) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    v.parse().map_err(|e| format!("{name}: {e}"))
}

fn real_main() -> Result<(), String> {
    let mut threaded = true;
    let mut net = Loopback::default();
    let mut netargs = NetArgs::default();
    let mut scene = PhysConfig::new(1000, SceneMode::Rain);
    let mut headless = false;
    let mut ticks: Option<u64> = None;
    let mut opts = Options {
        label: String::new(),
        vsync: true,
        seconds: None,
        remote_mode: InterpMode::Prediction,
        view: ViewConfig::default(),
        relay: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        if netargs.parse_option(&arg, &mut |name| value(name))? {
            continue;
        }
        match arg.as_str() {
            "--bodies" => scene.bodies = parse("--bodies", value("--bodies")?)?,
            "--mode" => {
                scene.mode = match value("--mode")?.as_str() {
                    "rain" => SceneMode::Rain,
                    "pile" => SceneMode::Pile,
                    "mixer" => SceneMode::Mixer,
                    other => return Err(format!("unknown mode '{other}'")),
                }
            }
            "--spawn-rate" => scene.spawn_rate = parse("--spawn-rate", value("--spawn-rate")?)?,
            "--max-entities" => scene.max_entities = parse("--max-entities", value("--max-entities")?)?,
            "--bridge" => {
                threaded = match value("--bridge")?.as_str() {
                    "threaded" => true,
                    "inproc" => false,
                    other => return Err(format!("unknown bridge '{other}'")),
                }
            }
            "--latency" => net.latency_ticks = parse("--latency", value("--latency")?)?,
            "--jitter" => net.jitter_ticks = parse("--jitter", value("--jitter")?)?,
            "--remote" => {
                opts.remote_mode = match value("--remote")?.as_str() {
                    "snapshot" => InterpMode::Snapshot,
                    "prediction" => InterpMode::Prediction,
                    "none" => InterpMode::None,
                    other => return Err(format!("unknown remote mode '{other}'")),
                }
            }
            "--tau" => opts.view.correction_tau = parse("--tau", value("--tau")?)?,
            "--no-vsync" => opts.vsync = false,
            "--seconds" => opts.seconds = Some(parse("--seconds", value("--seconds")?)?),
            "--headless" => headless = true,
            "--ticks" => ticks = Some(parse("--ticks", value("--ticks")?)?),
            other => return Err(format!("unknown option '{other}' (see the header of this file)")),
        }
    }
    if scene.bodies == 0 {
        return Err("--bodies must be at least 1".to_string());
    }

    if netargs.connect.is_some() {
        if headless {
            if !netargs.bot {
                return Err("--headless needs --bot (nobody is at the keyboard)".to_string());
            }
            let report = run_physics_bot(&netargs, opts.seconds.unwrap_or(30.0))?;
            print_bot_report(&netargs.name, &report);
            return Ok(());
        }
        opts.label = format!("relay {}", netargs.name);
        let relay = RelayMetrics::new();
        opts.relay = Some(relay.clone());
        let sim = SimMetrics::new();
        eprintln!("connecting to {} ...", netargs.connect.as_deref().unwrap_or(""));
        // The window needs the scene size for the camera: the scene is the server room's.
        let (bridge, room_scene) = physics_bridge(&netargs, relay.clone(), sim.clone())?;
        scene = room_scene;
        let summary = run_with(bridge, scene, sim, opts)?;
        print_summary(&summary);
        print_relay_status(&relay.status());
        return Ok(());
    }

    let summary = if headless {
        let limit = match ticks {
            Some(n) => HeadlessLimit::Steps(n),
            None => HeadlessLimit::Seconds(opts.seconds.unwrap_or(8.0)),
        };
        run_headless(scene, net, limit)
    } else if threaded {
        opts.label = "threaded".to_string();
        let metrics = SimMetrics::new();
        let sim_metrics = metrics.clone();
        let bridge = Threaded::spawn(
            move || physics_pair(scene, net, sim_metrics.clone()),
            physics_bridge_config(metrics.clone()),
            ThreadedConfig::default(),
        )
        .map_err(|e| format!("start sim thread: {e}"))?;
        run_with(bridge, scene, metrics, opts)?
    } else {
        opts.label = "inproc".to_string();
        let metrics = SimMetrics::new();
        let bridge = InProc::new(physics_pair(scene, net, metrics.clone()), physics_bridge_config(metrics.clone()));
        run_with(bridge, scene, metrics, opts)?
    };
    print_summary(&summary);
    Ok(())
}

fn run_with<B: Bridge<PhysGame>>(
    bridge: B,
    scene: PhysConfig,
    metrics: std::sync::Arc<SimMetrics>,
    opts: Options,
) -> Result<PhysSummary, String> {
    run_window(bridge, scene, metrics, opts)
}
