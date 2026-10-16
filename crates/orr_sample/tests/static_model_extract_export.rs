//! Real static-model authoring CLI -> package -> unchanged Room runtime/export.
//! Explicit, hosted Linux acceptance only; ignored inventory is not GPU proof.
//! Build the extractor with `-p orr_model --features import --example static_model_extract`
//! and Room tools with `-p orr_sample --features room-project,project-export`.
//! Set ORR_STATIC_MODEL_EXTRACT_BIN, ORR_ROOM_RUNTIME, ORR_ROOM_EXPORTER to
//! exact prebuilt executables, ORR_REQUIRE_GPU=1, ORR_REQUIRE_PROJECT_ISOLATION=1,
//! and ORR_STATIC_MODEL_EXTRACT_EVIDENCE to an existing empty evidence directory.
//! Run with `--features room-project,project-export --test static_model_extract_export
//! -- --ignored --exact static_model_extract_real_cli_source_hidden_room_export`.
#![cfg(all(
    feature = "room-project",
    feature = "project-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use orr_bridge::PlayerSlot;
use orr_ecs::Frame;
use orr_model::{import::import_with_resolver, StaticModel};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::{Project, Runtime};
use orr_reflect::Scene;
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, RoomInput, RoomRun, INTERACT, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::{Simulation, TickInputs};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    os::{fd::AsFd, unix::process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const TEST: &str = "static_model_extract_real_cli_source_hidden_room_export";
const ASSET_ID: &str = "models/scene.glb";
const ASSET: &str = "chair.model.json";
const PACKAGE: &str = "extracted-static-prop";
const STAGES: [(&str, u32, &str); 4] = [
    ("initial", 0, ""),
    ("moved", 18, "right"),
    ("collision", 180, "right"),
    // A fresh press occurs on tick 1: canonical player x=0 moves about 0.05
    // toward key x=1, already inside the unchanged 1.25 interaction distance.
    // Continuing to hold never creates another press or activates the exit.
    ("interacted", 18, "right,interact"),
];

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn file_hash(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn tree(root: &Path) -> BTreeMap<String, (u64, String)> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<String, (u64, String)>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(
                !kind.is_symlink(),
                "unexpected symlink: {}",
                entry.path().display()
            );
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

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(!kind.is_symlink());
        if kind.is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        } else {
            assert!(kind.is_file());
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

fn good(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn bad(output: Output, diagnostic: &str) {
    assert!(!output.status.success(), "unexpected success");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(diagnostic),
        "expected {diagnostic:?}: {stderr}"
    );
}

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

// Independently edit a genuine embedded GLB's hierarchy. Never call the authoring
// helper or construct its output model here. Source slot 3 precedes slot 2 in
// traversal; slot 1 is an omitted sibling. Only omitted geometry uses material 0.
fn scene_glb() -> Vec<u8> {
    let original = include_bytes!("../../orr_model/tests/fixtures/model.glb");
    let json_len = u32::from_le_bytes(original[12..16].try_into().unwrap()) as usize;
    assert_eq!(&original[..4], b"glTF");
    assert_eq!(u32::from_le_bytes(original[4..8].try_into().unwrap()), 2);
    let mut doc: Value = serde_json::from_slice(&original[20..20 + json_len]).unwrap();
    let bin_chunk = &original[20 + json_len..];
    assert_eq!(&bin_chunk[4..8], b"BIN\0");
    let mut panel = doc["meshes"][0]["primitives"][0].clone();
    panel["material"] = json!(1);
    let marker = doc["meshes"][0]["primitives"][1].clone();
    doc["meshes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"primitives":[panel]}));
    doc["meshes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"primitives":[marker]}));
    doc["scenes"] = json!([{"nodes":[0]}]);
    doc["scene"] = json!(0);
    doc["nodes"] = json!([
        {"name":"source ancestor", "matrix":[1.25,0,0,0, 0.25,1,0,0, 0,0,0.75,0, 0.5,0.25,0.125,1], "children":[3,1]},
        {"name":"omitted sibling", "mesh":0, "translation":[3,2,0]},
        {"name":"selected nested child", "mesh":2, "translation":[1.25,1,0], "scale":[0.5,0.5,1]},
        {"name":"selected prop root", "mesh":1, "translation":[-2,2,0], "scale":[1,1.5,1], "children":[2]}
    ]);
    let mut json_bytes = serde_json::to_vec(&doc).unwrap();
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(
        &u32::try_from(20 + json_bytes.len() + bin_chunk.len())
            .unwrap()
            .to_le_bytes(),
    );
    glb.extend_from_slice(&u32::try_from(json_bytes.len()).unwrap().to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(bin_chunk);
    glb
}

fn assert_extraction(selected: &StaticModel, full: &StaticModel, glb: &[u8]) {
    let source = selected.source();
    let all = full.source();
    let ids = [
        "models/scene.glb#node=3/mesh=1/primitive=0",
        "models/scene.glb#node=2/mesh=2/primitive=0",
    ];
    assert_eq!(all.primitives.len(), 4);
    assert_eq!(source.primitives.len(), 2);
    assert_eq!(source.asset_id, ASSET_ID);
    assert_eq!(source.dependencies, all.dependencies);
    assert_eq!(source.dependencies.len(), 1);
    assert_eq!(source.dependencies[0].uri, "$source");
    assert_eq!(source.dependencies[0].sha256, digest(glb));
    assert_eq!(source.materials, all.materials);
    assert_eq!(source.images, all.images);
    assert_eq!((source.materials.len(), source.images.len()), (2, 2));
    assert_eq!(source.materials[0].image, 0);
    assert_eq!(source.materials[1].image, 1);
    for (primitive, id) in source.primitives.iter().zip(ids) {
        assert_eq!(primitive.id, id);
        assert_eq!(
            primitive.material, 1,
            "unused lower source table slots must not be compacted"
        );
        assert_eq!(
            primitive,
            all.primitives.iter().find(|p| p.id == id).unwrap()
        );
    }
    assert_eq!(
        all.primitives
            .iter()
            .filter(|p| p.id.contains("#node=1/"))
            .count(),
        2
    );
    assert!(source.primitives.iter().all(|p| !p.id.contains("#node=1/")));
    // Literal world transforms distinguish retained ancestor shear/scale and
    // nested placement from local-only extraction or selected-root recentering.
    assert_eq!(
        source.primitives[0].transform,
        [
            [1.25, 0.0, 0.0, 0.0],
            [0.375, 1.5, 0.0, 0.0],
            [0.0, 0.0, 0.75, 0.0],
            [-1.5, 2.25, 0.125, 1.0],
        ]
    );
    assert_eq!(
        source.primitives[1].transform,
        [
            [0.625, 0.0, 0.0, 0.0],
            [0.1875, 0.75, 0.0, 0.0],
            [0.0, 0.0, 0.75, 0.0],
            [0.4375, 3.75, 0.125, 1.0],
        ]
    );
    assert_eq!(source.primitives[0].vertices[0].position, [-1.0, -1.0, 0.0]);
    assert_eq!(source.primitives[0].vertices[0].normal, [0.0, 0.0, 1.0]);
    assert_eq!(source.primitives[0].vertices[0].uv, [0.0, 1.0]);
    assert_eq!(source.primitives[0].vertices[1].uv, [1.0, 1.0]);
    assert_eq!(source.primitives[0].indices, [0, 1, 2, 0, 2, 3, 0, 3, 4]);
    assert!(source.primitives[1]
        .vertices
        .iter()
        .all(|v| v.uv == [0.25, 0.25]));
    assert_eq!(source.primitives[1].indices, [0, 1, 2]);
    let cooked = selected.to_bytes().unwrap();
    assert_eq!(StaticModel::from_bytes(&cooked).unwrap().source(), source);
}

fn input(held: &str) -> RoomInput {
    match held {
        "" => RoomInput::default(),
        "right" => RoomInput {
            move_x: 1,
            ..RoomInput::default()
        },
        "right,interact" => RoomInput {
            move_x: 1,
            buttons: INTERACT,
            ..RoomInput::default()
        },
        _ => panic!("unexpected fixed acceptance input"),
    }
}

fn simulate(scene: &PreparedScene, ticks: u32, held: &str) -> Frame {
    let mut sim = scene.simulation().unwrap();
    for tick in 1..=ticks {
        let mut samples = TickInputs::new(u64::from(tick), 1);
        samples.set_input(PlayerSlot(0), input(held));
        sim.step(&samples);
    }
    sim.frame().clone()
}

fn package(source: &Path, name: &str) {
    fs::write(
        source.join("orr.package.json"),
        serde_json::to_vec_pretty(&json!({
            "schema":1, "name":name, "version":"1.0.0", "engine":"*",
            "capabilities":["models"], "dependencies":{}, "files":[ASSET]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn project(root: &Path, source: &Path, package: &str, yaml: &str, hidden: bool) {
    fs::create_dir(root).unwrap();
    fs::write(root.join("room.scene.yaml"), yaml).unwrap();
    fs::write(
        root.join("orr.project.json"),
        serde_json::to_vec_pretty(&json!({
            "schema":2, "engine":"*", "entry":{
                "game":"room-escape-v1", "scene":"room.scene.yaml", "models":"room.models.json"
            }
        }))
        .unwrap(),
    )
    .unwrap();
    Project::open_for_install(root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[source.to_path_buf()])
        .unwrap();
    let admitted = PreparedScene::parse(yaml).unwrap();
    let player = admitted
        .frame()
        .entities()
        .find(|&entity| {
            admitted
                .frame()
                .get::<RoomActor>(entity)
                .is_some_and(|actor| actor.kind == PLAYER)
        })
        .unwrap();
    let loaded = model_bindings::load_asset(root, package, ASSET).unwrap();
    let binding = Binding::from_asset(
        package.into(),
        ASSET.into(),
        &loaded,
        LocalTransform {
            translation: if hidden { [1000.0; 3] } else { [0.0, 1.0, 0.0] },
            ..LocalTransform::default()
        },
    )
    .unwrap();
    let doc = Document {
        version: 2,
        scene: "room.scene.yaml".into(),
        project: ".".into(),
        bindings: [(admitted.index().guid(player).unwrap().to_string(), binding)].into(),
    };
    fs::write(
        root.join("room.models.json"),
        serde_json::to_vec_pretty(&doc).unwrap(),
    )
    .unwrap();
    assert_eq!(
        PreparedProject::open(root)
            .unwrap()
            .scene()
            .frame()
            .to_bytes(),
        admitted.frame().to_bytes()
    );
}

struct Work {
    temp: tempfile::TempDir,
    workspace: PathBuf,
    evidence: PathBuf,
    authoring: PathBuf,
    sources: PathBuf,
    projects: [PathBuf; 3],
    runtime: PathBuf,
    exporter: PathBuf,
    extractor: PathBuf,
    probe: PathBuf,
    originals: Vec<PathBuf>,
    cli_stdout: String,
    baseline: PreparedScene,
    glb: Vec<u8>,
    cooked: Vec<u8>,
}

impl Work {
    fn new() -> Self {
        for key in ["ORR_REQUIRE_GPU", "ORR_REQUIRE_PROJECT_ISOLATION"] {
            assert_eq!(
                std::env::var(key).as_deref(),
                Ok("1"),
                "explicit acceptance requires {key}=1"
            );
        }
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .unwrap();
        let evidence = PathBuf::from(
            std::env::var_os("ORR_STATIC_MODEL_EXTRACT_EVIDENCE")
                .expect("mandatory evidence directory"),
        );
        assert!(evidence.is_absolute() && evidence.is_dir());
        assert_eq!(
            evidence,
            evidence.canonicalize().unwrap(),
            "canonical evidence directory required"
        );
        assert!(!evidence.starts_with(&workspace));
        assert_eq!(
            fs::read_dir(&evidence).unwrap().count(),
            0,
            "evidence must start empty"
        );
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        assert!(
            !base.starts_with(&workspace),
            "fixture must survive whole-workspace hiding"
        );
        assert!(!evidence.starts_with(&base) && !base.starts_with(&evidence));
        for name in [
            "authoring",
            "sources",
            "tools",
            "probe",
            "empty",
            "captures",
            "readonly",
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
            ("ORR_STATIC_MODEL_EXTRACT_BIN", "static_model_extract"),
        ] {
            let original = PathBuf::from(
                std::env::var_os(key)
                    .unwrap_or_else(|| panic!("mandatory prebuilt producer {key}")),
            );
            assert!(
                original.is_absolute() && original.is_file(),
                "absolute executable required: {key}"
            );
            let original = original.canonicalize().unwrap();
            assert!(!original.starts_with(&base));
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
        let authoring = base.join("authoring");
        let sources = base.join("sources");
        let selected_source = sources.join("selected");
        let full_source = sources.join("whole");
        fs::create_dir(&selected_source).unwrap();
        fs::create_dir(&full_source).unwrap();
        let glb = scene_glb();
        fs::write(authoring.join("scene.glb"), &glb).unwrap();
        let full = import_with_resolver(ASSET_ID, &glb, |uri| {
            panic!("embedded fixture resolved {uri}")
        })
        .unwrap();
        let invoke = |output: &Path| {
            let mut command = Command::new(&copies[2]);
            command
                .arg("--input")
                .arg(authoring.join("scene.glb"))
                .args(["--asset-id", ASSET_ID, "--node", "3", "--output"])
                .arg(output)
                .current_dir(base.join("empty"));
            good(bounded_output(&mut command))
        };
        let cli_stdout = invoke(&selected_source.join(ASSET));
        let second = authoring.join("repeat.model.json");
        invoke(&second);
        let cooked = fs::read(selected_source.join(ASSET)).unwrap();
        assert_eq!(
            cooked,
            fs::read(&second).unwrap(),
            "real CLI output must be byte deterministic"
        );
        fs::remove_file(second).unwrap();
        assert_eq!(
            fs::read(authoring.join("scene.glb")).unwrap(),
            glb,
            "authoring input changed"
        );
        assert_extraction(&StaticModel::from_bytes(&cooked).unwrap(), &full, &glb);
        package(&selected_source, PACKAGE);
        fs::write(full_source.join(ASSET), full.to_bytes().unwrap()).unwrap();
        package(&full_source, "whole-scene-control");
        let sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        // No reflected scene edits: authoring changes only presentation assets.
        let yaml = Scene::unbake(&room_project::types(), sim.frame(), None)
            .unwrap()
            .to_yaml();
        let baseline = PreparedScene::parse(&yaml).unwrap();
        let projects = [
            base.join("selected project"),
            base.join("offscreen control"),
            base.join("whole control"),
        ];
        project(&projects[0], &selected_source, PACKAGE, &yaml, false);
        project(&projects[1], &selected_source, PACKAGE, &yaml, true);
        project(
            &projects[2],
            &full_source,
            "whole-scene-control",
            &yaml,
            false,
        );
        for root in &projects {
            let prepared = PreparedProject::open(root).unwrap();
            assert_eq!(
                prepared.scene().frame().to_bytes(),
                baseline.frame().to_bytes()
            );
            assert_eq!(
                fs::read(root.join("room.scene.yaml")).unwrap(),
                yaml.as_bytes()
            );
        }
        let installed = model_bindings::load_asset(&projects[0], PACKAGE, ASSET).unwrap();
        assert_eq!(
            installed.static_model().unwrap().to_bytes().unwrap(),
            cooked
        );
        Self {
            temp,
            workspace,
            evidence,
            authoring,
            sources,
            projects,
            runtime: copies[0].clone(),
            exporter: copies[1].clone(),
            extractor: copies[2].clone(),
            probe,
            originals,
            cli_stdout,
            baseline,
            glb,
            cooked,
        }
    }

    fn isolated(&self, executable: &Path, project: &Path, bundle: Option<&Path>) -> Command {
        let base = self.temp.path();
        let mut command = Command::new("bwrap");
        command
            .args([
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
            .arg(&self.authoring)
            .arg("--tmpfs")
            .arg(&self.sources)
            .arg("--ro-bind")
            .arg(base.join("readonly"))
            .arg(base.join("readonly"));
        let mut missing = vec![
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"),
            self.authoring.join("scene.glb"),
            self.sources.join("selected").join(ASSET),
            self.sources.join("whole").join(ASSET),
        ];
        for root in &self.projects {
            if bundle.is_some() || root != project {
                command.arg("--tmpfs").arg(root);
                missing.push(root.join("orr.project.json"));
            }
        }
        for original in &self.originals {
            if !original.starts_with(&self.workspace) {
                command.args(["--ro-bind", "/dev/null"]).arg(original);
            }
        }
        if let Some(bundle) = bundle {
            command
                .arg("--tmpfs")
                .arg(base.join("tools"))
                .arg("--ro-bind")
                .arg(bundle)
                .arg(bundle);
            missing.extend([
                self.runtime.clone(),
                self.exporter.clone(),
                self.extractor.clone(),
            ]);
        } else {
            command.arg("--ro-bind").arg(project).arg(project);
        }
        // Verify every namespace mask before ANY runtime/export/probe execution.
        command.args(["--", "/bin/sh", "-c",
            "n=$1; shift; while [ \"$n\" -gt 0 ]; do test ! -e \"$1\" || exit 91; shift; n=$((n-1)); done; n=$1; shift; while [ \"$n\" -gt 0 ]; do test ! -s \"$1\" || exit 92; shift; n=$((n-1)); done; export ORR_STATIC_MODEL_EXTRACT_NAMESPACE=verified; exec \"$@\"",
            "sh"])
            .arg(missing.len().to_string()).args(&missing)
            .arg(self.originals.len().to_string()).args(&self.originals)
            .arg(executable).current_dir(base.join("empty"))
            .env("HOME", base.join("home"))
            .env("XDG_DATA_HOME", base.join("xdg-data"))
            .env("XDG_CONFIG_HOME", base.join("xdg-config"))
            .env("XDG_CACHE_HOME", base.join("xdg-cache"))
            .env("XDG_RUNTIME_DIR", base.join("xdg-runtime"))
            .env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
        command
    }

    fn runtime(
        &self,
        project: &Path,
        bundle: Option<&Path>,
        ticks: u32,
        held: &str,
        capture: &Path,
    ) -> Command {
        let executable = bundle
            .map(|path| path.join("run-room-escape"))
            .unwrap_or_else(|| self.runtime.clone());
        let mut command = self.isolated(&executable, project, bundle);
        if bundle.is_none() {
            command.arg("--project").arg(project);
        }
        command
            .args(["--headless", "--ticks", &ticks.to_string(), "--capture"])
            .arg(capture);
        if !held.is_empty() {
            command.args(["--hold", held]);
        }
        command
    }

    fn export(&self, output: &Path) -> Command {
        let mut command = self.isolated(&self.exporter, &self.projects[0], None);
        command
            .arg("--project")
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
        command
    }

    fn probe(
        &self,
        project: &Path,
        bundle: Option<&Path>,
        ticks: u32,
        held: &str,
        output: &Path,
    ) -> String {
        let mut command = self.isolated(&self.probe, project, bundle);
        command
            .args(["--ignored", "--exact", TEST, "--nocapture"])
            .env("ORR_STATIC_MODEL_EXTRACT_CHILD", "source-hidden-v1")
            .env("ORR_STATIC_MODEL_EXTRACT_CHILD_PROJECT", project)
            .env("ORR_STATIC_MODEL_EXTRACT_CHILD_TICKS", ticks.to_string())
            .env("ORR_STATIC_MODEL_EXTRACT_CHILD_HELD", held)
            .env("ORR_STATIC_MODEL_EXTRACT_CHILD_OUTPUT", output);
        good(bounded_output(&mut command))
    }
}

// Fresh process using the current production PreparedProject/Simulation route.
// This is separately labelled Frame evidence, not invented room_escape output.
fn child_probe() {
    assert_eq!(
        std::env::var("ORR_STATIC_MODEL_EXTRACT_CHILD").as_deref(),
        Ok("source-hidden-v1")
    );
    assert_eq!(
        std::env::var("ORR_STATIC_MODEL_EXTRACT_NAMESPACE").as_deref(),
        Ok("verified")
    );
    let root = PathBuf::from(std::env::var_os("ORR_STATIC_MODEL_EXTRACT_CHILD_PROJECT").unwrap());
    let error = fs::write(root.join("probe-must-not-write"), b"forbidden")
        .expect_err("project must be read-only");
    assert_eq!(
        error.raw_os_error(),
        Some(30),
        "read-only filesystem required"
    );
    let ticks: u32 = std::env::var("ORR_STATIC_MODEL_EXTRACT_CHILD_TICKS")
        .unwrap()
        .parse()
        .unwrap();
    assert!(ticks <= 180);
    let held = std::env::var("ORR_STATIC_MODEL_EXTRACT_CHILD_HELD").unwrap();
    let prepared = PreparedProject::open(root).unwrap();
    let frame = simulate(prepared.scene(), ticks, &held);
    let output = PathBuf::from(std::env::var_os("ORR_STATIC_MODEL_EXTRACT_CHILD_OUTPUT").unwrap());
    assert!(!output.exists());
    fs::write(output, frame.to_bytes()).unwrap();
    println!(
        "PreparedProject frame tick={ticks} checksum=0x{:016x}",
        frame.checksum()
    );
}

fn pixels(path: &Path) -> Vec<u8> {
    let mut reader = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap()))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(
        (info.width, info.height, info.color_type, info.bit_depth),
        (1024, 768, png::ColorType::Rgba, png::BitDepth::Eight)
    );
    pixels.truncate(info.buffer_size());
    assert_eq!(pixels.len(), 1024 * 768 * 4);
    assert!(
        pixels
            .chunks_exact(4)
            .filter(|pixel| *pixel != &pixels[..4])
            .count()
            > 100,
        "blank GPU capture"
    );
    pixels
}

fn changed(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

#[test]
#[ignore = "mandatory exact authoring CLI, production Room tools, real GPU and source-hidden read-only Linux namespace"]
fn static_model_extract_real_cli_source_hidden_room_export() {
    if std::env::var_os("ORR_STATIC_MODEL_EXTRACT_CHILD").is_some() {
        child_probe();
        return;
    }
    let work = Work::new();
    let root = &work.projects[0];
    let source_before = tree(root);
    let authoring_before = tree(&work.authoring);
    let packages_before = tree(&work.sources);
    let controls_before = [tree(&work.projects[1]), tree(&work.projects[2])];
    let staged = work.temp.path().join("staged export");
    good(bounded_output(&mut work.export(&staged)));
    let second = work.temp.path().join("second export");
    good(bounded_output(&mut work.export(&second)));
    assert_eq!(
        tree(&staged),
        tree(&second),
        "all exported bytes must be reproducible"
    );
    fs::remove_dir_all(second).unwrap();
    let bundle = work.temp.path().join("relocated read-only room");
    fs::rename(staged, &bundle).unwrap();
    let bundle_before = tree(&bundle);
    assert_eq!(
        file_hash(&bundle.join("bin/room_escape")),
        file_hash(&work.runtime)
    );
    let exported_project = tree(&bundle.join("project"));
    for (path, bytes) in &exported_project {
        assert_eq!(
            source_before.get(path),
            Some(bytes),
            "export changed exact admitted bytes: {path}"
        );
    }
    for path in [
        "orr.project.json",
        "room.scene.yaml",
        "room.models.json",
        "orr.packages.lock.json",
    ] {
        assert_eq!(
            fs::read(bundle.join("project").join(path)).unwrap(),
            fs::read(root.join(path)).unwrap()
        );
    }
    let exported_models: Vec<_> = exported_project
        .keys()
        .filter(|path| path.ends_with(".model.json"))
        .collect();
    assert_eq!(
        exported_models.len(),
        1,
        "export must carry only the selected cooked runtime asset"
    );
    assert_eq!(
        fs::read(bundle.join("project").join(exported_models[0])).unwrap(),
        work.cooked
    );
    assert!(exported_project
        .keys()
        .any(|path| path.ends_with("orr.package.json")));
    assert!(!exported_project
        .keys()
        .any(|path| path.ends_with(".glb") || path.contains("whole-scene-control")));
    let capture = work.temp.path().join("captures");
    let baseline_run = work.baseline.frame().singleton::<RoomRun>();
    assert_eq!(baseline_run.previous_buttons, 0);
    assert_eq!(baseline_run.key_collected, 0);
    let first_press = simulate(&work.baseline, 1, "right,interact");
    assert_eq!(first_press.singleton::<RoomRun>().key_collected, 1);
    assert_eq!(first_press.singleton::<RoomRun>().won, 0);
    let mut first_pixels = Vec::new();
    let mut observations = BTreeMap::new();
    for (name, ticks, held) in STAGES {
        let expected = simulate(&work.baseline, ticks, held);
        let source_png = capture.join(format!("runtime-{name}-source.png"));
        let export_png = capture.join(format!("runtime-{name}-export.png"));
        let source_stdout = good(bounded_output(&mut work.runtime(
            root,
            None,
            ticks,
            held,
            &source_png,
        )));
        let export_stdout = good(bounded_output(&mut work.runtime(
            &bundle.join("project"),
            Some(&bundle),
            ticks,
            held,
            &export_png,
        )));
        assert!(
            source_stdout.contains("room capture adapter:"),
            "actual production GPU required"
        );
        assert_eq!(
            source_stdout, export_stdout,
            "runtime checksum/state/adapter parity: {name}"
        );
        assert!(source_stdout.contains(&format!(
            "room initial checksum: 0x{:016x}",
            work.baseline.frame().checksum()
        )));
        assert!(source_stdout.contains(&format!(
            "room tick: {ticks} checksum: 0x{:016x}",
            expected.checksum()
        )));
        assert_eq!(
            fs::read(&source_png).unwrap(),
            fs::read(&export_png).unwrap(),
            "exact source/export PNG bytes: {name}"
        );
        let image = pixels(&source_png);
        if ticks == 0 {
            first_pixels = image;
        } else {
            assert!(
                changed(&first_pixels, &image) > 20,
                "scripted input must visibly change presentation: {name}"
            );
        }
        if name == "interacted" {
            assert!(
                source_stdout.contains("room key: 1 won: 0"),
                "canonical Room interaction failed"
            );
            assert_eq!(expected.singleton::<RoomRun>().key_collected, 1);
            assert_eq!(expected.singleton::<RoomRun>().won, 0);
        }
        let source_frame = capture.join(format!("prepared-project-{name}-source.frame"));
        let export_frame = capture.join(format!("prepared-project-{name}-export.frame"));
        let source_probe = work.probe(root, None, ticks, held, &source_frame);
        let export_probe = work.probe(
            &bundle.join("project"),
            Some(&bundle),
            ticks,
            held,
            &export_frame,
        );
        for path in [&source_frame, &export_frame] {
            assert_eq!(
                fs::read(path).unwrap(),
                expected.to_bytes(),
                "fresh source-hidden PreparedProject Frame bytes: {name}"
            );
        }
        observations.insert(name, json!({
            "ticks":ticks, "held":held, "checksum":format!("0x{:016x}", expected.checksum()),
            "source_stdout":source_stdout, "export_stdout":export_stdout,
            "source_probe_stdout":source_probe, "export_probe_stdout":export_probe,
            "source_frame_sha256":file_hash(&source_frame), "export_frame_sha256":file_hash(&export_frame),
            "source_png_sha256":file_hash(&source_png), "export_png_sha256":file_hash(&export_png)
        }));
    }
    // Both controls use the same scene/Frame and current production runtime.
    // The first establishes selected geometry is visible; the second proves the
    // omitted sibling was visibly present in the full scene and is now absent.
    let no_model = capture.join("control-no-model.png");
    let whole_scene = capture.join("control-whole-scene.png");
    let mut control_stdout = Vec::new();
    for (control, png) in [
        (&work.projects[1], &no_model),
        (&work.projects[2], &whole_scene),
    ] {
        let report = good(bounded_output(&mut work.runtime(control, None, 0, "", png)));
        assert!(report.contains("room capture adapter:"));
        assert!(report.contains(&format!(
            "room tick: 0 checksum: 0x{:016x}",
            work.baseline.frame().checksum()
        )));
        control_stdout.push(report);
    }
    let selected_changed_pixels = changed(&first_pixels, &pixels(&no_model));
    let omitted_changed_pixels = changed(&first_pixels, &pixels(&whole_scene));
    assert!(
        selected_changed_pixels > 20,
        "selected prop contributed no visible runtime pixels"
    );
    assert!(
        omitted_changed_pixels > 20,
        "omitted sibling had no positive visible runtime evidence"
    );
    let existing_png = capture.join("runtime-initial-source.png");
    let existing_bytes = fs::read(&existing_png).unwrap();
    bad(
        bounded_output(&mut work.runtime(root, None, 0, "", &existing_png)),
        "already exists",
    );
    assert_eq!(fs::read(existing_png).unwrap(), existing_bytes);
    bad(
        bounded_output(&mut work.runtime(
            &bundle.join("project"),
            Some(&bundle),
            0,
            "",
            &bundle.join("must-not-write.png"),
        )),
        "Read-only file system",
    );
    let readonly_export = work.temp.path().join("readonly/export");
    bad(
        bounded_output(&mut work.export(&readonly_export)),
        "Read-only file system",
    );
    assert!(!readonly_export.exists());
    bad(
        bounded_output(&mut work.export(&bundle)),
        "output destination already exists",
    );
    assert_eq!(
        tree(&bundle),
        bundle_before,
        "relocated read-only bundle changed"
    );
    assert_eq!(tree(root), source_before, "selected project changed");
    assert_eq!(
        tree(&work.authoring),
        authoring_before,
        "GLB authoring input changed"
    );
    assert_eq!(
        tree(&work.sources),
        packages_before,
        "source package changed"
    );
    assert_eq!(
        [tree(&work.projects[1]), tree(&work.projects[2])],
        controls_before
    );
    assert_eq!(
        fs::read_dir(work.temp.path().join("empty"))
            .unwrap()
            .count(),
        0
    );
    // Publish evidence only after every isolated consumer has finished; evidence
    // itself must never provide an unhidden alternate asset during acceptance.
    copy_tree(&capture, &work.evidence);
    copy_tree(root, &work.evidence.join("authored-project"));
    fs::copy(
        bundle.join("orr.export.json"),
        work.evidence.join("orr.export.json"),
    )
    .unwrap();
    fs::write(work.evidence.join("scene.glb"), &work.glb).unwrap();
    fs::write(work.evidence.join(ASSET), &work.cooked).unwrap();
    fs::write(work.evidence.join("acceptance-source.json"), serde_json::to_vec_pretty(&json!({
        "test":TEST, "test_source_sha256":digest(include_bytes!("static_model_extract_export.rs")),
        "asset_id":ASSET_ID, "node":3, "source_sha256":digest(&work.glb), "cooked_sha256":digest(&work.cooked),
        "selected_primitive_ids":["models/scene.glb#node=3/mesh=1/primitive=0","models/scene.glb#node=2/mesh=2/primitive=0"],
        "selected_primitives":2, "full_scene_primitives":4, "retained_materials":2, "retained_images":2,
        "unreferenced_material_slot_retained":0, "original_source_digest_retained":true,
        "initial_frame_sha256":digest(&work.baseline.frame().to_bytes()), "cli_stdout":work.cli_stdout
    })).unwrap()).unwrap();
    fs::write(work.evidence.join("production-receipt.json"), serde_json::to_vec_pretty(&json!({
        "runtime_sha256":file_hash(&work.runtime), "exporter_sha256":file_hash(&work.exporter),
        "extractor_sha256":file_hash(&work.extractor), "acceptance_executable_sha256":file_hash(&work.probe),
        "source_hidden":true, "read_only_bundle":true, "observations":observations,
        "selected_changed_pixels":selected_changed_pixels, "omitted_changed_pixels":omitted_changed_pixels,
        "control_stdout":control_stdout, "exported_file_inventory":bundle_before,
        "frame_evidence_boundary":"Frame bytes are from fresh PreparedProject probes of the same read-only source/export bytes; room_escape independently supplies PNG and checksum evidence"
    })).unwrap()).unwrap();
    fs::write(
        work.evidence.join("capture-inventory.json"),
        serde_json::to_vec_pretty(&tree(&work.evidence)).unwrap(),
    )
    .unwrap();
    eprintln!("Verified real CLI extraction, selected/omitted GPU content, four exact source/export PNG pairs and eight fresh Frame probes");
}
