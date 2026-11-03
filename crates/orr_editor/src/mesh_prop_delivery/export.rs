//! Explicitly trusted local-runtime publisher; supervision is not sandboxing.
use super::{
    io::{self, Snapshot},
    manifest::*,
    source::SavedSource,
};
use orr_sample::trusted_runtime::{publish_no_replace, run_bounded, validate_elf};
use std::{
    ffi::OsString,
    fs::{self, File},
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
pub const USAGE:&str="orr_mesh_prop_export --scene FILE --runtime TRUSTED_PLAYER --runtime-sha256 HEX --trusted-runtime --output NEW_DIRECTORY\nLinux x86_64 deterministic headless delivery. The supplied runtime executes locally; hashes are not authentication.";
#[derive(Clone, Debug)]
pub struct ExportRequest {
    pub runtime: PathBuf,
    pub runtime_sha256: String,
    pub output: PathBuf,
    pub trusted_runtime: bool,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct ExportReport {
    pub output: PathBuf,
    pub initial_checksum: u64,
    pub manifest_sha256: String,
}
pub fn run_cli(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut argsv = Vec::new();
    let mut bytes = 0;
    for a in args {
        bytes += a.len();
        if bytes > 16384 || argsv.len() >= 9 {
            return Err("export argument budget exceeded".into());
        }
        argsv.push(a);
    }
    if argsv == [OsString::from("--help")] {
        println!("{USAGE}");
        return Ok(());
    }
    let (mut scene, mut runtime, mut digest, mut output, mut trusted) =
        (None, None, None, None, false);
    let mut args = argsv.into_iter();
    while let Some(a) = args.next() {
        let key = a.to_str().ok_or("option must be UTF-8")?;
        if key == "--trusted-runtime" && !trusted {
            trusted = true;
            continue;
        }
        let value = args.next().ok_or("missing option value")?;
        match key {
            "--scene" if scene.is_none() => scene = Some(PathBuf::from(value)),
            "--runtime" if runtime.is_none() => runtime = Some(PathBuf::from(value)),
            "--runtime-sha256" if digest.is_none() => {
                digest = Some(value.into_string().map_err(|_| "digest must be UTF-8")?)
            }
            "--output" if output.is_none() => output = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown/duplicate option {key}")),
        }
    }
    let request = ExportRequest {
        runtime: runtime.ok_or("runtime required")?,
        runtime_sha256: digest.ok_or("runtime digest required")?,
        output: output.ok_or("output required")?,
        trusted_runtime: trusted,
    };
    let source = SavedSource::capture(&scene.ok_or("scene required")?)?;
    let report = export_for_source(&request, &source)?;
    println!(
        "{}",
        serde_json::to_string(&report).map_err(|e| e.to_string())?
    );
    Ok(())
}
fn fresh_output(path: &Path, source: &Path, runtime: &Path) -> Result<PathBuf, String> {
    let path = io::checked_path(path)?;
    let parent = path.parent().ok_or("output parent missing")?;
    io::directory(parent)?;
    if path.starts_with(source) || source.starts_with(&path) || runtime.starts_with(&path) {
        return Err("output must not overlap source directory or runtime".into());
    }
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(e) => Err(e.to_string()),
        Ok(_) => Err("output already exists".into()),
    }
}
fn write(root: &Path, name: &str, bytes: &[u8], mode: u32) -> Result<(), String> {
    let parent = io::directory(root)?;
    let mut f = File::from(
        rustix::fs::openat(
            &parent,
            name,
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
fn same_dir(path: &Path, old: &fs::Metadata) -> Result<(), String> {
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_dir() || m.dev() != old.dev() || m.ino() != old.ino() {
        return Err("owned directory identity changed".into());
    }
    Ok(())
}
pub fn export_for_source(
    request: &ExportRequest,
    source: &SavedSource,
) -> Result<ExportReport, String> {
    export_checked(request, source, |_| Ok(()))
}
pub(crate) fn export_checked(
    request: &ExportRequest,
    source: &SavedSource,
    mut checkpoint: impl FnMut(&str) -> Result<(), String>,
) -> Result<ExportReport, String> {
    if !request.trusted_runtime {
        return Err("explicit runtime trust required; runtime is not sandboxed".into());
    }
    if !valid_hash(&request.runtime_sha256) {
        return Err("runtime SHA256 must be lowercase hex".into());
    }
    source.recheck()?;
    let runtime_path = io::checked_path(&request.runtime)?;
    let runtime = Snapshot::read(&runtime_path, MAX_RUNTIME)?;
    if runtime.mode & 0o111 == 0 || hash(&runtime.bytes) != request.runtime_sha256 {
        return Err("trusted runtime executable mode or digest mismatch".into());
    }
    validate_elf(&runtime.bytes)?;
    let source_root = source.path.parent().ok_or("scene parent missing")?;
    let output = fresh_output(&request.output, source_root, &runtime_path)?;
    let parent = output.parent().ok_or("output parent missing")?;
    let parent_fd = io::directory(parent)?;
    let parent_meta = parent_fd.metadata().map_err(|e| e.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix(".orr-mesh-props-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?
        .keep();
    let stage_meta = fs::symlink_metadata(&stage).map_err(|e| e.to_string())?;
    let mut published = false;
    let result = (|| {
        checkpoint("stage-created")?;
        same_dir(&stage, &stage_meta)?;
        io::recheck_directory(parent, &parent_fd)?;
        let mut files = Vec::new();
        for (name, snapshot) in &source.files {
            write(&stage, name, &snapshot.bytes, 0o644)?;
            files.push(Descriptor::new(name.clone(), &snapshot.bytes, 0o644));
        }
        write(&stage, RUNTIME_NAME, &runtime.bytes, 0o755)?;
        files.push(Descriptor::new(RUNTIME_NAME.into(), &runtime.bytes, 0o755));
        let manifest = Manifest::new(
            source.files[0].0.clone(),
            source.initial_checksum(),
            source.graph_revision(),
            files,
        );
        let encoded = manifest.encode()?;
        write(&stage, MANIFEST_NAME, &encoded, 0o644)?;
        let verified = super::verify(&stage, None)?;
        checkpoint("before-smoke")?;
        verified.recheck()?;
        same_dir(&stage, &stage_meta)?;
        io::recheck_directory(parent, &parent_fd)?;
        let mut capability = command(&stage);
        capability.arg("--capabilities");
        if run_bounded(&mut capability, Duration::from_secs(10), 65536)? != CAPABILITY.as_bytes() {
            return Err("runtime capability mismatch".into());
        }
        verified.recheck()?;
        same_dir(&stage, &stage_meta)?;
        io::recheck_directory(parent, &parent_fd)?;
        let expected = super::player::trajectory(&verified.scene, 60)?;
        let mut smoke = command(&stage);
        smoke
            .arg("--bundle")
            .arg(&stage)
            .args(["--ticks", "60", "--restart-check"]);
        verified.recheck()?;
        same_dir(&stage, &stage_meta)?;
        io::recheck_directory(parent, &parent_fd)?;
        let result = run_bounded(&mut smoke, Duration::from_secs(30), 65536)?;
        let receipt: serde_json::Value =
            serde_json::from_slice(&result).map_err(|e| format!("runtime smoke receipt: {e}"))?;
        if receipt.get("trajectory")
            != Some(&serde_json::to_value(&expected).map_err(|e| e.to_string())?)
        {
            return Err("runtime smoke trajectory differs from independently admitted host".into());
        }
        if receipt.get("manifest_sha256").and_then(|v| v.as_str()) != Some(hash(&encoded).as_str())
        {
            return Err("runtime receipt manifest identity mismatch".into());
        }
        if receipt.get("restart_checked").and_then(|v| v.as_bool()) != Some(true) {
            return Err("runtime did not confirm restart verification".into());
        }
        checkpoint("before-publish")?;
        source.recheck()?;
        runtime.recheck()?;
        verified.recheck()?;
        same_dir(&stage, &stage_meta)?;
        same_dir(parent, &parent_meta)?;
        io::recheck_directory(parent, &parent_fd)?;
        File::open(&stage)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        fresh_output(&output, source_root, &runtime_path)?;
        publish_no_replace(&stage, &output)?;
        published = true;
        checkpoint("after-publish")?;
        same_dir(&output, &stage_meta)?;
        io::recheck_directory(parent, &parent_fd)?;
        parent_fd.sync_all().map_err(|e| e.to_string())?;
        Ok(ExportReport {
            output: output.clone(),
            initial_checksum: source.initial_checksum(),
            manifest_sha256: hash(&encoded),
        })
    })();
    result.map_err(|e: String| {
        if published {
            format!(
                "bundle published at {}; durability/identity uncertain: {e}; no rollback or retry",
                output.display()
            )
        } else {
            format!(
                "bundle not published; partial private stage retained at {}: {e}",
                stage.display()
            )
        }
    })
}
fn command(stage: &Path) -> Command {
    let mut cmd = Command::new(stage.join(RUNTIME_NAME));
    cmd.current_dir(stage)
        .env_remove("LD_PRELOAD")
        .env_remove("LD_LIBRARY_PATH");
    cmd
}
