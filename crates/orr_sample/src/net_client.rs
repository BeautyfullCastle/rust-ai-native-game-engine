//! Network play for the samples: command line options for a relay client,
//! building the client for the arena and the physics game, the headless bot,
//! and the text the window title and the summary show.
//!
//! A relay client runs on the sim thread of a `Threaded` bridge
//! (`orr_bridge::RelayHost`), so the view code is the same as in the local
//! loopback mode. `connect` blocks until the room has started.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use orr_bridge::{
    BridgeConfig, Lifecycle, PlayerSlot, RelayHost, RelayHostOptions, RelayMetrics, RelayStatus, StatusFn, Threaded, ThreadedConfig,
};
use orr_relay_net::{
    connect, drive, fresh_seed, parse_fingerprint, ClientReport, ConnectOptions, DirSink, DriveOptions, NetLink, SimConditions, TransportKind,
    Trust,
};
use orr_session::{RelayClient, RelayClientConfig};
use orr_sim::Game;
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

use crate::arena_view::arena_fire_commands;
use crate::physics_game::{NoCommand, PhysConfig, PhysGame, PhysInput};
use crate::physics_host::{physics_bridge_config, SimMetrics};

/// Frame-format-bound build id of the arena sample; the room's build hash is
/// `build_hash_of(ARENA_BUILD_ID, 0)` (`orr_server --game arena`).
pub const ARENA_BUILD_ID: u64 = orr_sim::frame_build_id(0x0A2E_4A00_0001);
/// Frame-format-bound build id of the physics sample (`orr_server --game physics`).
pub const PHYSICS_BUILD_ID: u64 = orr_sim::frame_build_id(0x0A2E_4A00_0002);

/// Client mode options of the sample programs.
#[derive(Clone, Debug)]
pub struct NetArgs {
    /// `host:port` of the server. `Some` turns client mode on.
    pub connect: Option<String>,
    pub kind: TransportKind,
    pub fingerprint: Option<[u8; 32]>,
    pub insecure_dev: bool,
    pub room: u64,
    pub slot: Option<u8>,
    /// Label in log lines.
    pub name: String,
    pub sim: SimConditions,
    /// `--sim-seed`. `None` picks a fresh seed per process (printed at connect).
    pub sim_seed: Option<u64>,
    pub desync_dir: PathBuf,
    /// Headless only: play with a scripted bot.
    pub bot: bool,
    pub connect_timeout: Duration,
    /// Print no status lines (a library host such as `orr_ffi` must not write to the process's output).
    pub quiet: bool,
}

impl Default for NetArgs {
    fn default() -> Self {
        Self {
            connect: None,
            kind: TransportKind::Quic,
            fingerprint: None,
            insecure_dev: false,
            room: 1,
            slot: None,
            name: "client".to_string(),
            sim: SimConditions { latency_ms: 0, jitter_ms: 0, loss: 0.0, seed: 0 },
            sim_seed: None,
            desync_dir: PathBuf::from("desync"),
            bot: false,
            connect_timeout: Duration::from_secs(60),
            quiet: false,
        }
    }
}

/// Text of the network options, for the `--help` header of the programs.
pub const NET_HELP: &str = "\
  --connect HOST:PORT          play on a relay server instead of the local loopback
  --transport quic|ws          (default quic)
  --trust-fingerprint HEX      QUIC: pin the server certificate (the server prints it)
  --insecure-dev               QUIC: accept any certificate (development only)
  --room N                     room id (default 1)
  --slot N                     ask for this slot (default: any free slot)
  --name TEXT                  label in log lines
  --sim-latency MS             add MS of one-way delay in each direction (test the network)
  --sim-jitter MS              add up to MS of random delay
  --sim-loss P                 lose this share (0.02 = 2%) of unreliable messages, each way
  --sim-seed N                 seed of the simulated loss and jitter (default: new per run, printed)
  --desync-dir DIR             where desync dumps (.orrd) go (default ./desync)
  --bot                        with --headless: play with a scripted bot
  --connect-timeout SECONDS    wait this long for the room to start (default 60)";

impl NetArgs {
    /// Handles `arg` if it is a network option (reading its value with
    /// `next`). Returns whether it was one.
    pub fn parse_option(&mut self, arg: &str, next: &mut dyn FnMut(&str) -> Result<String, String>) -> Result<bool, String> {
        fn num<T: std::str::FromStr>(name: &str, v: String) -> Result<T, String>
        where
            T::Err: std::fmt::Display,
        {
            v.parse().map_err(|e| format!("{name}: {e}"))
        }
        match arg {
            "--connect" => self.connect = Some(next(arg)?),
            "--transport" => self.kind = next(arg)?.parse()?,
            "--trust-fingerprint" => self.fingerprint = Some(parse_fingerprint(&next(arg)?)?),
            "--insecure-dev" => self.insecure_dev = true,
            "--room" => self.room = num(arg, next(arg)?)?,
            "--slot" => self.slot = Some(num(arg, next(arg)?)?),
            "--name" => self.name = next(arg)?,
            "--sim-latency" => self.sim.latency_ms = num(arg, next(arg)?)?,
            "--sim-jitter" => self.sim.jitter_ms = num(arg, next(arg)?)?,
            "--sim-loss" => self.sim.loss = num(arg, next(arg)?)?,
            "--sim-seed" => self.sim_seed = Some(num(arg, next(arg)?)?),
            "--desync-dir" => self.desync_dir = PathBuf::from(next(arg)?),
            "--bot" => self.bot = true,
            "--connect-timeout" => self.connect_timeout = Duration::from_secs_f32(num(arg, next(arg)?)?),
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn connect_options(&self) -> Result<ConnectOptions, String> {
        let addr = self.connect.clone().ok_or("--connect is not set")?;
        let trust = match (self.fingerprint, self.insecure_dev, self.kind) {
            (Some(fp), _, _) => Trust::Fingerprint(fp),
            (None, true, _) => Trust::InsecureDev,
            (None, false, TransportKind::Ws) => Trust::InsecureDev, // unused by WebSocket
            (None, false, TransportKind::Quic) => {
                return Err("QUIC needs --trust-fingerprint HEX (printed by the server) or --insecure-dev".into())
            }
        };
        let mut o = ConnectOptions::new(addr, self.kind, trust);
        let mut sim = self.sim;
        if sim.is_active() {
            sim.seed = self.sim_seed.unwrap_or_else(fresh_seed);
            if !self.quiet {
                println!("{}: network simulation {sim:?} (repeat with --sim-seed {})", self.name, sim.seed);
            }
        }
        o.sim = Some(sim).filter(SimConditions::is_active);
        Ok(o)
    }

    /// The link to the server (connecting starts at once).
    fn link(&self) -> Result<NetLink, String> {
        connect(&self.connect_options()?)
    }

    fn client_config(&self, build_id: u64) -> RelayClientConfig {
        let mut cfg = RelayClientConfig::new(self.room, build_id);
        cfg.want_slot = self.slot.map(PlayerSlot);
        cfg
    }

    fn host_options<G: Game>(&self, metrics: Arc<RelayMetrics>) -> RelayHostOptions<G> {
        let tag = self.name.clone();
        RelayHostOptions {
            connect_timeout: self.connect_timeout,
            on_status: (!self.quiet).then(|| Box::new(move |s: &str| eprintln!("[{tag}] {s}")) as StatusFn),
            commands_from_input: None,
            metrics: Some(metrics),
        }
    }
}

/// A relay client for the arena game.
pub fn arena_client(args: &NetArgs) -> Result<RelayClient<Arena, NetLink>, String> {
    let link = args.link()?;
    Ok(RelayClient::new(
        args.client_config(ARENA_BUILD_ID),
        link,
        |w| ArenaConfig { player_count: w.player_count },
        DirSink::new(&args.desync_dir),
    ))
}

/// A relay client for the physics game. The scene comes from the room's config blob.
pub fn physics_client(args: &NetArgs) -> Result<RelayClient<PhysGame, NetLink>, String> {
    physics_client_with(args, None)
}

/// Where a physics client leaves the scene it got from the server.
pub type SceneSink = Arc<Mutex<Option<PhysConfig>>>;

fn physics_client_with(args: &NetArgs, sink: Option<SceneSink>) -> Result<RelayClient<PhysGame, NetLink>, String> {
    let link = args.link()?;
    Ok(RelayClient::new(
        args.client_config(PHYSICS_BUILD_ID),
        link,
        move |w| {
            let scene = PhysConfig::from_blob(&w.config, w.player_count)
                .expect("the room config is not a physics scene (start the server with --game physics)");
            if let Some(s) = &sink {
                *s.lock().unwrap_or_else(PoisonError::into_inner) = Some(scene);
            }
            scene
        },
        DirSink::new(&args.desync_dir),
    ))
}

/// The arena on a `Threaded` bridge over the relay. Blocks until the room started.
pub fn arena_bridge(args: &NetArgs, metrics: Arc<RelayMetrics>) -> Result<Threaded<Arena>, String> {
    let args = args.clone();
    Threaded::try_spawn(
        move || {
            let client = arena_client(&args)?;
            let mut opts = args.host_options::<Arena>(metrics);
            opts.commands_from_input = Some(Box::new(|slot, input| arena_fire_commands(u32::from(slot.0), input)));
            RelayHost::connect(client, opts).map_err(|e| e.to_string())
        },
        BridgeConfig::default(),
        ThreadedConfig::default(),
    )
}

/// The physics game on a `Threaded` bridge over the relay, and the scene the
/// server's room uses (the window needs its size). Blocks until the room started.
pub fn physics_bridge(
    args: &NetArgs,
    metrics: Arc<RelayMetrics>,
    sim: Arc<SimMetrics>,
) -> Result<(Threaded<PhysGame>, PhysConfig), String> {
    let args = args.clone();
    let sink: SceneSink = Arc::new(Mutex::new(None));
    let factory_sink = sink.clone();
    let bridge = Threaded::try_spawn(
        move || {
            let client = physics_client_with(&args, Some(factory_sink))?;
            RelayHost::connect(client, args.host_options::<PhysGame>(metrics)).map_err(|e| e.to_string())
        },
        physics_bridge_config(sim),
        ThreadedConfig::default(),
    )?;
    let scene = sink.lock().unwrap_or_else(PoisonError::into_inner).ok_or("the server sent no scene")?;
    Ok((bridge, scene))
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// A player-like arena input, a pure function of `(slot, tick)`: holds a
/// direction for a few ticks and fires now and then.
pub fn arena_bot_input(slot: u8, tick: u64) -> (ArenaInput, Vec<SpawnBulletCmd>) {
    use orr_fp::FP;
    let s = u64::from(slot);
    let h = mix((tick / 7 + s * 3) ^ (s << 40));
    let (ax, ay) = ((h % 3) as i32 - 1, ((h >> 8) % 3) as i32 - 1);
    let fire = tick % 11 == s % 11 && (h >> 20) % 3 != 0;
    let input = ArenaInput::new(FP::from_int(ax), FP::from_int(ay), fire);
    let cmds = arena_fire_commands(u32::from(slot), &input);
    (input, cmds)
}

/// A player-like physics input, a pure function of `(slot, tick)`.
pub fn physics_bot_input(slot: u8, tick: u64) -> (PhysInput, Vec<NoCommand>) {
    let s = u64::from(slot);
    let h = mix((tick / 9 + s * 5) ^ (s << 33));
    let input = PhysInput::new((h % 3) as i32 - 1, ((h >> 8) % 3) as i32 - 1, ((h >> 16) % 3) as i32 - 1, (h >> 24) % 4 == 0);
    (input, Vec::new())
}

fn run_bot<G: Game>(
    mut client: RelayClient<G, NetLink>,
    mut script: impl FnMut(u8, u64) -> (G::Input, Vec<G::Command>),
    args: &NetArgs,
    seconds: f32,
) -> ClientReport {
    let opts = DriveOptions {
        play_for: Some(Duration::from_secs_f32(seconds)),
        connect_timeout: args.connect_timeout,
        log_every: Some(Duration::from_secs(5)),
        tag: args.name.clone(),
        ..DriveOptions::default()
    };
    drive(&mut client, &mut script, &opts)
}

/// Headless arena bot: plays `seconds` after the room started and returns what it measured.
pub fn run_arena_bot(args: &NetArgs, seconds: f32) -> Result<ClientReport, String> {
    Ok(run_bot(arena_client(args)?, arena_bot_input, args, seconds))
}

/// Headless physics bot.
pub fn run_physics_bot(args: &NetArgs, seconds: f32) -> Result<ClientReport, String> {
    Ok(run_bot(physics_client(args)?, physics_bot_input, args, seconds))
}

/// Prints the report of a headless bot run, one line per number.
pub fn print_bot_report(name: &str, r: &ClientReport) {
    println!("client         : {name} ({:?})", r.state);
    println!("slot           : {}", r.slot.map_or("-".to_string(), |s| s.to_string()));
    println!("rtt            : {:.1} ms", r.rtt_ms);
    println!("input delay    : {} ticks ({} changes)", r.delay, r.delay_changes);
    println!("rollbacks      : {} ({:.2}/s), {} ticks resimulated, deepest prediction {}", r.rollbacks, r.rollbacks_per_s, r.resim_ticks, r.max_prediction_depth);
    println!("stalls         : {} episodes, {} ms, longest {} ms", r.stall_episodes, r.stalled_ms, r.max_stall_ms);
    println!("repeated inputs: {} (server repeated this client's input), overridden {}", r.repeats, r.overridden);
    println!("sim rate       : {:+} ppm", i64::from(r.rate_ppm) - 1_000_000);
    println!("ticks          : head {} verified {} over {:.1} s", r.head_tick, r.verified_tick, r.play_secs);
    println!("desyncs        : {} (dumps written {})", r.desyncs, r.dumps);
    let last = r.checksums.last().map_or("-".to_string(), |(t, c)| format!("tick {t} = {c:016x}"));
    println!("last checksum  : {last} ({} verified checkpoints)", r.checksums.len());
}

/// The relay part of a window title.
pub fn relay_title(s: &RelayStatus) -> String {
    format!(
        " | net rtt {} ms, delay {}, stall {}x/{} ms, repeats {}, rate {:+} ppm{}",
        s.rtt_ms,
        s.delay,
        s.stall_episodes,
        s.stalled_ms,
        s.repeats,
        i64::from(s.rate_ppm) - 1_000_000,
        if s.connected { "" } else { " DISCONNECTED" }
    )
}

/// The relay numbers for the summary printed at exit.
pub fn print_relay_status(s: &RelayStatus) {
    println!(
        "relay          : rtt {} ms, input delay {} ticks, rollbacks {} ({} ticks resimulated), stalls {} ({} ms), repeated inputs {}, rate {:+} ppm, desyncs {}, {}",
        s.rtt_ms,
        s.delay,
        s.rollbacks,
        s.resim_ticks,
        s.stall_episodes,
        s.stalled_ms,
        s.repeats,
        i64::from(s.rate_ppm) - 1_000_000,
        s.desyncs,
        if s.connected { "connected" } else { "DISCONNECTED" }
    );
}

/// Logs the bridge notes that matter to a player of a relay game.
pub fn log_lifecycle(note: &Lifecycle) {
    match note {
        Lifecycle::Desync { tick } => eprintln!("DESYNC at tick {tick}: a dump was written (see --desync-dir)"),
        Lifecycle::Disconnected => eprintln!("DISCONNECTED from the server"),
        Lifecycle::Seeked { from, to } => println!("seeked: tick {from} -> {to}"),
        Lifecycle::Branched { tick, dropped } => println!("branched at tick {tick} ({dropped} recorded ticks dropped)"),
        Lifecycle::Paused { tick } => println!("paused at tick {tick}"),
        Lifecycle::Resumed { tick } => println!("resumed at tick {tick}"),
        Lifecycle::DebugRejected(e) => println!("debug command refused: {e}"),
        Lifecycle::SeekRejected { target } => println!("seek to tick {target} refused (outside the recording)"),
        _ => {}
    }
}

/// Recovery summaries are diagnostics, not a replay of lifecycle transitions.
pub fn log_view_resync(reset: &orr_bridge::ViewResync) {
    eprintln!(
        "view resynced at tick {}: {} presentation notifications discarded",
        reset.head_tick, reset.discarded_events
    );
    for note in &reset.lifecycle {
        eprintln!(
            "  coalesced {} lifecycle notifications; latest: {:?}",
            note.count, note.last
        );
    }
}
