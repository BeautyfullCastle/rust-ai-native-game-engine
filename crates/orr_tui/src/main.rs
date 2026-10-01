//! `orr_tui`: a terminal view of an Orrery simulation, from its view stream alone.
//!
//! ```text
//! orr_tui --connect ws://127.0.0.1:7777 [--token T]      over ERP (WebSocket; tcp://host:port also works)
//! orr_tui --ffi [--lib PATH] [--scene PATH]              through the C ABI of liborr_ffi, loaded at run time
//!         [--player N] [--fps N]
//!         [--headless [--frames N] [--dump FILE] [--size WxH]]
//! ```
#![allow(clippy::disallowed_types)]

use std::path::PathBuf;
use std::process::ExitCode;

use orr_tui::headless::{self, HeadlessOpts};
use orr_tui::source::{Session, SocketSource, Source};
use orr_tui::ui::{self, UiOpts};

const USAGE: &str = "usage: orr_tui (--connect URL [--token T] | --ffi [--lib PATH] [--scene PATH]) [--player N] [--fps N]\n\
                     \x20               [--headless [--frames N] [--dump FILE] [--size WxH]]\n\
                     keys: arrows/a/d move, z/c spin, f fire, space/p play-pause, s step, r refit camera, q quit\n\
                     --headless plays a scripted scenario (every player, one step per frame), writes the ASCII\n\
                     grid of each frame to --dump and prints `RESULT entities=.. frames=.. fnv=0x..`";

#[derive(Default)]
struct Args {
    connect: Option<String>,
    token: Option<String>,
    ffi: bool,
    lib: Option<PathBuf>,
    scene: Option<PathBuf>,
    player: u8,
    fps: u32,
    headless: bool,
    frames: u32,
    dump: Option<PathBuf>,
    size: (usize, usize),
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { fps: 30, frames: 120, size: (80, 30), ..Args::default() };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--connect" => a.connect = Some(value("--connect")?),
            "--token" => a.token = Some(value("--token")?),
            "--ffi" => a.ffi = true,
            "--lib" => a.lib = Some(PathBuf::from(value("--lib")?)),
            "--scene" => a.scene = Some(PathBuf::from(value("--scene")?)),
            "--player" => a.player = value("--player")?.parse().map_err(|e| format!("--player: {e}"))?,
            "--fps" => a.fps = value("--fps")?.parse().map_err(|e| format!("--fps: {e}"))?,
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
    if a.connect.is_some() == a.ffi {
        return Err("give exactly one of --connect URL or --ffi".into());
    }
    Ok(a)
}

/// The source, and whether it is a host of our own (which starts paused: the viewer plays it).
fn open(a: &Args) -> Result<(Box<dyn Source>, bool), String> {
    if let Some(url) = &a.connect {
        // A headless run wants a paused session to step; the interactive view wants one that runs.
        let session = Session::Ensure { run: !a.headless };
        let max_fps = if a.headless { 1000 } else { 60 };
        return Ok((Box::new(SocketSource::connect(url, a.token.as_deref(), max_fps, session)?), false));
    }
    #[cfg(feature = "ffi")]
    {
        let lib = a.lib.clone().unwrap_or_else(orr_tui::ffi::default_lib_path);
        Ok((Box::new(orr_tui::ffi::FfiSource::open(&lib, a.scene.as_deref())?), true))
    }
    #[cfg(not(feature = "ffi"))]
    Err("this build has no --ffi (build orr_tui with the `ffi` feature)".into())
}

fn run() -> Result<(), String> {
    let a = parse_args()?;
    let (mut src, own_host) = open(&a)?;
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
