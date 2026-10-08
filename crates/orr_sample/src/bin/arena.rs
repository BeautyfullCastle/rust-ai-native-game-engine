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
//!   --project DIR               saved authored Arena project (build --features project)
//!   --headless --ticks N         run that project for 0..6000 deterministic ticks
//!   --hold right,fire            optional held keys for project headless smoke
//!   --capture NEW.png            optional project headless compositor readback (GPU required)
//!   --sprite-project DIR        installed sample-sprites project (build --features sprites)
//!   --player-settings-dir ABS_DIR external Arena preference directory (player-settings feature)
//!   --game-ui-project DIR       installed Korean font project (build --features game-ui)
//!   --headless                   with --connect --bot: no window, play as a scripted bot (default 30 s)
//! ```
//! Play on a relay server (`orr_server`) instead of the local loopback with
//! `--connect HOST:PORT` (options: see `orr_sample::net_client::NET_HELP`):
//! ```text
//!   --connect HOST:PORT  --transport quic|ws  --trust-fingerprint HEX | --insecure-dev
//!   --room N  --slot N  --name TEXT  --sim-latency MS  --sim-jitter MS  --sim-loss P  --sim-seed N
//!   --desync-dir DIR  --bot  --connect-timeout SECONDS
//! ```
//! With `--features input-actions`: `--input-bindings FILE` loads bindings;
//! `--save-input-bindings NEW_FILE` exports and exits (never overwrites).
//! P toggles local input blocking; the network simulation continues.
//! Keys: WASD or arrows move, space fires, Escape quits. In local mode the other player is a bot.
use std::process::ExitCode;

use orr_bridge::RelayMetrics;
use orr_bridge::{Bridge, InProc, Threaded, ThreadedConfig};
use orr_sample::app::{run, Options, Summary};
use orr_sample::arena_view::{arena_bridge_config, loopback_pair, Loopback};
use orr_sample::net_client::{
    arena_bridge, print_bot_report, print_relay_status, run_arena_bot, NetArgs,
};
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
    let mut project = None;
    let mut ticks: Option<u32> = None;
    let mut capture = None;
    let mut held = None;
    let mut project_incompatible = Vec::new();
    let mut bridge_specified = false;
    let mut sprite_project = None;
    let mut game_ui_project = None;
    let mut player_settings_dir = None;
    let mut input_bindings = None;
    let mut save_input_bindings = None;
    let mut opts = Options {
        label: String::new(),
        #[cfg(feature = "input-actions")]
        input_map: None,
        #[cfg(feature = "game-ui")]
        game_ui_project: None,
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
            project_incompatible.push(arg);
            continue;
        }
        if matches!(
            arg.as_str(),
            "--latency" | "--jitter" | "--remote" | "--tau"
        ) {
            project_incompatible.push(arg.clone());
        }
        match arg.as_str() {
            "--project" => {
                if project.is_some() {
                    return Err("--project may be supplied only once".into());
                }
                project = Some(std::path::PathBuf::from(value("--project")?));
            }
            "--ticks" => {
                ticks = Some(
                    value("--ticks")?
                        .parse()
                        .map_err(|e| format!("--ticks: {e}"))?,
                )
            }
            "--capture" => capture = Some(std::path::PathBuf::from(value("--capture")?)),
            "--hold" => held = Some(parse_held(&value("--hold")?)?),
            "--player-settings-dir" => {
                if player_settings_dir.is_some() {
                    return Err("--player-settings-dir may be supplied only once".into());
                }
                player_settings_dir =
                    Some(std::path::PathBuf::from(value("--player-settings-dir")?));
            }
            "--input-bindings" => input_bindings = Some(value("--input-bindings")?),
            "--save-input-bindings" => save_input_bindings = Some(value("--save-input-bindings")?),
            "--game-ui-project" => {
                game_ui_project = Some(std::path::PathBuf::from(value("--game-ui-project")?))
            }
            "--headless" => headless = true,
            "--sprite-project" => {
                sprite_project = Some(std::path::PathBuf::from(value("--sprite-project")?))
            }
            "--audio" => opts.audio = value("--audio")?.parse()?,
            "--bridge" => {
                bridge_specified = true;
                threaded = match value("--bridge")?.as_str() {
                    "threaded" => true,
                    "inproc" => false,
                    other => return Err(format!("unknown bridge '{other}'")),
                }
            }
            "--latency" => {
                net.latency_ticks = value("--latency")?
                    .parse()
                    .map_err(|e| format!("--latency: {e}"))?
            }
            "--jitter" => {
                net.jitter_ticks = value("--jitter")?
                    .parse()
                    .map_err(|e| format!("--jitter: {e}"))?
            }
            "--remote" => {
                opts.remote_mode = match value("--remote")?.as_str() {
                    "snapshot" => InterpMode::Snapshot,
                    "prediction" => InterpMode::Prediction,
                    "none" => InterpMode::None,
                    other => return Err(format!("unknown remote mode '{other}'")),
                }
            }
            "--tau" => {
                opts.view.correction_tau =
                    value("--tau")?.parse().map_err(|e| format!("--tau: {e}"))?
            }
            "--no-vsync" => opts.vsync = false,
            "--seconds" => {
                opts.seconds = Some(
                    value("--seconds")?
                        .parse()
                        .map_err(|e| format!("--seconds: {e}"))?,
                )
            }
            other => {
                return Err(format!(
                    "unknown option '{other}' (see the header of this file)"
                ))
            }
        }
    }

    if player_settings_dir.is_some() {
        #[cfg(not(feature = "player-settings"))]
        return Err("--player-settings-dir requires --features player-settings".into());
        #[cfg(feature = "player-settings")]
        if project.is_none() {
            return Err("--player-settings-dir requires --project".into());
        }
    }
    if project.is_some() {
        if sprite_project.is_some() || game_ui_project.is_some() || save_input_bindings.is_some() {
            return Err("--project cannot be combined with --sprite-project, --game-ui-project or --save-input-bindings".into());
        }
        if !project_incompatible.is_empty() {
            return Err(format!(
                "--project cannot use relay/loopback/interpolation options: {}",
                project_incompatible.join(", ")
            ));
        }
        if headless {
            if ticks.is_none() {
                return Err("--project --headless requires --ticks 0..6000".into());
            }
            if opts.seconds.is_some() || input_bindings.is_some() || (bridge_specified && threaded)
            {
                return Err("headless projects use deterministic --ticks and --hold, without --seconds, --input-bindings or --bridge threaded".into());
            }
            if opts.audio == orr_sample::arena_audio::AudioMode::Required {
                return Err("--audio required is unavailable for headless projects".into());
            }
        } else if ticks.is_some() || held.is_some() || capture.is_some() {
            return Err("--ticks, --hold and --capture require --project --headless".into());
        }
        if ticks.is_some_and(|n| n > 6000) {
            return Err("--ticks must be between 0 and 6000".into());
        }
        if opts.seconds.is_some_and(|n| !n.is_finite() || n < 0.0) {
            return Err("--seconds must be finite and nonnegative".into());
        }
        #[cfg(not(feature = "project"))]
        return Err("--project requires building orr_sample with --features project".into());
    } else if ticks.is_some() || held.is_some() || capture.is_some() {
        return Err("--ticks, --hold and --capture require --project --headless".into());
    }

    if input_bindings.is_some() || save_input_bindings.is_some() {
        #[cfg(not(feature = "input-actions"))]
        return Err("input binding options require --features input-actions".into());
        #[cfg(feature = "input-actions")]
        {
            let map = if let Some(path) = input_bindings {
                orr_input::ActionMap::load_file(path)?
            } else {
                orr_sample::arena_input::default_map()
            };
            orr_sample::arena_input::validate_map(&map)?;
            if let Some(path) = save_input_bindings {
                map.save_new(&path)?;
                return Ok(());
            }
            opts.input_map = Some(map);
        }
    }
    #[cfg(feature = "project")]
    if let Some(root) = project {
        // This includes the full lock, all decoded assets and scene bake. No
        // runtime thread, audio device, window or GPU exists before admission.
        let prepared = orr_sample::project_runtime::PreparedRuntime::open(&root)?;
        if headless {
            return orr_sample::project_runtime::headless(
                prepared,
                ticks.expect("validated ticks"),
                held.unwrap_or_default(),
                capture.as_deref(),
            );
        }
        #[cfg(feature = "game-ui")]
        let summary = {
            let (seed, presentation, prepared_ui) = prepared.into_launch_parts();
            // Decode/install the already admitted font before spawning the host.
            #[allow(unused_mut)]
            let mut ui = prepared_ui
                .map(|ui| orr_sample::game_ui::GameUi::from_font(ui.font, true))
                .transpose()?;
            // Headless/capture returned above. No settings path or environment
            // discovery is permitted on those deterministic admission routes.
            #[cfg(feature = "player-settings")]
            if let Some(ui) = &mut ui {
                use orr_sample::player_controls::{resolve_paths, PlayerSettingsSession};
                let session = if let Some(map) = &opts.input_map {
                    PlayerSettingsSession::external(map.clone())
                } else {
                    let paths = (|| {
                        let mut protected = vec![root
                            .canonicalize()
                            .map_err(|e| format!("project path: {e}"))?];
                        let cwd = std::env::current_dir()
                            .map_err(|e| format!("working directory: {e}"))?;
                        let executable =
                            std::env::current_exe().map_err(|e| format!("runtime path: {e}"))?;
                        if let Some(parent) = executable.parent() {
                            // Export layout is bin/arena beside project and run-arena.
                            let bundle = if parent.file_name().is_some_and(|name| name == "bin") {
                                parent.parent().unwrap_or(parent)
                            } else {
                                parent
                            };
                            protected.push(bundle.to_path_buf());
                        }
                        let paths = resolve_paths(player_settings_dir, &protected)?;
                        if paths.directory() == cwd {
                            return Err(
                                "settings directory must not be the working directory".into()
                            );
                        }
                        Ok(paths)
                    })();
                    PlayerSettingsSession::open(paths)
                };
                eprintln!("player settings: {}", session.status());
                opts.input_map = Some(session.action_map());
                ui.set_player_settings(session);
            } else if player_settings_dir.is_some() {
                return Err(
                    "--player-settings-dir requires an authored Korean Arena UI preset".into(),
                );
            }
            if threaded {
                opts.label = "authored project / threaded".into();
                orr_sample::app::run_project_restartable(
                    move || {
                        let session = seed.session()?;
                        Threaded::spawn(
                            move || orr_bridge::PlayHost::new(session, orr_bridge::PlayerSlot(0)),
                            arena_bridge_config(),
                            ThreadedConfig::default(),
                        )
                        .map_err(|e| format!("start project thread: {e}"))
                    },
                    opts,
                    presentation,
                    ui,
                )?
            } else {
                opts.label = "authored project / inproc".into();
                orr_sample::app::run_project_restartable(
                    move || seed.bridge(),
                    opts,
                    presentation,
                    ui,
                )?
            }
        };
        #[cfg(not(feature = "game-ui"))]
        let summary = {
            let (session, presentation) = prepared.into_parts()?;
            let summary = if threaded {
                opts.label = "authored project / threaded".into();
                let bridge = Threaded::spawn(
                    move || orr_bridge::PlayHost::new(session, orr_bridge::PlayerSlot(0)),
                    arena_bridge_config(),
                    ThreadedConfig::default(),
                )
                .map_err(|e| format!("start project thread: {e}"))?;
                orr_sample::app::run_project(bridge, opts, presentation)?
            } else {
                opts.label = "authored project / inproc".into();
                let bridge = InProc::new(
                    orr_bridge::PlayHost::new(session, orr_bridge::PlayerSlot(0)),
                    arena_bridge_config(),
                );
                orr_sample::app::run_project(bridge, opts, presentation)?
            };
            summary
        };
        print_summary(&summary);
        return Ok(());
    }
    if game_ui_project.is_some() {
        if headless {
            return Err("--game-ui-project requires a window".into());
        }
        #[cfg(not(feature = "game-ui"))]
        return Err("--game-ui-project requires --features game-ui".into());
        #[cfg(feature = "game-ui")]
        {
            opts.game_ui_project = game_ui_project;
        }
    }
    if sprite_project.is_some() {
        if headless {
            return Err("--sprite-project requires a window".into());
        }
        #[cfg(not(feature = "sprites"))]
        return Err("--sprite-project requires building orr_sample with --features sprites".into());
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
        eprintln!(
            "connecting to {} ...",
            netargs.connect.as_deref().unwrap_or("")
        );
        let bridge = arena_bridge(&netargs, metrics.clone())?;
        let summary = run_with(bridge, opts, sprite_project.as_deref())?;
        print_summary(&summary);
        print_relay_status(&metrics.status());
        return Ok(());
    }
    if headless {
        return Err("--headless is only for --connect --bot".to_string());
    }

    let summary = if threaded {
        opts.label = "threaded".to_string();
        #[cfg(not(feature = "game-ui"))]
        let bridge = Threaded::spawn(
            move || loopback_pair(net),
            arena_bridge_config(),
            ThreadedConfig::default(),
        )
        .map_err(|e| format!("start sim thread: {e}"))?;
        #[cfg(feature = "game-ui")]
        {
            orr_sample::app::run_restartable(
                move || {
                    Threaded::spawn(
                        move || loopback_pair(net),
                        arena_bridge_config(),
                        ThreadedConfig::default(),
                    )
                    .map_err(|e| format!("start sim thread: {e}"))
                },
                opts,
                sprite_project.as_deref(),
            )?
        }
        #[cfg(not(feature = "game-ui"))]
        run_with(bridge, opts, sprite_project.as_deref())?
    } else {
        opts.label = "inproc".to_string();
        #[cfg(feature = "game-ui")]
        {
            orr_sample::app::run_restartable(
                move || Ok(InProc::new(loopback_pair(net), arena_bridge_config())),
                opts,
                sprite_project.as_deref(),
            )?
        }
        #[cfg(not(feature = "game-ui"))]
        run_with(
            InProc::new(loopback_pair(net), arena_bridge_config()),
            opts,
            sprite_project.as_deref(),
        )?
    };
    print_summary(&summary);
    Ok(())
}

fn run_with<B: Bridge<orr_testgame::Arena>>(
    bridge: B,
    opts: Options,
    sprite_project: Option<&std::path::Path>,
) -> Result<Summary, String> {
    #[cfg(feature = "sprites")]
    if let Some(root) = sprite_project {
        return orr_sample::app::run_sprites(bridge, opts, root);
    }
    #[cfg(not(feature = "sprites"))]
    let _ = sprite_project;
    run(bridge, opts)
}

fn print_summary(s: &Summary) {
    println!("adapter        : {}", s.adapter);
    println!(
        "audio          : {} ({} voices started)",
        s.audio_status, s.audio_started
    );
    println!(
        "frames         : {} in {:.2} s = {:.1} fps (worst frame {:.1} ms)",
        s.frames, s.seconds, s.fps, s.worst_frame_ms
    );
    println!(
        "sim tick       : {} (verified {})",
        s.sim_tick, s.verified_tick
    );
    println!(
        "rollbacks      : {} (deepest {} ticks), stalls {}",
        s.rollbacks, s.max_rollback_depth, s.stalls
    );
    println!(
        "hit events     : predicted {}, verified {}, canceled {}",
        s.predicted_hits, s.verified_hits, s.canceled_events
    );
}

fn parse_held(text: &str) -> Result<orr_sample::arena_view::Keys, String> {
    let mut keys = orr_sample::arena_view::Keys::default();
    if text == "idle" {
        return Ok(keys);
    }
    for key in text.split(',') {
        match key {
            "left" => keys.left = true,
            "right" => keys.right = true,
            "up" => keys.up = true,
            "down" => keys.down = true,
            "fire" => keys.fire = true,
            _ => {
                return Err("--hold expects idle or comma-separated left,right,up,down,fire".into())
            }
        }
    }
    Ok(keys)
}
