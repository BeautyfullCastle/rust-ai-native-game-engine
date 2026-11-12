use orr_terrain_demo::{Course, Game};
use std::path::PathBuf;
use std::process::ExitCode;

const HELP: &str = "terrain_demo [--course FILE] [--headless]\nDefault: checked-in Terrain Trail hillside course. --headless loads and reports without a GPU.\nNative play requires --features window. WASD/arrows move; drag orbits; wheel zooms; R restarts; Esc exits.\n--course reads a trusted regular JSON file and its sibling .orrt once; it never writes source files.";

fn run() -> Result<(), String> {
    let mut path = None;
    let mut headless = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") if path.is_none() && !headless && args.next().is_none() => {
                println!("{HELP}");
                return Ok(());
            }
            Some("--course") if path.is_none() => {
                path = Some(PathBuf::from(args.next().ok_or(HELP)?));
            }
            Some("--headless") if !headless => headless = true,
            _ => return Err(HELP.into()),
        }
    }
    let course = match path {
        Some(path) => Course::load(&path)?,
        None => Course::builtin()?,
    };
    if headless {
        let game = Game::new(course);
        let state = game.state();
        println!(
            "{}",
            serde_json::json!({
                "title": game.course().title(), "width":game.course().terrain().width(),
                "depth":game.course().terrain().depth(), "anchor_raw":state.anchor.map(orr_fp::FP::raw),
                "beacons":4,"collected":state.collected,"finished":state.finished,
                "mode":"CPU snapshot admission only; no play or GPU proof"
            })
        );
        return Ok(());
    }
    #[cfg(feature = "window")]
    {
        orr_terrain_demo::window::run(course)
    }
    #[cfg(not(feature = "window"))]
    {
        let _ = course;
        Err("native play requires cargo run -p orr_terrain_demo --features window --bin terrain_demo; use --headless for CPU admission".into())
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("terrain_demo: {error}");
            ExitCode::FAILURE
        }
    }
}
