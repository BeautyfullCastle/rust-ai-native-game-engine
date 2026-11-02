//! Closed NavigationYard3D bundle publisher. Supplied runtime is explicitly trusted
//! local code, not authenticated or sandboxed by hash/ELF/capability/smoke checks.
use super::{
    io::{checked_path, Snapshot},
    manifest::Manifest,
    verify_bundle, LAUNCHER, MANIFEST_PATH, RUNTIME_PATH, SCENE_PATH,
};
use crate::{authored::AuthoredPlayer, player};
use orr_remote::navigation_yard3d::NavigationScenePin;
use orr_sample::trusted_runtime::{publish_no_replace, run_bounded, validate_elf};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, File},
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
const MAX_RUNTIME: usize = 64 * 1024 * 1024;
const SMOKE_TICKS: &str = "60";
static SERIAL: AtomicU64 = AtomicU64::new(1);
pub const USAGE: &str = "navigation_export --scene FILE --runtime TRUSTED_PLAYER --runtime-sha256 HEX --trusted-runtime --output NEW_DIRECTORY [--source-revision USER_CLAIM]\nLinux x86-64 closed point-agent bundle; hash/capabilities are not authentication or license proof.";
#[derive(Debug)]
struct Options {
    scene: PathBuf,
    runtime: PathBuf,
    hash: String,
    output: PathBuf,
    revision: Option<String>,
}
fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Option<Options>, String> {
    let mut argsv = Vec::new();
    let mut bytes = 0usize;
    for arg in args {
        bytes = bytes
            .checked_add(arg.len())
            .ok_or("argument size overflow")?;
        if bytes > 16 * 1024 || argsv.len() >= 11 {
            return Err("export argument budget exceeded".into());
        }
        argsv.push(arg);
    }
    if argsv.len() == 1 && argsv[0] == "--help" {
        return Ok(None);
    }
    let (mut scene, mut runtime, mut hash, mut output, mut revision, mut trusted) =
        (None, None, None, None, None, false);
    let mut args = argsv.into_iter();
    while let Some(arg) = args.next() {
        let key = arg.to_str().ok_or("option must be UTF-8")?;
        if key == "--trusted-runtime" {
            if trusted {
                return Err("duplicate trust flag".into());
            }
            trusted = true;
            continue;
        }
        let value = args.next().ok_or("missing option value")?;
        match key {
            "--scene" if scene.is_none() => scene = Some(PathBuf::from(value)),
            "--runtime" if runtime.is_none() => runtime = Some(PathBuf::from(value)),
            "--output" if output.is_none() => output = Some(PathBuf::from(value)),
            "--runtime-sha256" if hash.is_none() => {
                hash = Some(value.into_string().map_err(|_| "hash must be UTF-8")?)
            }
            "--source-revision" if revision.is_none() => {
                let text = value.into_string().map_err(|_| "revision must be UTF-8")?;
                if text.is_empty() || text.len() > 4096 || text.chars().any(char::is_control) {
                    return Err("revision must be 1..4096 noncontrol UTF-8 bytes".into());
                }
                revision = Some(text)
            }
            _ => return Err(format!("unknown or duplicate option: {key}")),
        }
    }
    if !trusted {
        return Err("explicit --trusted-runtime is required; runtime is not sandboxed".into());
    }
    let hash: String = hash.ok_or("runtime hash required")?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("runtime SHA256 must be 64 lowercase hex characters".into());
    }
    Ok(Some(Options {
        scene: scene.ok_or("scene required")?,
        runtime: runtime.ok_or("runtime required")?,
        hash,
        output: output.ok_or("output required")?,
        revision,
    }))
}
/// Execute only after an operator explicitly asserts trust in the supplied runtime.
pub fn run_cli(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let Some(options) = parse(args)? else {
        println!("{USAGE}");
        return Ok(());
    };
    let path = export(&options, |_| Ok(()))?;
    println!("{}", path.display());
    Ok(())
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn fresh_output(path: &Path, source_root: &Path, runtime: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().len() > 4096 {
        return Err("output path exceeds4KiB".into());
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    if !absolute.components().all(|c| {
        matches!(
            c,
            std::path::Component::RootDir | std::path::Component::Normal(_)
        )
    }) {
        return Err("output path must be canonical without traversal".into());
    }
    let parent = checked_path(absolute.parent().ok_or("output parent missing")?)?;
    super::io::directory(&parent)?;
    let output = parent.join(absolute.file_name().ok_or("output name missing")?);
    if output.starts_with(source_root)
        || source_root.starts_with(&output)
        || runtime.starts_with(&output)
    {
        return Err("output and authored inputs must not overlap".into());
    }
    match fs::symlink_metadata(&output) {
        Ok(_) => Err("output already exists".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(output),
        Err(e) => Err(e.to_string()),
    }
}
fn write(root: &Path, relative: &str, bytes: &[u8], mode: u32) -> Result<(), String> {
    let path = root.join(relative);
    let parent = path.parent().ok_or("payload parent missing")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let parent_fd = super::io::directory(parent)?;
    let mut f = File::from(
        rustix::fs::openat(
            &parent_fd,
            path.file_name().ok_or("filename missing")?,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_bits_truncate(0o600),
        )
        .map_err(|e| e.to_string())?,
    );
    f.write_all(bytes).map_err(|e| e.to_string())?;
    f.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}
fn same_dir(path: &Path, expected: &fs::Metadata) -> Result<(), String> {
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_dir() || m.dev() != expected.dev() || m.ino() != expected.ino() {
        return Err("owned stage/parent directory identity changed".into());
    }
    Ok(())
}
fn export(
    options: &Options,
    mut checkpoint: impl FnMut(&str) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let scene_path = checked_path(&options.scene)?;
    let runtime_path = checked_path(&options.runtime)?;
    let source_root = scene_path.parent().ok_or("scene parent missing")?;
    let output = fresh_output(&options.output, source_root, &runtime_path)?;
    let scene = Snapshot::read(&scene_path, 1024 * 1024)?;
    let admitted = AuthoredPlayer::open(&scene_path)?;
    let terrain_relative = admitted
        .frame()
        .singleton::<NavigationScenePin>()
        .source()?
        .to_owned();
    let terrain = Snapshot::read(
        &source_root.join(&terrain_relative),
        orr_terrain::MAX_FILE_BYTES,
    )?;
    let runtime = Snapshot::read(&runtime_path, MAX_RUNTIME)?;
    if runtime.mode & 0o111 == 0 {
        return Err("trusted runtime must already be executable".into());
    }
    if hash(&runtime.bytes) != options.hash {
        return Err("trusted runtime SHA256 mismatch".into());
    }
    validate_elf(&runtime.bytes)?;
    let manifest = Manifest::from_snapshots(
        &scene,
        &terrain_relative,
        &terrain,
        &runtime,
        options.revision.clone(),
        &admitted,
    )?;
    let manifest_bytes = manifest.encode()?;
    scene.recheck()?;
    terrain.recheck()?;
    runtime.recheck()?;
    let parent = output.parent().ok_or("output parent missing")?;
    let parent_fd = super::io::directory(parent)?;
    let parent_meta = fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
    let mut stage = None;
    for _ in 0..32 {
        let candidate = parent.join(format!(
            ".orr-navigation-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::DirBuilder::new().mode(0o700).create(&candidate) {
            Ok(()) => {
                stage = Some(candidate);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    let stage = stage.ok_or("cannot claim unique private stage")?;
    let stage_meta = fs::symlink_metadata(&stage).map_err(|e| e.to_string())?;
    let mut published = false;
    let result: Result<PathBuf, String> = (|| {
        checkpoint("stage-created")?;
        same_dir(&stage, &stage_meta)?;
        super::io::recheck_directory(parent, &parent_fd)?;
        write(&stage, SCENE_PATH, &scene.bytes, 0o644)?;
        let terrain_path = format!("project/{terrain_relative}");
        write(&stage, &terrain_path, &terrain.bytes, 0o644)?;
        write(&stage, RUNTIME_PATH, &runtime.bytes, 0o755)?;
        write(&stage, "run-navigation", LAUNCHER, 0o755)?;
        write(&stage, MANIFEST_PATH, &manifest_bytes, 0o644)?;
        let staged = verify_bundle(&stage, None)?;
        if staged.frame().to_bytes() != admitted.frame().to_bytes() {
            return Err("staged initial Frame differs from source admission".into());
        }
        checkpoint("before-smoke")?;
        let runtime_stage = stage.join(RUNTIME_PATH);
        let mut capability = Command::new(&runtime_stage);
        capability
            .arg("--capabilities")
            .current_dir(&stage)
            .env_remove("LD_PRELOAD")
            .env_remove("LD_LIBRARY_PATH");
        if run_bounded(&mut capability, Duration::from_secs(10), 65536)?
            != b"ORR_NAVIGATION_PLAYER_V1\nORR_NAVIGATION_BUNDLE_V1\n"
        {
            return Err("trusted runtime lacks exact navigation bundle capabilities".into());
        }
        let args = [
            OsString::from("--scene"),
            stage.join(SCENE_PATH).into_os_string(),
            OsString::from("--ticks"),
            OsString::from(SMOKE_TICKS),
            OsString::from("--restart-check"),
        ];
        let expected = player::run(player::parse(args)?.ok_or("missing local smoke options")?)?;
        let mut expected = serde_json::to_vec(&expected).map_err(|e| e.to_string())?;
        expected.push(b'\n');
        let mut command = Command::new(&runtime_stage);
        command
            .arg("--bundle")
            .arg(&stage)
            .args(["--ticks", SMOKE_TICKS, "--restart-check"])
            .current_dir(&stage)
            .env_remove("LD_PRELOAD")
            .env_remove("LD_LIBRARY_PATH");
        if run_bounded(&mut command, Duration::from_secs(10), 65536)? != expected {
            return Err(
                "staged runtime smoke differs from independent local Frame trajectory receipt"
                    .into(),
            );
        }
        checkpoint("before-publish")?;
        scene.recheck()?;
        terrain.recheck()?;
        runtime.recheck()?;
        let final_staged = verify_bundle(&stage, None)?;
        if final_staged.frame().to_bytes() != admitted.frame().to_bytes() {
            return Err("stage admission changed".into());
        }
        if Snapshot::read(&stage.join(MANIFEST_PATH), 65536)?.bytes != manifest_bytes {
            return Err("stage manifest changed".into());
        }
        let mut dirs = BTreeSet::new();
        for relative in [
            SCENE_PATH,
            &terrain_path,
            RUNTIME_PATH,
            MANIFEST_PATH,
            "run-navigation",
        ] {
            let mut d = stage
                .join(relative)
                .parent()
                .ok_or("payload parent missing")?
                .to_path_buf();
            loop {
                dirs.insert(d.clone());
                if d == stage {
                    break;
                }
                d = d.parent().ok_or("stage parent escaped")?.to_path_buf();
            }
        }
        for dir in dirs.iter().rev() {
            File::open(dir)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
        super::io::recheck_directory(parent, &parent_fd)?;
        same_dir(parent, &parent_meta)?;
        same_dir(&stage, &stage_meta)?;
        fresh_output(&output, source_root, &runtime_path)?;
        publish_no_replace(&stage, &output)?;
        published = true;
        checkpoint("after-publish")?;
        same_dir(parent, &parent_meta)?;
        same_dir(&output, &stage_meta)?;
        parent_fd.sync_all().map_err(|e| e.to_string())?;
        Ok(output.clone())
    })();
    result.map_err(|e| {
        if published {
            format!(
                "bundle published at {}; durability/identity uncertain: {e}; no rollback or retry",
                output.display()
            )
        } else {
            format!(
                "bundle not published; private partial stage retained at {} for inspection: {e}",
                stage.display()
            )
        }
    })
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
