//! Offline individual PNG frames to an installable sprite package.
#![forbid(unsafe_code)]
#[cfg(all(feature = "project-create", target_os = "linux"))]
#[path = "png_sequence_package/authoring.rs"]
mod authoring;

#[cfg(all(feature = "project-create", target_os = "linux"))]
mod cli {
    use super::authoring::{import_package, Request};
    use std::{ffi::OsString, path::PathBuf};
    const USAGE: &str = "png_sequence_package --recipe PATH --out NEW_DIR\nRequires Linux and --features project-create. Offline output only; no install or project edits. Recipe paths are relative to the recipe file. See docs/png-sequence-package.md.";
    pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Option<Request>, String> {
        let args: Vec<_> = args.into_iter().collect();
        if args.len() == 1 && args[0] == "--help" {
            return Ok(None);
        }
        if args.len() != 4 {
            return Err("expected --recipe PATH --out NEW_DIR exactly once".into());
        }
        let mut recipe = None;
        let mut out = None;
        for pair in args.chunks_exact(2) {
            if pair[1].is_empty() {
                return Err("empty option value".into());
            }
            let slot = match pair[0].to_str() {
                Some("--recipe") => &mut recipe,
                Some("--out") => &mut out,
                _ => return Err("unknown option".into()),
            };
            if slot.replace(PathBuf::from(&pair[1])).is_some() {
                return Err("duplicate option".into());
            }
        }
        Ok(Some(Request {
            recipe: recipe.ok_or("missing --recipe")?,
            out: out.ok_or("missing --out")?,
        }))
    }
    pub fn run() -> Result<(), String> {
        let Some(request) = parse_args(std::env::args_os().skip(1))? else {
            println!("{USAGE}");
            return Ok(());
        };
        let report = import_package(&request)?;
        println!("Created sprite package: {} frames, {}x{} atlas, {} RGBA bytes. Installation and rendering are separate steps.",report.frames,report.width,report.height,report.rgba_bytes);
        Ok(())
    }
}
fn main() {
    #[cfg(all(feature = "project-create", target_os = "linux"))]
    let result = cli::run();
    #[cfg(not(all(feature = "project-create", target_os = "linux")))]
    let result: Result<(), String> =
        Err("png_sequence_package requires Linux and --features project-create".into());
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
