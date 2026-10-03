//! Runs the arena test game in a window.
//!
//! ```text
//! cargo run -p orr_sample --release -- [options]
//!   --bridge threaded|inproc     where the sim runs (default threaded)
//!   --latency TICKS              one-way loopback latency (default 6)
//!   --jitter TICKS               loopback jitter (default 2)
//!   --remote snapshot|prediction|none   how the other player is shown (default snapshot)
//!   --tau SECONDS                rollback smoothing time constant (default 0.12, 0 = off)
//!   --audio off|auto|required     optional native output (default auto; build --features audio-native)
//!   --no-vsync                   do not wait for the display (measure raw speed)
//!   --seconds N                  close after N seconds and print a summary
//!   --headless                   with --connect --bot: no window, play as a scripted bot (default 30 s)
//! ```
//! Play on a relay server (`orr_server`) instead of the local loopback with
//! `--connect HOST:PORT` (options: see `orr_sample::net_client::NET_HELP`):
//! ```text
//!   --connect HOST:PORT  --transport quic|ws  --trust-fingerprint HEX | --insecure-dev
//!   --room N  --slot N  --name TEXT  --sim-latency MS  --sim-jitter MS  --sim-loss P  --sim-seed N
//!   --desync-dir DIR  --bot  --connect-timeout SECONDS
//! ```
//! Keys: WASD or arrows move, space fires, Escape quits. In local mode the other player is a bot.
use std::process::ExitCode;

use orr_bridge::{Bridge, InProc, Threaded, ThreadedConfig};
use orr_bridge::RelayMetrics;
use orr_sample::app::{run, Options, Summary};
use orr_sample::net_client::{arena_bridge, print_bot_report, print_relay_status, run_arena_bot, NetArgs};
use orr_sample::arena_view::{arena_bridge_config, loopback_pair, Loopback};
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

fn real_main() -> Result<(), String> {
    let mut threaded = true;
    let mut net = Loopback::default();
    let mut netargs = NetArgs::default();
    let mut headless = false;
    let mut opts = Options {
        label: String::new(),
        vsync: true,
        audio: Default::default(),
        seconds: None,
        remote_mode: InterpMode::Snapshot,
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
            "--headless" => headless = true,
            "--audio" => opts.audio = value("--audio")?.parse()?,
            "--bridge" => {
                threaded = match value("--bridge")?.as_str() {
                    "threaded" => true,
                    "inproc" => false,
                    other => return Err(format!("unknown bridge '{other}'")),
                }
            }
            "--latency" => net.latency_ticks = value("--latency")?.parse().map_err(|e| format!("--latency: {e}"))?,
            "--jitter" => net.jitter_ticks = value("--jitter")?.parse().map_err(|e| format!("--jitter: {e}"))?,
            "--remote" => {
                opts.remote_mode = match value("--remote")?.as_str() {
                    "snapshot" => InterpMode::Snapshot,
                    "prediction" => InterpMode::Prediction,
                    "none" => InterpMode::None,
                    other => return Err(format!("unknown remote mode '{other}'")),
                }
            }
            "--tau" => opts.view.correction_tau = value("--tau")?.parse().map_err(|e| format!("--tau: {e}"))?,
            "--no-vsync" => opts.vsync = false,
            "--seconds" => opts.seconds = Some(value("--seconds")?.parse().map_err(|e| format!("--seconds: {e}"))?),
            other => return Err(format!("unknown option '{other}' (see the header of this file)")),
        }
    }

    if headless {
        if opts.audio == orr_sample::arena_audio::AudioMode::Required {
            return Err("--audio required is unavailable in headless bot mode".into());
        }
        eprintln!("audio: off (headless bot mode)");
    }
    if netargs.connect.is_some() {
        if headless {
            if !netargs.bot {
                return Err("--headless needs --bot (nobody is at the keyboard)".to_string());
            }
            let report = run_arena_bot(&netargs, opts.seconds.unwrap_or(30.0))?;
            print_bot_report(&netargs.name, &report);
            return Ok(());
        }
        opts.label = format!("relay {}", netargs.name);
        let metrics = RelayMetrics::new();
        opts.relay = Some(metrics.clone());
        eprintln!("connecting to {} ...", netargs.connect.as_deref().unwrap_or(""));
        let bridge = arena_bridge(&netargs, metrics.clone())?;
        let summary = run_with(bridge, opts)?;
        print_summary(&summary);
        print_relay_status(&metrics.status());
        return Ok(());
    }
    if headless {
        return Err("--headless is only for --connect --bot".to_string());
    }

    let summary = if threaded {
        opts.label = "threaded".to_string();
        let bridge = Threaded::spawn(move || loopback_pair(net), arena_bridge_config(), ThreadedConfig::default())
            .map_err(|e| format!("start sim thread: {e}"))?;
        run_with(bridge, opts)?
    } else {
        opts.label = "inproc".to_string();
        run_with(InProc::new(loopback_pair(net), arena_bridge_config()), opts)?
    };
    print_summary(&summary);
    Ok(())
}

fn run_with<B: Bridge<orr_testgame::Arena>>(bridge: B, opts: Options) -> Result<Summary, String> {
    run(bridge, opts)
}

fn print_summary(s: &Summary) {
    println!("adapter        : {}", s.adapter);
    println!("audio          : {} ({} voices started)", s.audio_status, s.audio_started);
    println!("frames         : {} in {:.2} s = {:.1} fps (worst frame {:.1} ms)", s.frames, s.seconds, s.fps, s.worst_frame_ms);
    println!("sim tick       : {} (verified {})", s.sim_tick, s.verified_tick);
    println!("rollbacks      : {} (deepest {} ticks), stalls {}", s.rollbacks, s.max_rollback_depth, s.stalls);
    println!(
        "hit events     : predicted {}, verified {}, canceled {}",
        s.predicted_hits, s.verified_hits, s.canceled_events
    );
}
