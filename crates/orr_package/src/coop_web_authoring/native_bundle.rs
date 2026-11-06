//! A bounded Linux x86-64 Stormglass delivery, not a general executable sandbox.
//! Only explicitly trusted, hash-pinned runtime bytes are executed. Flat layout
//! makes the existing native run-store exclude the whole bundle from saves.
mod process;
use super::{absolute, prepared::digest, publish};
use crate::authoring_io::{directory, error, read_at, recheck_directory, DIR_FLAGS};
use orr_games::{coop_defense_game as game, coop_defense_scene::parse_config};
use orr_sim::{Simulation, TickInputs};
use rustix::fs::{self as unix, AtFlags, Mode, OFlags, RenameFlags};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, File},
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const MAX_RUNTIME: usize = 256 * 1024 * 1024;
const MAX_SCENE: usize = 65536;
const SMOKE_TICKS: u32 = 60;
const LAUNCHER: &str = include_str!("native_bundle/launch.sh.in");
const USAGE: &str = "orr_pkg pack-coop-native --runtime TRUSTED_EXECUTABLE --runtime-sha256 LOWERCASE_SHA256 --scene SCENE --output NEW_DIRECTORY --trusted-runtime\nLinux x86-64 only. Executes the copied, explicitly trusted runtime for 60 idle ticks against the strictly admitted scene and compares the exact native summary/checksum. No compilation, downloads, GPU/window, implicit saves or replacement. Hashes detect mix-ups, not malicious code. Launcher needs /bin/sh and /usr/bin/sha256sum; host graphics/libraries remain required. Keep private saves outside the flat bundle.";
static SERIAL: AtomicU64 = AtomicU64::new(0);

struct Options {
    runtime: PathBuf,
    runtime_sha256: String,
    scene: PathBuf,
    output: PathBuf,
}
fn options(args: &[OsString]) -> Result<Options, String> {
    let mut paths = BTreeMap::new();
    let mut trusted = false;
    let mut seen = BTreeSet::new();
    let mut index = 0;
    while index < args.len() {
        let key = args[index].to_str().ok_or("flag must be Unicode")?;
        if !seen.insert(key) {
            return Err("duplicate native bundle flag".into());
        }
        index += 1;
        if key == "--trusted-runtime" {
            trusted = true;
            continue;
        }
        if !matches!(
            key,
            "--runtime" | "--runtime-sha256" | "--scene" | "--output"
        ) {
            return Err("unknown native bundle flag".into());
        }
        let value = args.get(index).ok_or("missing native bundle flag value")?;
        paths.insert(key, value);
        index += 1;
    }
    if !trusted {
        return Err("--trusted-runtime is required before executing supplied code".into());
    }
    let get = |key| {
        paths
            .get(key)
            .copied()
            .ok_or_else(|| format!("{key} required"))
    };
    let sha = get("--runtime-sha256")?
        .to_str()
        .ok_or("digest must be Unicode")?;
    if sha.len() != 64
        || !sha
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("runtime SHA-256 must be 64 lowercase hexadecimal characters".into());
    }
    Ok(Options {
        runtime: absolute(Path::new(get("--runtime")?))?,
        runtime_sha256: sha.to_owned(),
        scene: absolute(Path::new(get("--scene")?))?,
        output: absolute(Path::new(get("--output")?))?,
    })
}

struct Snapshot {
    bytes: Vec<u8>,
    metadata: fs::Metadata,
    path: PathBuf,
    max: usize,
}
impl Snapshot {
    fn read(path: &Path, max: usize) -> Result<Self, String> {
        let root = path.parent().ok_or("input parent missing")?;
        let name = path.file_name().ok_or("input filename missing")?;
        let parent = directory(root)?;
        let file = File::from(
            unix::openat(
                &parent,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(error)?,
        );
        let metadata = file.metadata().map_err(error)?;
        if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > max as u64 {
            return Err("input must be a bounded single-link regular file".into());
        }
        let bytes = read_at(&parent, name, max)?;
        let after = file.metadata().map_err(error)?;
        let named = unix::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if !crate::authoring_io::same_snapshot(&metadata, &after)
            || metadata.mode() != after.mode()
            || metadata.nlink() != after.nlink()
            || named.st_dev != metadata.dev()
            || named.st_ino != metadata.ino()
        {
            return Err("input changed while snapshotting".into());
        }
        recheck_directory(root, &parent)?;
        Ok(Self {
            bytes,
            metadata,
            path: path.to_owned(),
            max,
        })
    }
    fn recheck(&self) -> Result<(), String> {
        let now = Self::read(&self.path, self.max)?;
        if !crate::authoring_io::same_snapshot(&self.metadata, &now.metadata)
            || self.metadata.mode() != now.metadata.mode()
            || self.bytes != now.bytes
        {
            return Err("input changed before publication".into());
        }
        Ok(())
    }
}
fn validate_runtime(snapshot: &Snapshot, expected: &str) -> Result<(), String> {
    let bytes = &snapshot.bytes;
    if digest(bytes) != expected {
        return Err("runtime SHA-256 mismatch".into());
    }
    if snapshot.metadata.mode() & 0o111 == 0 || snapshot.metadata.mode() & 0o7022 != 0 {
        return Err("runtime must be executable without special/shared-write permissions".into());
    }
    if bytes.len() < 64
        || &bytes[..7] != b"\x7fELF\x02\x01\x01"
        || !matches!(u16::from_le_bytes([bytes[16], bytes[17]]), 2 | 3)
        || u16::from_le_bytes([bytes[18], bytes[19]]) != 62
        || u16::from_le_bytes([bytes[52], bytes[53]]) != 64
    {
        return Err("runtime must be a little-endian x86-64 ELF64 executable".into());
    }
    Ok(())
}
fn expected_smoke(scene: &[u8]) -> Result<String, String> {
    let config = parse_config(std::str::from_utf8(scene).map_err(error)?)?;
    let mut simulation = Simulation::<game::DefenseGame>::with_build_id(
        config,
        game::TICK_RATE,
        u64::from(config.seed),
        game::SCENARIO_BUILD_ID,
    );
    for tick in 1..=SMOKE_TICKS {
        simulation.step(&TickInputs::new(u64::from(tick), 1));
    }
    let frame = simulation.frame();
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
fn verify_smoke(bytes: &[u8], expected: &str) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(error)?;
    let mut lines = text.lines();
    let first = lines.next().ok_or("missing native summary")?;
    let actual = if let Some(extra) = first.strip_prefix("Stormglass pixelfx ") {
        let fields: Vec<_> = extra.split(' ').collect();
        if fields.len() != 4
            || fields
                .iter()
                .zip(["active=", "started=", "dropped=", "canceled="])
                .any(|(field, prefix)| {
                    field
                        .strip_prefix(prefix)
                        .is_none_or(|v| v.parse::<u32>().is_err())
                })
        {
            return Err("invalid optional pixelfx summary".into());
        }
        lines.next().ok_or("missing native summary after pixelfx")?
    } else {
        first
    };
    if actual != expected || lines.next().is_some() {
        return Err("copied native runtime summary/checksum does not match admitted scene".into());
    }
    Ok(())
}
fn smoke(root: &Path) -> Result<Vec<u8>, String> {
    let mut command = Command::new(root.join("coop_defense"));
    command
        .env_clear()
        .env("PATH", "")
        .env("LANG", "C")
        .current_dir(root)
        .args(["--headless", "--ticks", "60", "--scene"])
        .arg(root.join("game.scene.yaml"));
    process::run_bounded(&mut command, Duration::from_secs(15), 65536)
}

struct StagedFile {
    bytes: Vec<u8>,
    mode: u32,
    dev: u64,
    ino: u64,
    // Pin the inode until publication, including across unlink/recreate attacks.
    // This must be read-only: a retained writer would prevent executing the player.
    handle: File,
}
fn write_file(stage: &File, name: &str, bytes: Vec<u8>, mode: u32) -> Result<StagedFile, String> {
    let mut file = File::from(
        unix::openat(
            stage,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(error)?,
    );
    file.write_all(&bytes).map_err(error)?;
    file.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(error)?;
    file.sync_all().map_err(error)?;
    let info = file.metadata().map_err(error)?;
    let handle = File::from(
        unix::openat(
            stage,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(error)?,
    );
    let pinned = handle.metadata().map_err(error)?;
    if !pinned.is_file() || pinned.dev() != info.dev() || pinned.ino() != info.ino() {
        return Err("staged identity changed while pinning file".into());
    }
    Ok(StagedFile {
        bytes,
        mode,
        dev: info.dev(),
        ino: info.ino(),
        handle,
    })
}
fn verify_stage(
    path: &Path,
    stage: &File,
    files: &BTreeMap<&str, StagedFile>,
) -> Result<(), String> {
    recheck_directory(path, stage)?;
    if stage.metadata().map_err(error)?.mode() & 0o7777 != 0o700 {
        return Err("staged root must remain private mode 0700".into());
    }
    let mut seen = BTreeSet::new();
    for entry in fs::read_dir(path).map_err(error)? {
        let entry = entry.map_err(error)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "non-UTF8 staged entry")?;
        if !files.contains_key(name.as_str())
            || !entry.file_type().map_err(error)?.is_file()
            || !seen.insert(name)
        {
            return Err("unexpected staged entry".into());
        }
    }
    if seen.len() != files.len() {
        return Err("missing staged entry".into());
    }
    for (name, file) in files {
        let pinned = file.handle.metadata().map_err(error)?;
        let current = unix::statat(stage, *name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if pinned.dev() != file.dev
            || pinned.ino() != file.ino
            || pinned.nlink() != 1
            || pinned.mode() & 0o7777 != file.mode
            || current.st_dev != file.dev
            || current.st_ino != file.ino
            || current.st_nlink != 1
            || current.st_mode & 0o7777 != file.mode
            || read_at(stage, name.as_ref(), file.bytes.len())? != file.bytes
        {
            return Err("staged bytes, identity or permissions changed".into());
        }
    }
    recheck_directory(path, stage)
}
fn create(options: &Options) -> Result<(Value, Option<String>), String> {
    create_with(options, smoke, |_, _| Ok(()))
}
fn create_with(
    options: &Options,
    mut run: impl FnMut(&Path) -> Result<Vec<u8>, String>,
    mut checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
) -> Result<(Value, Option<String>), String> {
    if !cfg!(target_arch = "x86_64") {
        return Err("native bundles require Linux x86-64".into());
    }
    publish::preflight(&options.output)?;
    for input in [&options.runtime, &options.scene] {
        if input.starts_with(&options.output) || options.output.starts_with(input) {
            return Err("output overlaps input".into());
        }
    }
    let runtime = Snapshot::read(&options.runtime, MAX_RUNTIME)?;
    validate_runtime(&runtime, &options.runtime_sha256)?;
    let scene = Snapshot::read(&options.scene, MAX_SCENE)?;
    let expected = expected_smoke(&scene.bytes)?;
    let scene_sha = digest(&scene.bytes);
    let root = options.output.parent().ok_or("output parent missing")?;
    let target = options.output.file_name().ok_or("output name missing")?;
    let parent = directory(root)?;
    let mut name = None;
    for _ in 0..128 {
        let candidate = format!(
            ".orr-native-bundle-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        match unix::mkdirat(
            &parent,
            candidate.as_str(),
            Mode::RUSR | Mode::WUSR | Mode::XUSR,
        ) {
            Ok(()) => {
                name = Some(candidate);
                break;
            }
            Err(rustix::io::Errno::EXIST) => continue,
            Err(e) => return Err(error(e)),
        }
    }
    let name = name.ok_or("cannot allocate private native bundle stage")?;
    let stage_path = root.join(&name);
    let stage = File::from(
        unix::openat(&parent, name.as_str(), DIR_FLAGS, Mode::empty()).map_err(|e| {
            format!(
                "{e}; unpublished stage retained at {}",
                stage_path.display()
            )
        })?,
    );
    let mut published = false;
    let receipt = json!({
        "kind": "orr.coop-native-bundle/1",
        "target": "x86_64-unknown-linux-gnu",
        "runtime": {"file": "coop_defense", "bytes": runtime.bytes.len(), "sha256": options.runtime_sha256},
        "scene": {"file": "game.scene.yaml", "bytes": scene.bytes.len(), "sha256": scene_sha},
        "admissionBuildId": format!("{:016x}", game::SCENARIO_BUILD_ID),
        "smoke": {"idleTicks": SMOKE_TICKS, "expectedSummary": expected},
        "savePolicy": "explicit-existing-private-directory-outside-bundle; matching-executable-only",
        "limits": "trusted runtime only; no source attestation, GPU/window, hardware, other OS or full campaign acceptance"
    });
    let result = (|| -> Result<(), String> {
        checkpoint("stage-created", &stage_path)?;
        let mut files = BTreeMap::new();
        files.insert(
            "coop_defense",
            write_file(&stage, "coop_defense", runtime.bytes.clone(), 0o755)?,
        );
        files.insert(
            "game.scene.yaml",
            write_file(&stage, "game.scene.yaml", scene.bytes.clone(), 0o644)?,
        );
        verify_stage(&stage_path, &stage, &files)?;
        verify_smoke(&run(&stage_path)?, &expected)?;
        checkpoint("after-smoke", &stage_path)?;
        verify_stage(&stage_path, &stage, &files)?;
        let launcher = LAUNCHER
            .replace("@RUNTIME_SHA256@", &options.runtime_sha256)
            .replace("@SCENE_SHA256@", &scene_sha);
        files.insert(
            "run-game",
            write_file(&stage, "run-game", launcher.into_bytes(), 0o755)?,
        );
        files.insert(
            "bundle.json",
            write_file(
                &stage,
                "bundle.json",
                serde_json::to_vec_pretty(&receipt).map_err(error)?,
                0o644,
            )?,
        );
        stage.sync_all().map_err(error)?;
        checkpoint("before-rename", &stage_path)?;
        runtime.recheck()?;
        scene.recheck()?;
        verify_stage(&stage_path, &stage, &files)?;
        recheck_directory(root, &parent)?;
        unix::renameat_with(
            &parent,
            name.as_str(),
            &parent,
            target,
            RenameFlags::NOREPLACE,
        )
        .map_err(error)?;
        published = true;
        checkpoint("after-rename", &options.output)?;
        recheck_directory(&options.output, &stage)?;
        parent.sync_all().map_err(error)?;
        Ok(())
    })();
    match result {
        Ok(()) => Ok((receipt, None)),
        Err(e) if published => Ok((
            receipt,
            Some(format!(
                "published; durability uncertain: {e}; inspect output; do not retry"
            )),
        )),
        Err(e) => Err(format!(
            "{e}; unpublished stage retained at {}; no automatic deletion",
            stage_path.display()
        )),
    }
}
pub(super) fn run_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    let mut values = Vec::new();
    let mut total = 0usize;
    for arg in args {
        total = total.saturating_add(arg.as_encoded_bytes().len());
        if values.len() >= 10 || total > 32768 {
            eprintln!("native bundle argv exceeds bounds");
            return 2;
        }
        values.push(arg);
    }
    if values.len() == 1 && (values[0] == "--help" || values[0] == "-h") {
        println!("{USAGE}");
        return 0;
    }
    let options = match options(&values) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return 2;
        }
    };
    match create(&options) {
        Ok((receipt, warning)) => {
            if let Some(warning) = warning {
                eprintln!("warning: {warning}");
            }
            println!(
                "{}",
                serde_json::to_string(&receipt).expect("native bundle receipt serialization")
            );
            0
        }
        Err(e) => {
            eprintln!("Native bundle refused: {e}");
            1
        }
    }
}
#[cfg(test)]
#[path = "native_bundle/tests.rs"]
mod tests;
