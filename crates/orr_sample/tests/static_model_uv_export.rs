//! Real uv CLI -> two cooked variants -> unchanged Room runtime/export.
//! Explicit, hosted Linux acceptance only; ignored inventory is not GPU proof.
//! Build the uv CLI with `-p orr_model --features import --example static_model_uv`
//! and Room tools with `-p orr_sample --features room-project,project-export`.
//! Set ORR_STATIC_MODEL_UV_BIN, ORR_ROOM_RUNTIME, ORR_ROOM_EXPORTER to
//! exact prebuilt executables, ORR_REQUIRE_GPU=1, ORR_REQUIRE_PROJECT_ISOLATION=1,
//! and ORR_STATIC_MODEL_UV_EVIDENCE to an existing empty evidence directory.
//! Run with `--features room-project,project-export --test static_model_uv_export
//! -- --ignored --exact static_model_uv_real_cli_source_hidden_room_export`.
#![cfg(all(
    feature = "room-project",
    feature = "project-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use orr_bridge::PlayerSlot;
use orr_ecs::Frame;
use orr_model::{import::import_with_resolver, Dependency, StaticModel};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::{Project, Runtime};
use orr_reflect::Scene;
use orr_sample::{
    project_export::ExportManifest,
    room_camera,
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, RoomInput, RoomRun, INTERACT, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::{Simulation, TickInputs};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    os::{fd::AsFd, unix::process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const TEST: &str = "static_model_uv_real_cli_source_hidden_room_export";
const SOURCE_ID: &str = "models/scene.glb";
const SOURCE_PATH: &str = "inputs/source.model.json";
const PACKAGE: &str = "uv-static-props";
const ORIGINAL_PACKAGE: &str = "original-uv-control";
const ORIGINAL_ASSET: &str = "original.model.json";
const VARIANTS: [(&str, &str, u32); 2] = [
    ("rotate", "variants/rotate.uv.json", 1),
    ("counter", "variants/counter.uv.json", 3),
];
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

// Independently edit source material settings, retaining the genuine embedded
// GLB's transformed geometry, primitive slots and original image tables.
fn scene_glb() -> Vec<u8> {
    let original = include_bytes!("../../orr_model/tests/fixtures/model.glb");
    let json_len = u32::from_le_bytes(original[12..16].try_into().unwrap()) as usize;
    assert_eq!(&original[..4], b"glTF");
    assert_eq!(u32::from_le_bytes(original[4..8].try_into().unwrap()), 2);
    let mut doc: Value = serde_json::from_slice(&original[20..20 + json_len]).unwrap();
    let bin_chunk = &original[20 + json_len..];
    assert_eq!(&bin_chunk[4..8], b"BIN\0");
    doc["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] = json!([0.75, 0.5, 1.0, 0.5]);
    doc["samplers"][0]["wrapS"] = json!(33648);
    doc["samplers"][0]["wrapT"] = json!(10497);
    // The unselected material shares the image: a texture-wide UV rewrite is wrong.
    doc["materials"][1]["pbrMetallicRoughness"]["baseColorTexture"] =
        doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"].clone();
    // A second source node shares both material slots. UV authoring one slot
    // must affect every user while preserving the other slot and node transform.
    doc["nodes"][0]["children"] = json!([1, 2]);
    doc["nodes"].as_array_mut().unwrap().push(json!({
        "name":"shared material sibling", "mesh":0,
        "translation":[0,2,0.2], "scale":[0.5,0.5,1]
    }));
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

// Independent full-source oracle. Expanded affine expressions are hand-derived:
// rotate: scale (1.5,0.5), turn +90, offset (0.25,0.125).
// counter: scale (0.5,1.5), turn +270, offset (-0.375,0.75).
fn assert_uv(
    variant: &StaticModel,
    original: &StaticModel,
    recipe: &[u8],
    asset_id: &str,
    quarter_turns: u32,
) {
    let source_bytes = original.to_bytes().unwrap();
    let mut expected = original.source().clone();
    assert_eq!(
        (
            expected.primitives.len(),
            expected.materials.len(),
            expected.images.len()
        ),
        (4, 2, 2)
    );
    for material in [0, 1] {
        assert_eq!(
            expected
                .primitives
                .iter()
                .filter(|p| p.material == material)
                .count(),
            2
        );
    }
    assert_eq!(expected.materials[0].image, expected.materials[1].image);
    assert_eq!(expected.materials[0].base_color, [0.75, 0.5, 1.0, 0.5]);
    assert!(!expected.materials[0].linear_filter);
    assert_eq!(expected.materials[0].wrap_s, orr_model::Wrap::Mirror);
    assert_eq!(expected.materials[0].wrap_t, orr_model::Wrap::Repeat);
    expected.asset_id = asset_id.into();
    for primitive in &mut expected.primitives {
        let suffix = primitive.id.strip_prefix(SOURCE_ID).unwrap();
        assert!(suffix.starts_with("#node="));
        primitive.id = format!("{asset_id}{suffix}");
        if primitive.material == 0 {
            for vertex in &mut primitive.vertices {
                let [u, v] = vertex.uv;
                vertex.uv = match quarter_turns {
                    1 => [0.25 - 0.5 * v, 0.125 + 1.5 * u],
                    3 => [-0.375 + 1.5 * v, 0.75 - 0.5 * u],
                    _ => panic!("unexpected fixed acceptance transform"),
                };
            }
        }
    }
    expected.dependencies = [("$source", recipe), (SOURCE_PATH, source_bytes.as_slice())]
        .into_iter()
        .map(|(uri, bytes)| Dependency {
            uri: uri.into(),
            sha256: digest(bytes),
        })
        .collect();
    assert_eq!(variant.source(), &expected);
    assert_eq!(
        StaticModel::from_bytes(&variant.to_bytes().unwrap())
            .unwrap()
            .source(),
        &expected
    );
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

fn package(source: &Path, name: &str, files: &[String]) {
    fs::write(
        source.join("orr.package.json"),
        serde_json::to_vec_pretty(&json!({
            "schema":1, "name":name, "version":"1.0.0", "engine":"*",
            "capabilities":["models"], "dependencies":{}, "files":files
        }))
        .unwrap(),
    )
    .unwrap();
}

fn project(
    root: &Path,
    source: &Path,
    package: &str,
    assets: &[String; 2],
    yaml: &str,
    hidden: bool,
) {
    fs::create_dir(root).unwrap();
    fs::write(root.join("room.scene.yaml"), yaml).unwrap();
    let camera = room_camera::Document {
        target: [0.25, 0.75, 0.0],
        yaw: 0.5,
        pitch: 0.7,
        ..room_camera::Document::readable_default()
    };
    fs::write(root.join("room.camera.json"), camera.to_bytes().unwrap()).unwrap();
    fs::write(
        root.join("orr.project.json"),
        serde_json::to_vec_pretty(&json!({
            "schema":2, "engine":"*", "entry":{
                "game":"room-escape-v1", "scene":"room.scene.yaml", "models":"room.models.json",
                "camera":"room.camera.json"
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
    let mut bindings = BTreeMap::new();
    for (index, kind) in [PLAYER, KEY].into_iter().enumerate() {
        let entity = admitted
            .frame()
            .entities()
            .find(|&entity| {
                admitted
                    .frame()
                    .get::<RoomActor>(entity)
                    .is_some_and(|actor| actor.kind == kind)
            })
            .unwrap();
        let loaded = model_bindings::load_asset(root, package, &assets[index]).unwrap();
        let binding = Binding::from_asset(
            package.into(),
            assets[index].clone(),
            &loaded,
            LocalTransform {
                translation: if hidden {
                    [1000.0; 3]
                } else {
                    [if index == 0 { -2.0 } else { 2.0 }, 1.0, 0.0]
                },
                ..LocalTransform::default()
            },
        )
        .unwrap();
        bindings.insert(admitted.index().guid(entity).unwrap().to_string(), binding);
    }
    let doc = Document {
        version: 2,
        scene: "room.scene.yaml".into(),
        project: ".".into(),
        bindings,
    };
    fs::write(
        root.join("room.models.json"),
        serde_json::to_vec_pretty(&doc).unwrap(),
    )
    .unwrap();
    let prepared = PreparedProject::open(root).unwrap();
    assert_eq!(
        prepared.scene().frame().to_bytes(),
        admitted.frame().to_bytes()
    );
    assert_eq!(prepared.camera().unwrap().document, camera);
}

// Arm before any setup or producer invocation, including Work::new failures.
// On ordinary success TempDir retains its normal cleanup. On failed/incomplete
// unwind keep the entire fixture, then copy only existing captures for hosted
// upload. All nested BoundedChild guards have unwound before this guard runs;
// nothing is copied into evidence while a namespace consumer is active.
// SIGKILL/abort cannot run Drop: the fixture remains on disk, with its path in
// the test log, but capture-copy/marker publication is necessarily best-effort.
struct RetainedFixture {
    temp: Option<tempfile::TempDir>,
    evidence: PathBuf,
    complete: bool,
}
impl RetainedFixture {
    fn new(temp: tempfile::TempDir, evidence: PathBuf) -> Self {
        let guard = Self {
            temp: Some(temp),
            evidence,
            complete: false,
        };
        let _ = writeln!(
            std::io::stderr().lock(),
            "uv export fixture: {}",
            guard.path().display()
        );
        guard
    }
    fn path(&self) -> &Path {
        self.temp.as_ref().expect("live fixture ownership").path()
    }
    fn complete(&mut self) {
        self.complete = true;
    }
}

// Failure salvage must not panic and replace the original assertion. The full
// retained fixture is authoritative even when a partial-copy or I/O error occurs.
fn copy_failed_captures(from: &Path, to: &Path) -> Result<usize, String> {
    let metadata = match fs::symlink_metadata(from) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("capture metadata: {error}")),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("capture root is not a regular directory".into());
    }
    fs::create_dir(to).map_err(|error| format!("create failure capture directory: {error}"))?;
    let mut copied = 0;
    for entry in fs::read_dir(from).map_err(|error| format!("list failed captures: {error}"))? {
        let entry = entry.map_err(|error| format!("read failed capture entry: {error}"))?;
        if !entry
            .file_type()
            .map_err(|error| format!("failed capture kind: {error}"))?
            .is_file()
        {
            return Err("failed capture entry is not a regular file".into());
        }
        let mut input = fs::File::open(entry.path())
            .map_err(|error| format!("open failed capture: {error}"))?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(to.join(entry.file_name()))
            .map_err(|error| format!("create failed capture: {error}"))?;
        std::io::copy(&mut input, &mut output)
            .map_err(|error| format!("copy failed capture: {error}"))?;
        copied += 1;
    }
    Ok(copied)
}
impl Drop for RetainedFixture {
    fn drop(&mut self) {
        if self.complete {
            return;
        }
        let Some(temp) = self.temp.take() else {
            return;
        };
        // keep() disables TempDir deletion before any fallible salvage operation.
        let retained = temp.keep();
        let copy = copy_failed_captures(
            &retained.join("captures"),
            &self.evidence.join("failure-captures"),
        );
        // If an evidence-publication I/O error occurred, incomplete receipts are
        // retained under explicit names and must never masquerade as a pass.
        let mut quarantine_errors = Vec::new();
        for name in [
            "acceptance-source.json",
            "production-receipt.json",
            "capture-inventory.json",
        ] {
            let source = self.evidence.join(name);
            let destination = self.evidence.join(format!("incomplete-{name}"));
            match fs::symlink_metadata(&source) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => quarantine_errors.push(format!("{name}: {error}")),
                Ok(_) => {
                    if fs::symlink_metadata(&destination).is_ok() {
                        quarantine_errors.push(format!("incomplete-{name} already exists"));
                    } else if let Err(error) = fs::rename(source, destination) {
                        quarantine_errors.push(format!("{name}: {error}"));
                    }
                }
            }
        }
        let _ = writeln!(std::io::stderr().lock(), "FAILED/INCOMPLETE uv export fixture retained: {}; capture salvage: {copy:?}; receipt quarantine: {quarantine_errors:?}", retained.display());
        let marker = serde_json::to_vec_pretty(&json!({
            "schema":1, "status":"failed-or-incomplete", "retained_fixture":retained.to_string_lossy(),
            "capture_directory":"failure-captures", "captures_copied":copy.as_ref().ok(),
            "capture_copy_error":copy.as_ref().err(), "receipt_quarantine_errors":quarantine_errors
        }));
        if let Ok(bytes) = marker {
            if let Ok(mut file) = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.evidence.join("failure-retention.json"))
            {
                let _ = file.write_all(&bytes);
            }
        }
    }
}

struct Work {
    temp: RetainedFixture,
    workspace: PathBuf,
    evidence: PathBuf,
    authoring: PathBuf,
    sources: PathBuf,
    projects: [PathBuf; 3],
    runtime: PathBuf,
    exporter: PathBuf,
    uv_cli: PathBuf,
    probe: PathBuf,
    originals: Vec<PathBuf>,
    cli_stdout: Vec<String>,
    baseline: PreparedScene,
    glb: Vec<u8>,
    source_cooked: Vec<u8>,
    recipes: Vec<Vec<u8>>,
    cooked: Vec<Vec<u8>>,
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
            std::env::var_os("ORR_STATIC_MODEL_UV_EVIDENCE").expect("mandatory evidence directory"),
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
        let temp = RetainedFixture::new(tempfile::tempdir().unwrap(), evidence.clone());
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
            ("ORR_STATIC_MODEL_UV_BIN", "static_model_uv"),
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
        let variant_source = sources.join("variants");
        let original_source = sources.join("original");
        fs::create_dir(&variant_source).unwrap();
        fs::create_dir(&original_source).unwrap();
        fs::create_dir(authoring.join("inputs")).unwrap();
        let glb = scene_glb();
        fs::write(authoring.join("scene.glb"), &glb).unwrap();
        let original = import_with_resolver(SOURCE_ID, &glb, |uri| {
            panic!("embedded fixture resolved {uri}")
        })
        .unwrap();
        let source_cooked = original.to_bytes().unwrap();
        fs::write(authoring.join(SOURCE_PATH), &source_cooked).unwrap();
        let mut cli_stdout = Vec::new();
        let mut recipes = Vec::new();
        let mut cooked = Vec::new();
        for (name, asset_id, quarter_turns) in VARIANTS {
            let recipe = serde_json::to_vec_pretty(&json!({
                "format":"orr_static_material_uv", "version":1,"source":SOURCE_PATH,"material":0,
                "offset":if quarter_turns == 1 { [0.25,0.125] } else { [-0.375,0.75] },
                "scale":if quarter_turns == 1 { [1.5,0.5] } else { [0.5,1.5] },
                "quarter_turns":quarter_turns
            }))
            .unwrap();
            let recipe_path = authoring.join(format!("{name}.uv.json"));
            fs::write(&recipe_path, &recipe).unwrap();
            let invoke = |output: &Path| {
                let mut command = Command::new(&copies[2]);
                command
                    .arg("--recipe")
                    .arg(&recipe_path)
                    .args(["--asset-id", asset_id, "--output"])
                    .arg(output)
                    .current_dir(base.join("empty"));
                good(bounded_output(&mut command))
            };
            let asset = format!("{name}.model.json");
            cli_stdout.push(invoke(&variant_source.join(&asset)));
            let second = authoring.join(format!("repeat-{asset}"));
            invoke(&second);
            let bytes = fs::read(variant_source.join(&asset)).unwrap();
            assert_eq!(
                bytes,
                fs::read(&second).unwrap(),
                "real CLI output must be byte deterministic"
            );
            fs::remove_file(second).unwrap();
            assert_uv(
                &StaticModel::from_bytes(&bytes).unwrap(),
                &original,
                &recipe,
                asset_id,
                quarter_turns,
            );
            assert_eq!(fs::read(&recipe_path).unwrap(), recipe);
            recipes.push(recipe);
            cooked.push(bytes);
        }
        assert_eq!(fs::read(authoring.join("scene.glb")).unwrap(), glb);
        assert_eq!(
            fs::read(authoring.join(SOURCE_PATH)).unwrap(),
            source_cooked
        );
        let assets = VARIANTS.map(|(name, _, _)| format!("{name}.model.json"));
        package(&variant_source, PACKAGE, &assets);
        fs::write(original_source.join(ORIGINAL_ASSET), &source_cooked).unwrap();
        package(&original_source, ORIGINAL_PACKAGE, &[ORIGINAL_ASSET.into()]);
        let sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        // UV authoring never rewrites authored simulation values or scene metadata.
        let yaml = Scene::unbake(&room_project::types(), sim.frame(), None)
            .unwrap()
            .to_yaml();
        let baseline = PreparedScene::parse(&yaml).unwrap();
        let projects = [
            base.join("variant project"),
            base.join("offscreen control"),
            base.join("original control"),
        ];
        project(
            &projects[0],
            &variant_source,
            PACKAGE,
            &assets,
            &yaml,
            false,
        );
        project(&projects[1], &variant_source, PACKAGE, &assets, &yaml, true);
        project(
            &projects[2],
            &original_source,
            ORIGINAL_PACKAGE,
            &[ORIGINAL_ASSET.into(), ORIGINAL_ASSET.into()],
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
        for (asset, bytes) in assets.iter().zip(&cooked) {
            let installed = model_bindings::load_asset(&projects[0], PACKAGE, asset).unwrap();
            assert_eq!(
                installed.static_model().unwrap().to_bytes().unwrap(),
                *bytes
            );
        }
        Self {
            temp,
            workspace,
            evidence,
            authoring,
            sources,
            projects,
            runtime: copies[0].clone(),
            exporter: copies[1].clone(),
            uv_cli: copies[2].clone(),
            probe,
            originals,
            cli_stdout,
            baseline,
            glb,
            source_cooked,
            recipes,
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
            self.authoring.join(SOURCE_PATH),
            self.sources.join("variants/rotate.model.json"),
            self.sources.join("variants/counter.model.json"),
            self.sources.join("original").join(ORIGINAL_ASSET),
        ];
        for (name, _, _) in VARIANTS {
            missing.push(self.authoring.join(format!("{name}.uv.json")));
        }
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
                self.uv_cli.clone(),
            ]);
        } else {
            command.arg("--ro-bind").arg(project).arg(project);
        }
        // Verify every namespace mask before ANY runtime/export/probe execution.
        command.args(["--", "/bin/sh", "-c",
            "n=$1; shift; while [ \"$n\" -gt 0 ]; do test ! -e \"$1\" || exit 91; shift; n=$((n-1)); done; n=$1; shift; while [ \"$n\" -gt 0 ]; do test ! -s \"$1\" || exit 92; shift; n=$((n-1)); done; export ORR_STATIC_MODEL_UV_NAMESPACE=verified; exec \"$@\"",
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
            .env("ORR_STATIC_MODEL_UV_CHILD", "source-hidden-v1")
            .env("ORR_STATIC_MODEL_UV_CHILD_PROJECT", project)
            .env("ORR_STATIC_MODEL_UV_CHILD_TICKS", ticks.to_string())
            .env("ORR_STATIC_MODEL_UV_CHILD_HELD", held)
            .env(
                "ORR_STATIC_MODEL_UV_CHILD_MODEL_HASHES",
                self.cooked
                    .iter()
                    .map(|bytes| digest(bytes))
                    .collect::<Vec<_>>()
                    .join(","),
            )
            .env("ORR_STATIC_MODEL_UV_CHILD_OUTPUT", output);
        good(bounded_output(&mut command))
    }
}

// Fresh process using the current production PreparedProject/Simulation route.
// This is separately labelled Frame evidence, not invented room_escape output.
fn child_probe() {
    assert_eq!(
        std::env::var("ORR_STATIC_MODEL_UV_CHILD").as_deref(),
        Ok("source-hidden-v1")
    );
    assert_eq!(
        std::env::var("ORR_STATIC_MODEL_UV_NAMESPACE").as_deref(),
        Ok("verified")
    );
    let root = PathBuf::from(std::env::var_os("ORR_STATIC_MODEL_UV_CHILD_PROJECT").unwrap());
    let error = fs::write(root.join("probe-must-not-write"), b"forbidden")
        .expect_err("project must be read-only");
    assert_eq!(
        error.raw_os_error(),
        Some(30),
        "read-only filesystem required"
    );
    let ticks: u32 = std::env::var("ORR_STATIC_MODEL_UV_CHILD_TICKS")
        .unwrap()
        .parse()
        .unwrap();
    assert!(ticks <= 180);
    let held = std::env::var("ORR_STATIC_MODEL_UV_CHILD_HELD").unwrap();
    let prepared = PreparedProject::open(root).unwrap();
    let expected = std::env::var("ORR_STATIC_MODEL_UV_CHILD_MODEL_HASHES").unwrap();
    let expected: Vec<_> = expected.split(',').collect();
    assert_eq!(expected.len(), VARIANTS.len());
    assert_eq!(prepared.models().assets.len(), VARIANTS.len());
    let mut identities = BTreeSet::new();
    for (index, (name, asset_id, _)) in VARIANTS.into_iter().enumerate() {
        let loaded = &prepared.models().assets[&(PACKAGE.into(), format!("{name}.model.json"))];
        let model = loaded.static_model().unwrap();
        assert_eq!(model.source().asset_id, asset_id);
        assert!(identities.insert(model.source().asset_id.clone()));
        assert_eq!(
            digest(&model.to_bytes().unwrap()),
            expected[index],
            "fresh source-hidden process must reload the exact CLI output"
        );
    }
    let frame = simulate(prepared.scene(), ticks, &held);
    let output = PathBuf::from(std::env::var_os("ORR_STATIC_MODEL_UV_CHILD_OUTPUT").unwrap());
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
fn static_model_uv_real_cli_source_hidden_room_export() {
    if std::env::var_os("ORR_STATIC_MODEL_UV_CHILD").is_some() {
        child_probe();
        return;
    }
    let mut work = Work::new();
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
    let mut content_digest = Sha256::new();
    content_digest.update(b"orrery.room-escape.export.content.v1\0");
    content_digest.update(serde_json::to_vec(&manifest.payload).unwrap());
    assert_eq!(
        manifest.content_digest,
        content_digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    assert_eq!(manifest.payload.runtime.sha256, file_hash(&work.runtime));
    assert_eq!(
        manifest.payload.initial_checksum,
        format!("0x{:016x}", work.baseline.frame().checksum())
    );
    assert_eq!(manifest.payload.packages.len(), 1);
    let package_digest = manifest.payload.packages.get(PACKAGE).unwrap();
    let object = format!(".orr/packages/objects/{package_digest}");
    let expected_models: BTreeSet<_> = VARIANTS
        .iter()
        .map(|(name, _, _)| format!("{object}/{name}.model.json"))
        .collect();
    let exported_models: BTreeSet<_> = exported_project
        .keys()
        .filter(|path| path.ends_with(".model.json"))
        .cloned()
        .collect();
    assert_eq!(
        exported_models, expected_models,
        "only the two explicitly packaged cooked variants may export"
    );
    let manifest_models: BTreeSet<_> = manifest
        .payload
        .files
        .iter()
        .filter(|file| file.path.ends_with(".model.json"))
        .map(|file| file.path.clone())
        .collect();
    assert_eq!(
        manifest_models,
        expected_models
            .iter()
            .map(|path| format!("project/{path}"))
            .collect()
    );
    let expected_project: BTreeSet<_> = [
        "orr.project.json".into(),
        "room.scene.yaml".into(),
        "room.models.json".into(),
        "room.camera.json".into(),
        "orr.packages.lock.json".into(),
        format!("{object}/orr.package.json"),
    ]
    .into_iter()
    .chain(expected_models)
    .collect();
    assert_eq!(
        exported_project.keys().cloned().collect::<BTreeSet<_>>(),
        expected_project
    );
    let listed: BTreeSet<_> = manifest
        .payload
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    assert_eq!(
        manifest.payload.files.len(),
        listed.len(),
        "no repeated manifest paths"
    );
    assert_eq!(
        listed,
        bundle_before
            .keys()
            .filter(|path| path.as_str() != "orr.export.json")
            .cloned()
            .collect()
    );
    for file in &manifest.payload.files {
        assert_eq!(bundle_before[&file.path], (file.bytes, file.sha256.clone()));
    }
    let reloaded = PreparedProject::open(bundle.join("project")).unwrap();
    let before = PreparedProject::open(root).unwrap();
    assert_eq!(reloaded.models().document, before.models().document);
    assert_eq!(reloaded.scene().text(), before.scene().text());
    assert_eq!(
        reloaded.scene().frame().to_bytes(),
        before.scene().frame().to_bytes()
    );
    assert_eq!(
        reloaded.camera().unwrap().bytes,
        before.camera().unwrap().bytes
    );
    assert_eq!(reloaded.models().assets.len(), 2);
    let original = StaticModel::from_bytes(&work.source_cooked).unwrap();
    let mut identities = BTreeSet::new();
    let mut primitive_ids = BTreeSet::new();
    for (index, (name, asset_id, quarter_turns)) in VARIANTS.into_iter().enumerate() {
        let asset = format!("{name}.model.json");
        let loaded = &reloaded.models().assets[&(PACKAGE.into(), asset.clone())];
        let model = loaded.static_model().unwrap();
        assert!(identities.insert(model.source().asset_id.clone()));
        assert_eq!(model.source().asset_id, asset_id);
        assert_eq!(
            fs::read(bundle.join("project").join(&object).join(asset)).unwrap(),
            work.cooked[index]
        );
        assert_uv(
            model,
            &original,
            &work.recipes[index],
            asset_id,
            quarter_turns,
        );
        for primitive in &model.source().primitives {
            assert!(
                primitive_ids.insert(primitive.id.clone()),
                "variant primitive IDs collided"
            );
        }
    }
    assert_eq!(
        identities,
        VARIANTS.iter().map(|(_, id, _)| (*id).to_owned()).collect()
    );
    assert!(!identities.contains(SOURCE_ID));
    assert!(!exported_project.keys().any(|path| path.ends_with(".glb")
        || path.ends_with(".png")
        || path.ends_with(".uv.json")
        || path.ends_with(ORIGINAL_ASSET)
        || path.ends_with(SOURCE_PATH)));
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
    // Both controls use the same scene, camera and production runtime. Room
    // requires real bindings, so translating models out of view is explicitly an
    // offscreen-model control, not empty-binding admission. The original control
    // retains exact visible transforms to isolate the authored UV effect.
    let offscreen = capture.join("control-offscreen.png");
    let whole_scene = capture.join("control-original.png");
    let mut control_stdout = Vec::new();
    for (control, png) in [
        (&work.projects[1], &offscreen),
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
    let visible_changed_pixels = changed(&first_pixels, &pixels(&offscreen));
    let uv_changed_pixels = changed(&first_pixels, &pixels(&whole_scene));
    assert!(
        visible_changed_pixels > 20,
        "cooked variants contributed no visible runtime pixels"
    );
    assert!(
        uv_changed_pixels > 20,
        "authored UV changes had no positive visible runtime evidence"
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
    assert_eq!(tree(root), source_before, "variant project changed");
    assert_eq!(
        tree(&work.authoring),
        authoring_before,
        "GLB/cooked/recipe authoring inputs changed"
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
    copy_tree(
        &work.projects[1],
        &work.evidence.join("control-offscreen-project"),
    );
    copy_tree(
        &work.projects[2],
        &work.evidence.join("control-original-project"),
    );
    fs::copy(
        bundle.join("orr.export.json"),
        work.evidence.join("orr.export.json"),
    )
    .unwrap();
    fs::write(work.evidence.join("scene.glb"), &work.glb).unwrap();
    fs::write(
        work.evidence.join("authoring-source.model.json"),
        &work.source_cooked,
    )
    .unwrap();
    let mut variant_evidence = Vec::new();
    for (index, (name, asset_id, quarter_turns)) in VARIANTS.into_iter().enumerate() {
        fs::write(
            work.evidence.join(format!("{name}.model.json")),
            &work.cooked[index],
        )
        .unwrap();
        fs::write(
            work.evidence.join(format!("{name}.uv.json")),
            &work.recipes[index],
        )
        .unwrap();
        variant_evidence.push(json!({
            "name":name, "asset_id":asset_id, "material":0, "quarter_turns":quarter_turns,
            "offset":if quarter_turns == 1 { [0.25,0.125] } else { [-0.375,0.75] },
            "scale":if quarter_turns == 1 { [1.5,0.5] } else { [0.5,1.5] },
            "recipe_sha256":digest(&work.recipes[index]),
            "cooked_sha256":digest(&work.cooked[index]), "cli_stdout":work.cli_stdout[index]
        }));
    }
    fs::write(work.evidence.join("acceptance-source.json"), serde_json::to_vec_pretty(&json!({
        "test":TEST, "test_source_sha256":digest(include_bytes!("static_model_uv_export.rs")),
        "cli_source_sha256":digest(include_bytes!("../../orr_model/examples/static_model_uv.rs")),
        "authoring_source_sha256":digest(include_bytes!("../../orr_model/examples/static_model_uv/authoring.rs")),
        "source_asset_id":SOURCE_ID, "source_path":SOURCE_PATH,
        "glb_sha256":digest(&work.glb), "source_cooked_sha256":digest(&work.source_cooked),
        "variants":variant_evidence, "original_source_exported":false,
        "initial_frame_sha256":digest(&work.baseline.frame().to_bytes())
    })).unwrap()).unwrap();
    fs::write(work.evidence.join("production-receipt.json"), serde_json::to_vec_pretty(&json!({
        "runtime_sha256":file_hash(&work.runtime), "exporter_sha256":file_hash(&work.exporter),
        "uv_cli_sha256":file_hash(&work.uv_cli), "acceptance_executable_sha256":file_hash(&work.probe),
        "source_hidden":true, "read_only_bundle":true, "no_visible_models_control":true, "offscreen_control_preserves_bindings":true, "observations":observations,
        "visible_changed_pixels":visible_changed_pixels, "uv_changed_pixels":uv_changed_pixels,
        "control_stdout":control_stdout, "exported_file_inventory":bundle_before,
        "control_project_inventories":{"offscreen":controls_before[0],"original":controls_before[1]},
        "frame_evidence_boundary":"Frame bytes are from fresh PreparedProject probes of the same read-only source/export bytes; room_escape independently supplies PNG and checksum evidence"
    })).unwrap()).unwrap();
    fs::write(
        work.evidence.join("capture-inventory.json"),
        serde_json::to_vec_pretty(&tree(&work.evidence)).unwrap(),
    )
    .unwrap();
    eprintln!("Verified real CLI uv variants, exact export model set and identities, original/variant GPU controls, four exact PNG pairs and eight fresh Frame probes");
    work.temp.complete();
}
