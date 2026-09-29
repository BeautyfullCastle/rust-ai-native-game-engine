//! Runs the arena test game in a window.
//!
//! ```text
//! cargo run -p orr_sample --release -- [options]
//!   --bridge threaded|inproc     where the sim runs (default threaded)
//!   --latency TICKS              one-way loopback latency (default 6)
//!   --jitter TICKS               loopback jitter (default 2)
//!   --remote snapshot|prediction|none   how the other player is shown (default snapshot)
//!   --tau SECONDS                rollback smoothing time constant (default 0.12, 0 = off)
//!   --no-vsync                   do not wait for the display (measure raw speed)
//!   --seconds N                  close after N seconds and print a summary
//! ```
//! Keys: WASD or arrows move, space fires, Escape quits. The other player is a bot.
use std::process::ExitCode;

use orr_bridge::{Bridge, InProc, Threaded, ThreadedConfig};
use orr_sample::app::{run, Options, Summary};
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
    let mut opts = Options {
        label: String::new(),
        vsync: true,
        seconds: None,
        remote_mode: InterpMode::Snapshot,
        view: ViewConfig::default(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
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
    println!("frames         : {} in {:.2} s = {:.1} fps (worst frame {:.1} ms)", s.frames, s.seconds, s.fps, s.worst_frame_ms);
    println!("sim tick       : {} (verified {})", s.sim_tick, s.verified_tick);
    println!("rollbacks      : {} (deepest {} ticks), stalls {}", s.rollbacks, s.max_rollback_depth, s.stalls);
    println!(
        "hit events     : predicted {}, verified {}, canceled {}",
        s.predicted_hits, s.verified_hits, s.canceled_events
    );
}
