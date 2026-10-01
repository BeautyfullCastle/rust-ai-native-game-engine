//! Runs the 3D physics yard (`orr_physics3d`) in a window, or headless as a benchmark.
//!
//! ```text
//! cargo run -p orr_sample --release --bin physics3d -- [options]
//!   --bodies N                   bodies raining down at the start (default 400)
//!   --rain R                     keep adding R bodies per second (default 6)
//!   --max-entities N             stop spawning at this many entities (default 2500)
//!   --bridge threaded|inproc     where the sim runs (default threaded)
//!   --latency TICKS              one-way loopback latency of the 2 peers (default 6)
//!   --jitter TICKS               loopback jitter (default 2)
//!   --tau SECONDS                rollback smoothing time constant (default 0.12, 0 = off)
//!   --msaa N                     MSAA samples, 1 2 4 8 (default 4, falls back to what the adapter has)
//!   --shadow-map N               shadow map size in texels (default 2048)
//!   --no-shadows                 start with shadow mapping off (H toggles)
//!   --debug                      start with boxes and velocity arrows on (G toggles)
//!   --size WxH                   window size (default 1280x720)
//!   --no-vsync                   do not wait for the display (measure raw speed)
//!   --seconds N                  close after N seconds and print a summary
//!   --frames N                   close after N frames; each frame advances the sim one fixed
//!                                tick in process, so the picture is repeatable
//!   --screenshot FILE.png        with --frames (default 120): write the last presented frame
//!                                (read back from the window's own framebuffer) to a PNG
//!   --headless                   no window, no GPU: sim and the view's CPU work as fast as possible
//!   --ticks N                    headless: stop after N sim steps instead of --seconds
//! ```
//! The scene is two peers of one rollback session in this process (like the 2D
//! `physics` sample): you are slot 0, a bot on slot 1 shoots from the yard's rim and
//! its input arrives late, so you see rollbacks. Camera: drag the left mouse button
//! to orbit, right or middle button to pan, wheel to zoom, R resets. Space shoots a
//! ball along the ray under the cursor; hold B, N or C to drop boxes, balls or
//! capsules where the cursor meets the floor. G debug lines (boxes, velocity), H
//! shadows, T tone mapping, Escape quits. The on-screen stats show fps, sim cost, tick
//! and rollbacks.
use std::path::PathBuf;
use std::process::ExitCode;

use orr_bridge::{InProc, Threaded, ThreadedConfig};
use orr_sample::arena_view::Loopback;
use orr_sample::physics_host::SimMetrics;
use orr_sample::yard3d_app::{print_summary, run_headless, run_window, HeadlessLimit, YardOptions};
use orr_sample::yard3d_game::YardConfig;
use orr_sample::yard3d_host::{yard_bridge_config, yard_pair};

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
    let mut scene = YardConfig::new(400);
    let mut headless = false;
    let mut ticks: Option<u64> = None;
    let mut opts = YardOptions::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--bodies" => scene.bodies = parse("--bodies", value("--bodies")?)?,
            "--rain" => scene.rain_per_second = parse("--rain", value("--rain")?)?,
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
            "--tau" => opts.view.correction_tau = parse("--tau", value("--tau")?)?,
            "--msaa" => opts.msaa = parse("--msaa", value("--msaa")?)?,
            "--shadow-map" => opts.shadow_map = parse("--shadow-map", value("--shadow-map")?)?,
            "--no-shadows" => opts.shadows = false,
            "--debug" => opts.debug = true,
            "--size" => {
                let v = value("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH, for example 1280x720")?;
                opts.size = (parse("--size", w.to_string())?, parse("--size", h.to_string())?);
            }
            "--no-vsync" => opts.vsync = false,
            "--seconds" => opts.seconds = Some(parse("--seconds", value("--seconds")?)?),
            "--frames" => opts.frames = Some(parse("--frames", value("--frames")?)?),
            "--screenshot" => opts.screenshot = Some(PathBuf::from(value("--screenshot")?)),
            "--headless" => headless = true,
            "--ticks" => ticks = Some(parse("--ticks", value("--ticks")?)?),
            other => return Err(format!("unknown option '{other}' (see the header of this file)")),
        }
    }
    if opts.screenshot.is_some() && opts.frames.is_none() {
        opts.frames = Some(120);
    }

    if headless {
        let limit = match ticks {
            Some(n) => HeadlessLimit::Steps(n),
            None => HeadlessLimit::Seconds(opts.seconds.unwrap_or(8.0)),
        };
        let summary = run_headless(scene, net, limit);
        print_summary(&summary, true);
        return Ok(());
    }

    let metrics = SimMetrics::new();
    // A frame-counted run steps the sim itself, once per frame: in process.
    let summary = if threaded && opts.frames.is_none() {
        opts.label = "threaded".to_string();
        let sim_metrics = metrics.clone();
        let bridge = Threaded::spawn(
            move || yard_pair(scene, net, sim_metrics.clone()),
            yard_bridge_config(metrics.clone()),
            ThreadedConfig::default(),
        )
        .map_err(|e| format!("start sim thread: {e}"))?;
        run_window(bridge, metrics, opts)?
    } else {
        opts.label = "inproc".to_string();
        let bridge = InProc::new(yard_pair(scene, net, metrics.clone()), yard_bridge_config(metrics.clone()));
        run_window(bridge, metrics, opts)?
    };
    print_summary(&summary, false);
    Ok(())
}
