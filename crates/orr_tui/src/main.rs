//! `orr_tui`: a terminal view of an Orrery simulation, from its view stream alone.
//!
//! ```text
//! orr_tui --connect ws://127.0.0.1:7777 [--token T]      over ERP (WebSocket; tcp://host:port also works);
//!         [--headless [--ticks N] [--check-tick T]]      a host in client mode (`orr_remote_host --join`) shows its
//!                                                        slot, RTT, delay and rollbacks; headless prints `RESULT client ...`
//! orr_tui --ffi [--lib PATH] [--scene PATH]              through the C ABI of liborr_ffi, loaded at run time
//!         [--player N] [--fps N]
//!         [--headless [--frames N] [--dump FILE] [--size WxH]]
//! orr_tui --server HOST:PORT [--lib PATH]                play on an orr_server room (prediction and rollback),
//!         [--fingerprint HEX | --insecure] [--ws] [--room N] [--slot N]      through the C ABI
//!         [--sim-latency MS] [--sim-jitter MS] [--sim-loss PCT] [--sim-seed N] [--connect-timeout S]
//!         [--headless [--ticks N] [--check-tick T]]
//! ```
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)] // a view: percent to permille

use std::path::PathBuf;
use std::process::ExitCode;

use orr_tui::headless::{self, HeadlessOpts};
use orr_tui::source::{Session, SocketOptions, SocketSource, Source};
use orr_tui::ui::{self, UiOpts};

const USAGE: &str = "usage: orr_tui (--connect URL [--token T] [--ca-file PEM] | --ffi [--lib PATH] [--scene PATH]) [--player N] [--fps N]\n\
                     \x20               [--headless [--frames N] [--dump FILE] [--size WxH]]\n\
                     \x20      orr_tui --server HOST:PORT [--lib PATH] [--fingerprint HEX | --insecure] [--ws] [--room N] [--slot N]\n\
                     \x20               [--sim-latency MS] [--sim-jitter MS] [--sim-loss PCT] [--sim-seed N] [--connect-timeout S]\n\
                     \x20               [--headless [--ticks N] [--check-tick T]]   (play on a relay server through the C ABI)\n\
                     \x20      --connect also works against `orr_remote_host --join` (a host that plays on a relay server): the status line\n\
                     \x20      shows slot, RTT, delay and rollbacks; --headless then prints `RESULT client ...` like --server\n\
                     keys: arrows/a/d move, z/c spin, f fire, space/p play-pause, s step, r refit camera, q quit\n\
                     --headless plays a scripted scenario (every player, one step per frame), writes the ASCII\n\
                     grid of each frame to --dump and prints `RESULT entities=.. frames=.. fnv=0x..`;\n\
                     with --server it plays the joined slot for --ticks (default 480) and prints\n\
                     `RESULT client slot=.. ... checkpoint=T checksum=0x..` (the part from `checkpoint=` is the\n\
                     confirmed state at --check-tick, default 300: equal for every player of the room)";

#[derive(Default)]
struct Args {
    connect: Option<String>,
    token: Option<String>,
    ca_file: Option<PathBuf>,
    ffi: bool,
    lib: Option<PathBuf>,
    scene: Option<PathBuf>,
    player: u8,
    fps: u32,
    headless: bool,
    frames: u32,
    dump: Option<PathBuf>,
    size: (usize, usize),
    server: Option<String>,
    fingerprint: Option<String>,
    insecure: bool,
    ws: bool,
    room: u64,
    slot: Option<u8>,
    sim_latency: u32,
    sim_jitter: u32,
    /// Simulated loss in percent.
    sim_loss: f32,
    sim_seed: u64,
    connect_timeout: f32,
    ticks: u64,
    check_tick: u64,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { fps: 30, frames: 120, size: (80, 30), ticks: 480, check_tick: 300, connect_timeout: 60.0, ..Args::default() };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--connect" => a.connect = Some(value("--connect")?),
            "--ca-file" => a.ca_file = Some(PathBuf::from(value("--ca-file")?)),
            "--token" => a.token = Some(value("--token")?),
            "--ffi" => a.ffi = true,
            "--lib" => a.lib = Some(PathBuf::from(value("--lib")?)),
            "--scene" => a.scene = Some(PathBuf::from(value("--scene")?)),
            "--player" => a.player = value("--player")?.parse().map_err(|e| format!("--player: {e}"))?,
            "--fps" => a.fps = value("--fps")?.parse().map_err(|e| format!("--fps: {e}"))?,
            "--server" => a.server = Some(value("--server")?),
            "--fingerprint" => a.fingerprint = Some(value("--fingerprint")?),
            "--insecure" => a.insecure = true,
            "--ws" => a.ws = true,
            "--room" => a.room = value("--room")?.parse().map_err(|e| format!("--room: {e}"))?,
            "--slot" => a.slot = Some(value("--slot")?.parse().map_err(|e| format!("--slot: {e}"))?),
            "--sim-latency" => a.sim_latency = value("--sim-latency")?.parse().map_err(|e| format!("--sim-latency: {e}"))?,
            "--sim-jitter" => a.sim_jitter = value("--sim-jitter")?.parse().map_err(|e| format!("--sim-jitter: {e}"))?,
            "--sim-loss" => a.sim_loss = value("--sim-loss")?.parse().map_err(|e| format!("--sim-loss: {e}"))?,
            "--sim-seed" => a.sim_seed = value("--sim-seed")?.parse().map_err(|e| format!("--sim-seed: {e}"))?,
            "--connect-timeout" => a.connect_timeout = value("--connect-timeout")?.parse().map_err(|e| format!("--connect-timeout: {e}"))?,
            "--ticks" => a.ticks = value("--ticks")?.parse().map_err(|e| format!("--ticks: {e}"))?,
            "--check-tick" => a.check_tick = value("--check-tick")?.parse().map_err(|e| format!("--check-tick: {e}"))?,
            "--headless" => a.headless = true,
            "--frames" => a.frames = value("--frames")?.parse().map_err(|e| format!("--frames: {e}"))?,
            "--dump" => a.dump = Some(PathBuf::from(value("--dump")?)),
            "--size" => {
                let v = value("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size: expected WxH")?;
                a.size = (w.parse().map_err(|e| format!("--size: {e}"))?, h.parse().map_err(|e| format!("--size: {e}"))?);
            }
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if usize::from(a.connect.is_some()) + usize::from(a.ffi) + usize::from(a.server.is_some()) != 1 {
        return Err("give exactly one of --connect URL, --ffi or --server HOST:PORT".into());
    }
    if a.ca_file.is_some() && !a.connect.as_deref().is_some_and(|url| url.starts_with("wss://")) {
        return Err("--ca-file requires --connect wss://...".into());
    }
    if a.connect.is_some() && a.insecure {
        return Err("--insecure is not supported for ERP connections".into());
    }
    Ok(a)
}

/// The source, and whether it is a host of our own (which starts paused: the viewer plays it).
fn open(a: &Args) -> Result<(Box<dyn Source>, bool), String> {
    if let Some(url) = &a.connect {
        // A headless run wants a paused session to step; the interactive view wants one that runs.
        let session = Session::Ensure { run: !a.headless };
        let max_fps = if a.headless { 1000 } else { 60 };
        return Ok((Box::new(SocketSource::connect_with_options(url, a.token.as_deref(), max_fps, session, &SocketOptions { ca_file: a.ca_file.clone(), ..SocketOptions::default() })?), false));
    }
    #[cfg(feature = "ffi")]
    {
        let lib = a.lib.clone().unwrap_or_else(orr_tui::ffi::default_lib_path);
        if let Some(server) = &a.server {
            let opts = orr_tui::ffi::ClientOpts {
                server: server.clone(),
                fingerprint: a.fingerprint.clone(),
                insecure: a.insecure,
                ws: a.ws,
                room: a.room,
                slot: a.slot,
                sim_latency_ms: a.sim_latency,
                sim_jitter_ms: a.sim_jitter,
                sim_loss_permille: (a.sim_loss * 10.0).round().clamp(0.0, 1000.0) as u32,
                sim_seed: a.sim_seed,
                connect_timeout: std::time::Duration::from_secs_f32(a.connect_timeout.max(1.0)),
            };
            if !a.ws && a.fingerprint.is_none() && !a.insecure {
                return Err("QUIC needs --fingerprint HEX (printed by the server) or --insecure".into());
            }
            eprintln!("orr_tui: joining {server} (the room starts when every player is in) ...");
            return Ok((Box::new(orr_tui::ffi::FfiSource::open_client(&lib, &opts)?), false));
        }
        Ok((Box::new(orr_tui::ffi::FfiSource::open(&lib, a.scene.as_deref())?), true))
    }
    #[cfg(not(feature = "ffi"))]
    Err("this build has no --ffi (build orr_tui with the `ffi` feature)".into())
}

fn run() -> Result<(), String> {
    let a = parse_args()?;
    let (mut src, own_host) = open(&a)?;
    // A client of a relay server: through the C ABI (`--server`) or a host in client mode (`--connect`).
    if a.headless && (a.server.is_some() || src.net_status().is_some()) {
        let opts = headless::ClientOpts { ticks: a.ticks, check_tick: a.check_tick };
        let summary = headless::run_client(src.as_mut(), &opts)?;
        println!("{summary}");
        return Ok(());
    }
    if a.headless {
        let opts = HeadlessOpts { frames: a.frames, size: a.size, dump: a.dump.clone() };
        let summary = headless::run(src.as_mut(), &opts)?;
        println!("{summary}");
        return Ok(());
    }
    ui::run(src.as_mut(), &UiOpts { player: a.player, fps: a.fps, autoplay: own_host })
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if e.is_empty() {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            eprintln!("orr_tui: {e}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
