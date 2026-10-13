//! Mandatory saved-project export proof. No builds, downloads, synthetic images,
//! optional GPU path, or executable lookup through PATH are permitted here.
//! The caller supplies an already-authored project and independently expected
//! sample regions. Only private copies are changed; namespace mounts hide sources.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use orr_bridge::Bridge;
use orr_sample::project_runtime::PreparedRuntime;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

const HERO: &str = "e_00000001";
const PACKAGE: &str = "sample-sprites";
const DOCUMENT: &str = "sprites.json";
const LOCK: &str = "orr.packages.lock.json";
const MAX_PROJECT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PROJECT_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_INVENTORY_ENTRIES: usize = 256;
const MAX_INVENTORY_DEPTH: usize = 12;
const MAX_RUNTIME_BYTES: u64 = 96 * 1024 * 1024;
const MAX_EXPORT_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_LAUNCHER_BYTES: u64 = 16 * 1024;
// The ELF is budgeted separately from the project's 16MiB active closure.
const MAX_EXPORT_BYTES: u64 = MAX_PROJECT_BYTES + MAX_RUNTIME_BYTES
    + MAX_EXPORT_MANIFEST_BYTES + MAX_LAUNCHER_BYTES;
const MAX_CHILD_LOG: u64 = 4 * 1024 * 1024;
const CHILD_TIMEOUT: Duration = Duration::from_secs(90);
const PNG_SHA256: &str = "e7366af75c9a9bb44114c3914e8dc6cee447cdb18c762f805c1f564a12409693";

#[derive(Clone, Copy, Debug, Serialize)]
pub struct CaptureSample {
    pub tick: u32,
    pub region: u32,
}

#[derive(Debug)]
pub struct ExportEvidence {
    /// Retained PNGs, exact argv, child stdout/stderr/status, locks and inventories.
    pub directory: PathBuf,
    /// Measured from the admitted runtime's authoritative bridge snapshot.
    pub tick_rate: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Entry {
    kind: &'static str,
    mode: u32,
    bytes: u64,
    sha256: Option<String>,
}
type Inventory = BTreeMap<String, Entry>;

struct Pinned {
    path: PathBuf,
    sha256: String,
    bytes: u64,
}
impl Pinned {
    fn from_env(path_env: &str, hash_env: &str, cap: u64) -> Self {
        let path = env_path(path_env, false);
        let sha256 = std::env::var(hash_env).unwrap_or_else(|_| panic!("{hash_env} is required"));
        valid_hash(&sha256);
        let bytes = fs::metadata(&path).unwrap().len();
        assert!((64..=cap).contains(&bytes), "{path_env} exceeds its admitted cap");
        let pinned = Self { path, sha256, bytes };
        pinned.verify();
        pinned
    }
    fn verify(&self) {
        let metadata = fs::symlink_metadata(&self.path).unwrap();
        assert!(metadata.is_file() && !metadata.file_type().is_symlink());
        assert_ne!(metadata.permissions().mode() & 0o111, 0, "pinned tool must already be executable");
        assert_eq!(metadata.len(), self.bytes);
        assert_eq!(hash_file(&self.path), self.sha256, "pinned tool changed");
        let mut header = [0; 64];
        File::open(&self.path).unwrap().read_exact(&mut header).unwrap();
        assert_eq!(&header[..4], b"\x7fELF", "pinned tool is not ELF");
        assert_eq!(&header[4..7], &[2, 1, 1]);
        assert!(matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3));
        assert_eq!(u16::from_le_bytes([header[18], header[19]]), 62);
        assert_eq!(u16::from_le_bytes([header[52], header[53]]), 64);
    }
}

struct Work {
    root: tempfile::TempDir,
    evidence: PathBuf,
    source: PathBuf,
    original: PathBuf,
    hidden_workspace: PathBuf,
    bwrap: PathBuf,
    runtime: Pinned,
    exporter: Pinned,
}
fn supported_export_platform() -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64"))
}

impl Work {
    fn new(project: &Path) -> Self {
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"), "ORR_REQUIRE_GPU=1 is mandatory");
        assert!(supported_export_platform());
        let runtime = Pinned::from_env("ORR_SPRITE_ARENA_BIN", "ORR_SPRITE_ARENA_SHA256", MAX_RUNTIME_BYTES);
        let exporter = Pinned::from_env("ORR_SPRITE_EXPORTER_BIN", "ORR_SPRITE_EXPORTER_SHA256", 48 * 1024 * 1024);
        let bwrap = env_path("ORR_SPRITE_BWRAP", false);
        assert_ne!(fs::metadata(&bwrap).unwrap().permissions().mode() & 0o111, 0);
        let hidden_workspace = env_path("ORR_SPRITE_HIDE_ROOT", true);
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
        assert!(repository.starts_with(&hidden_workspace), "hide root must contain this exact source checkout");
        assert!(hidden_workspace.parent().is_some() && hidden_workspace != Path::new("/"));
        let work_parent = env_path("ORR_SPRITE_WORK_ROOT", true);
        let capture_parent = env_path("ORR_SPRITE_CAPTURE_DIR", true);
        for path in [&work_parent, &capture_parent, &bwrap] {
            assert!(!path.starts_with(&hidden_workspace), "work, evidence and bwrap must survive source hiding");
        }
        let original = project.canonicalize().unwrap();
        assert!(original.is_dir());
        assert!(!hidden_workspace.starts_with(&original), "project cannot contain the source workspace");
        for path in [&work_parent, &capture_parent] {
            assert!(!path.starts_with(&original), "output roots must not be inside the caller's project");
        }
        let root = tempfile::Builder::new().prefix("sprite-export-work-").tempdir_in(work_parent).unwrap();
        let evidence = tempfile::Builder::new().prefix("sprite-export-evidence-").tempdir_in(capture_parent).unwrap().keep();
        let source = root.path().join("authored-project");
        let original_inventory = inventory(&original);
        copy_tree(&original, &source);
        assert_eq!(inventory(&source), original_inventory, "staged original must preserve bytes and modes");
        fs::create_dir(root.path().join("empty-cwd")).unwrap();
        fs::create_dir(root.path().join("runtime-dir")).unwrap();
        fs::set_permissions(root.path().join("runtime-dir"), fs::Permissions::from_mode(0o700)).unwrap();
        write_json(&evidence.join("pins.json"), &json!({
            "runtime": {"path": runtime.path, "sha256": runtime.sha256, "bytes": runtime.bytes},
            "exporter": {"path": exporter.path, "sha256": exporter.sha256, "bytes": exporter.bytes},
            "bwrap": bwrap, "hidden_workspace": hidden_workspace, "original_project": original,
            "staged_project": source, "work_root": root.path(),
            "trust": "operator supplied pinned, already-built executables; hashes alone do not establish trust"
        }));
        Self { root, evidence, source, original, hidden_workspace, bwrap, runtime, exporter }
    }

    fn sandbox(&self) -> Command {
        let mut command = Command::new(&self.bwrap);
        command.env_clear()
            .args(["--die-with-parent", "--unshare-net", "--ro-bind", "/", "/", "--dev-bind", "/dev", "/dev", "--bind"])
            .arg(self.root.path()).arg(self.root.path())
            .arg("--bind").arg(&self.evidence).arg(&self.evidence)
            .arg("--ro-bind").arg(&self.source).arg(&self.source)
            .current_dir(self.root.path().join("empty-cwd"))
            .env("PATH", "/nonexistent")
            .env("HOME", self.root.path().join("empty-cwd"))
            .env("TMPDIR", self.root.path().join("runtime-dir"))
            .env("XDG_RUNTIME_DIR", self.root.path().join("runtime-dir"))
            .env("MESA_SHADER_CACHE_DISABLE", "true")
            .env("ORR_REQUIRE_GPU", "1");
        // These are GPU selection options, never authentication or Cargo config.
        for name in ["VK_ICD_FILENAMES", "VK_DRIVER_FILES", "WGPU_BACKEND", "LIBGL_ALWAYS_SOFTWARE"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
    }

    fn export(&self, output: &Path, label: &str) -> ChildResult {
        self.runtime.verify();
        self.exporter.verify();
        assert!(!output.exists(), "export destination must be new");
        let mut command = self.sandbox();
        command.arg("--").arg(&self.exporter.path)
            .arg("--project").arg(&self.source)
            .arg("--runtime").arg(&self.runtime.path)
            .arg("--runtime-sha256").arg(&self.runtime.sha256)
            .arg("--output").arg(output).arg("--trusted-runtime");
        child(&mut command, &self.evidence, label)
    }

    fn runtime(&self, bundle: Option<&Path>, sample: CaptureSample, label: &str) -> (ChildResult, PathBuf) {
        self.runtime.verify();
        let capture = self.evidence.join(format!("{label}.png"));
        assert!(!capture.exists());
        let mut command = self.sandbox();
        let (binary, project) = if let Some(bundle) = bundle {
            // All hiding is namespace-local. No rename/removal of caller or
            // tracked source paths occurs in the host filesystem.
            command.arg("--tmpfs").arg(&self.hidden_workspace);
            if !self.original.starts_with(&self.hidden_workspace) {
                command.arg("--tmpfs").arg(&self.original);
            }
            command.arg("--tmpfs").arg(&self.source)
                .arg("--ro-bind").arg(bundle).arg(bundle);
            let binary = bundle.join("bin/arena");
            assert_eq!(hash_file(&binary), self.runtime.sha256);
            command.args(["--", "/bin/sh", "-c",
                "test ! -e \"$1/orr.project.json\" && test ! -e \"$2/orr.project.json\" && test ! -e \"$3/Cargo.toml\" || exit 91; if (: > \"$4/.orr-write-probe\") 2>/dev/null; then exit 92; fi; printf '%s\\n' 'sprite export isolation: source absent; export write rejected' >&2; shift 4; exec \"$@\"", "sh"])
                .arg(&self.source).arg(&self.original)
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap())
                .arg(bundle);
            (binary, bundle.join("project"))
        } else {
            command.arg("--");
            (self.runtime.path.clone(), self.source.clone())
        };
        command.arg(binary).arg("--project").arg(project)
            .args(["--headless", "--ticks"]).arg(sample.tick.to_string())
            .arg("--capture").arg(&capture);
        (child(&mut command, &self.evidence, label), capture)
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        // Only this private TempDir is made removable after read-only proof.
        // Its staged project and generated export (including the runtime copy)
        // are intentionally removed. Retained JSON, PNGs and child logs live
        // in the separate kept evidence directory, outside this TempDir.
        fn writable_directories(path: &Path) {
            let Ok(metadata) = fs::symlink_metadata(path) else { return };
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
                if let Ok(entries) = fs::read_dir(path) {
                    for entry in entries.flatten() { writable_directories(&entry.path()); }
                }
            }
        }
        writable_directories(self.root.path());
    }
}

struct ChildResult {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}
impl ChildResult {
    fn success(&self, label: &str) {
        assert!(self.status.success(), "{label}: {}\nstdout:\n{}\nstderr:\n{}", self.status, self.stdout, self.stderr);
        assert!(!self.stdout.contains("SKIP") && !self.stderr.contains("SKIP"), "mandatory acceptance cannot skip");
    }
    fn rejection(&self, label: &str) {
        assert_eq!(self.status.code(), Some(1), "{label} must fail asset admission, not a launcher/usage failure: {}", self.stderr);
        assert!(self.stderr.contains("installed content changed: lantern_keeper.png"), "{label}: {}", self.stderr);
        assert!(!self.stdout.contains("project initial checksum:") && !self.stdout.contains("project capture adapter:"), "tamper must reject before play or GPU creation");
    }
}

fn child(command: &mut Command, evidence: &Path, label: &str) -> ChildResult {
    let stdout_path = evidence.join(format!("{label}.stdout"));
    let stderr_path = evidence.join(format!("{label}.stderr"));
    let arguments: Vec<_> = command.get_args().map(|value| value.to_str().unwrap()).collect();
    let environment: BTreeMap<_, _> = command.get_envs().map(|(key, value)| (key.to_str().unwrap(), value.map(|v| v.to_str().unwrap()))).collect();
    write_json(&evidence.join(format!("{label}.command.json")), &json!({
        "program": command.get_program().to_str().unwrap(), "arguments": arguments,
        "cwd": command.get_current_dir(), "environment_cleared": true, "environment": environment,
        "timeout_seconds": CHILD_TIMEOUT.as_secs(), "log_byte_cap_each": MAX_CHILD_LOG
    }));
    command.stdin(Stdio::null())
        .stdout(Stdio::from(create_new(&stdout_path)))
        .stderr(Stdio::from(create_new(&stderr_path)));
    let started = Instant::now();
    let mut child = command.spawn().unwrap_or_else(|error| panic!("{label}: cannot start mandatory child: {error}; evidence {}", evidence.display()));
    let mut termination = None;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() { break status; }
        if started.elapsed() >= CHILD_TIMEOUT {
            termination = Some("timeout");
        } else if [&stdout_path, &stderr_path].iter().any(|path| fs::metadata(path).unwrap().len() > MAX_CHILD_LOG) {
            termination = Some("log cap exceeded");
        }
        if termination.is_some() {
            child.kill().unwrap();
            break child.wait().unwrap();
        }
        thread::sleep(Duration::from_millis(20));
    };
    write_json(&evidence.join(format!("{label}.status.json")), &json!({
        "code": status.code(), "success": status.success(), "termination": termination,
        "elapsed_ms": started.elapsed().as_millis()
    }));
    assert!(termination.is_none(), "{label}: {termination:?}; retained evidence {}", evidence.display());
    for path in [&stdout_path, &stderr_path] { assert!(fs::metadata(path).unwrap().len() <= MAX_CHILD_LOG); }
    ChildResult { status, stdout: fs::read_to_string(stdout_path).unwrap(), stderr: fs::read_to_string(stderr_path).unwrap() }
}

fn env_path(name: &str, directory: bool) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required")));
    assert!(path.is_absolute(), "{name} must be absolute");
    let metadata = fs::symlink_metadata(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(!metadata.file_type().is_symlink(), "{name} cannot be a symlink");
    assert_eq!(metadata.is_dir(), directory, "{name}: wrong file kind");
    if !directory { assert!(metadata.is_file()); }
    path.canonicalize().unwrap()
}
fn valid_hash(hash: &str) {
    assert_eq!(hash.len(), 64);
    assert!(hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
}
fn digest(bytes: &[u8]) -> String { Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect() }
fn hash_file(path: &Path) -> String {
    let metadata = fs::symlink_metadata(path).unwrap();
    assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    assert!(metadata.len() <= MAX_RUNTIME_BYTES, "unbounded file hashing is forbidden");
    hash_exact_file(path, metadata.len())
}
fn hash_exact_file(path: &Path, expected_bytes: u64) -> String {
    let mut file = File::open(path).unwrap();
    assert!(file.metadata().unwrap().is_file());
    assert_eq!(file.metadata().unwrap().len(), expected_bytes, "file changed after metadata preflight");
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    // At most one extra byte is read to detect growth. Even a changing input
    // cannot turn a preflight-admitted file into an unbounded hashing stream.
    let mut bounded = (&mut file).take(expected_bytes.checked_add(1).unwrap());
    let mut read_bytes = 0_u64;
    loop {
        let size = bounded.read(&mut buffer).unwrap();
        if size == 0 { break; }
        read_bytes = read_bytes.checked_add(size as u64).unwrap();
        assert!(read_bytes <= expected_bytes, "file grew after metadata preflight");
        hash.update(&buffer[..size]);
    }
    assert_eq!(read_bytes, expected_bytes, "file shrank after metadata preflight");
    assert_eq!(file.metadata().unwrap().len(), expected_bytes);
    hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}
fn create_new(path: &Path) -> File { OpenOptions::new().write(true).create_new(true).open(path).unwrap() }
fn write_json(path: &Path, value: &impl Serialize) {
    let mut file = create_new(path);
    file.write_all(&serde_json::to_vec_pretty(value).unwrap()).unwrap();
    file.write_all(b"\n").unwrap();
    file.sync_all().unwrap();
}
fn inventory(root: &Path) -> Inventory { bounded_inventory(root, false) }
fn export_inventory(root: &Path) -> Inventory { bounded_inventory(root, true) }

/// Complete metadata admission precedes *all* byte hashing. The traversal stops
/// at its entry/depth limits rather than collecting an unbounded directory tree.
fn bounded_inventory(root: &Path, exported: bool) -> Inventory {
    struct Scan {
        exported: bool,
        total_bytes: u64,
        project_bytes: u64,
        entries: Inventory,
    }
    fn visit(root: &Path, path: &Path, depth: usize, scan: &mut Scan) {
        assert!(depth <= MAX_INVENTORY_DEPTH, "inventory depth cap exceeded");
        assert!(scan.entries.len() < MAX_INVENTORY_ENTRIES, "inventory entry cap exceeded");
        let metadata = fs::symlink_metadata(path).unwrap();
        assert!(!metadata.file_type().is_symlink(), "link in acceptance inventory: {}", path.display());
        let relative = path.strip_prefix(root).unwrap().to_str().unwrap();
        let key = if relative.is_empty() { "." } else { relative };
        let mode = metadata.permissions().mode() & 0o7777;
        let entry = if metadata.is_dir() {
            Entry { kind: "directory", mode, bytes: 0, sha256: None }
        } else {
            assert!(metadata.is_file(), "special file in acceptance inventory");
            let project_file = !scan.exported || key.starts_with("project/");
            let cap = if project_file {
                MAX_PROJECT_FILE_BYTES
            } else {
                match key {
                    "bin/arena" => MAX_RUNTIME_BYTES,
                    "orr.export.json" => MAX_EXPORT_MANIFEST_BYTES,
                    "run-arena" => MAX_LAUNCHER_BYTES,
                    _ => panic!("unexpected exported file outside the project: {key}"),
                }
            };
            assert!(metadata.len() <= cap, "inventory per-file cap exceeded: {key}");
            if project_file {
                scan.project_bytes = scan.project_bytes.checked_add(metadata.len()).unwrap();
                assert!(scan.project_bytes <= MAX_PROJECT_BYTES, "project aggregate byte cap exceeded");
            }
            scan.total_bytes = scan.total_bytes.checked_add(metadata.len()).unwrap();
            let total_cap = if scan.exported { MAX_EXPORT_BYTES } else { MAX_PROJECT_BYTES };
            assert!(scan.total_bytes <= total_cap, "inventory aggregate byte cap exceeded");
            Entry { kind: "file", mode, bytes: metadata.len(), sha256: None }
        };
        assert!(scan.entries.insert(key.into(), entry).is_none());
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), depth + 1, scan);
            }
        }
    }
    let mut scan = Scan { exported, total_bytes: 0, project_bytes: 0, entries: BTreeMap::new() };
    visit(root, root, 0, &mut scan);
    // A late oversized/too-deep entry rejects before even the first file hash.
    for (relative, entry) in &mut scan.entries {
        let path = root.join(relative);
        let metadata = fs::symlink_metadata(&path).unwrap();
        assert!(!metadata.file_type().is_symlink());
        assert_eq!(metadata.permissions().mode() & 0o7777, entry.mode);
        if entry.kind == "file" {
            assert!(metadata.is_file());
            assert_eq!(metadata.len(), entry.bytes);
            entry.sha256 = Some(hash_exact_file(&path, entry.bytes));
        } else {
            assert!(metadata.is_dir());
        }
    }
    scan.entries
}
fn copy_tree(source: &Path, target: &Path) {
    let metadata = fs::symlink_metadata(source).unwrap();
    assert!(!metadata.file_type().is_symlink());
    if metadata.is_dir() {
        fs::create_dir(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            copy_tree(&entry.path(), &target.join(entry.file_name()));
        }
    } else {
        assert!(metadata.is_file());
        fs::copy(source, target).unwrap();
    }
    fs::set_permissions(target, metadata.permissions()).unwrap();
}
fn readonly_tree(root: &Path) {
    let before = export_inventory(root);
    for (relative, entry) in before.iter().rev() {
        fs::set_permissions(root.join(relative), fs::Permissions::from_mode(entry.mode & !0o222)).unwrap();
    }
    assert!(export_inventory(root).values().all(|entry| entry.mode & 0o222 == 0));
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct RuntimeState {
    initial: String,
    tick: u32,
    checksum: String,
    positions: BTreeMap<String, (i64, i64)>,
}
fn runtime_state(stdout: &str) -> RuntimeState {
    let unique = |prefix: &str| {
        let mut values = stdout.lines().filter_map(|line| line.strip_prefix(prefix));
        let value = values.next().unwrap_or_else(|| panic!("missing runtime output {prefix}"));
        assert!(values.next().is_none(), "duplicate runtime output {prefix}");
        value.to_owned()
    };
    let initial = unique("project initial checksum: ");
    let tick = unique("project tick: ");
    let (tick, checksum) = tick.split_once(" checksum: ").unwrap();
    let mut positions = BTreeMap::new();
    for line in stdout.lines().filter_map(|line| line.strip_prefix("project entity: ")) {
        let (guid, rest) = line.split_once(" handle:").unwrap();
        let (_, position) = rest.split_once(" position:").unwrap();
        let (x, y) = position.split_once(',').unwrap();
        assert!(positions.insert(guid.into(), (x.parse().unwrap(), y.parse().unwrap())).is_none());
    }
    assert!(!positions.is_empty());
    RuntimeState { initial, tick: tick.parse().unwrap(), checksum: checksum.into(), positions }
}

struct Expected {
    state: RuntimeState,
    tick_rate: u32,
    elapsed_ms: u64,
    camera: orr_render::Camera,
    instance: orr_sprite::SpriteInstance,
    atlas: Vec<[u8; 4]>,
}
fn expected(project: &Path, sample: CaptureSample) -> Expected {
    let prepared = PreparedRuntime::open(project).unwrap();
    let initial = format!("0x{:016x}", prepared.initial_frame().checksum());
    let mut bridge = prepared.bridge().unwrap();
    let (_, mut presentation, ui) = prepared.into_launch_parts();
    assert!(ui.is_none(), "this exact sprite fixture must not declare a UI overlay");
    let first = bridge.snapshot().unwrap();
    let tick_rate = first.tick_rate();
    assert!(tick_rate > 0);
    presentation.update(Some(&first)).unwrap();
    for _ in 0..sample.tick {
        bridge.step(1);
        presentation.update(bridge.snapshot().as_ref()).unwrap();
    }
    let snapshot = bridge.snapshot().unwrap();
    assert_eq!(snapshot.tick(), u64::from(sample.tick));
    assert_eq!(snapshot.tick_rate(), tick_rate);
    let document = presentation.document().unwrap();
    let binding = &document.bindings[HERO];
    assert_eq!((&*binding.package, &*binding.document), (PACKAGE, DOCUMENT));
    assert_eq!(binding.source, orr_sample::project_sprites::Source::Clip("walk".into()));
    let hero_index = document.bindings.keys().position(|guid| guid == HERO).unwrap();
    assert_eq!(presentation.sprites().len(), document.bindings.len());
    let draw = &presentation.sprites()[hero_index];
    assert_eq!(draw.instance.region, sample.region, "independent expected region at tick {}", sample.tick);
    assert_eq!(draw.instance.rotation, 0.0);
    assert_eq!(draw.instance.tint, [1.0; 4]);
    assert!(!draw.instance.flip_x && !draw.instance.flip_y);
    let asset = &presentation.assets()[&(PACKAGE.into(), DOCUMENT.into())];
    let clip = asset.document.clip("walk").unwrap();
    assert_eq!(clip.mode(), orr_sprite::PlaybackMode::Once);
    assert_eq!(clip.frames().iter().map(|frame| (frame.region, frame.duration_ms)).collect::<Vec<_>>(), [(21, 140), (20, 125)]);
    assert_eq!(clip.duration_ms(), 265);
    let region = asset.document.region(sample.region).unwrap();
    assert_eq!((region.width, region.height), (16, 16));
    let mut atlas = Vec::new();
    for y in region.y..region.y + region.height {
        for x in region.x..region.x + region.width {
            let offset = ((y * asset.document.atlas().width + x) * 4) as usize;
            atlas.push(asset.rgba[offset..offset + 4].try_into().unwrap());
        }
    }
    let mut positions = BTreeMap::new();
    for (guid, entity) in presentation.index().iter() {
        let position = snapshot.predicted().get::<orr_sample::arena_game::Position>(entity).unwrap();
        positions.insert(guid.to_string(), (position.pos.x.raw(), position.pos.y.raw()));
    }
    let elapsed_ms = presentation.state(HERO).elapsed_ms;
    assert_eq!(elapsed_ms, u64::from(sample.tick) * 1000 / u64::from(tick_rate));
    Expected {
        state: RuntimeState { initial, tick: sample.tick, checksum: format!("0x{:016x}", snapshot.predicted().checksum()), positions },
        tick_rate, elapsed_ms, camera: presentation.camera, instance: draw.instance, atlas,
    }
}

struct Capture {
    rgba: Vec<u8>,
    texels: Vec<[u8; 4]>,
}
fn assert_capture(output: &ChildResult, path: &Path, oracle: &Expected) -> Capture {
    output.success("real runtime capture");
    assert_eq!(runtime_state(&output.stdout), oracle.state);
    let adapters: Vec<_> = output.stdout.lines().filter_map(|line| line.strip_prefix("project capture adapter: ")).collect();
    assert_eq!(adapters.len(), 1, "runtime must report the adapter used for this capture");
    assert!(adapters[0].ends_with(" (software: true)") || adapters[0].ends_with(" (software: false)"));
    assert!((101..=2 * 1024 * 1024).contains(&fs::metadata(path).unwrap().len()));
    let mut reader = png::Decoder::new(File::open(path).unwrap()).read_info().unwrap();
    assert!(reader.info().animation_control.is_none());
    assert_eq!((reader.info().width, reader.info().height), (512, 512));
    assert_eq!(reader.info().color_type, png::ColorType::Rgba);
    assert_eq!(reader.info().bit_depth, png::BitDepth::Eight);
    let mut rgba = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgba).unwrap();
    rgba.truncate(info.buffer_size());
    assert_eq!(rgba.len(), 512 * 512 * 4);
    assert!(rgba.as_chunks::<4>().0.iter().any(|pixel| pixel != &rgba[..4]), "blank capture");
    let mut texels = Vec::new();
    let mut opaque_count = 0;
    let mut distinctive_count = 0;
    for y in 0..16 {
        for x in 0..16 {
            let world = [
                oracle.instance.position[0] + ((x as f32 + 0.5) / 16.0 - 0.5) * oracle.instance.size[0],
                oracle.instance.position[1] + (0.5 - (y as f32 + 0.5) / 16.0) * oracle.instance.size[1],
            ];
            let at = oracle.camera.world_to_screen(world, (512, 512));
            assert!(at.iter().all(|coordinate| coordinate.is_finite() && (0.0..512.0).contains(coordinate)));
            let offset = ((at[1].floor() as usize * 512) + at[0].floor() as usize) * 4;
            let pixel: [u8; 4] = rgba[offset..offset + 4].try_into().unwrap();
            let atlas = oracle.atlas[y * 16 + x];
            if atlas[3] == 255 {
                assert!(color_matches(&pixel, &atlas), "tick {} region {} opaque PNG texel ({x},{y}) expected {atlas:?}, got {pixel:?}", oracle.state.tick, oracle.instance.region);
                opaque_count += 1;
                if [[54, 154, 165, 255], [255, 201, 76, 255]].iter().any(|color| color_matches(&pixel, color)) { distinctive_count += 1; }
            }
            texels.push(pixel);
        }
    }
    assert!(opaque_count >= 40 && distinctive_count >= 10, "missing genuine lantern atlas colors");
    Capture { rgba, texels }
}
fn color_matches(a: &[u8; 4], b: &[u8; 4]) -> bool { a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 3) }

fn assert_changed_pose(before: &Capture, after: &Capture, left: &Expected, right: &Expected) {
    assert_eq!(left.instance.position, right.instance.position, "fixture must remain stationary");
    assert_eq!(left.instance.size, right.instance.size);
    assert_eq!(left.camera, right.camera);
    if left.instance.region == right.instance.region {
        assert_eq!(before.texels, after.texels, "Once must retain the final sprite pose");
        assert_eq!(before.rgba, after.rgba, "unchanged pose in the static no-input scene must preserve the full framebuffer");
        return;
    }
    let mut distinguished = 0;
    for (index, (a, b)) in left.atlas.iter().zip(&right.atlas).enumerate() {
        if a[3] == 255 && (b[3] != 255 || a[..3] != b[..3]) {
            assert!(color_matches(&before.texels[index], a));
            assert!(!color_matches(&after.texels[index], a), "different frame identity texel must change");
            distinguished += 1;
        }
    }
    assert!(distinguished >= 6, "atlas frames need independent opaque identity evidence");
    let center = left.camera.world_to_screen(left.instance.position, (512, 512));
    let ppu = left.camera.pixels_per_unit(512, 512);
    let half = [left.instance.size[0] * ppu * 0.5 + 2.0, left.instance.size[1] * ppu * 0.5 + 2.0];
    let x0 = (center[0] - half[0]).floor().max(0.0) as usize;
    let x1 = (center[0] + half[0]).ceil().min(512.0) as usize;
    let y0 = (center[1] - half[1]).floor().max(0.0) as usize;
    let y1 = (center[1] + half[1]).ceil().min(512.0) as usize;
    let changed = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (y * 512 + x) * 4))
        .filter(|offset| before.rgba[*offset..*offset + 4] != after.rgba[*offset..*offset + 4]).count();
    let outside_changed = before.rgba.as_chunks::<4>().0.iter().zip(after.rgba.as_chunks::<4>().0.iter())
        .enumerate().filter(|(index, (a, b))| {
            let x = *index % 512;
            let y = *index / 512;
            (!(x0..x1).contains(&x) || !(y0..y1).contains(&y)) && a != b
        }).count();
    let area = (x1 - x0) * (y1 - y0);
    assert!((6..=area / 2).contains(&changed), "bounded hero-only frame difference {changed}/{area}");
    assert_eq!(outside_changed, 0, "static no-input scene changed outside the intended hero rectangle");
}

fn verify_export(work: &Work, bundle: &Path, source: &Inventory) -> Value {
    // Bound the full exported tree before reading even its JSON manifest.
    let tree = export_inventory(bundle);
    let manifest_path = bundle.join("orr.export.json");
    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["schema"], 1);
    let payload = &manifest["payload"];
    assert_eq!(payload["profile"], "arena-authored-linux-x86_64-v1");
    assert_eq!(payload["runtime"]["sha256"], work.runtime.sha256);
    assert_eq!(payload["runtime"]["bytes"], work.runtime.bytes);
    let mut content = b"orrery.arena.export.content.v1\0".to_vec();
    content.extend(serde_json::to_vec(payload).unwrap());
    assert_eq!(manifest["content_digest"], digest(&content));
    let lock_bytes = fs::read(work.source.join(LOCK)).unwrap();
    assert_eq!(fs::read(bundle.join("project").join(LOCK)).unwrap(), lock_bytes);
    let lock: Value = serde_json::from_slice(&lock_bytes).unwrap();
    let packages = lock["packages"].as_object().unwrap();
    let exported_packages = payload["packages"].as_object().unwrap();
    assert_eq!(exported_packages.len(), packages.len());
    for (name, package) in packages { assert_eq!(exported_packages[name], package["digest"]); }
    let project_manifest: Value = serde_json::from_slice(&fs::read(work.source.join("orr.project.json")).unwrap()).unwrap();
    assert_eq!(payload["entry"], project_manifest["entry"]);
    let mut active_project_files: BTreeSet<String> = ["orr.project.json", LOCK,
        project_manifest["entry"]["scene"].as_str().unwrap(),
        project_manifest["entry"]["sprites"].as_str().unwrap()].into_iter().map(String::from).collect();
    for package in packages.values() {
        let digest = package["digest"].as_str().unwrap();
        valid_hash(digest);
        for path in package["files"].as_object().unwrap().keys().map(String::as_str).chain(std::iter::once("orr.package.json")) {
            active_project_files.insert(format!(".orr/packages/objects/{digest}/{path}"));
        }
    }
    let actual_project_files: BTreeSet<_> = inventory(&bundle.join("project")).into_iter()
        .filter(|(_, entry)| entry.kind == "file").map(|(path, _)| path).collect();
    assert_eq!(actual_project_files, active_project_files, "export must contain exactly the active authored closure");
    assert!(PreparedRuntime::open(bundle.join("project")).is_ok(), "exported package must independently pass admission");
    let expected_paths: BTreeSet<_> = tree.iter().filter(|(path, entry)| entry.kind == "file" && path.as_str() != "orr.export.json").map(|(path, _)| path.clone()).collect();
    let mut listed = BTreeSet::new();
    for file in payload["files"].as_array().unwrap() {
        let relative = file["path"].as_str().unwrap();
        assert!(!relative.is_empty() && Path::new(relative).components().all(|component| matches!(component, Component::Normal(_))));
        assert!(listed.insert(relative.to_owned()));
        let actual = &tree[relative];
        assert_eq!(actual.kind, "file");
        assert_eq!(file["sha256"], actual.sha256.as_ref().unwrap().as_str());
        assert_eq!(file["bytes"], actual.bytes);
        assert_eq!(file["mode"], actual.mode);
        assert_eq!(actual.mode, if matches!(relative, "bin/arena" | "run-arena") { 0o755 } else { 0o644 });
        if let Some(project_path) = relative.strip_prefix("project/") {
            let original = &source[project_path];
            assert_eq!(actual.sha256, original.sha256);
            assert_eq!(actual.bytes, original.bytes);
            let a = fs::metadata(work.source.join(project_path)).unwrap();
            let b = fs::metadata(bundle.join(relative)).unwrap();
            assert_ne!((a.dev(), a.ino()), (b.dev(), b.ino()), "export must use independent bytes");
        }
    }
    assert_eq!(listed, expected_paths, "manifest must enumerate every exported file");
    assert_eq!(hash_file(&bundle.join("bin/arena")), work.runtime.sha256);
    let text = fs::read_to_string(manifest_path).unwrap();
    for path in [&work.source, &work.original, &work.hidden_workspace] { assert!(!text.contains(path.to_str().unwrap()), "no source paths in portable manifest"); }
    write_json(&work.evidence.join("export-manifest.json"), &manifest);
    create_new(&work.evidence.join("export-package-lock.json")).write_all(&lock_bytes).unwrap();
    manifest
}

/// Execute the exact real Arena/exporter CLI against an authored package.
/// Requires all ORR_SPRITE_* paths/pins documented above; absent prerequisites
/// panic rather than silently building tools, skipping, or changing consumers.
pub fn assert_authored_export(project: &Path, samples: &[CaptureSample]) -> ExportEvidence {
    assert!((3..=12).contains(&samples.len()));
    assert_eq!(samples[0].tick, 0);
    assert!(samples.windows(2).all(|pair| pair[0].tick < pair[1].tick));
    assert!(samples.iter().all(|sample| sample.tick <= 6000));
    assert!(samples.iter().any(|sample| sample.region != samples[0].region));
    let original_before = inventory(project);
    let work = Work::new(project);
    eprintln!("mandatory sprite export evidence: {}", work.evidence.display());
    let source_before = inventory(&work.source);
    write_json(&work.evidence.join("source-before.json"), &source_before);
    let package = orr_package::Project::open(&work.source, orr_sample::project_runtime::compiled_runtime()).unwrap();
    let lock = package.verify().unwrap();
    let saved = &lock.packages[PACKAGE];
    assert_eq!(lock.direct[PACKAGE], saved.manifest.version);
    assert_eq!(saved.manifest.version, "1.0.1", "must admit the exact newly authored package version");
    assert!(saved.manifest.dependencies.is_empty());
    assert_eq!(saved.files.keys().map(String::as_str).collect::<Vec<_>>(), ["LICENSE.txt", "lantern_keeper.png", "lantern_keeper.rgba", "sprites.json"]);
    assert_eq!(saved.files["lantern_keeper.png"], PNG_SHA256, "must use real sprite_demo art");
    assert_eq!(saved.files["lantern_keeper.rgba"], "6928c2058440a8f508b4069f9e83349e11643dbec8573aa164e03e5b2a0ad682");
    assert_eq!(saved.files["LICENSE.txt"], "318e191782bebb2e0fa21170cd2cbd7161b6f9a843d4ced7d3e135da5c589b98");
    let png_path = work.source.join(".orr/packages/objects").join(&saved.digest).join("lantern_keeper.png");
    let atlas_bytes = fs::read(&png_path).unwrap();
    let atlas_mode = fs::metadata(&png_path).unwrap().permissions();
    let expected: Vec<_> = samples.iter().map(|sample| expected(&work.source, *sample)).collect();
    let tick_rate = expected[0].tick_rate;
    assert!(expected.iter().all(|oracle| oracle.tick_rate == tick_rate));
    for millis in [140_u64, 265] {
        let boundary_tick = (millis * u64::from(tick_rate)).div_ceil(1000);
        for tick in [boundary_tick - 1, boundary_tick] {
            assert!(samples.iter().any(|sample| u64::from(sample.tick) == tick), "sample both sides of authored {millis}ms boundary");
        }
    }
    assert!(expected.last().unwrap().elapsed_ms >= 530, "sample well beyond Once end");
    let mut originals = Vec::new();
    for (index, sample) in samples.iter().enumerate() {
        let label = format!("source-{index:02}-tick-{}", sample.tick);
        let (output, capture) = work.runtime(None, *sample, &label);
        write_json(&work.evidence.join(format!("{label}.inventory-after.json")), &inventory(&work.source));
        assert_eq!(inventory(&work.source), source_before);
        let pixels = assert_capture(&output, &capture, &expected[index]);
        write_json(&work.evidence.join(format!("{label}.oracle.json")), &json!({
            "sample": sample, "tick_rate": tick_rate, "elapsed_ms": expected[index].elapsed_ms,
            "runtime_state": expected[index].state, "png_sha256": hash_file(&capture),
            "camera_center": expected[index].camera.center, "camera_half_extent": expected[index].camera.half_extent
        }));
        originals.push(pixels);
    }
    for index in 1..samples.len() { assert_changed_pose(&originals[index - 1], &originals[index], &expected[index - 1], &expected[index]); }

    // Corrupt only this owned project's locked PNG. The authoritative lock is
    // unchanged, so both admission paths must fail before executing any play.
    fs::set_permissions(&png_path, fs::Permissions::from_mode(atlas_mode.mode() | 0o200)).unwrap();
    let mut tampered = atlas_bytes.clone();
    tampered[0] ^= 1;
    fs::write(&png_path, tampered).unwrap();
    let rejected = match PreparedRuntime::open(&work.source) { Ok(_) => panic!("tampered PNG was admitted"), Err(error) => error };
    assert!(rejected.contains("installed content changed: lantern_keeper.png"));
    let tampered_before = inventory(&work.source);
    write_json(&work.evidence.join("tampered-before.json"), &tampered_before);
    let (rejected_runtime, rejected_capture) = work.runtime(None, samples[0], "tampered-runtime");
    let rejected_destination = work.root.path().join("must-not-export");
    let rejected_export = work.export(&rejected_destination, "tampered-export");
    write_json(&work.evidence.join("tampered-after.json"), &inventory(&work.source));
    assert_eq!(inventory(&work.source), tampered_before);
    rejected_runtime.rejection("tampered runtime");
    rejected_export.rejection("tampered exporter");
    assert!(!rejected_capture.exists() && !rejected_destination.exists());
    fs::write(&png_path, &atlas_bytes).unwrap();
    fs::set_permissions(&png_path, atlas_mode).unwrap();
    assert_eq!(inventory(&work.source), source_before);
    assert!(PreparedRuntime::open(&work.source).is_ok());
    let (restored, restored_path) = work.runtime(None, samples[0], "restored-source");
    let restored_capture = assert_capture(&restored, &restored_path, &expected[0]);
    assert_eq!(restored_capture.rgba, originals[0].rgba);

    let bundle = work.root.path().join("exported-arena");
    let exported = work.export(&bundle, "restored-positive-export");
    exported.success("restored positive export");
    let manifest = verify_export(&work, &bundle, &source_before);
    assert_eq!(manifest["payload"]["initial_checksum"], expected[0].state.initial);
    readonly_tree(&bundle);
    let exported_before = export_inventory(&bundle);
    write_json(&work.evidence.join("export-readonly-baseline.json"), &exported_before);
    for (index, sample) in samples.iter().enumerate() {
        let label = format!("export-{index:02}-tick-{}", sample.tick);
        let before = export_inventory(&bundle);
        assert_eq!(before, exported_before);
        assert!(before.values().all(|entry| entry.mode & 0o222 == 0));
        write_json(&work.evidence.join(format!("{label}.inventory-before.json")), &before);
        let (output, path) = work.runtime(Some(&bundle), *sample, &label);
        let after = export_inventory(&bundle);
        write_json(&work.evidence.join(format!("{label}.inventory-after.json")), &after);
        assert_eq!(after, before, "read-only exported run changed bytes, modes, locks or paths");
        output.success(&label);
        assert!(output.stderr.lines().any(|line| line == "sprite export isolation: source absent; export write rejected"));
        let capture = assert_capture(&output, &path, &expected[index]);
        assert_eq!(capture.rgba, originals[index].rgba, "source-hidden exported framebuffer must match the original at the exact tick");
        assert_eq!(capture.texels, originals[index].texels);
    }
    assert_eq!(inventory(&work.source), source_before);
    assert_eq!(inventory(project), original_before, "caller's authored project must remain unchanged");
    work.runtime.verify();
    work.exporter.verify();
    write_json(&work.evidence.join("complete.json"), &json!({
        "schema": 1, "samples": samples, "tick_rate": tick_rate,
        "runtime_sha256": work.runtime.sha256, "exporter_sha256": work.exporter.sha256,
        "export_content_digest": manifest["content_digest"], "source_independent": true,
        "export_mount_readonly": true, "export_inventory_unchanged": true,
        "tamper_rejected_before_play_and_export": true, "restored_positive": true,
        "capture_kind": "actual Arena headless WGPU compositor; not a native-window capture"
    }));
    ExportEvidence { directory: work.evidence.clone(), tick_rate }
}
