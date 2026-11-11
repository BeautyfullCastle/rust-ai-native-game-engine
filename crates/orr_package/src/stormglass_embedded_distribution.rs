//! Publish one independently trusted, already built Stormglass embedded executable.
//! No build, download, extraction, installation, launcher, or replacement occurs.
//! Windows x64 fixed local NTFS and cooperating directories only; not a sandbox.
use serde::Serialize;
use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Path, PathBuf},
};
#[cfg(any(all(windows, target_arch = "x86_64"), test))]
mod content;
#[cfg(all(windows, target_arch = "x86_64"))]
mod windows;
type Result<T> = std::result::Result<T, String>;
#[cfg(any(all(windows, target_arch = "x86_64"), test))]
const MAX_RUNTIME_BYTES: usize = 128 * 1024 * 1024;
pub const HELP: &str = "orr_publish_stormglass --runtime ABSOLUTE.exe --runtime-sha256 HEX --scene ABSOLUTE.yaml --output NEW_ABSOLUTE.exe [--trusted-runtime] [--dry-run]\nWindows x64, fixed local NTFS, canonical ASCII paths only. All directories must already exist and be trusted/cooperating. --trusted-runtime acknowledges independently verified code, its dependencies, and trusted input/output directories. A hash is integrity, not authentication. Dry-run performs read-only admission, never executes or creates files. Publishing runs three bounded checks and never overwrites. Failed stages are retained. No automatic build/install, extraction, or portability guarantee.";

#[derive(Debug, Clone)]
pub struct Options {
    pub runtime: PathBuf,
    pub runtime_sha256: String,
    pub scene: PathBuf,
    pub output: PathBuf,
    pub trusted_runtime: bool,
    pub dry_run: bool,
}
impl Options {
    /// Parse the complete command before opening any path. Unknown/duplicate flags fail.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self> {
        let (mut runtime, mut pin, mut scene, mut output) = (None, None, None, None);
        let (mut trusted_runtime, mut dry_run) = (false, false);
        let mut args = args.into_iter();
        let mut seen = BTreeSet::new();
        while let Some(arg) = args.next() {
            let flag = arg.to_str().ok_or("options must be Unicode")?;
            if !seen.insert(flag.to_owned()) {
                return Err(format!("duplicate option {flag}"));
            }
            match flag {
                "--trusted-runtime" => trusted_runtime = true,
                "--dry-run" => dry_run = true,
                "--runtime" | "--runtime-sha256" | "--scene" | "--output" => {
                    let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
                    match flag {
                        "--runtime" => runtime = Some(PathBuf::from(value)),
                        "--scene" => scene = Some(PathBuf::from(value)),
                        "--output" => output = Some(PathBuf::from(value)),
                        _ => pin = Some(value.into_string().map_err(|_| "SHA256 must be ASCII")?),
                    }
                }
                _ => return Err(format!("unknown option {flag}")),
            }
        }
        let options = Self {
            runtime: runtime.ok_or("--runtime required")?,
            runtime_sha256: pin.ok_or("--runtime-sha256 required")?,
            scene: scene.ok_or("--scene required")?,
            output: output.ok_or("--output required")?,
            trusted_runtime,
            dry_run,
        };
        validate(&options)?;
        Ok(options)
    }
}
#[derive(Debug, Serialize)]
pub struct Receipt {
    pub status: &'static str,
    pub output: PathBuf,
    pub runtime_sha256: String,
    pub runtime_bytes: usize,
    pub scene_sha256: String,
    pub expected_summary: String,
    pub embedded_scene_verified: bool,
    pub smoke_verified: bool,
    pub override_rejected: bool,
}
/// Dry-run reads only. Publication requires explicit independent runtime trust.
/// Each process has a 15-second polling budget and 5-second confirmed cleanup;
/// synchronous OS calls are outside these budgets. Retained failures are not deleted.
/// The returned receipt covers these local checks, not GPU or clean-machine readiness.
pub fn publish(options: &Options) -> Result<Receipt> {
    validate(options)?;
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        windows::publish(options)
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        Err("publication requires Windows x64; no input/output opened".into())
    }
}
/// Always one JSON result on stdout or one JSON error on stderr. Usage=2, operation=1.
pub fn run_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    let args: Vec<_> = args.into_iter().collect();
    if args == [OsString::from("--help")] || args == [OsString::from("-h")] {
        println!("{HELP}");
        return 0;
    }
    let options = match Options::parse(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{}", serde_json::json!({"error":error,"kind":"usage"}));
            return 2;
        }
    };
    match publish(&options) {
        Ok(receipt) => {
            println!(
                "{}",
                serde_json::to_string(&receipt).expect("receipt serialization")
            );
            0
        }
        Err(error) => {
            eprintln!("{}", serde_json::json!({"error":error,"kind":"operation"}));
            1
        }
    }
}
fn validate(options: &Options) -> Result<()> {
    if !options.dry_run && !options.trusted_runtime {
        return Err("--trusted-runtime required for execution; verify code origin and all directories independently, SHA256 is not trust".into());
    }
    if options.runtime_sha256.len() != 64
        || !options
            .runtime_sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err("runtime SHA256 must be exactly 64 hexadecimal characters".into());
    }
    for path in [&options.runtime, &options.scene, &options.output] {
        validate_path(path)?;
    }
    for path in [&options.runtime, &options.output] {
        if !path
            .to_str()
            .unwrap()
            .to_ascii_lowercase()
            .ends_with(".exe")
        {
            return Err("runtime and output must end in .exe".into());
        }
    }
    let paths = [&options.runtime, &options.scene, &options.output]
        .map(|p| p.to_str().unwrap().to_ascii_lowercase());
    if paths[0] == paths[1] || paths[0] == paths[2] || paths[1] == paths[2] {
        return Err("runtime, scene and output must be distinct paths".into());
    }
    Ok(())
}
fn validate_path(path: &Path) -> Result<()> {
    let text = path.to_str().ok_or("path must be ASCII")?;
    let bytes = text.as_bytes();
    if bytes.len() < 6
        || bytes.len() > 240
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1..3] != *b":\\"
    {
        return Err(
            "use an absolute local drive file path of at most 240 bytes with backslashes".into(),
        );
    }
    let parts: Vec<_> = text[3..].split('\\').collect();
    if parts.len() < 2 {
        return Err("file needs a non-root parent directory".into());
    }
    for part in parts {
        basename(part)?;
    }
    Ok(())
}
fn basename(name: &str) -> Result<()> {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if name.is_empty()
        || name.len() > 150
        || name.ends_with(['.', ' '])
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || name == "."
        || name == ".."
        || [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&stem.as_str())
    {
        return Err("canonical ASCII basename required (letters, digits, dot, underscore, hyphen; no device/alias names)".into());
    }
    Ok(())
}
#[cfg(any(all(windows, target_arch = "x86_64"), test))]
fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[cfg(any(all(windows, target_arch = "x86_64"), test))]
fn expected(scene: &[u8]) -> Result<String> {
    use orr_games::coop_defense_game as game;
    use orr_sim::{Simulation, TickInputs};
    let config = orr_games::coop_defense_scene::parse_config(
        std::str::from_utf8(scene).map_err(|_| "scene must be UTF-8")?,
    )?;
    let mut sim = Simulation::<game::DefenseGame>::with_build_id(
        config,
        game::TICK_RATE,
        u64::from(config.seed),
        game::SCENARIO_BUILD_ID,
    );
    for tick in 1..=60 {
        sim.step(&TickInputs::new(tick, 1));
    }
    let frame = sim.frame();
    let run = game::run(frame);
    Ok(format!(
        "Stormglass tick={} wave={} phase={} reactor={} kills={} merges={} checksum=0x{:016x}",
        run.tick,
        run.wave,
        run.phase,
        run.reactor_hp,
        run.kills,
        run.merges,
        frame.checksum()
    ))
}
#[cfg(test)]
mod tests;
