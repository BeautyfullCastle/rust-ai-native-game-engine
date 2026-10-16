//! Mandatory production Room export acceptance for an actual emitted wall operation.
//! Frame probes are separately labelled from production runtime PNG/checksum evidence.
//! Fresh retained CLI/runtime/exporter, software GPU, bwrap, and read-only mounts are
//! required. A missing capability is a failure; normal test runs list this as ignored.
#![cfg(all(
    feature = "room-project",
    feature = "project-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types)]
use orr_reflect::{Scene, Value};
use orr_sample::{
    room_game::{RoomConfig, RoomEscapeV1},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::Simulation;
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
const GUID: &str = "e_abcdef0123456789";
const PACKAGE: &str = "sample-imported-scene";
const PACKAGE_DIGEST: &str = "9c7708914c9e5fbe61dad5cc717dff6df0d811d0e14f3aa9a967896785522cf0";
// Fixture identity is independently reproducible with the package manager's
// canonical ["orr-package-v1", manifest, sorted file hashes] encoding. Opening
// PreparedProject verifies these hashes, including every embedded source file.
const LOCK: &str = r#"{"schema":1,"direct":{"sample-imported-scene":"1.0.0"},"packages":{"sample-imported-scene":{"manifest":{"schema":1,"name":"sample-imported-scene","version":"1.0.0","engine":"^0.0.1","capabilities":["models"],"dependencies":{},"files":["LICENSE.txt","background.glb","foreground.glb","generate.py"]},"digest":"9c7708914c9e5fbe61dad5cc717dff6df0d811d0e14f3aa9a967896785522cf0","files":{"LICENSE.txt":"398039150c615dec7d81ca90231e4d94fa142fe2ffbec784d09ab3407c49f37c","background.glb":"f93967c23bec3fbbe3bf64445e364665e08fbebecd211429f2b16ca9e615340d","foreground.glb":"465f0e8acb9b567b575f05004113580302d57f585b18d14cee402841925652fd","generate.py":"b7bb27c9bcf2ea582a1babb9a7c9ea2475437f936c2a19d4b23f0cc31e9cc2e8"}}}}"#;
const ASSETS: &[(&str, &[u8])] = &[
    (
        "orr.package.json",
        include_bytes!("../../../assets/imported_scene_demo/orr.package.json"),
    ),
    (
        "LICENSE.txt",
        include_bytes!("../../../assets/imported_scene_demo/LICENSE.txt"),
    ),
    (
        "background.glb",
        include_bytes!("../../../assets/imported_scene_demo/background.glb"),
    ),
    (
        "foreground.glb",
        include_bytes!("../../../assets/imported_scene_demo/foreground.glb"),
    ),
    (
        "generate.py",
        include_bytes!("../../../assets/imported_scene_demo/generate.py"),
    ),
];

struct Fixture {
    base: PathBuf,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir().join(format!(
            "orr-room-wall-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let root = base.join("admitted room");
        fs::create_dir(&root).unwrap();
        let object = root.join(".orr/packages/objects").join(PACKAGE_DIGEST);
        fs::create_dir_all(&object).unwrap();
        for (name, bytes) in ASSETS {
            fs::write(object.join(name), bytes).unwrap();
        }
        fs::write(root.join("orr.packages.lock.json"), LOCK).unwrap();
        let sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        let scene = Scene::unbake(&room_project::types(), sim.frame(), None).unwrap();
        fs::write(root.join("room.scene.yaml"), scene.to_yaml()).unwrap();
        fs::write(root.join("orr.project.json"), serde_json::to_vec_pretty(&json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
        fs::write(
            root.join("room.camera.json"),
            orr_sample::room_camera::Document::readable_default()
                .to_bytes()
                .unwrap(),
        )
        .unwrap();
        let mut bindings = serde_json::Map::new();
        for (guid, entity) in &scene.entities {
            let kind = entity
                .components
                .iter()
                .find(|(name, _)| name == room_project::ACTOR)
                .unwrap()
                .1
                .field("kind")
                .unwrap();
            if matches!(kind, Value::Int(1 | 2)) {
                bindings.insert(guid.to_string(), json!({"kind":"static","package":PACKAGE,"asset":"foreground.glb","package_digest":PACKAGE_DIGEST,"source_hash":"465f0e8acb9b567b575f05004113580302d57f585b18d14cee402841925652fd","transform":{"translation":[0,0,0],"rotation":[0,0,0,1],"scale":[1,1,1]}}));
            }
        }
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec_pretty(
                &json!({"version":2,"scene":"room.scene.yaml","project":".","bindings":bindings}),
            )
            .unwrap(),
        )
        .unwrap();
        PreparedProject::open(&root)
            .expect("pinned fixture must pass production package/model/scene/camera admission");
        Self { base, root }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "FAILED Room wall export fixture retained: {}",
                self.base.display()
            );
        } else {
            let _ = fs::remove_dir_all(&self.base);
        }
    }
}

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                visit(root, &entry.path(), out);
            } else {
                assert!(kind.is_file());
                out.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap();
    for (path, bytes) in tree(from) {
        let target = to.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }
}
fn fnv(bytes: &[u8]) -> String {
    format!(
        "{:016x}",
        bytes
            .iter()
            .fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b))
                .wrapping_mul(0x100000001b3))
    )
}
fn producer() -> PathBuf {
    let path =
        PathBuf::from(std::env::var_os("ORR_ROOM_WALL_BIN").expect("mandatory fresh wall CLI"));
    assert!(path.is_absolute() && path.is_file());
    path
}
fn produce(root: &Path, output: &Path, center: [&str; 2], half: [&str; 2]) -> std::process::Output {
    let result = bounded_output(
        Command::new(producer())
            .arg("--project")
            .arg(root)
            .args([
                "--guid",
                GUID,
                "--name",
                "authored wall",
                "--center",
                center[0],
                center[1],
                "--half-extents",
                half[0],
                half[1],
                "--output",
            ])
            .arg(output),
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
}
// Read the ordinary emitted spawn through the production reflection parser.
// This adapter does not author a replacement wall or special operation schema.
// ERP guarded acceptance and save are independently exercised in orr_remote;
// here EditorDoc applies the same emitted op, then full project admission checks it.
fn consume(root: &Path, bytes: &[u8]) {
    let json: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 1);
    let op = &json[0];
    assert_eq!(op["op"], "spawn");
    assert_eq!(op["guid"], GUID);
    assert_eq!(
        op.as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        ["op", "guid", "name", "components"].into_iter().collect()
    );
    let mut yaml = format!(
        "schema: orr.scene/1\nentities:\n  {GUID}:\n    name: {}\n",
        op["name"]
    );
    for (name, value) in op["components"].as_object().unwrap() {
        yaml.push_str(&format!(
            "    {}: {}\n",
            serde_json::to_string(name).unwrap(),
            value
        ));
    }
    let fragment = Scene::parse(&yaml, &room_project::types()).unwrap();
    let entity = fragment.entities.values().next().unwrap();
    let prepared = PreparedProject::open(root).unwrap();
    let before = tree(root);
    let mut doc = orr_edit::EditorDoc::from_yaml(
        prepared.scene().text(),
        room_project::types(),
        Simulation::<RoomEscapeV1>::build_registry(),
        SEED,
    )
    .unwrap();
    doc.apply(
        orr_edit::Op::SpawnEntity {
            guid: Some(orr_reflect::Guid::parse(GUID).unwrap()),
            name: entity.name.clone(),
            components: entity.components.clone(),
        },
        orr_edit::Origin::User,
    )
    .unwrap();
    PreparedScene::parse(&doc.to_yaml()).unwrap();
    let initial = Scene::parse(prepared.scene().text(), &room_project::types()).unwrap();
    for (guid, entity) in &initial.entities {
        assert_eq!(doc.scene().entities[guid], *entity);
    }
    assert_eq!(doc.scene().singletons, initial.singletons);
    fs::write(root.join("room.scene.yaml"), doc.to_yaml()).unwrap();
    let reopened = PreparedProject::open(root).unwrap();
    assert_eq!(reopened.scene().frame().to_bytes(), doc.frame().to_bytes());
    let mut after = tree(root);
    let mut before = before;
    before.remove(Path::new("room.scene.yaml"));
    after.remove(Path::new("room.scene.yaml"));
    assert_eq!(
        before, after,
        "wall adoption changed model/camera/package data"
    );
}
fn artifact(dir: &Path, name: &str, bytes: &[u8]) -> serde_json::Value {
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(name))
        .unwrap();
    f.write_all(bytes).unwrap();
    f.sync_all().unwrap();
    json!({"file":name,"bytes":bytes.len(),"fnv1a64":fnv(bytes)})
}

use orr_bridge::PlayerSlot;
use orr_ecs::Frame;
use orr_sample::{
    project_export::ExportManifest,
    room_game::{RoomInput, RoomRun, INTERACT},
};
use orr_sim::TickInputs;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::Read,
    os::{fd::AsFd, unix::process::CommandExt},
    process::{Child, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
// Local child limits complement the CI runner's whole-test/process-tree budget.
// Like the production exporter, retain the child PID until group cleanup so an
// exited launcher cannot leave descendants or race PID reuse during cleanup.
struct BoundedChild(Child, bool);
impl BoundedChild {
    fn pid(&self) -> rustix::process::Pid {
        rustix::process::Pid::from_raw(i32::try_from(self.0.id()).unwrap()).unwrap()
    }
    fn terminate_group(&self) -> Result<(), rustix::io::Errno> {
        match rustix::process::kill_process_group(self.pid(), rustix::process::Signal::KILL) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
            Err(error) => Err(error),
        }
    }
}
impl Drop for BoundedChild {
    fn drop(&mut self) {
        if self.1 {
            let _ = self.terminate_group();
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn nonblocking(fd: impl AsFd) {
    let flags = rustix::fs::fcntl_getfl(&fd).unwrap();
    rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK).unwrap();
}

fn drain(stream: &mut impl Read, bytes: &mut Vec<u8>, remaining: usize) -> bool {
    let mut buffer = [0; 8192];
    // Exactly one read per iteration keeps a noisy child from starving timeout.
    match stream.read(&mut buffer) {
        Ok(0) => true,
        Ok(count) => {
            assert!(count <= remaining, "acceptance child output exceeds 1 MiB");
            bytes.extend_from_slice(&buffer[..count]);
            false
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) =>
        {
            false
        }
        Err(error) => panic!("acceptance child read failed: {error}"),
    }
}

fn bounded_output(command: &mut Command) -> Output {
    let mut child = BoundedChild(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .expect("launch exact acceptance tool"),
        true,
    );
    let mut stdout = child.0.stdout.take().unwrap();
    let mut stderr = child.0.stderr.take().unwrap();
    nonblocking(&stdout);
    nonblocking(&stderr);
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut status = None;
    let started = Instant::now();
    loop {
        let remaining = (1024usize * 1024).saturating_sub(out.len() + err.len());
        let out_done = drain(&mut stdout, &mut out, remaining);
        let remaining = (1024usize * 1024).saturating_sub(out.len() + err.len());
        let err_done = drain(&mut stderr, &mut err, remaining);
        if status.is_none() {
            use rustix::process::{waitid, WaitId, WaitIdOptions};
            let exited = waitid(
                WaitId::Pid(child.pid()),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )
            .unwrap()
            .is_some();
            if exited {
                child.terminate_group().unwrap();
                status = Some(child.0.wait().unwrap());
                child.1 = false;
            }
        }
        if let Some(status) = status {
            if out_done && err_done {
                return Output {
                    status,
                    stdout: out,
                    stderr: err,
                };
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "acceptance child exceeded 120s: {command:?}"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

const TEST: &str = "room_wall_real_cli_source_hidden_room_export";
const STAGES: [(&str, u32, &str); 5] = [
    ("initial", 0, ""),
    ("moved", 1, "right"),
    ("collision", 120, "right"),
    ("interacted", 1, "interact"),
    ("restarted", 0, ""),
];
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn file_hash(path: &Path) -> String {
    let mut f = fs::File::open(path).unwrap();
    let mut h = Sha256::new();
    let mut b = [0; 65536];
    loop {
        let n = f.read(&mut b).unwrap();
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
fn inventory(root: &Path) -> BTreeMap<String, (u64, String)> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<String, (u64, String)>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                visit(root, &entry.path(), out);
            } else {
                assert!(kind.is_file());
                out.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    (entry.metadata().unwrap().len(), file_hash(&entry.path())),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn good(out: Output) -> String {
    assert!(
        out.status.success(),
        "{}\n{}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
fn bad(out: Output, needle: &str) -> serde_json::Value {
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(needle),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    json!({"exit":out.status.code().expect("negative control must exit normally"),"stdout":String::from_utf8(out.stdout).unwrap(),"stderr":String::from_utf8(out.stderr).unwrap()})
}
fn simulate(scene: &PreparedScene, ticks: u32, held: &str) -> Frame {
    let input = match held {
        "" => RoomInput::default(),
        "right" => RoomInput {
            move_x: 1,
            ..RoomInput::default()
        },
        "interact" => RoomInput {
            buttons: INTERACT,
            ..RoomInput::default()
        },
        _ => panic!("bounded held fixture"),
    };
    let mut sim = scene.simulation().unwrap();
    for _ in 0..ticks {
        let mut inputs = TickInputs::new(sim.tick() + 1, 1);
        inputs.set_input(PlayerSlot(0), input);
        sim.step(&inputs);
    }
    sim.frame().clone()
}
fn pixels(path: &Path) -> Vec<u8> {
    let mut reader = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap()))
        .read_info()
        .unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(
        (info.width, info.height, info.color_type, info.bit_depth),
        (1024, 768, png::ColorType::Rgba, png::BitDepth::Eight)
    );
    bytes.truncate(info.buffer_size());
    assert_eq!(bytes.len(), 1024 * 768 * 4);
    assert!(
        bytes.chunks_exact(4).filter(|p| *p != &bytes[..4]).count() > 100,
        "blank GPU capture"
    );
    bytes
}
fn differences(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

struct Work {
    fixture: Fixture,
    workspace: PathBuf,
    evidence: PathBuf,
    projects: [PathBuf; 3],
    runtime: PathBuf,
    exporter: PathBuf,
    cli: PathBuf,
    probe: PathBuf,
    originals: Vec<PathBuf>,
}
impl Work {
    fn new() -> Self {
        for key in ["ORR_REQUIRE_GPU", "ORR_REQUIRE_PROJECT_ISOLATION"] {
            assert_eq!(std::env::var(key).as_deref(), Ok("1"), "mandatory {key}");
        }
        let fixture = Fixture::new();
        let base = &fixture.base;
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .unwrap();
        assert!(!base.starts_with(&workspace));
        let evidence = PathBuf::from(
            std::env::var_os("ORR_ROOM_WALL_EVIDENCE").expect("mandatory export evidence"),
        );
        assert!(evidence.is_absolute() && evidence.is_dir());
        assert_eq!(evidence, evidence.canonicalize().unwrap());
        assert_eq!(fs::read_dir(&evidence).unwrap().count(), 0);
        assert!(!evidence.starts_with(&workspace));
        assert!(!evidence.starts_with(base) && !base.starts_with(&evidence));
        for name in [
            "tools",
            "probe",
            "authoring",
            "captures",
            "readonly",
            "empty",
            "home",
            "xdg-data",
            "xdg-config",
            "xdg-cache",
            "xdg-runtime",
        ] {
            fs::create_dir(base.join(name)).unwrap();
        }
        let mut originals = Vec::new();
        let mut copies = Vec::new();
        for (key, name) in [
            ("ORR_ROOM_RUNTIME", "room_escape"),
            ("ORR_ROOM_EXPORTER", "orr_export_room"),
            ("ORR_ROOM_WALL_BIN", "room_wall"),
        ] {
            let original =
                PathBuf::from(std::env::var_os(key).expect("mandatory fresh production binary"));
            assert!(original.is_absolute() && original.is_file());
            let original = original.canonicalize().unwrap();
            assert!(!original.starts_with(base));
            let copy = base.join("tools").join(name);
            fs::copy(&original, &copy).unwrap();
            assert_eq!(file_hash(&original), file_hash(&copy));
            originals.push(original);
            copies.push(copy);
        }
        let original = std::env::current_exe().unwrap().canonicalize().unwrap();
        let probe = base.join("probe/acceptance");
        fs::copy(&original, &probe).unwrap();
        assert_eq!(file_hash(&original), file_hash(&probe));
        originals.push(original);
        let projects = [
            base.join("authored"),
            fixture.root.clone(),
            base.join("wrong-position"),
        ];
        copy_tree(&fixture.root, &projects[0]);
        copy_tree(&fixture.root, &projects[2]);
        let original_tree = tree(&fixture.root);
        let ops = base.join("authoring/wall.ops.json");
        let wrong = base.join("authoring/wrong-position.ops.json");
        produce(&projects[0], &ops, ["0.625", "0"], ["0.125", "0.75"]);
        produce(&projects[2], &wrong, ["-2", "0"], ["0.125", "0.75"]);
        assert_eq!(tree(&fixture.root), original_tree);
        // Repeated independently emitted operation bytes are deterministic.
        let repeat = base.join("authoring/repeat.ops.json");
        produce(&projects[0], &repeat, ["0.625", "0"], ["0.125", "0.75"]);
        assert_eq!(fs::read(&ops).unwrap(), fs::read(&repeat).unwrap());
        fs::remove_file(repeat).unwrap();
        consume(&projects[0], &fs::read(ops).unwrap());
        consume(&projects[2], &fs::read(wrong).unwrap());
        Self {
            fixture,
            workspace,
            evidence,
            projects,
            runtime: copies[0].clone(),
            exporter: copies[1].clone(),
            cli: copies[2].clone(),
            probe,
            originals,
        }
    }
    fn isolated(&self, executable: &Path, project: &Path, bundle: Option<&Path>) -> Command {
        let base = &self.fixture.base;
        let mut c = Command::new("bwrap");
        c.args([
            "--die-with-parent",
            "--ro-bind",
            "/",
            "/",
            "--dev-bind",
            "/dev/null",
            "/dev/null",
            "--bind",
        ])
        .arg(base)
        .arg(base)
        .arg("--tmpfs")
        .arg(&self.workspace)
        .arg("--tmpfs")
        .arg(base.join("authoring"))
        .arg("--ro-bind")
        .arg(base.join("readonly"))
        .arg(base.join("readonly"));
        let mut missing = vec![
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"),
            base.join("authoring/wall.ops.json"),
            base.join("authoring/wrong-position.ops.json"),
        ];
        for root in &self.projects {
            if bundle.is_some() || root != project {
                c.arg("--tmpfs").arg(root);
                missing.push(root.join("orr.project.json"));
            }
        }
        for original in &self.originals {
            if !original.starts_with(&self.workspace) {
                c.args(["--ro-bind", "/dev/null"]).arg(original);
            }
        }
        if let Some(bundle) = bundle {
            c.arg("--tmpfs")
                .arg(base.join("tools"))
                .arg("--ro-bind")
                .arg(bundle)
                .arg(bundle);
            missing.extend([
                self.runtime.clone(),
                self.exporter.clone(),
                self.cli.clone(),
            ]);
        } else {
            c.arg("--ro-bind").arg(project).arg(project);
        }
        c.args(["--","/bin/sh","-c","n=$1; shift; while [ \"$n\" -gt 0 ]; do test ! -e \"$1\" || exit 91; shift; n=$((n-1)); done; n=$1; shift; while [ \"$n\" -gt 0 ]; do test ! -s \"$1\" || exit 92; shift; n=$((n-1)); done; export ORR_ROOM_WALL_NAMESPACE=verified; exec \"$@\"","sh"]).arg(missing.len().to_string()).args(&missing).arg(self.originals.len().to_string()).args(&self.originals).arg(executable).current_dir(base.join("empty")).env("HOME",base.join("home")).env("XDG_DATA_HOME",base.join("xdg-data")).env("XDG_CONFIG_HOME",base.join("xdg-config")).env("XDG_CACHE_HOME",base.join("xdg-cache")).env("XDG_RUNTIME_DIR",base.join("xdg-runtime")).env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
        c
    }
    fn runtime(
        &self,
        project: &Path,
        bundle: Option<&Path>,
        ticks: u32,
        held: &str,
        png: &Path,
    ) -> Command {
        let executable = bundle
            .map(|b| b.join("run-room-escape"))
            .unwrap_or_else(|| self.runtime.clone());
        let mut c = self.isolated(&executable, project, bundle);
        if bundle.is_none() {
            c.arg("--project").arg(project);
        }
        c.args(["--headless", "--ticks", &ticks.to_string(), "--capture"])
            .arg(png);
        if !held.is_empty() {
            c.args(["--hold", held]);
        }
        c
    }
    fn export(&self, output: &Path) -> Command {
        let mut c = self.isolated(&self.exporter, &self.projects[0], None);
        c.arg("--project")
            .arg(&self.projects[0])
            .arg("--runtime")
            .arg(&self.runtime)
            .args([
                "--runtime-sha256",
                &file_hash(&self.runtime),
                "--trusted-runtime",
                "--output",
            ])
            .arg(output);
        c
    }
    fn probe(
        &self,
        project: &Path,
        bundle: Option<&Path>,
        ticks: u32,
        held: &str,
        output: &Path,
    ) -> String {
        let mut c = self.isolated(&self.probe, project, bundle);
        c.args([
            "--ignored",
            "--exact",
            TEST,
            "--nocapture",
            "--test-threads=1",
        ])
        .env("ORR_ROOM_WALL_CHILD", "source-hidden-v1")
        .env("ORR_ROOM_WALL_CHILD_PROJECT", project)
        .env("ORR_ROOM_WALL_CHILD_TICKS", ticks.to_string())
        .env("ORR_ROOM_WALL_CHILD_HELD", held)
        .env("ORR_ROOM_WALL_CHILD_OUTPUT", output);
        good(bounded_output(&mut c))
    }
}
fn child_probe() {
    assert_eq!(
        std::env::var("ORR_ROOM_WALL_CHILD").as_deref(),
        Ok("source-hidden-v1")
    );
    assert_eq!(
        std::env::var("ORR_ROOM_WALL_NAMESPACE").as_deref(),
        Ok("verified")
    );
    let root = PathBuf::from(std::env::var_os("ORR_ROOM_WALL_CHILD_PROJECT").unwrap());
    let error = fs::write(root.join("must-not-write"), b"forbidden")
        .expect_err("read-only project required");
    assert_eq!(error.raw_os_error(), Some(30));
    println!("Room wall namespace: verified; project read-only");
    let ticks: u32 = std::env::var("ORR_ROOM_WALL_CHILD_TICKS")
        .unwrap()
        .parse()
        .unwrap();
    assert!(ticks <= 120);
    let held = std::env::var("ORR_ROOM_WALL_CHILD_HELD").unwrap();
    let prepared = PreparedProject::open(&root).unwrap();
    let frame = simulate(prepared.scene(), ticks, &held);
    let output = PathBuf::from(std::env::var_os("ORR_ROOM_WALL_CHILD_OUTPUT").unwrap());
    assert!(!output.exists());
    fs::write(output, frame.to_bytes()).unwrap();
    println!(
        "PreparedProject frame tick={ticks} checksum=0x{:016x}",
        frame.checksum()
    );
}

#[test]
#[ignore = "mandatory gate: fresh wall CLI, real production Room binaries, software GPU and source-hidden read-only namespace"]
fn room_wall_real_cli_source_hidden_room_export() {
    if std::env::var_os("ORR_ROOM_WALL_CHILD").is_some() {
        child_probe();
        return;
    }
    let w = Work::new();
    let base = &w.fixture.base;
    let root = &w.projects[0];
    let before = inventory(root);
    let controls = [inventory(&w.projects[1]), inventory(&w.projects[2])];
    let authoring_before = inventory(&base.join("authoring"));
    let prepared = PreparedProject::open(root).unwrap();
    let staged = base.join("staged-export");
    good(bounded_output(&mut w.export(&staged)));
    let repeated = base.join("repeated-export");
    good(bounded_output(&mut w.export(&repeated)));
    assert_eq!(inventory(&staged), inventory(&repeated));
    fs::remove_dir_all(repeated).unwrap();
    let bundle = base.join("relocated read-only room");
    fs::rename(staged, &bundle).unwrap();
    let bundle_before = inventory(&bundle);
    assert_eq!(
        file_hash(&bundle.join("bin/room_escape")),
        file_hash(&w.runtime)
    );
    let exported = inventory(&bundle.join("project"));
    for (path, entry) in &exported {
        assert_eq!(
            before.get(path),
            Some(entry),
            "export changed source bytes: {path}"
        );
    }
    for path in [
        "orr.project.json",
        "room.scene.yaml",
        "room.models.json",
        "room.camera.json",
        "orr.packages.lock.json",
    ] {
        assert_eq!(
            fs::read(bundle.join("project").join(path)).unwrap(),
            fs::read(root.join(path)).unwrap()
        );
    }
    let manifest: ExportManifest =
        serde_json::from_slice(&fs::read(bundle.join("orr.export.json")).unwrap()).unwrap();
    assert_eq!(manifest.schema, 1);
    assert_eq!(
        manifest.payload.profile,
        "room-escape-authored-linux-x86_64-v1"
    );
    let mut hash = Sha256::new();
    hash.update(b"orrery.room-escape.export.content.v1\0");
    hash.update(serde_json::to_vec(&manifest.payload).unwrap());
    assert_eq!(
        manifest.content_digest,
        hash.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    assert_eq!(manifest.payload.runtime.sha256, file_hash(&w.runtime));
    assert_eq!(
        manifest.payload.initial_checksum,
        format!("0x{:016x}", prepared.scene().frame().checksum())
    );
    assert_eq!(manifest.payload.packages.len(), 1);
    assert_eq!(manifest.payload.packages[PACKAGE], PACKAGE_DIGEST);
    let listed = manifest
        .payload
        .files
        .iter()
        .map(|f| f.path.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(listed.len(), manifest.payload.files.len());
    assert_eq!(
        listed,
        bundle_before
            .keys()
            .filter(|p| p.as_str() != "orr.export.json")
            .cloned()
            .collect()
    );
    for file in &manifest.payload.files {
        assert_eq!(bundle_before[&file.path], (file.bytes, file.sha256.clone()));
    }
    let reloaded = PreparedProject::open(bundle.join("project")).unwrap();
    assert_eq!(
        reloaded.scene().frame().to_bytes(),
        prepared.scene().frame().to_bytes()
    );
    assert_eq!(reloaded.models().document, prepared.models().document);
    assert_eq!(
        reloaded.camera().unwrap().bytes,
        prepared.camera().unwrap().bytes
    );
    let captures = base.join("captures");
    let mut observations = BTreeMap::new();
    let mut initial_pixels = Vec::new();
    let mut initial_frame = Vec::new();
    for (name, ticks, held) in STAGES {
        let expected = simulate(prepared.scene(), ticks, held);
        let source_png = captures.join(format!("runtime-{name}-source.png"));
        let export_png = captures.join(format!("runtime-{name}-export.png"));
        let source_stdout = good(bounded_output(&mut w.runtime(
            root,
            None,
            ticks,
            held,
            &source_png,
        )));
        let export_stdout = good(bounded_output(&mut w.runtime(
            &bundle.join("project"),
            Some(&bundle),
            ticks,
            held,
            &export_png,
        )));
        assert!(source_stdout.contains("room capture adapter:"));
        assert_eq!(source_stdout, export_stdout);
        assert!(source_stdout.contains(&format!(
            "room initial checksum: 0x{:016x}",
            prepared.scene().frame().checksum()
        )));
        assert!(source_stdout.contains(&format!(
            "room tick: {ticks} checksum: 0x{:016x}",
            expected.checksum()
        )));
        assert!(source_stdout.contains("room key: 0 won: 0"));
        assert_eq!(expected.singleton::<RoomRun>().key_collected, 0);
        assert_eq!(
            fs::read(&source_png).unwrap(),
            fs::read(&export_png).unwrap()
        );
        let image = pixels(&source_png);
        if name == "initial" {
            initial_pixels = image.clone();
            initial_frame = expected.to_bytes();
        }
        if name == "collision" {
            assert!(
                differences(&initial_pixels, &image) > 20,
                "wall-limited motion must be visible"
            );
        }
        if name == "restarted" {
            assert_eq!(image, initial_pixels);
            assert_eq!(expected.to_bytes(), initial_frame);
        }
        let source_frame = captures.join(format!("prepared-project-{name}-source.frame"));
        let export_frame = captures.join(format!("prepared-project-{name}-export.frame"));
        let source_probe = w.probe(root, None, ticks, held, &source_frame);
        let export_probe = w.probe(
            &bundle.join("project"),
            Some(&bundle),
            ticks,
            held,
            &export_frame,
        );
        for frame in [&source_frame, &export_frame] {
            assert_eq!(fs::read(frame).unwrap(), expected.to_bytes());
        }
        observations.insert(name,json!({"ticks":ticks,"held":held,"checksum":format!("0x{:016x}",expected.checksum()),"source_stdout":source_stdout,"export_stdout":export_stdout,"source_probe_stdout":source_probe,"export_probe_stdout":export_probe,"source_frame_sha256":file_hash(&source_frame),"export_frame_sha256":file_hash(&export_frame),"source_png_sha256":file_hash(&source_png),"export_png_sha256":file_hash(&export_png)}));
    }
    let mut control_observations = BTreeMap::new();
    for (name, project) in [
        ("baseline", &w.projects[1]),
        ("wrong-position", &w.projects[2]),
    ] {
        let control = PreparedProject::open(project).unwrap();
        for (stage, ticks, held) in [
            ("initial", 0, ""),
            ("collision", 120, "right"),
            ("interacted", 1, "interact"),
        ] {
            let expected = simulate(control.scene(), ticks, held);
            let png = captures.join(format!("control-{name}-{stage}.png"));
            let frame = captures.join(format!("control-{name}-{stage}.frame"));
            let stdout = good(bounded_output(
                &mut w.runtime(project, None, ticks, held, &png),
            ));
            let probe = w.probe(project, None, ticks, held, &frame);
            assert!(stdout.contains("room capture adapter:"));
            assert!(stdout.contains(&format!(
                "room tick: {ticks} checksum: 0x{:016x}",
                expected.checksum()
            )));
            assert_eq!(fs::read(&frame).unwrap(), expected.to_bytes());
            if stage == "interacted" {
                assert_eq!(expected.singleton::<RoomRun>().key_collected, 1);
                assert!(stdout.contains("room key: 1 won: 0"));
            }
            if stage == "initial" {
                assert!(
                    differences(&initial_pixels, &pixels(&png)) > 20,
                    "wall must appear in actual production runtime"
                );
            }
            control_observations.insert(format!("{name}-{stage}"),json!({"ticks":ticks,"held":held,"checksum":format!("0x{:016x}",expected.checksum()),"stdout":stdout,"probe_stdout":probe,"png_sha256":file_hash(&png),"frame_sha256":file_hash(&frame)}));
        }
    }
    let existing = captures.join("runtime-initial-source.png");
    let old = fs::read(&existing).unwrap();
    let capture_no_overwrite = bad(
        bounded_output(&mut w.runtime(root, None, 0, "", &existing)),
        "already exists",
    );
    assert_eq!(fs::read(existing).unwrap(), old);
    let runtime_read_only = bad(
        bounded_output(&mut w.runtime(
            &bundle.join("project"),
            Some(&bundle),
            0,
            "",
            &bundle.join("must-not-write.png"),
        )),
        "Read-only file system",
    );
    let exporter_read_only = bad(
        bounded_output(&mut w.export(&base.join("readonly/export"))),
        "Read-only file system",
    );
    let export_no_overwrite = bad(
        bounded_output(&mut w.export(&bundle)),
        "output destination already exists",
    );
    assert_eq!(inventory(&bundle), bundle_before);
    assert_eq!(inventory(root), before);
    assert_eq!(
        [inventory(&w.projects[1]), inventory(&w.projects[2])],
        controls
    );
    assert_eq!(inventory(&base.join("authoring")), authoring_before);
    assert_eq!(fs::read_dir(base.join("empty")).unwrap().count(), 0);
    // Publish evidence only after namespace consumers finish. No evidence copy
    // can provide an alternate asset path during source-hidden acceptance.
    for (path, bytes) in tree(&captures) {
        artifact(&w.evidence, path.to_str().unwrap(), &bytes);
    }
    for (name, project) in [
        ("authored-project", root),
        ("baseline-project", &w.projects[1]),
        ("wrong-position-project", &w.projects[2]),
    ] {
        copy_tree(project, &w.evidence.join(name));
    }
    for name in ["wall.ops.json", "wrong-position.ops.json"] {
        artifact(
            &w.evidence,
            name,
            &fs::read(base.join("authoring").join(name)).unwrap(),
        );
    }
    artifact(
        &w.evidence,
        "orr.export.json",
        &fs::read(bundle.join("orr.export.json")).unwrap(),
    );
    let negative_controls = json!({"capture-no-overwrite":capture_no_overwrite,"runtime-read-only":runtime_read_only,"exporter-read-only":exporter_read_only,"export-no-overwrite":export_no_overwrite});
    artifact(
        &w.evidence,
        "negative-controls.json",
        &serde_json::to_vec_pretty(&negative_controls).unwrap(),
    );
    let source_sha256 = json!({"test":digest(include_bytes!("room_wall_export.rs")),"cli":digest(include_bytes!("../../orr_remote/examples/room_wall.rs")),"authoring":digest(include_bytes!("../../orr_remote/examples/room_wall/authoring.rs"))});
    let producer_sha256 = json!({"room-wall-cli":file_hash(&w.cli),"room_escape":file_hash(&w.runtime),"orr_export_room":file_hash(&w.exporter),"export-test":file_hash(&w.probe)});
    let receipt = json!({"schema":1,"status":"passed","source_sha256":source_sha256,"producer_sha256":producer_sha256,"observations":observations,"controls":control_observations,"exported_file_inventory":bundle_before,"frame_evidence_boundary":"Fresh source-hidden read-only PreparedProject probes supply Frame bytes; production room_escape independently supplies PNG and checksums. Restart-labelled runs reopen the same admitted initial bytes, and ERP stop/start restart is tested in CPU acceptance."});
    artifact(
        &w.evidence,
        "production-receipt.json",
        &serde_json::to_vec_pretty(&receipt).unwrap(),
    );
    artifact(
        &w.evidence,
        "capture-inventory.json",
        &serde_json::to_vec_pretty(&inventory(&w.evidence)).unwrap(),
    );
}
