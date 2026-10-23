//! Bounded application-owned Linux x86-64 delivery, not a general project exporter.
//!
//! The supplied executable must already be trusted. Hashes, ELF headers and a
//! self-reported revision detect mix-ups; they do not authenticate arbitrary code.
//! Snapshots detect ordinary source mutation, not hostile concurrent path attacks.
use crate::game_contract::{admit, Starter};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            process::CommandExt,
        },
    },
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

// Wall time bounds a trusted executable query; it never enters simulation state.
#[allow(
    clippy::disallowed_types,
    reason = "monotonic process-query deadline enforcement outside deterministic simulation"
)]
mod query_clock {
    use std::time::{Duration, Instant};

    pub(super) struct Deadline(Instant);
    impl Deadline {
        pub(super) fn now() -> Self {
            Self(Instant::now())
        }
        pub(super) fn elapsed(&self) -> Duration {
            self.0.elapsed()
        }
    }
}

pub const TARGET: &str = "x86_64-unknown-linux-gnu";
pub const MAX_SCENE_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_QUERY_BYTES: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
/// Only these explicit external evidence paths can be forwarded. The delivered
/// scene and statically selected game cannot be overridden, even with `--`.
pub const LAUNCHER: &str = r#"#!/bin/sh
set -eu
record= replay= observe=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --record|--replay|--observe)
            key=$1
            [ "$#" -ge 2 ] || { echo 'missing evidence path' >&2; exit 2; }
            case "$2" in /*) ;; *) echo 'evidence path must be absolute' >&2; exit 2;; esac
            case "$key" in
                --record) [ -z "$record$replay" ] || exit 2; record=$2;;
                --replay) [ -z "$record$replay" ] || exit 2; replay=$2;;
                --observe) [ -z "$observe" ] || exit 2; observe=$2;;
            esac
            shift 2;;
        *) echo 'unsupported launcher argument' >&2; exit 2;;
    esac
done
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
set -- --scene "$root/scene/game.scene.yaml"
[ -z "$record" ] || set -- "$@" --record "$record"
[ -z "$replay" ] || set -- "$@" --replay "$replay"
[ -z "$observe" ] || set -- "$@" --observe "$observe"
exec "$root/bin/standalone" "$@"
"#;

#[derive(Clone, Debug)]
pub struct Options {
    pub runtime: PathBuf,
    pub runtime_sha256: String,
    pub scene: PathBuf,
    pub output: PathBuf,
    /// Expected compile-bound ORR_STATIC_STANDALONE_BUILD_SHA.
    pub source_sha: String,
    /// Explicit caller assertion, not inferred from path or hash.
    pub trusted_runtime: bool,
}

/// Publish exactly one concrete game executable, its admitted initial scene,
/// fixed launcher and deterministic receipt. Never replaces an existing output.
pub fn export<G: Starter>(options: &Options) -> Result<Value, String> {
    export_with::<G>(options, |binary, scene, provenance| {
        let mut command = Command::new(binary);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C");
        command.current_dir(binary.parent().ok_or("missing binary parent")?);
        if provenance {
            command.arg("--provenance");
        } else {
            command.arg("--identity").arg("--scene").arg(scene);
        }
        run_bounded(&mut command, TIMEOUT, MAX_QUERY_BYTES)
    })
}

fn export_with<G: Starter>(
    options: &Options,
    mut query: impl FnMut(&Path, &Path, bool) -> Result<Vec<u8>, String>,
) -> Result<Value, String> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err("bundle supports Linux x86-64 only".into());
    }
    if !options.trusted_runtime {
        return Err("explicit trusted runtime assertion required".into());
    }
    hex_string(&options.runtime_sha256, 64)?;
    hex_string(&options.source_sha, 40)?;
    let binary = Snapshot::read(&options.runtime, MAX_BINARY_BYTES)?;
    let scene = Snapshot::read(&options.scene, MAX_SCENE_BYTES)?;
    if binary.digest != options.runtime_sha256 {
        return Err("runtime SHA-256 mismatch".into());
    }
    if binary.identity.mode & 0o111 == 0 || binary.identity.mode & 0o7022 != 0 {
        return Err("runtime requires executable mode without special or shared-write bits".into());
    }
    validate_elf(&binary.bytes)?;
    let admitted = admit::<G>(std::str::from_utf8(&scene.bytes).map_err(error)?)?;
    let identity = admitted.identity().clone();
    let output = output_path(&options.output)?;
    for input in [&binary.path, &scene.path] {
        if input.starts_with(&output) || output.starts_with(input) {
            return Err("output overlaps bundle input".into());
        }
    }
    let parent = output.parent().ok_or("output parent missing")?;
    let stage = tempfile::Builder::new()
        .prefix(".orr-static-bundle-")
        .tempdir_in(parent)
        .map_err(error)?;
    fs::create_dir(stage.path().join("bin")).map_err(error)?;
    fs::create_dir(stage.path().join("scene")).map_err(error)?;
    let mut inventory = BTreeMap::new();
    for (path, bytes, mode) in [
        ("bin/standalone", binary.bytes.as_slice(), 0o755),
        ("scene/game.scene.yaml", scene.bytes.as_slice(), 0o644),
        ("run-game", LAUNCHER.as_bytes(), 0o755),
    ] {
        write_new(&stage.path().join(path), bytes, mode)?;
        inventory.insert(path.to_owned(), (bytes.len() as u64, hash(bytes), mode));
    }
    let staged_binary = stage.path().join("bin/standalone");
    let staged_scene = stage.path().join("scene/game.scene.yaml");
    let measured_identity: Value =
        serde_json::from_slice(&query(&staged_binary, &staged_scene, false)?).map_err(error)?;
    if measured_identity != identity {
        return Err("concrete runtime identity or admitted scene mismatch".into());
    }
    let provenance: Value =
        serde_json::from_slice(&query(&staged_binary, &staged_scene, true)?).map_err(error)?;
    if provenance != json!({"source_sha": options.source_sha}) {
        return Err("compile-bound runtime source SHA mismatch".into());
    }
    let scene_descriptor = identity
        .get("schema")
        .filter(|v| v.is_object())
        .ok_or("missing scene descriptor object in runtime identity")?;
    let input_contract = identity
        .get("input_contract")
        .filter(|v| v.is_object())
        .ok_or("missing input contract object in runtime identity")?;
    let receipt = json!({
        "schema": 1,
        "target": TARGET,
        "identity": identity,
        "scene_descriptor_sha256": hash(&serde_json::to_vec(scene_descriptor).map_err(error)?),
        "input_contract_sha256": hash(&serde_json::to_vec(input_contract).map_err(error)?),
        "source_sha": options.source_sha,
        "runtime_sha256": binary.digest,
        "runtime_provenance": provenance,
        "trust_boundary": "Executable hash and compile-bound source_sha are mix-up checks, not authentication or source attestation. Exact source-to-executable provenance requires an independent build receipt; supplied executable must already be trusted.",
        "runtime_requirements": "Linux x86-64 glibc, system ELF loader, display server and compatible Vulkan/graphics driver; not a fully static executable or universal Linux bundle",
        "files": inventory.iter().map(|(path,(bytes,sha256,mode))| json!({"path":path,"bytes":bytes,"sha256":sha256,"mode":mode})).collect::<Vec<_>>(),
    });
    let receipt_bytes = serde_json::to_vec_pretty(&receipt).map_err(error)?;
    if receipt_bytes.len() > MAX_QUERY_BYTES {
        return Err("receipt exceeds limit".into());
    }
    write_new(&stage.path().join("bundle.json"), &receipt_bytes, 0o644)?;
    inventory.insert(
        "bundle.json".into(),
        (receipt_bytes.len() as u64, hash(&receipt_bytes), 0o644),
    );
    verify_stage(stage.path(), &inventory)?;
    binary.recheck()?;
    scene.recheck()?;
    check_path(parent, true)?;
    // No check-then-replacing rename and no unsupported-filesystem fallback.
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        stage.path(),
        rustix::fs::CWD,
        &output,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|e| format!("atomic no-replace publication failed: {e}"))?;
    Ok(receipt)
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn hex_string(text: &str, len: usize) -> Result<(), String> {
    if text.len() != len
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(format!("expected {len} lowercase hex characters"));
    }
    Ok(())
}
fn check_path(path: &Path, directory: bool) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("path must be UTF-8")?;
    if text.is_empty()
        || text.len() > 4096
        || text.chars().any(char::is_control)
        || path.components().count() > 64
    {
        return Err("invalid or excessive filesystem path".into());
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(error)?.join(path)
    };
    let mut current = PathBuf::new();
    let mut parts = absolute.components().peekable();
    while let Some(part) = parts.next() {
        if matches!(part, Component::ParentDir) {
            return Err("parent traversal rejected".into());
        }
        current.push(part);
        let meta = fs::symlink_metadata(&current).map_err(error)?;
        if meta.file_type().is_symlink()
            || if directory || parts.peek().is_some() {
                !meta.is_dir()
            } else {
                !meta.is_file()
            }
        {
            return Err("requires regular link-free paths".into());
        }
    }
    Ok(current)
}
fn output_path(path: &Path) -> Result<PathBuf, String> {
    let leaf = path
        .file_name()
        .ok_or("output needs a final directory name")?;
    if path.components().any(|p| matches!(p, Component::ParentDir)) {
        return Err("output traversal rejected".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let output = check_path(parent, true)?.join(leaf);
    if output.as_os_str().len() > 4096
        || output
            .to_str()
            .is_none_or(|s| s.chars().any(char::is_control))
    {
        return Err("invalid output path".into());
    }
    match fs::symlink_metadata(&output) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(output),
        Ok(_) => Err("output already exists".into()),
        Err(e) => Err(error(e)),
    }
}
#[derive(Debug, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
    len: u64,
    mode: u32,
    mtime: (i64, i64),
    ctime: (i64, i64),
    links: u64,
}
impl FileIdentity {
    fn of(m: &fs::Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            len: m.len(),
            mode: m.mode(),
            mtime: (m.mtime(), m.mtime_nsec()),
            ctime: (m.ctime(), m.ctime_nsec()),
            links: m.nlink(),
        }
    }
}
struct Snapshot {
    path: PathBuf,
    bytes: Vec<u8>,
    digest: String,
    identity: FileIdentity,
    limit: u64,
}
impl Snapshot {
    fn read(path: &Path, limit: u64) -> Result<Self, String> {
        let path = check_path(path, false)?;
        let before = fs::symlink_metadata(&path).map_err(error)?;
        if before.nlink() != 1 || before.len() > limit {
            return Err("input must be single-link and within byte limit".into());
        }
        // NONBLOCK prevents a raced FIFO from hanging; NOFOLLOW rejects a raced final symlink.
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            )
            .open(&path)
            .map_err(error)?;
        if !file.metadata().map_err(error)?.is_file()
            || FileIdentity::of(&file.metadata().map_err(error)?) != FileIdentity::of(&before)
        {
            return Err("input changed while opening".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        check_path(&path, false)?;
        let identity = FileIdentity::of(&before);
        if bytes.len() as u64 != identity.len
            || bytes.len() as u64 > limit
            || identity != FileIdentity::of(&file.metadata().map_err(error)?)
            || identity != FileIdentity::of(&fs::symlink_metadata(&path).map_err(error)?)
        {
            return Err("input changed while reading".into());
        }
        Ok(Self {
            path,
            digest: hash(&bytes),
            bytes,
            identity,
            limit,
        })
    }
    fn recheck(&self) -> Result<(), String> {
        let now = Self::read(&self.path, self.limit)?;
        if now.identity != self.identity || now.digest != self.digest {
            return Err("source changed before publication".into());
        }
        Ok(())
    }
}
fn validate_elf(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 64
        || &bytes[..7] != b"\x7fELF\x02\x01\x01"
        || !matches!(u16::from_le_bytes([bytes[16], bytes[17]]), 2 | 3)
        || u16::from_le_bytes([bytes[18], bytes[19]]) != 62
        || u16::from_le_bytes([bytes[52], bytes[53]]) != 64
    {
        return Err("runtime must be little-endian x86-64 ELF64 executable".into());
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
type Inventory = BTreeMap<String, (u64, String, u32)>;
fn verify_stage(root: &Path, expected: &Inventory) -> Result<(), String> {
    check_path(root, true)?;
    let mut actual = BTreeMap::new();
    for entry in fs::read_dir(root).map_err(error)? {
        let entry = entry.map_err(error)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "non-UTF8 stage entry")?;
        if name == "bin" || name == "scene" {
            check_path(&entry.path(), true)?;
            for child in fs::read_dir(entry.path()).map_err(error)? {
                let child = child.map_err(error)?;
                let leaf = child
                    .file_name()
                    .into_string()
                    .map_err(|_| "non-UTF8 stage file")?;
                collect(
                    &child.path(),
                    format!("{name}/{leaf}"),
                    expected,
                    &mut actual,
                )?;
            }
        } else {
            collect(&entry.path(), name, expected, &mut actual)?;
        }
    }
    if &actual != expected {
        return Err("staging inventory mismatch".into());
    }
    Ok(())
}
fn collect(
    path: &Path,
    name: String,
    expected: &Inventory,
    actual: &mut Inventory,
) -> Result<(), String> {
    let bound = expected.get(&name).ok_or("unexpected staging entry")?.0;
    let snap = Snapshot::read(path, bound)?;
    actual.insert(
        name,
        (
            snap.bytes.len() as u64,
            snap.digest,
            snap.identity.mode & 0o7777,
        ),
    );
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
    let started = query_clock::Deadline::now();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::DeliveryGame;
    use std::os::unix::fs::symlink;

    struct Fixture {
        root: tempfile::TempDir,
        options: Options,
        identity: Value,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let runtime = root.path().join("delivery-player");
            let mut elf = vec![0u8; 64];
            elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
            elf[16] = 3;
            elf[18] = 62;
            elf[52] = 64;
            write_new(&runtime, &elf, 0o755).unwrap();
            let scene = root.path().join("original.scene.yaml");
            write_new(&scene, crate::delivery::SCENE.as_bytes(), 0o644).unwrap();
            let identity = admit::<DeliveryGame>(crate::delivery::SCENE)
                .unwrap()
                .identity()
                .clone();
            let options = Options {
                runtime,
                runtime_sha256: hash(&elf),
                scene,
                output: root.path().join("new bundle"),
                source_sha: "a".repeat(40),
                trusted_runtime: true,
            };
            Self {
                root,
                options,
                identity,
            }
        }
        fn answer(&self, provenance: bool) -> Vec<u8> {
            serde_json::to_vec(&if provenance {
                json!({"source_sha":self.options.source_sha})
            } else {
                self.identity.clone()
            })
            .unwrap()
        }
        fn clean_failure(&self) {
            assert!(!self.options.output.exists());
            assert!(!fs::read_dir(self.root.path()).unwrap().any(|p| p
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".orr-static-bundle-")));
            assert_eq!(
                fs::read(&self.options.scene).unwrap(),
                crate::delivery::SCENE.as_bytes()
            );
        }
    }
    #[test]
    fn deterministic_receipt_and_exact_inventory_without_source_paths() {
        let first = Fixture::new();
        let second = Fixture::new();
        let a = export_with::<DeliveryGame>(&first.options, |_, _, p| Ok(first.answer(p))).unwrap();
        let b =
            export_with::<DeliveryGame>(&second.options, |_, _, p| Ok(second.answer(p))).unwrap();
        assert_eq!(a, b);
        assert_eq!(a["identity"], first.identity);
        assert_eq!(a["files"].as_array().unwrap().len(), 3);
        assert!(!a.to_string().contains(first.root.path().to_str().unwrap()));
        assert_eq!(
            fs::read(first.options.output.join("scene/game.scene.yaml")).unwrap(),
            crate::delivery::SCENE.as_bytes()
        );
        assert_eq!(
            fs::metadata(first.options.output.join("bin/standalone"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o755
        );
        assert_eq!(
            fs::read(first.options.output.join("bundle.json")).unwrap(),
            fs::read(second.options.output.join("bundle.json")).unwrap()
        );
    }
    #[test]
    fn wrong_identity_or_compile_revision_never_publishes() {
        for wrong_provenance in [false, true] {
            let f = Fixture::new();
            assert!(export_with::<DeliveryGame>(&f.options, |_, _, p| Ok(
                if p == wrong_provenance {
                    b"{}".to_vec()
                } else {
                    f.answer(p)
                }
            ))
            .is_err());
            f.clean_failure();
        }
    }
    #[test]
    fn wrong_hash_mode_architecture_and_invalid_initial_scene_rejected() {
        for case in 0..6 {
            let mut f = Fixture::new();
            match case {
                0 => f.options.runtime_sha256 = "0".repeat(64),
                1 => fs::set_permissions(&f.options.runtime, fs::Permissions::from_mode(0o644))
                    .unwrap(),
                2 => fs::set_permissions(&f.options.runtime, fs::Permissions::from_mode(0o777))
                    .unwrap(),
                3 => {
                    let mut bytes = fs::read(&f.options.runtime).unwrap();
                    bytes[18] = 183;
                    fs::write(&f.options.runtime, &bytes).unwrap();
                    f.options.runtime_sha256 = hash(&bytes);
                }
                4 => {
                    fs::write(&f.options.scene, b"not a valid scene").unwrap();
                }
                _ => {
                    fs::write(
                        &f.options.scene,
                        crate::delivery::SCENE.replace("won: 0", "won: 1"),
                    )
                    .unwrap();
                }
            }
            assert!(export_with::<DeliveryGame>(&f.options, |_, _, _| panic!(
                "invalid inputs must not execute code"
            ))
            .is_err());
            assert!(!f.options.output.exists());
        }
    }
    #[test]
    fn source_mutation_dirty_stage_and_staged_mutation_rejected() {
        for case in 0..4 {
            let f = Fixture::new();
            assert!(export_with::<DeliveryGame>(&f.options, |binary, _, p| {
                if p {
                    match case {
                        0 => {
                            fs::write(&f.options.runtime, b"source mutation").unwrap();
                        }
                        1 => {
                            fs::write(
                                binary
                                    .parent()
                                    .unwrap()
                                    .parent()
                                    .unwrap()
                                    .join("unexpected"),
                                b"dirty",
                            )
                            .unwrap();
                        }
                        2 => {
                            fs::write(binary, b"binary mutation").unwrap();
                        }
                        _ => {
                            fs::set_permissions(binary, fs::Permissions::from_mode(0o777)).unwrap();
                        }
                    }
                }
                Ok(f.answer(p))
            })
            .is_err());
            f.clean_failure();
        }
    }
    #[test]
    fn existing_and_racing_destinations_remain_untouched() {
        for preexisting in [true, false] {
            let f = Fixture::new();
            if preexisting {
                fs::create_dir(&f.options.output).unwrap();
                fs::write(f.options.output.join("sentinel"), b"keep").unwrap();
            }
            assert!(export_with::<DeliveryGame>(&f.options, |_, _, p| {
                if p {
                    fs::create_dir(&f.options.output).unwrap();
                    fs::write(f.options.output.join("sentinel"), b"keep").unwrap();
                }
                Ok(f.answer(p))
            })
            .is_err());
            assert_eq!(
                fs::read(f.options.output.join("sentinel")).unwrap(),
                b"keep"
            );
            assert_eq!(fs::read_dir(&f.options.output).unwrap().count(), 1);
        }
    }
    #[test]
    fn snapshots_reject_symlinks_hardlinks_special_files_and_bounds() {
        let f = Fixture::new();
        let alias = f.root.path().join("alias");
        symlink(&f.options.runtime, &alias).unwrap();
        assert!(Snapshot::read(&alias, MAX_BINARY_BYTES).is_err());
        fs::remove_file(&alias).unwrap();
        fs::hard_link(&f.options.runtime, &alias).unwrap();
        assert!(Snapshot::read(&alias, MAX_BINARY_BYTES).is_err());
        assert!(Snapshot::read(&f.options.scene, 1).is_err());
        assert!(Snapshot::read(Path::new("/dev/null"), MAX_BINARY_BYTES).is_err());
        assert!(Snapshot::read(&f.root.path().join("../escape"), MAX_BINARY_BYTES).is_err());
        let dirlink = f.root.path().join("dirlink");
        symlink(f.root.path(), &dirlink).unwrap();
        assert!(Snapshot::read(&dirlink.join("original.scene.yaml"), MAX_SCENE_BYTES).is_err());
    }
    #[test]
    fn stage_symlink_and_added_empty_directory_rejected() {
        for linked in [true, false] {
            let f = Fixture::new();
            assert!(export_with::<DeliveryGame>(&f.options, |binary, _, p| {
                if p {
                    if linked {
                        fs::remove_file(binary).unwrap();
                        symlink(&f.options.runtime, binary).unwrap();
                    } else {
                        fs::create_dir(binary.parent().unwrap().join("empty")).unwrap();
                    }
                }
                Ok(f.answer(p))
            })
            .is_err());
            f.clean_failure();
        }
    }
    #[test]
    fn bounded_process_rejects_overflow_timeout_and_failure() {
        for (script, timeout, bound) in [
            ("printf 123456789", Duration::from_secs(1), 4),
            ("sleep 2", Duration::from_millis(20), 64),
            ("exit 7", Duration::from_secs(1), 64),
        ] {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", script]);
            assert!(run_bounded(&mut command, timeout, bound).is_err());
        }
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf ok"]);
        assert_eq!(
            run_bounded(&mut command, Duration::from_secs(1), 64).unwrap(),
            b"ok"
        );
    }
    #[test]
    fn process_group_cleanup_covers_inherited_redirected_and_timeout_descendants() {
        for mode in ["inherited", "redirected", "timeout"] {
            let tmp = tempfile::tempdir().unwrap();
            let marker = tmp.path().join("marker");
            let pid_file = tmp.path().join("pid");
            let script = match mode {
                "redirected" => "(sleep 1; printf survived > \"$1\") >/dev/null 2>&1 & printf '%s' \"$!\" > \"$2\"; exit 0",
                "timeout" => "(sleep 1; printf survived > \"$1\") & printf '%s' \"$!\" > \"$2\"; wait",
                _ => "(sleep 1; printf survived > \"$1\") & printf '%s' \"$!\" > \"$2\"; exit 0",
            };
            let mut command = Command::new("/bin/sh");
            command
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .args(["-c", script, "bundle-test"])
                .arg(&marker)
                .arg(&pid_file);
            let result = run_bounded(&mut command, Duration::from_millis(300), 1024);
            assert_eq!(result.is_err(), mode == "timeout", "{mode}: {result:?}");
            let pid = fs::read_to_string(pid_file).unwrap();
            thread::sleep(Duration::from_millis(1100));
            assert!(
                !marker.exists(),
                "{mode} descendant survived process-group cleanup"
            );
            if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
                assert!(
                    stat.split_once(") ").unwrap().1.starts_with('Z'),
                    "live descendant: {stat}"
                );
            }
        }
    }
}
