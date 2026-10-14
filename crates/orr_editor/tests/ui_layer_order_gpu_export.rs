//! Explicit software-GPU and source-hidden read-only export acceptance.
//! The owning controller must pin every binary and allocate this ignored gate.
//! Missing prerequisites are failures, never successful skips. No native window
//! or physical-GPU coverage is claimed. See the source-stage verification plan.
#![cfg(all(feature = "collect-ui", target_os = "linux", target_arch = "x86_64"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{collect_ui_panel::Panel, Editor, EditorApp, HostSpec, Mode};
use orr_sample::{
    authored_ui::{Action, Document, Kind, Node, Screen},
    collect_game::{CollectInput, PLAYING},
    collect_project::{PreparedProject, ProgressSupport, SpriteSupport},
    collect_ui::{CollectUi, Hud},
};
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

const FRONT: &str = "Bring subtree to front";
const BACK: &str = "Send subtree to back";
const UNDO: &str = "Undo UI edit";
const REDO: &str = "Redo UI edit";
const SAVE: &str = "Save UI document";
const UI_FILE: &str = "level.ui.json";
const HELPER: &str = "layer_order_relocated_bundle_probe";
const PROBES: [(u32, bool); 4] = [(0, false), (8, true), (16, true), (40, false)];
const GAME_ID: &str = "12345678-1234-4234-8234-123456789abc";
const ISOLATION_OK: &str = "ORR_LAYER_ORDER_ISOLATION_OK";
const OUTPUT_CAP: usize = 512 * 1024;

struct Captured {
    output: Output,
    overflow: bool,
    expired: bool,
    transport_errors: Vec<String>,
}

type StreamResult = (Vec<u8>, Option<String>);

fn capped_stream(stream: impl Read + Send + 'static) -> Receiver<StreamResult> {
    let (send, receive) = mpsc::channel();
    let _reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        // At most CAP+1 bytes can be read/retained. The extra byte proves overflow;
        // dropping the reader also closes this pipe rather than draining forever.
        let error = stream.take((OUTPUT_CAP + 1) as u64).read_to_end(&mut bytes).err().map(|e| e.to_string());
        let _ = send.send((bytes, error));
    });
    receive
}

/// The two pipes have independent hard byte caps and are read concurrently.
/// A local kill reaps only our direct wrapper. An interrupted/incomplete transport
/// always FAILS; cleanup of every descendant remains the outer controller's duty.
/// `timeout --foreground` below preserves that controller's owned process group.
fn bounded(mut command: Command, seconds: u64) -> Captured {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().expect("launch controller-admitted subprocess");
    let stdout_receiver = capped_stream(child.stdout.take().unwrap());
    let stderr_receiver = capped_stream(child.stderr.take().unwrap());
    let mut stdout = None;
    let mut stderr = None;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut expired = false;
    let mut overflow = false;
    let status = loop {
        if stdout.is_none() {
            stdout = stdout_receiver.try_recv().ok();
        }
        if stderr.is_none() {
            stderr = stderr_receiver.try_recv().ok();
        }
        overflow |= [&stdout, &stderr].into_iter().flatten().any(|(bytes, _)| bytes.len() > OUTPUT_CAP);
        expired |= Instant::now() >= deadline;
        if overflow || expired {
            let _ = child.kill();
            break child.wait().expect("reap direct subprocess wrapper");
        }
        if let Some(status) = child.try_wait().expect("poll direct subprocess wrapper") {
            break status;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let finish = |value: Option<StreamResult>, receive: Receiver<StreamResult>| {
        value.unwrap_or_else(|| receive.recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|error| (Vec::new(), Some(format!("pipe incomplete; owner must clean descendants: {error}")))))
    };
    let (mut stdout, stdout_error) = finish(stdout, stdout_receiver);
    let (mut stderr, stderr_error) = finish(stderr, stderr_receiver);
    overflow |= stdout.len() > OUTPUT_CAP || stderr.len() > OUTPUT_CAP;
    stdout.truncate(OUTPUT_CAP);
    stderr.truncate(OUTPUT_CAP);
    Captured {
        output: Output { status, stdout, stderr }, overflow, expired,
        transport_errors: [stdout_error, stderr_error].into_iter().flatten().collect(),
    }
}

fn good(captured: Captured) -> String {
    assert!(!captured.overflow && !captured.expired && captured.transport_errors.is_empty(),
        "bounded transport failed; descendant cleanup belongs to controller: overflow={} expired={} errors={:?}",
        captured.overflow, captured.expired, captured.transport_errors);
    let output = captured.output;
    assert!(output.status.success(), "status {}\nstdout {}\nstderr {}", output.status,
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn required_path(key: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("mandatory {key}")));
    assert!(path.is_absolute(), "{key} must be absolute");
    path
}

fn required_hash(key: &str) -> String {
    let digest = std::env::var(key).unwrap_or_else(|_| panic!("mandatory independent {key}"));
    assert!(digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()), "invalid {key}");
    digest.to_ascii_lowercase()
}

fn infrastructure(name: &str) -> PathBuf {
    let path = required_path(&format!("ORR_LAYER_ORDER_{name}_BIN")).canonicalize().unwrap();
    assert!(path.is_file(), "controller must resolve infrastructure identity: {name}");
    path
}

/// Streaming infrastructure utility, not a Rust/GPU/fixture build. Every such
/// process is bounded; the outer controller additionally owns the whole test tree.
fn digest(path: &Path) -> String {
    let mut command = Command::new(infrastructure("TIMEOUT"));
    command.args(["--foreground", "--signal=TERM", "--kill-after=5s", "30s"])
        .arg(infrastructure("SHA256SUM")).arg("--").arg(path);
    let text = good(bounded(command, 40));
    let digest = text.split_whitespace().next().unwrap();
    assert!(digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()));
    digest.to_ascii_lowercase()
}

fn write_new(path: &Path, bytes: &[u8]) {
    assert!(bytes.len() <= 4 * 1024 * 1024, "bounded fixture text/proof file");
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

fn bounded_copy(source: &Path, destination: &Path, maximum: u64) -> u64 {
    let before = fs::symlink_metadata(source).unwrap();
    assert!(before.file_type().is_file() && before.len() <= maximum);
    let mut input = fs::File::open(source).unwrap();
    let mut output = fs::OpenOptions::new().write(true).create_new(true).open(destination).unwrap();
    let mut buffer = [0_u8; 64 * 1024];
    let mut written = 0_u64;
    loop {
        let count = input.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        written = written.checked_add(count as u64).unwrap();
        assert!(written <= maximum, "copy cap checked before every write");
        output.write_all(&buffer[..count]).unwrap();
    }
    output.sync_all().unwrap();
    assert_eq!(written, before.len(), "source length changed during copy");
    assert_eq!(fs::metadata(source).unwrap().len(), before.len());
    fs::set_permissions(destination, before.permissions()).unwrap();
    written
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Stamp {
    directory: bool,
    mode: u32,
    bytes: Option<u64>,
    sha256: Option<String>,
}

/// Hash files incrementally instead of holding copied executable bytes in RAM.
/// Include directory names/modes so additions and permission changes are visible.
fn inventory(root: &Path) -> BTreeMap<PathBuf, Stamp> {
    fn visit(root: &Path, current: &Path, result: &mut BTreeMap<PathBuf, Stamp>) {
        let metadata = fs::symlink_metadata(current).unwrap();
        assert!(!metadata.file_type().is_symlink(), "inventory refuses symlinks");
        assert!(metadata.is_dir() || metadata.is_file());
        let directory = metadata.is_dir();
        result.insert(current.strip_prefix(root).unwrap().to_owned(), Stamp {
            directory,
            mode: metadata.permissions().mode(),
            bytes: if directory { None } else { Some(metadata.len()) },
            sha256: if directory { None } else { Some(digest(current)) },
        });
        if directory {
            for entry in fs::read_dir(current).unwrap() {
                visit(root, &entry.unwrap().path(), result);
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn open(root: &Path) -> PreparedProject {
    PreparedProject::open_with_ui(root, ProgressSupport::MetadataOnly, SpriteSupport::Supported, true).unwrap()
}

fn overlap_document() -> Document {
    let node = |id: &str, parent: Option<&str>, kind, offset, size| Node {
        id: id.into(), parent: parent.map(str::to_owned), kind, screen: Screen::Playing,
        anchor: [0, 0], offset, size,
    };
    let document = Document { schema: 1, nodes: vec![
        // Empty text is admitted by the current validator: no minimum length.
        // It creates a flat lower-button pixel reference without glyph ambiguity.
        node("lower", None, Kind::Button { text: String::new(), action: Action::Menu }, [20, 20], [180, 100]),
        node("cover", None, Kind::Container, [40, 20], [80, 80]),
        // Absolute child is [60,50]..[160,80], visibly clipped at x=120.
        node("child", Some("cover"), Kind::Button { text: "Restart".into(), action: Action::Restart }, [20, 30], [100, 30]),
    ] };
    document.validate().unwrap();
    document
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameProof {
    ticks: u32,
    right: bool,
    bytes: Vec<u8>,
    checksum: u64,
}

fn frames(root: &Path) -> Vec<FrameProof> {
    PROBES.into_iter().map(|(ticks, right)| {
        let prepared = open(root);
        let mut simulation = prepared.scene().simulation().unwrap();
        for tick in 1..=u64::from(ticks) {
            let mut input = orr_sim::TickInputs::new(tick, 1);
            input.set_input(orr_bridge::PlayerSlot(0), CollectInput {
                x: if right { orr_fp::FP::ONE } else { orr_fp::FP::ZERO },
                ..Default::default()
            });
            simulation.step(&input);
        }
        FrameProof { ticks, right, bytes: simulation.frame().to_bytes(), checksum: simulation.checksum() }
    }).collect()
}

fn actions(root: &Path) -> Vec<Option<Action>> {
    let prepared = open(root);
    let admitted = prepared.ui().unwrap();
    let mut ui = CollectUi::new(admitted.document.clone(), admitted.font.clone()).unwrap();
    ui.apply(Action::Play);
    let show = |ui: &mut CollectUi, events| {
        let (output, action) = ui.show(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 1024.0))),
            events, ..Default::default()
        }, Hud { score: 0, phase: PLAYING, best: "-".into() });
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        action
    };
    assert_eq!(show(&mut ui, vec![]), None);
    // Visible child, clipped-away child, parent-only region, lower-only region.
    [egui::pos2(100.0, 60.0), egui::pos2(150.0, 60.0), egui::pos2(60.0, 30.0), egui::pos2(30.0, 30.0)]
        .into_iter().map(|point| {
            let button = |pressed| egui::Event::PointerButton {
                pos: point, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
            };
            assert_eq!(show(&mut ui, vec![egui::Event::PointerMoved(point), button(true)]), None);
            show(&mut ui, vec![button(false)])
        }).collect()
}

fn front_actions() -> Vec<Option<Action>> {
    vec![Some(Action::Restart), Some(Action::Menu), None, Some(Action::Menu)]
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleProof {
    document: Document,
    frames: Vec<FrameProof>,
    actions: Vec<Option<Action>>,
}

/// Invoked only by the owning integration executable inside verified namespaces.
/// Missing environment fails. An exact name plus output file prevents a zero-test
/// libtest invocation from being mistaken for a successful source-hidden proof.
#[test]
#[ignore = "internal explicit source-hidden bundle probe; run through the owning GPU/export gate"]
fn layer_order_relocated_bundle_probe() {
    let project = required_path("ORR_LAYER_ORDER_PROBE_PROJECT");
    let output = required_path("ORR_LAYER_ORDER_PROBE_OUTPUT");
    let prepared = open(&project);
    let document = prepared.ui().unwrap().document.clone();
    assert_eq!(document, overlap_document(), "the exported saved Front document");
    let action_results = actions(&project);
    assert_eq!(action_results, front_actions());
    let proof = BundleProof { document, frames: frames(&project), actions: action_results };
    write_new(&output, &serde_json::to_vec_pretty(&proof).unwrap());
}

struct Work {
    root: tempfile::TempDir,
    profile: tempfile::TempDir,
    evidence: PathBuf,
    retained_bytes: Cell<u64>,
    hidden: Vec<PathBuf>,
    tools: PathBuf,
    generator: PathBuf,
    runtime: PathBuf,
    exporter: PathBuf,
    probe: PathBuf,
    runtime_hash: String,
}

impl Work {
    fn new() -> Self {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let profile = tempfile::Builder::new().permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
        let evidence = required_path("ORR_LAYER_ORDER_EVIDENCE");
        assert!(!evidence.exists(), "use a new evidence directory for every producer, including negative/restored runs");
        fs::create_dir(&evidence).unwrap();
        let evidence = evidence.canonicalize().unwrap();
        let workspace = required_path("ORR_LAYER_ORDER_HIDE_ROOT").canonicalize().unwrap();
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
        assert!(repository.starts_with(&workspace), "source mask must contain this exact test checkout");
        assert!(!root.path().starts_with(&workspace) && !profile.path().starts_with(&workspace));
        let tools = root.path().join("tools");
        fs::create_dir(&tools).unwrap();
        for name in ["captures", "empty cwd", "probe"] {
            fs::create_dir(root.path().join(name)).unwrap();
        }
        let profile_owner = fs::metadata(profile.path()).unwrap().uid();
        for name in ["user-home", "user-data", "user-cache", "user-runtime"] {
            let path = profile.path().join(name);
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(metadata.file_type().is_dir(), "owned profile leaf must be an ordinary directory");
            assert_eq!(metadata.uid(), profile_owner, "owned profile leaf owner");
            assert_eq!(metadata.mode() & 0o7777, 0o700, "private profile leaf mode");
        }
        let mut hidden = vec![workspace];
        let mut copies = Vec::new();
        let mut pins = BTreeMap::new();
        // These identities MUST already have been resolved and independently
        // verified by the controller. Self-hashing here is only a consistency
        // check; it does not bootstrap trust in the checksum utility itself.
        for name in ["TIMEOUT", "BWRAP", "SHA256SUM", "SHELL", "LS", "DIRNAME"] {
            let path = infrastructure(name);
            let expected = required_hash(&format!("ORR_LAYER_ORDER_{name}_SHA256"));
            assert_eq!(digest(&path), expected, "controller-admitted infrastructure identity: {name}");
            assert!(!hidden.iter().any(|root| path.starts_with(root)), "infrastructure cannot be source-masked");
            pins.insert(name.into(), serde_json::json!({"path": path, "sha256": expected}));
        }
        // The unchanged production launcher has a /bin/sh shebang and invokes
        // dirname by name. Bind those existing dependencies to admitted identities.
        assert_eq!(Path::new("/bin/sh").canonicalize().unwrap(), infrastructure("SHELL"));
        let dirname = infrastructure("DIRNAME");
        assert_eq!(dirname.parent().unwrap().join("dirname").canonicalize().unwrap(), dirname);
        for (path_key, hash_key, name) in [
            ("ORR_NEW_ARENA_BIN", "ORR_LAYER_ORDER_GENERATOR_SHA256", "orr_new_arena"),
            ("ORR_COLLECT_RUNTIME", "ORR_LAYER_ORDER_RUNTIME_SHA256", "collect_dodge"),
            ("ORR_COLLECT_EXPORTER", "ORR_LAYER_ORDER_EXPORTER_SHA256", "orr_export_collect"),
        ] {
            let original = required_path(path_key).canonicalize().unwrap();
            assert!(original.is_file());
            let expected = required_hash(hash_key);
            assert_eq!(digest(&original), expected, "independently supplied producer pin: {path_key}");
            let copy = tools.join(name);
            bounded_copy(&original, &copy, 192 * 1024 * 1024);
            assert_eq!(digest(&copy), expected, "copied producer must match its independent pin");
            hidden.push(original.parent().unwrap().to_owned());
            pins.insert(path_key.to_owned(), serde_json::json!({"source": original, "copy": copy, "sha256": expected}));
            copies.push(copy);
        }
        let original_probe = std::env::current_exe().unwrap().canonicalize().unwrap();
        let expected_probe = required_hash("ORR_LAYER_ORDER_ACCEPTANCE_SHA256");
        assert_eq!(digest(&original_probe), expected_probe, "pin the exact integration executable");
        let probe = root.path().join("probe/layer-order-acceptance");
        bounded_copy(&original_probe, &probe, 192 * 1024 * 1024);
        assert_eq!(digest(&probe), expected_probe);
        hidden.push(original_probe.parent().unwrap().to_owned());
        pins.insert("acceptance".into(), serde_json::json!({"source": original_probe, "copy": probe, "sha256": expected_probe}));
        hidden.sort();
        hidden.dedup();
        let all_hidden = hidden.clone();
        hidden.retain(|path| !all_hidden.iter().any(|parent| path != parent && path.starts_with(parent)));
        for path in &hidden {
            assert!(path.is_dir());
            assert!(!root.path().starts_with(path) && !profile.path().starts_with(path),
                "source masks must not mask the test-owned work/profile roots");
            for name in ["TIMEOUT", "BWRAP", "SHA256SUM", "SHELL", "LS", "DIRNAME"] {
                assert!(!infrastructure(name).starts_with(path), "source masks cannot hide infrastructure: {name}");
            }
        }
        let result = Self {
            generator: copies[0].clone(), runtime: copies[1].clone(), exporter: copies[2].clone(),
            runtime_hash: required_hash("ORR_LAYER_ORDER_RUNTIME_SHA256"),
            root, profile, evidence, hidden, tools, probe,
            retained_bytes: Cell::new(0),
        };
        result.record("producer-pins.json", &serde_json::to_vec_pretty(&pins).unwrap());
        result.record("source-masks.json", &serde_json::to_vec_pretty(&result.hidden).unwrap());
        result
    }

    fn reserve_retained(&self, count: u64) {
        let charged = self.retained_bytes.get().checked_add(count).unwrap();
        assert!(charged <= 120 * 1024 * 1024, "aggregate retained fixture byte cap");
        self.retained_bytes.set(charged);
    }

    fn record(&self, name: &str, bytes: &[u8]) {
        assert!(bytes.len() <= 4 * 1024 * 1024);
        self.reserve_retained(bytes.len() as u64);
        write_new(&self.evidence.join(name), bytes);
    }

    fn retain(&self, source: &Path) {
        let destination = self.evidence.join(source.file_name().unwrap());
        assert!(!destination.exists());
        let maximum = if source.extension().is_some_and(|value| value == "png") {
            8 * 1024 * 1024
        } else {
            4 * 1024 * 1024
        };
        let count = fs::symlink_metadata(source).unwrap().len();
        assert!(count <= maximum);
        self.reserve_retained(count);
        bounded_copy(source, &destination, count);
    }

    /// Source/build roots are masked for every executable. Exported processes
    /// additionally lose both the original project and copied producer tools.
    fn isolated(&self, executable: &Path, exported: Option<(&Path, &Path)>) -> Command {
        let mut hidden = self.hidden.clone();
        if let Some((source, _bundle)) = exported {
            hidden.extend([source.to_owned(), self.tools.clone()]);
        }
        for path in &hidden {
            assert!(!executable.starts_with(path));
            assert!(!self.root.path().starts_with(path));
        }
        let mut command = Command::new(infrastructure("TIMEOUT"));
        command.args(["--foreground", "--signal=TERM", "--kill-after=5s", "120s"])
            .arg(infrastructure("BWRAP"))
            .args(["--die-with-parent", "--ro-bind", "/", "/", "--dev-bind", "/dev/null", "/dev/null", "--bind"])
            // Preserve the admitted namespace pattern: /tmp is a private writable
            // profile parent, then restore only this test's owned Work subtree.
            .arg(self.profile.path()).arg("/tmp")
            .arg("--bind").arg(self.root.path()).arg(self.root.path());
        for path in &hidden {
            command.arg("--tmpfs").arg(path);
        }
        let bundle = exported.map(|(_, bundle)| bundle);
        if let Some(bundle) = bundle {
            assert!(!hidden.iter().any(|path| bundle.starts_with(path)));
            command.arg("--ro-bind").arg(bundle).arg(bundle);
        }
        // Empty-mask checks must succeed before exec. For exported processes,
        // prove a create attempt is denied inside the read-only bundle as well.
        command.arg("--").arg(infrastructure("SHELL")).args(["-c",
            "listing=$1; shift; n=$1; shift; while [ \"$n\" -gt 0 ]; do test -d \"$1\" || exit 90; contents=$(\"$listing\" -A -- \"$1\") || exit 90; test -z \"$contents\" || exit 91; shift; n=$((n-1)); done; ro=$1; shift; if [ \"$ro\" != - ]; then test -d \"$ro\" || exit 94; if ( : > \"$ro/.layer-order-write-probe\" ) 2>/dev/null; then exit 92; fi; test ! -e \"$ro/.layer-order-write-probe\" || exit 93; fi; printf 'ORR_LAYER_ORDER_ISOLATION_OK\\n'; exec \"$@\"", "layer-order-isolation"])
            .arg(infrastructure("LS"))
            .arg(hidden.len().to_string());
        for path in &hidden {
            command.arg(path);
        }
        command.arg(bundle.unwrap_or(Path::new("-")))
            .arg(executable).current_dir(self.root.path().join("empty cwd"))
            .env("HOME", "/tmp/user-home").env("XDG_DATA_HOME", "/tmp/user-data")
            .env("XDG_CACHE_HOME", "/tmp/user-cache").env("XDG_RUNTIME_DIR", "/tmp/user-runtime")
            .env("PATH", infrastructure("DIRNAME").parent().unwrap())
            .env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
        command
    }

    fn execute(&self, label: &str, command: Command, capture: Option<&Path>) -> String {
        assert!(!self.evidence.join(format!("{label}.stdout.txt")).exists());
        let captured = bounded(command, 130);
        let output = &captured.output;
        // Retain raw failure output and any completed PNG before checking status.
        self.record(&format!("{label}.stdout.txt"), &output.stdout);
        self.record(&format!("{label}.stderr.txt"), &output.stderr);
        self.record(&format!("{label}.status.txt"), output.status.to_string().as_bytes());
        self.record(&format!("{label}.transport.json"), &serde_json::to_vec_pretty(&serde_json::json!({
            "per_stream_cap": OUTPUT_CAP,
            "overflow": captured.overflow,
            "expired": captured.expired,
            "errors": captured.transport_errors,
            "descendant_cleanup_claim": false,
            "cleanup_owner": "outer reviewed controller"
        })).unwrap());
        if let Some(path) = capture.filter(|path| path.is_file()) {
            self.retain(path);
        }
        let stdout = good(captured);
        assert!(stdout.lines().any(|line| line == ISOLATION_OK), "verified source-mask preflight missing");
        stdout
    }

    fn capture(&self, label: &str, project: &Path, bundle: Option<&Path>, proof: &FrameProof) -> PathBuf {
        let path = self.root.path().join("captures").join(format!("{label}.png"));
        assert!(!path.exists());
        let executable = bundle.map_or_else(|| self.runtime.clone(), |bundle| bundle.join("run-collect-dodge"));
        let mut command = self.isolated(&executable, bundle.map(|bundle| (project, bundle)));
        if bundle.is_none() {
            command.arg("--project").arg(project);
        }
        command.args(["--headless", "--ticks", &proof.ticks.to_string()]);
        if proof.right {
            command.args(["--hold", "right"]);
        }
        command.arg("--capture").arg(&path);
        let stdout = self.execute(label, command, Some(&path));
        assert!(path.is_file(), "mandatory runtime PNG missing");
        assert!(stdout.contains("software: true"), "mandatory verified software adapter: {stdout}");
        let expected = format!("collect tick: {} checksum: 0x{:016x}", proof.ticks, proof.checksum);
        assert_eq!(stdout.lines().filter(|line| line.starts_with("collect tick:")).collect::<Vec<_>>(), vec![expected.as_str()]);
        path
    }
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn editor(root: &Path) -> Harness<'static, EditorApp> {
    let prepared = open(root);
    let mut editor = Editor::start(&HostSpec::PreparedCollect {
        scene: prepared.path().into(), text: prepared.scene().text().into(), listen: None, debug_hooks: false,
    }).unwrap();
    editor.sync();
    let mut h = Harness::builder().with_size([1800.0, 1600.0]).build_eframe(move |cc| {
        let mut app = EditorApp::new(editor, None);
        orr_editor::project::install_collect_presentation(&mut app, prepared, &cc.egui_ctx);
        app
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        h.run_steps(1);
        if h.state().editor.mode() == Mode::Edit && h.state().editor.yard_rows_coherent()
            && h.state().editor.snapshot().is_some_and(|snapshot| snapshot.timeline().is_none()) {
            break;
        }
        assert!(Instant::now() < deadline, "EditorApp admission: {:?}", h.state().editor.status());
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run_steps(3);
    click(&mut h, "Collect UI document");
    h.get_by_role_and_label(Role::ComboBox, "UI node").click();
    h.run_steps(3);
    click(&mut h, "cover");
    h
}

fn save(h: &mut Harness<'_, EditorApp>, root: &Path, expected: &Document) {
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), expected);
    h.get_by_label("Stable id: cover");
    let mut before = inventory(root);
    click(h, SAVE);
    assert_eq!(fs::read(root.join(UI_FILE)).unwrap(), expected.to_bytes().unwrap());
    assert_eq!(open(root).ui().unwrap().document, *expected);
    let after = inventory(root);
    // The production atomic replacement may change this file's permissions;
    // all other project paths, bytes and modes must remain exactly unchanged.
    before.insert(PathBuf::from(UI_FILE), after[Path::new(UI_FILE)].clone());
    assert_eq!(after, before, "UI Save only replaces its admitted file");
}

#[derive(Debug, PartialEq, Eq)]
struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn image(path: &Path) -> Image {
    let mut reader = png::Decoder::new(fs::File::open(path).unwrap()).read_info().unwrap();
    assert_eq!((reader.info().width, reader.info().height), (1024, 1024));
    assert_eq!(reader.output_buffer_size(), 1024 * 1024 * 4);
    let mut rgba = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgba).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    rgba.truncate(info.buffer_size());
    Image { width: info.width, height: info.height, rgba }
}

/// Independent fixed geometry from overlap_document, not runtime layout internals.
/// The child glyph ROI is intentional: both buttons share a normal-state fill,
/// so comparing arbitrary same-fill interiors would be a false-negative oracle.
fn assert_overlap_pixels(front: &Image, back: &Image) {
    assert_eq!((front.width, front.height), (back.width, back.height));
    assert_eq!(front.rgba.len(), back.rgba.len());
    assert_eq!(front.rgba.len(), front.width as usize * front.height as usize * 4);
    let mut parent_changes = 0;
    let mut child_changes = 0;
    let mut clipped_samples = 0;
    for (index, (a, b)) in front.rgba.as_chunks::<4>().0.iter().zip(back.rgba.as_chunks::<4>().0.iter()).enumerate() {
        let x = index as u32 % front.width;
        let y = index as u32 / front.width;
        let parent = (40..120).contains(&x) && (20..100).contains(&y);
        let parent_roi = (46..76).contains(&x) && (27..43).contains(&y);
        let child_roi = (70..116).contains(&x) && (54..76).contains(&y);
        let clipped_roi = (132..156).contains(&x) && (54..76).contains(&y);
        if a != b {
            assert!(parent, "layer-order pixels escaped the parent's visible bounds at ({x},{y})");
            parent_changes += usize::from(parent_roi);
            child_changes += usize::from(child_roi);
        }
        if clipped_roi {
            clipped_samples += 1;
            assert_eq!(a, b, "clipped-away child cannot change ({x},{y})");
        }
    }
    assert_eq!(clipped_samples, 24 * 22);
    assert!(parent_changes > 100, "filled-parent ROI must visibly reorder over lower button");
    assert!(child_changes > 10, "clipped Restart glyph ROI must visibly reorder over the blank lower button");
}

#[test]
#[ignore = "mandatory explicit pinned software-GPU, relocated source-hidden read-only export acceptance"]
fn gpu_saved_layer_order_source_hidden_readonly_export_preserves_frames() {
    let work = Work::new();
    let project = work.root.path().join("authored project");
    let mut generate = work.isolated(&work.generator, None);
    generate.arg("--output").arg(&project).args(["--template", "collect-dodge-ui-2d-v1", "--seed", "layer-order-gpu", "--game-id", GAME_ID]);
    work.execute("00-generate", generate, None);
    let baseline = frames(&project);
    work.record("baseline-frames.json", &serde_json::to_vec_pretty(&baseline).unwrap());
    let original = overlap_document();
    let back = Document { schema: original.schema, nodes: vec![original.nodes[1].clone(), original.nodes[2].clone(), original.nodes[0].clone()] };
    back.validate().unwrap();
    // Setup uses the production validated save path, then reopens empty history.
    let mut setup_before = inventory(&project);
    let mut prepared = open(&project);
    let mut panel = Panel::new(prepared.take_ui().unwrap());
    panel.apply_document(original.clone()).unwrap();
    panel.save().unwrap();
    drop(panel);
    let setup_after = inventory(&project);
    setup_before.insert(PathBuf::from(UI_FILE), setup_after[Path::new(UI_FILE)].clone());
    assert_eq!(setup_after, setup_before, "fixture admission/save changes only its UI file");
    work.record("original.ui.json", &fs::read(project.join(UI_FILE)).unwrap());
    work.record("pixel-oracles.json", &serde_json::to_vec_pretty(&serde_json::json!({
        "capture_size": [1024, 1024],
        "parent_changed_bounds_exclusive_max": [40, 20, 120, 100],
        "parent_only_roi": [46, 27, 76, 43],
        "child_glyph_roi": [70, 54, 116, 76],
        "clipped_away_unchanged_roi": [132, 54, 156, 76],
        "every_pixel_outside_parent": "must remain exactly equal"
    })).unwrap());
    assert_eq!(frames(&project), baseline, "fixture UI alone cannot change any Frame byte");
    let mut h = editor(&project);
    save(&mut h, &project, &original);
    assert_eq!(actions(&project), front_actions());
    let original_png = work.capture("01-original-t0", &project, None, &baseline[0]);
    let mut order_pngs = Vec::new();
    for (label, command, expected, expected_actions) in [
        ("02-back-t0", BACK, &back, vec![Some(Action::Menu); 4]),
        ("03-front-t0", FRONT, &original, front_actions()),
        ("04-undo-t0", UNDO, &back, vec![Some(Action::Menu); 4]),
        ("05-redo-t0", REDO, &original, front_actions()),
    ] {
        let before = inventory(&project);
        click(&mut h, command);
        assert_eq!(inventory(&project), before, "no implicit save during reorder/history");
        save(&mut h, &project, expected);
        work.record(&format!("{label}.ui.json"), &fs::read(project.join(UI_FILE)).unwrap());
        assert_eq!(actions(&project), expected_actions, "real saved-document pointer dispatch: {label}");
        assert_eq!(frames(&project), baseline, "four complete Frame bytes/checksums: {label}");
        order_pngs.push(work.capture(label, &project, None, &baseline[0]));
    }
    drop(h);
    assert_overlap_pixels(&image(&original_png), &image(&order_pngs[0]));
    assert_eq!(image(&order_pngs[1]), image(&original_png), "Front restores original exact runtime pixels");
    assert_eq!(image(&order_pngs[2]), image(&order_pngs[0]), "Undo restores Back pixels");
    assert_eq!(image(&order_pngs[3]), image(&original_png), "Redo restores Front pixels");

    let source_before = inventory(&project);
    work.record("source-inventory.json", &serde_json::to_vec_pretty(&source_before).unwrap());
    let built = work.root.path().join("export original");
    let mut export = work.isolated(&work.exporter, None);
    export.arg("--project").arg(&project).arg("--runtime").arg(&work.runtime)
        .arg("--runtime-sha256").arg(&work.runtime_hash).arg("--trusted-runtime").arg("--output").arg(&built);
    work.execute("06-export", export, None);
    let bundle = work.root.path().join("relocated readonly bundle");
    fs::rename(&built, &bundle).unwrap();
    assert!(!built.exists());
    let bundle_before = inventory(&bundle);
    work.record("bundle-inventory.json", &serde_json::to_vec_pretty(&bundle_before).unwrap());
    assert_eq!(inventory(&project), source_before, "exporter must preserve source bytes/modes/inventory");
    assert_eq!(fs::read(bundle.join("project").join(UI_FILE)).unwrap(), original.to_bytes().unwrap());

    // A copied exact-head helper reads ONLY the relocated admitted project while
    // the source, original binaries, original project and producer tools are masked.
    let proof_path = work.root.path().join("captures/relocated-proof.json");
    let mut probe = work.isolated(&work.probe, Some((&project, &bundle)));
    probe.args([HELPER, "--exact", "--ignored", "--nocapture", "--test-threads=1"])
        .env("ORR_LAYER_ORDER_PROBE_PROJECT", bundle.join("project"))
        .env("ORR_LAYER_ORDER_PROBE_OUTPUT", &proof_path);
    let probe_output = work.execute("07-relocated-proof", probe, None);
    assert!(probe_output.contains("running 1 test"), "source-hidden helper must actually execute");
    assert!(fs::metadata(&proof_path).unwrap().len() < 4 * 1024 * 1024);
    work.retain(&proof_path);
    let proof: BundleProof = serde_json::from_slice(&fs::read(&proof_path).unwrap()).unwrap();
    assert_eq!(proof, BundleProof { document: original.clone(), frames: baseline.clone(), actions: front_actions() });

    for (index, frame) in baseline.iter().enumerate() {
        let source_png = work.capture(&format!("08-probe-{index}-source"), &project, None, frame);
        let exported_png = work.capture(&format!("08-probe-{index}-exported"), &project, Some(&bundle), frame);
        assert_eq!(image(&source_png), image(&exported_png), "exact source-hidden read-only runtime pixels at probe {index}");
        if index == 0 {
            assert_eq!(image(&source_png), image(&original_png), "final source remained at saved Front order");
        }
    }
    assert_eq!(inventory(&project), source_before);
    assert_eq!(inventory(&bundle), bundle_before, "runtime/helper cannot modify relocated bundle");
    work.record("complete.json", &serde_json::to_vec_pretty(&serde_json::json!({
        "software_gpu_captures": 13,
        "order_states": ["original", "back", "front", "undo", "redo"],
        "export_pixel_pairs": 4,
        "frame_probes": PROBES,
        "source_hidden_bundle_actions_and_full_frames": true,
        "native_window_or_physical_gpu_claim": false
    })).unwrap());
}
