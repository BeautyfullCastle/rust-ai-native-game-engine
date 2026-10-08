//! Bounded Linux x86-64 export of an already-authored Arena project.
//!
//! The operator must supply known trusted, already-built code. A hash, ELF header,
//! declared revision and successful smoke output do not authenticate that code or
//! attest its source/capabilities. The smoke only checks this project on this host.
//! Inputs are read-only; this detects ordinary changes, not hostile filesystem races.
mod admission;

use admission::{check_path, hash, ProjectSnapshot, SnapshotFile, MAX_BINARY_BYTES};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const MANIFEST_NAME: &str = "orr.export.json";
pub const LAUNCHER_NAME: &str = "run-arena";
pub const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SMOKE_OUTPUT: usize = 4 * 1024 * 1024;
pub const SMOKE_TIMEOUT: Duration = Duration::from_secs(10);
const DOMAIN: &[u8] = b"orrery.arena.export.content.v1\0";
const LAUNCHER: &[u8] = b"#!/bin/sh\nset -eu\nroot=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd -P)\nexec \"$root/bin/arena\" --project \"$root/project\" \"$@\"\n";

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub project: PathBuf,
    pub runtime: PathBuf,
    pub runtime_sha256: String,
    pub output: PathBuf,
    /// Explicit caller assertion. This does not establish or verify trust.
    pub trusted_runtime: bool,
    /// Operator-declared provenance, never inferred from the exporter's checkout.
    pub source_revision: Option<String>,
}
#[derive(Debug)]
pub struct ExportReport {
    pub output: PathBuf,
    pub content_digest: String,
    pub project_files: usize,
    pub manifest: ExportManifest,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportFile {
    pub path: String,
    pub role: String,
    pub mode: u32,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MeasuredRuntime {
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeclaredProvenance {
    pub target: String,
    pub source_revision: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestPayload {
    pub exporter_version: String,
    pub profile: String,
    pub entry: orr_package::ProjectEntry,
    pub packages: BTreeMap<String, String>,
    pub files: Vec<ExportFile>,
    pub runtime: MeasuredRuntime,
    pub declared: DeclaredProvenance,
    pub initial_checksum: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportManifest {
    pub schema: u32,
    pub payload: ManifestPayload,
    pub content_digest: String,
}

/// Export into a new directory. Never overwrites any destination, including an
/// empty directory created concurrently. No compilation or installation occurs.
pub fn export(options: &ExportOptions) -> Result<ExportReport, String> {
    export_with(
        options,
        |_| Ok(()),
        |command| run_bounded(command, SMOKE_TIMEOUT, MAX_SMOKE_OUTPUT),
    )
}

fn export_with(
    options: &ExportOptions,
    mut checkpoint: impl FnMut(&str) -> Result<(), String>,
    smoke: impl FnOnce(&mut Command) -> Result<Vec<u8>, String>,
) -> Result<ExportReport, String> {
    if !options.trusted_runtime {
        return Err("export requires --trusted-runtime: supply known trusted already-built Arena code; its SHA256 does not establish trust".into());
    }
    if options.runtime_sha256.len() != 64
        || !options
            .runtime_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("runtime SHA256 must be exactly 64 lowercase hexadecimal characters".into());
    }
    if let Some(revision) = &options.source_revision {
        if revision.is_empty()
            || revision.starts_with('/')
            || revision.len() > 128
            || !revision
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b))
        {
            return Err(
                "declared source revision must be a nonempty ASCII token of at most 128 bytes"
                    .into(),
            );
        }
    }
    let project = ProjectSnapshot::open(&options.project)?;
    let runtime_path = check_path(&options.runtime, false)?;
    if fs::metadata(&runtime_path)
        .map_err(error)?
        .permissions()
        .mode()
        & 0o111
        == 0
    {
        return Err(
            "trusted runtime must already be executable; source permissions are never changed"
                .into(),
        );
    }
    let binary = SnapshotFile::read(&runtime_path, "bin/arena", MAX_BINARY_BYTES)?;
    if binary.sha256 != options.runtime_sha256 {
        return Err("trusted runtime SHA256 does not match expected hash".into());
    }
    validate_elf(&binary.bytes)?;
    let output = destination(&options.output, &project.root, &runtime_path)?;
    checkpoint("admitted")?;
    let parent = output.parent().ok_or("output parent missing")?;
    let stage = tempfile::Builder::new()
        .prefix(".orr-export-")
        .tempdir_in(parent)
        .map_err(error)?;
    let mut files = Vec::new();
    write_payload(
        stage.path(),
        "bin/arena",
        "runtime",
        &binary.bytes,
        0o755,
        &mut files,
    )?;
    checkpoint("binary-copied")?;
    for file in &project.files {
        write_payload(
            stage.path(),
            &format!("project/{}", file.relative),
            file.role,
            &file.bytes,
            0o644,
            &mut files,
        )?;
        checkpoint("content-copied")?;
    }
    write_payload(
        stage.path(),
        LAUNCHER_NAME,
        "launcher",
        LAUNCHER,
        0o755,
        &mut files,
    )?;
    checkpoint("before-validation")?;
    let staged = crate::project_runtime::PreparedRuntime::open(stage.path().join("project"))?;
    if staged.initial_frame().checksum() != project.initial_checksum {
        return Err("staged project initial checksum changed".into());
    }
    // The smoke uses only the copied, hash-checked artifact and staged project.
    // An empty cwd prevents incidental project lookup in the caller's directory.
    let cwd = stage.path().join(".smoke-cwd");
    fs::create_dir(&cwd).map_err(error)?;
    verify_staged_file(
        &stage.path().join("bin/arena"),
        binary.bytes.len() as u64,
        &binary.sha256,
        0o755,
    )?;
    let mut command = Command::new(stage.path().join("bin/arena"));
    command.env_clear();
    command
        .arg("--project")
        .arg(stage.path().join("project"))
        .args(["--headless", "--ticks", "0"])
        .current_dir(&cwd)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    let stdout = smoke(&mut command)?;
    validate_smoke(&stdout, &staged)?;
    fs::remove_dir(&cwd).map_err(|e| format!("smoke cwd must remain empty: {e}"))?;
    checkpoint("after-validation")?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let payload = ManifestPayload {
        exporter_version: env!("CARGO_PKG_VERSION").into(),
        profile: "arena-authored-linux-x86_64-v1".into(),
        entry: project.entry.clone(),
        packages: project.package_identities.clone(),
        files,
        runtime: MeasuredRuntime {
            sha256: binary.sha256.clone(),
            bytes: binary.bytes.len() as u64,
        },
        declared: DeclaredProvenance {
            target: "linux-x86_64".into(),
            source_revision: options.source_revision.clone(),
        },
        initial_checksum: format!("0x{:016x}", project.initial_checksum),
    };
    let canonical = serde_json::to_vec(&payload).map_err(error)?;
    let mut digest_bytes = Vec::with_capacity(DOMAIN.len() + canonical.len());
    digest_bytes.extend_from_slice(DOMAIN);
    digest_bytes.extend_from_slice(&canonical);
    let manifest = ExportManifest {
        schema: 1,
        payload,
        content_digest: hash(&digest_bytes),
    };
    let mut manifest_bytes = serde_json::to_vec(&manifest).map_err(error)?;
    manifest_bytes.push(b'\n');
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err("export manifest exceeds size limit".into());
    }
    write_new(&stage.path().join(MANIFEST_NAME), &manifest_bytes, 0o644)?;
    checkpoint("before-publish")?;
    // Re-read both sources (including aliases) and stage after the smoke. This
    // catches ordinary mutation, without claiming protection against hostile writers.
    project.recheck()?;
    binary.recheck()?;
    verify_stage(stage.path(), &manifest, &manifest_bytes)?;
    // Linux's one atomic operation closes the destination-existence TOCTOU gap.
    // Any unsupported kernel/filesystem error is returned, never downgraded to rename.
    publish_no_replace(stage.path(), &output)?;
    // TempDir still owns its old unique name; after rename that path is absent.
    drop(stage);
    Ok(ExportReport {
        output,
        content_digest: manifest.content_digest.clone(),
        project_files: project.files.len(),
        manifest,
    })
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn destination(path: &Path, project: &Path, runtime: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .ok_or("output needs a new directory leaf")?;
    if name.len() > 200
        || name
            .to_str()
            .is_none_or(|s| s.is_empty() || s.chars().any(char::is_control))
    {
        return Err(
            "output leaf must be UTF-8, at most 200 bytes, and contain no control characters"
                .into(),
        );
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let output = check_path(parent, true)?.join(name);
    if output.starts_with(project)
        || project.starts_with(&output)
        || output.starts_with(runtime)
        || runtime.starts_with(&output)
    {
        return Err("output must not overlap project or runtime source".into());
    }
    match fs::symlink_metadata(&output) {
        Ok(_) => Err("output destination already exists".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(output),
        Err(e) => Err(error(e)),
    }
}
fn validate_elf(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 64
        || &bytes[..4] != b"\x7fELF"
        || bytes[4..7] != [2, 1, 1]
        || !matches!(u16::from_le_bytes([bytes[16], bytes[17]]), 2 | 3)
        || u16::from_le_bytes([bytes[18], bytes[19]]) != 62
        || u16::from_le_bytes([bytes[52], bytes[53]]) != 64
    {
        return Err(
            "runtime must be a Linux-compatible little-endian x86-64 ELF executable".into(),
        );
    }
    Ok(())
}
fn write_new(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(error)?;
    file.write_all(bytes).map_err(error)?;
    file.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(error)?;
    file.sync_all().map_err(error)
}
fn write_payload(
    root: &Path,
    relative: &str,
    role: &str,
    bytes: &[u8],
    mode: u32,
    files: &mut Vec<ExportFile>,
) -> Result<(), String> {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().ok_or("payload parent missing")?).map_err(error)?;
    write_new(&path, bytes, mode)?;
    files.push(ExportFile {
        path: relative.into(),
        role: role.into(),
        mode,
        bytes: bytes.len() as u64,
        sha256: hash(bytes),
    });
    Ok(())
}
fn publish_no_replace(stage: &Path, output: &Path) -> Result<(), String> {
    publish_with(stage, output, |stage, output| {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            stage,
            rustix::fs::CWD,
            output,
            rustix::fs::RenameFlags::NOREPLACE,
        )
    })
}
fn publish_with(
    stage: &Path,
    output: &Path,
    operation: impl FnOnce(&Path, &Path) -> Result<(), rustix::io::Errno>,
) -> Result<(), String> {
    operation(stage, output).map_err(|e| format!("atomic no-replace publication failed: {e}"))
}

fn expected_smoke(prepared: &crate::project_runtime::PreparedRuntime) -> Result<Vec<u8>, String> {
    let checksum = prepared.initial_frame().checksum();
    let mut expected = format!("project initial checksum: 0x{checksum:016x}\nproject tick: 0 checksum: 0x{checksum:016x}\n");
    for (guid, entity) in prepared.index().iter() {
        if let Some(pos) = prepared
            .initial_frame()
            .get::<orr_testgame::Position>(entity)
        {
            use std::fmt::Write;
            writeln!(
                expected,
                "project entity: {guid} handle:{}v{} position:{},{}",
                entity.index,
                entity.version,
                pos.pos.x.raw(),
                pos.pos.y.raw()
            )
            .map_err(error)?;
        }
    }
    Ok(expected.into_bytes())
}
fn validate_smoke(
    stdout: &[u8],
    prepared: &crate::project_runtime::PreparedRuntime,
) -> Result<(), String> {
    if stdout != expected_smoke(prepared)? {
        return Err(
            "trusted runtime smoke output does not match this staged project's initial state"
                .into(),
        );
    }
    Ok(())
}

struct ChildGuard(Child, bool);
impl ChildGuard {
    fn pid(&self) -> Result<rustix::process::Pid, String> {
        let raw = i32::try_from(self.0.id()).map_err(error)?;
        rustix::process::Pid::from_raw(raw).ok_or("invalid owned child PID".into())
    }
    fn terminate_group(&self) -> Result<(), String> {
        match rustix::process::kill_process_group(self.pid()?, rustix::process::Signal::KILL) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
            Err(e) => Err(format!(
                "could not terminate owned smoke process group: {e}"
            )),
        }
    }
    fn finish(&mut self) -> Result<std::process::ExitStatus, String> {
        // The child has not been reaped, so its PID/group ID cannot be reused.
        self.terminate_group()?;
        let status = self.0.wait().map_err(error)?;
        self.1 = false;
        Ok(status)
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.1 {
            return;
        }
        let _ = self.terminate_group();
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn nonblocking(fd: impl AsFd) -> Result<(), String> {
    let flags = rustix::fs::fcntl_getfl(&fd).map_err(error)?;
    rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK).map_err(error)
}
fn drain(stream: &mut impl Read, bytes: &mut Vec<u8>, remaining: usize) -> Result<bool, String> {
    let mut buffer = [0u8; 8192];
    // A single bounded read per poll prevents an endless producer starving the timeout.
    match stream.read(&mut buffer) {
        Ok(0) => Ok(true),
        Ok(n) if n > remaining => Err("trusted runtime smoke output exceeds limit".into()),
        Ok(n) => {
            bytes.extend_from_slice(&buffer[..n]);
            Ok(false)
        }
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(e) => Err(error(e)),
    }
}
fn run_bounded(
    command: &mut Command,
    timeout: Duration,
    max_output: usize,
) -> Result<Vec<u8>, String> {
    let mut child = ChildGuard(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| format!("trusted runtime smoke launch failed: {e}"))?,
        true,
    );
    let mut stdout = child.0.stdout.take().ok_or("smoke stdout unavailable")?;
    let mut stderr = child.0.stderr.take().ok_or("smoke stderr unavailable")?;
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut status = None;
    let started = Instant::now();
    loop {
        let remaining = max_output.saturating_sub(out.len() + err.len());
        let out_done = drain(&mut stdout, &mut out, remaining)?;
        let remaining = max_output.saturating_sub(out.len() + err.len());
        let err_done = drain(&mut stderr, &mut err, remaining)?;
        if status.is_none() {
            // Observe exit without reaping. Cleanup also covers descendants
            // that redirected their output and outlive a completed launcher.
            use rustix::process::{waitid, WaitId, WaitIdOptions};
            let exited = waitid(
                WaitId::Pid(child.pid()?),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )
            .map_err(error)?
            .is_some();
            if exited {
                status = Some(child.finish()?);
            }
        }
        if let Some(status) = status {
            if out_done && err_done {
                if !status.success() {
                    return Err(format!(
                        "trusted runtime smoke failed ({status}): {}",
                        String::from_utf8_lossy(&err)
                    ));
                }
                return Ok(out);
            }
        }
        if started.elapsed() >= timeout {
            return Err("trusted runtime smoke timed out".into());
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn verify_stage(
    root: &Path,
    manifest: &ExportManifest,
    manifest_bytes: &[u8],
) -> Result<(), String> {
    let mut expected: BTreeMap<String, (u64, String, u32)> = manifest
        .payload
        .files
        .iter()
        .map(|f| (f.path.clone(), (f.bytes, f.sha256.clone(), f.mode)))
        .collect();
    expected.insert(
        MANIFEST_NAME.into(),
        (manifest_bytes.len() as u64, hash(manifest_bytes), 0o644),
    );
    let mut directories = BTreeSet::new();
    for path in expected.keys() {
        let mut p = Path::new(path).parent();
        while let Some(parent) = p.filter(|v| !v.as_os_str().is_empty()) {
            directories.insert(parent.to_path_buf());
            p = parent.parent();
        }
    }
    let mut pending = vec![PathBuf::new()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(root.join(&directory)).map_err(error)? {
            let entry = entry.map_err(error)?;
            let relative = directory.join(entry.file_name());
            let metadata = fs::symlink_metadata(entry.path()).map_err(error)?;
            if metadata.file_type().is_symlink() {
                return Err("staged symlink rejected".into());
            }
            if metadata.is_dir() {
                if !directories.remove(&relative) {
                    return Err("unexpected staged directory".into());
                }
                pending.push(relative);
            } else if metadata.is_file() {
                let key = relative.to_str().ok_or("non-UTF8 staged path")?;
                let (length, digest, mode) =
                    expected.remove(key).ok_or("unexpected staged file")?;
                verify_staged_file(&entry.path(), length, &digest, mode)?;
            } else {
                return Err("staged special file rejected".into());
            }
        }
    }
    if !expected.is_empty() || !directories.is_empty() {
        return Err("staged payload missing".into());
    }
    Ok(())
}

fn verify_staged_file(path: &Path, length: u64, digest: &str, mode: u32) -> Result<(), String> {
    check_path(path, false)?;
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    if metadata.len() != length || metadata.permissions().mode() & 0o7777 != mode {
        return Err("staged file length/mode changed".into());
    }
    let mut file = fs::File::open(path).map_err(error)?;
    let mut digest_state = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut length_read = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(error)?;
        if n == 0 {
            break;
        }
        length_read = length_read
            .checked_add(n as u64)
            .ok_or("staged byte count overflow")?;
        if length_read > length {
            return Err("staged file grew".into());
        }
        digest_state.update(&buffer[..n]);
    }
    let after = file.metadata().map_err(error)?;
    if length_read != length
        || digest_state
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
            != digest
        || after.dev() != metadata.dev()
        || after.ino() != metadata.ino()
        || after.len() != metadata.len()
        || after.mode() != metadata.mode()
        || after.mtime() != metadata.mtime()
        || after.mtime_nsec() != metadata.mtime_nsec()
        || after.ctime() != metadata.ctime()
        || after.ctime_nsec() != metadata.ctime_nsec()
    {
        return Err("staged file bytes/metadata changed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, MetadataExt};

    struct Fixture {
        temp: tempfile::TempDir,
        options: ExportOptions,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let project = temp.path().join("source");
            fs::create_dir(&project).unwrap();
            fs::write(
                project.join("orr.project.json"),
                br#"{"schema":2,"engine":"*","entry":{"game":"arena","scene":"arena.scene.yaml"}}"#,
            )
            .unwrap();
            fs::write(
                project.join("arena.scene.yaml"),
                include_bytes!("../../../assets/saved_arena_project/arena.scene.yaml"),
            )
            .unwrap();
            // This verified Rust test executable is copied but not executed by
            // transactional unit tests; real Arena execution lives in integration tests.
            let runtime = std::env::current_exe().unwrap();
            let runtime_sha256 = hash(&fs::read(&runtime).unwrap());
            let output = temp.path().join("export");
            Self {
                temp,
                options: ExportOptions {
                    project,
                    runtime,
                    runtime_sha256,
                    output,
                    trusted_runtime: true,
                    source_revision: Some("unit-fixture".into()),
                },
            }
        }
        fn smoke(&self) -> Vec<u8> {
            expected_smoke(
                &crate::project_runtime::PreparedRuntime::open(&self.options.project).unwrap(),
            )
            .unwrap()
        }
        fn no_stage(&self) {
            assert!(!self.options.output.exists());
            assert!(fs::read_dir(self.temp.path()).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".orr-export-")));
        }
        fn stage(&self) -> PathBuf {
            fs::read_dir(self.temp.path())
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".orr-export-")
                })
                .unwrap()
        }
    }

    #[test]
    fn transaction_failpoints_preserve_source_and_previous_export() {
        let fixture = Fixture::new();
        let source = fs::read(fixture.options.project.join("arena.scene.yaml")).unwrap();
        let previous = fixture.temp.path().join("previous");
        fs::create_dir(&previous).unwrap();
        fs::write(previous.join("keep"), b"previous-good").unwrap();
        for point in [
            "admitted",
            "binary-copied",
            "content-copied",
            "before-validation",
            "after-validation",
            "before-publish",
        ] {
            let result = export_with(
                &fixture.options,
                |name| {
                    if name == point {
                        Err(format!("injected failure: {point}"))
                    } else {
                        Ok(())
                    }
                },
                |_| Ok(fixture.smoke()),
            );
            assert!(result.unwrap_err().contains("injected failure"), "{point}");
            fixture.no_stage();
            assert_eq!(
                source,
                fs::read(fixture.options.project.join("arena.scene.yaml")).unwrap()
            );
            assert_eq!(fs::read(previous.join("keep")).unwrap(), b"previous-good");
        }
    }
    #[test]
    fn create_new_write_failure_cleans_only_owned_stage() {
        let fixture = Fixture::new();
        let unrelated = fixture.temp.path().join(".orr-export-unrelated");
        fs::create_dir(&unrelated).unwrap();
        fs::write(unrelated.join("keep"), b"keep").unwrap();
        let result = export_with(
            &fixture.options,
            |name| {
                if name == "binary-copied" {
                    let owned = fs::read_dir(fixture.temp.path())
                        .unwrap()
                        .map(|e| e.unwrap().path())
                        .find(|p| {
                            p != &unrelated
                                && p.file_name()
                                    .unwrap()
                                    .to_string_lossy()
                                    .starts_with(".orr-export-")
                        })
                        .unwrap();
                    fs::write(owned.join("project"), b"write failure").unwrap();
                }
                Ok(())
            },
            |_| Ok(fixture.smoke()),
        );
        assert!(result.is_err());
        assert!(!fixture.options.output.exists());
        assert_eq!(fs::read(unrelated.join("keep")).unwrap(), b"keep");
        assert_eq!(
            fs::read_dir(fixture.temp.path())
                .unwrap()
                .filter(|e| e
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".orr-export-"))
                .count(),
            1
        );
    }
    #[test]
    fn concurrent_destination_creator_is_never_overwritten() {
        let fixture = Fixture::new();
        let result = export_with(
            &fixture.options,
            |name| {
                if name == "before-publish" {
                    fs::create_dir(&fixture.options.output).unwrap();
                }
                Ok(())
            },
            |_| Ok(fixture.smoke()),
        );
        assert!(result.unwrap_err().contains("no-replace"));
        assert!(fixture.options.output.is_dir());
        assert_eq!(fs::read_dir(&fixture.options.output).unwrap().count(), 0);
    }
    #[test]
    fn publication_fails_closed_without_rename_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("absent-stage");
        let output = temp.path().join("output");
        assert!(publish_no_replace(&source, &output)
            .unwrap_err()
            .contains("no-replace"));
        assert!(!output.exists());
        let source = temp.path().join("stage");
        fs::create_dir(&source).unwrap();
        symlink("missing", &output).unwrap();
        assert!(publish_no_replace(&source, &output).is_err());
        assert!(fs::symlink_metadata(output)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(source.is_dir());
        let new_output = temp.path().join("unsupported-output");
        assert!(publish_with(&source, &new_output, |_, _| Err(rustix::io::Errno::NOSYS)).is_err());
        assert!(!new_output.exists());
        assert!(
            source.is_dir(),
            "unsupported no-replace cannot fall back to std rename"
        );
    }
    #[test]
    fn source_mutation_through_hardlink_alias_aborts_publication() {
        let fixture = Fixture::new();
        let scene = fixture.options.project.join("arena.scene.yaml");
        let alias = fixture.temp.path().join("scene-alias");
        fs::hard_link(&scene, &alias).unwrap();
        let result = export_with(
            &fixture.options,
            |name| {
                if name == "before-publish" {
                    fs::write(&alias, b"changed through alias").unwrap();
                }
                Ok(())
            },
            |_| Ok(fixture.smoke()),
        );
        assert!(result.is_err());
        fixture.no_stage();
        assert_eq!(
            fs::metadata(scene).unwrap().ino(),
            fs::metadata(alias).unwrap().ino()
        );
    }
    #[test]
    fn staged_mutation_or_extra_file_is_not_published() {
        for kind in ["bytes", "extra"] {
            let fixture = Fixture::new();
            let result = export_with(
                &fixture.options,
                |name| {
                    if name == "before-publish" {
                        let path = fixture.stage().join(if kind == "bytes" {
                            "run-arena"
                        } else {
                            "extra"
                        });
                        fs::write(path, b"changed").unwrap();
                    }
                    Ok(())
                },
                |_| Ok(fixture.smoke()),
            );
            assert!(result.is_err());
            fixture.no_stage();
        }
    }
    #[test]
    fn smoke_requires_exact_initial_state_output() {
        let fixture = Fixture::new();
        let prepared =
            crate::project_runtime::PreparedRuntime::open(&fixture.options.project).unwrap();
        let expected = expected_smoke(&prepared).unwrap();
        validate_smoke(&expected, &prepared).unwrap();
        for invalid in [
            b"".to_vec(),
            b"project initial checksum: 0x0000000000000000\n".to_vec(),
            [expected.as_slice(), b"extra\n"].concat(),
        ] {
            assert!(validate_smoke(&invalid, &prepared).is_err());
        }
    }
    #[test]
    fn malformed_or_wrong_architecture_elf_rejects_before_execution() {
        let mut elf = [0u8; 64];
        for bytes in [&b"not-elf"[..], &elf[..]] {
            assert!(validate_elf(bytes).is_err());
        }
        elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        elf[16] = 3;
        elf[18] = 62;
        elf[52] = 64;
        validate_elf(&elf).unwrap();
        for (index, value) in [(4, 1), (5, 2), (18, 183), (52, 0)] {
            let mut wrong = elf;
            wrong[index] = value;
            assert!(validate_elf(&wrong).is_err());
        }
    }
    #[test]
    fn output_overlap_and_ancestor_symlink_rejected() {
        let fixture = Fixture::new();
        assert!(destination(
            &fixture.options.project.join("export"),
            &fixture.options.project,
            &fixture.options.runtime
        )
        .is_err());
        let link = fixture.temp.path().join("link");
        symlink(&fixture.options.project, &link).unwrap();
        assert!(destination(
            &link.join("export"),
            &fixture.options.project,
            &fixture.options.runtime
        )
        .is_err());
        assert!(destination(
            &fixture.temp.path().join("missing/export"),
            &fixture.options.project,
            &fixture.options.runtime
        )
        .is_err());
    }
    #[test]
    #[expect(
        clippy::zombie_processes,
        reason = "the enclosing smoke guard deliberately owns descendant cleanup"
    )]
    fn bounded_child_helper() {
        match std::env::var("ORR_EXPORT_CHILD_MODE").as_deref() {
            Ok("timeout") => thread::sleep(Duration::from_secs(30)),
            Ok("output") => loop {
                std::io::stdout().write_all(&[b'x'; 8192]).unwrap();
            },
            Ok("failure") => std::process::exit(19),
            Ok("descendant") => {
                let child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "project_export::tests::bounded_child_helper",
                        "--nocapture",
                    ])
                    .env("ORR_EXPORT_CHILD_MODE", "delayed-marker")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                fs::write(
                    std::env::var_os("ORR_EXPORT_CHILD_PID").unwrap(),
                    child.id().to_string(),
                )
                .unwrap();
                if std::env::var_os("ORR_EXPORT_CHILD_FAIL").is_some() {
                    std::process::exit(19);
                }
            }
            Ok("delayed-marker") => {
                thread::sleep(Duration::from_millis(500));
                fs::write(
                    std::env::var_os("ORR_EXPORT_CHILD_MARKER").unwrap(),
                    b"orphan survived",
                )
                .unwrap();
                thread::sleep(Duration::from_secs(30));
            }
            _ => {}
        }
    }
    #[test]
    fn trusted_helper_timeout_output_and_exit_are_bounded() {
        for (mode, needle) in [
            ("timeout", "timed out"),
            ("output", "output exceeds"),
            ("failure", "smoke failed"),
        ] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "project_export::tests::bounded_child_helper",
                    "--nocapture",
                ])
                .env("ORR_EXPORT_CHILD_MODE", mode);
            let started = Instant::now();
            let error = run_bounded(&mut command, Duration::from_millis(200), 32768).unwrap_err();
            assert!(error.contains(needle), "{mode}: {error}");
            assert!(started.elapsed() < Duration::from_secs(5));
        }
    }
    #[test]
    fn completed_smoke_cleans_redirected_descendants_before_reaping() {
        for failure in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let pid_file = temp.path().join("pid");
            let marker = temp.path().join("marker");
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "project_export::tests::bounded_child_helper",
                    "--nocapture",
                ])
                .env("ORR_EXPORT_CHILD_MODE", "descendant")
                .env("ORR_EXPORT_CHILD_PID", &pid_file)
                .env("ORR_EXPORT_CHILD_MARKER", &marker);
            if failure {
                command.env("ORR_EXPORT_CHILD_FAIL", "1");
            }
            let result = run_bounded(&mut command, Duration::from_secs(2), 32768);
            assert_eq!(result.is_err(), failure);
            let pid = fs::read_to_string(pid_file).unwrap();
            thread::sleep(Duration::from_millis(650));
            assert!(!marker.exists(), "descendant outlived owned smoke cleanup");
            if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
                assert!(
                    stat.split_once(") ").unwrap().1.starts_with('Z'),
                    "descendant remains live: {stat}"
                );
            }
        }
    }
    #[test]
    fn staged_binary_is_rehashed_before_any_execution() {
        let fixture = Fixture::new();
        let mut executed = false;
        let result = export_with(
            &fixture.options,
            |name| {
                if name == "before-validation" {
                    let binary = fixture.stage().join("bin/arena");
                    let mut bytes = fs::read(&binary).unwrap();
                    let last = bytes.len() - 1;
                    bytes[last] ^= 1;
                    fs::write(binary, bytes).unwrap();
                }
                Ok(())
            },
            |_| {
                executed = true;
                Ok(fixture.smoke())
            },
        );
        assert!(result.unwrap_err().contains("staged file"));
        assert!(
            !executed,
            "corrupt staged bytes must never reach the smoke executor"
        );
        fixture.no_stage();
    }
}
