//! Explicit production-binary acceptance, not a claim of full game completion.
//! ORR_ROOM_RUNTIME=/absolute/room_escape ORR_ROOM_EXPORTER=/absolute/orr_export_room
//! ORR_REQUIRE_GPU=1 ORR_REQUIRE_PROJECT_ISOLATION=1 cargo test -p orr_sample
//! --features room-project,project-export --test room_export -- --ignored --exact
//! room_real_export_source_hidden_gpu_workflow
#![cfg(all(
    feature = "room-project",
    feature = "project-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types)]

use orr_ecs::{Entity, Frame};
use orr_fp::{FPVec3, FP};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::{Project, Runtime};
use orr_reflect::{Scene, Value};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, EXIT, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::Simulation;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output},
};

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
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(diagnostic),
        "wrong failure, expected {diagnostic:?}: {stderr}"
    );
}
fn hash(path: &Path) -> String {
    let mut input = fs::File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = input.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn tree(root: &Path) -> BTreeMap<String, (u64, String)> {
    fn visit(root: &Path, path: &Path, result: &mut BTreeMap<String, (u64, String)>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(
                !kind.is_symlink(),
                "unexpected symlink: {}",
                entry.path().display()
            );
            if kind.is_dir() {
                visit(root, &entry.path(), result);
            } else {
                assert!(kind.is_file());
                result.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    (entry.metadata().unwrap().len(), hash(&entry.path())),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
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
fn actor(frame: &Frame, kind: u32) -> Entity {
    frame
        .entities()
        .find(|&e| frame.get::<RoomActor>(e).is_some_and(|a| a.kind == kind))
        .unwrap()
}

struct Work {
    temp: tempfile::TempDir,
    project: PathBuf,
    source: PathBuf,
    workspace: PathBuf,
    runtime: PathBuf,
    exporter: PathBuf,
    originals: Vec<PathBuf>,
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
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .unwrap();
        assert!(
            !base.starts_with(&workspace),
            "test fixture must live outside hidden workspace"
        );
        for name in [
            "project",
            "tools",
            "empty",
            "captures",
            "home",
            "xdg-data",
            "xdg-config",
            "xdg-cache",
            "xdg-runtime",
            "readonly",
        ] {
            fs::create_dir(base.join(name)).unwrap();
        }
        let project = base.join("project");
        let source = base.join("source package");
        copy_tree(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo"),
            &source,
        );
        let sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        let mut scene = Scene::unbake(&room_project::types(), sim.frame(), None).unwrap();
        // Edit actual authored document values before the consuming parse/bake.
        // Shift all three roles to z=-1 and player/key to x=-1/0; setup fallback
        // would give a different initial checksum and different GPU pixels.
        let mut edited = 0;
        for entity in scene.entities.values_mut() {
            let kind = entity
                .components
                .iter()
                .find(|(name, _)| name == room_project::ACTOR)
                .unwrap()
                .1
                .field("kind")
                .unwrap();
            let x = match kind {
                Value::Int(n) if *n == i128::from(PLAYER) => -1,
                Value::Int(n) if *n == i128::from(KEY) => 0,
                Value::Int(n) if *n == i128::from(EXIT) => 2,
                _ => continue,
            };
            let body = &mut entity
                .components
                .iter_mut()
                .find(|(name, _)| name == "orr_physics3d::Body")
                .unwrap()
                .1;
            let Value::Struct(fields) = body else {
                panic!("Body must be reflected struct")
            };
            fields.iter_mut().find(|(name, _)| name == "pos").unwrap().1 =
                Value::Vec3(FPVec3::new(FP::from_int(x), FP::HALF, FP::from_int(-1)));
            edited += 1;
        }
        assert_eq!(edited, 3);
        let yaml = scene.to_yaml();
        fs::write(project.join("room.scene.yaml"), &yaml).unwrap();
        fs::write(project.join("orr.project.json"), serde_json::to_vec_pretty(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json"}})).unwrap()).unwrap();
        Project::open_for_install(&project, Runtime::content_only().engine_version)
            .unwrap()
            .install(std::slice::from_ref(&source))
            .unwrap();
        let loaded =
            model_bindings::load_asset(&project, "sample-imported-scene", "foreground.glb")
                .unwrap();
        let binding = Binding::from_asset(
            "sample-imported-scene".into(),
            "foreground.glb".into(),
            &loaded,
            LocalTransform::default(),
        )
        .unwrap();
        let admitted = PreparedScene::parse(&yaml).unwrap();
        let canonical = PreparedScene::parse(
            &Scene::unbake(&room_project::types(), sim.frame(), None)
                .unwrap()
                .to_yaml(),
        )
        .unwrap();
        assert_ne!(admitted.frame().checksum(), canonical.frame().checksum());
        let bindings = [PLAYER, KEY]
            .into_iter()
            .map(|kind| {
                (
                    admitted
                        .index()
                        .guid(actor(admitted.frame(), kind))
                        .unwrap()
                        .to_string(),
                    {
                        let mut authored = binding.clone();
                        authored.material_override = Some(orr_model::MaterialOverride {
                            material_slot: 0,
                            base_color_factor: if kind == PLAYER { [0.15, 0.9, 0.25] } else { [0.95, 0.12, 0.7] },
                        });
                        authored
                    },
                )
            })
            .collect();
        let document = Document {
            version: 3,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings,
        };
        fs::write(
            project.join("room.models.json"),
            serde_json::to_vec_pretty(&document).unwrap(),
        )
        .unwrap();
        PreparedProject::open(&project).unwrap();
        let mut originals = Vec::new();
        let mut copied = Vec::new();
        for (key, name) in [
            ("ORR_ROOM_RUNTIME", "room_escape"),
            ("ORR_ROOM_EXPORTER", "orr_export_room"),
        ] {
            let path = PathBuf::from(
                std::env::var_os(key)
                    .unwrap_or_else(|| panic!("explicit acceptance requires {key}")),
            );
            assert!(
                path.is_absolute() && path.is_file(),
                "{key} must be an absolute built executable"
            );
            let path = path.canonicalize().unwrap();
            let target = base.join("tools").join(name);
            fs::copy(&path, &target).unwrap();
            assert_eq!(hash(&path), hash(&target));
            originals.push(path);
            copied.push(target);
        }
        Self {
            temp,
            project,
            source,
            workspace,
            runtime: copied[0].clone(),
            exporter: copied[1].clone(),
            originals,
        }
    }
    fn isolated(&self, executable: &Path, bundle: Option<&Path>) -> Command {
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
            .arg(&self.source)
            .arg("--ro-bind")
            .arg(base.join("readonly"))
            .arg(base.join("readonly"));
        for original in &self.originals {
            if !original.starts_with(&self.workspace) {
                command.args(["--ro-bind", "/dev/null"]).arg(original);
            }
        }
        if let Some(bundle) = bundle {
            command
                .arg("--tmpfs")
                .arg(&self.project)
                .arg("--tmpfs")
                .arg(base.join("tools"))
                .arg("--ro-bind")
                .arg(bundle)
                .arg(bundle);
        } else {
            command
                .arg("--ro-bind")
                .arg(&self.project)
                .arg(&self.project);
        }
        command.arg("--");
        if bundle.is_some() {
            // Verify the namespace from inside it before launching production
            // code. A malformed hide setup fails instead of weakening proof.
            command.args(["/bin/sh", "-c", "test ! -e \"$1\" && test ! -e \"$2\" && test ! -e \"$3\" && test ! -e \"$4\" && test ! -s \"$5\" && test ! -s \"$6\" || exit 91; shift 6; exec \"$@\"", "sh"])
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"))
                .arg(self.project.join("orr.project.json"))
                .arg(self.source.join("orr.package.json"))
                .arg(&self.runtime)
                .arg(&self.originals[0])
                .arg(&self.originals[1]);
        }
        command
            .arg(executable)
            .current_dir(base.join("empty"))
            .env("HOME", base.join("home"))
            .env("XDG_DATA_HOME", base.join("xdg-data"))
            .env("XDG_CONFIG_HOME", base.join("xdg-config"))
            .env("XDG_CACHE_HOME", base.join("xdg-cache"))
            .env("XDG_RUNTIME_DIR", base.join("xdg-runtime"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        command
    }
    fn runtime_command(
        &self,
        bundle: Option<&Path>,
        ticks: u32,
        held: &str,
        capture: &Path,
    ) -> Command {
        let exe = bundle
            .map(|b| b.join("run-room-escape"))
            .unwrap_or_else(|| self.runtime.clone());
        let mut c = self.isolated(&exe, bundle);
        if bundle.is_none() {
            c.arg("--project").arg(&self.project);
        }
        c.args(["--headless", "--ticks", &ticks.to_string(), "--capture"])
            .arg(capture);
        if !held.is_empty() {
            c.args(["--hold", held]);
        }
        c
    }
    fn export_command(&self, output: &Path) -> Command {
        let mut c = self.isolated(&self.exporter, None);
        c.arg("--project")
            .arg(&self.project)
            .arg("--runtime")
            .arg(&self.runtime)
            .args([
                "--runtime-sha256",
                &hash(&self.runtime),
                "--trusted-runtime",
                "--output",
            ])
            .arg(output);
        c
    }
}
fn pixels(path: &Path) -> Vec<u8> {
    let mut reader = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap()))
        .read_info()
        .unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(
        (info.width, info.height, info.color_type),
        (1024, 768, png::ColorType::Rgba)
    );
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    bytes.truncate(info.buffer_size());
    assert_eq!(bytes.len(), 1024 * 768 * 4);
    bytes
}

#[test]
#[ignore = "requires explicit exact production tools, real GPU and source-hidden read-only Linux namespace"]
fn room_real_export_source_hidden_gpu_workflow() {
    let w = Work::new();
    let initial = PreparedProject::open(&w.project)
        .unwrap()
        .scene()
        .frame()
        .checksum();
    let project_before = tree(&w.project);
    let staged = w.temp.path().join("first export");
    good(w.export_command(&staged).output().unwrap());
    let second = w.temp.path().join("second export");
    good(w.export_command(&second).output().unwrap());
    assert_eq!(
        fs::read(staged.join("orr.export.json")).unwrap(),
        fs::read(second.join("orr.export.json")).unwrap(),
        "manifest must be byte deterministic across export destinations"
    );
    let bundle = w.temp.path().join("relocated room");
    fs::rename(staged, &bundle).unwrap();
    // Do not leave another unhidden export available as a fallback.
    fs::remove_dir_all(second).unwrap();
    let before = tree(&bundle);
    assert_eq!(hash(&bundle.join("bin/room_escape")), hash(&w.runtime));
    let exported_project = tree(&bundle.join("project"));
    for (path, bytes) in &exported_project {
        assert_eq!(
            project_before.get(path),
            Some(bytes),
            "export changed exact admitted project/package bytes: {path}"
        );
    }
    for path in ["orr.project.json", "room.scene.yaml", "room.models.json"] {
        assert_eq!(
            fs::read(bundle.join("project").join(path)).unwrap(),
            fs::read(w.project.join(path)).unwrap()
        );
    }
    assert!(exported_project
        .keys()
        .any(|p| p.ends_with("foreground.glb")));
    assert!(exported_project
        .keys()
        .any(|p| p.ends_with("orr.package.json")));
    let mut initial_image = Vec::new();
    for (name, ticks, held) in [
        ("initial", 0, ""),
        ("moved", 18, "right"),
        ("interacted", 18, "right,interact"),
    ] {
        let source = w
            .temp
            .path()
            .join("captures")
            .join(format!("{name}-source.png"));
        let exported = w
            .temp
            .path()
            .join("captures")
            .join(format!("{name}-export.png"));
        let a = good(
            w.runtime_command(None, ticks, held, &source)
                .output()
                .unwrap(),
        );
        let b = good(
            w.runtime_command(Some(&bundle), ticks, held, &exported)
                .output()
                .unwrap(),
        );
        assert!(a.contains(&format!("room initial checksum: 0x{initial:016x}")));
        assert!(
            a.contains("room capture adapter:"),
            "actual GPU capture must run"
        );
        assert_eq!(a, b, "production source/export checksum and state parity");
        assert_eq!(
            fs::read(&source).unwrap(),
            fs::read(&exported).unwrap(),
            "exact PNG parity: {name}"
        );
        let image = pixels(&source);
        assert!(
            image.chunks(4).filter(|p| *p != &image[..4]).count() > 100,
            "blank GPU output"
        );
        if ticks == 0 {
            initial_image = image;
        } else {
            assert_ne!(
                image, initial_image,
                "scripted input must change actual presentation"
            );
        }
        if name == "interacted" {
            assert!(
                a.contains("room key: 1"),
                "authored nearby key interaction did not occur: {a}"
            );
        }
        let old = fs::read(&source).unwrap();
        bad(
            w.runtime_command(None, 0, "", &source).output().unwrap(),
            "already exists",
        );
        assert_eq!(
            fs::read(&source).unwrap(),
            old,
            "existing capture was damaged"
        );
    }
    // An actual production-runtime negative control: move only the imported
    // model presentation outside the view; procedural room geometry is unchanged.
    let sidecar = w.project.join("room.models.json");
    let original = fs::read(&sidecar).unwrap();
    let mut doc: Document = serde_json::from_slice(&original).unwrap();
    assert_eq!(doc.version, 3);
    assert_eq!(doc.bindings.len(), 2);
    let authored: Vec<_> = doc.bindings.values().collect();
    assert_eq!(authored[0].asset, authored[1].asset);
    assert_eq!(authored[0].source_hash, authored[1].source_hash);
    assert_ne!(authored[0].material_override, authored[1].material_override);
    // Reset the authored factors through the exact production consumer. Only
    // sidecar presentation changes; scene/checksum and installed assets do not.
    let mut reset = doc.clone();
    for binding in reset.bindings.values_mut() { binding.material_override = None; }
    fs::write(&sidecar, serde_json::to_vec_pretty(&reset).unwrap()).unwrap();
    let material_captures = w.temp.path().join("captures/material");
    fs::create_dir(&material_captures).unwrap();
    let reset_capture = material_captures.join("reset-imported.png");
    let reset_output = good(w.runtime_command(None, 0, "", &reset_capture).output().unwrap());
    assert!(reset_output.contains(&format!("room initial checksum: 0x{initial:016x}")));
    assert!(initial_image.chunks(4).zip(pixels(&reset_capture).chunks(4)).filter(|(a,b)| a != b).count() > 20,
        "authored RGB factors did not change production pixels");
    fs::write(&sidecar, &original).unwrap();
    let restored_capture = material_captures.join("restored-factors.png");
    good(w.runtime_command(None, 0, "", &restored_capture).output().unwrap());
    assert_eq!(pixels(&restored_capture), initial_image, "restart must restore saved material appearance");
    for binding in doc.bindings.values_mut() {
        binding.transform.translation = [1000.0, 1000.0, 1000.0];
    }
    fs::write(&sidecar, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    let hidden_models = w.temp.path().join("captures/no-models.png");
    good(
        w.runtime_command(None, 0, "", &hidden_models)
            .output()
            .unwrap(),
    );
    assert!(
        initial_image
            .chunks(4)
            .zip(pixels(&hidden_models).chunks(4))
            .filter(|(a, b)| a != b)
            .count()
            > 20,
        "real imported model contributed no visible pixels"
    );
    fs::write(&sidecar, b"{malformed models").unwrap();
    let failed_capture = w.temp.path().join("captures/rejected.png");
    bad(
        w.runtime_command(None, 0, "", &failed_capture)
            .output()
            .unwrap(),
        "room model bindings",
    );
    let rejected_export = w.temp.path().join("rejected export");
    bad(
        w.export_command(&rejected_export).output().unwrap(),
        "room model bindings",
    );
    assert!(!failed_capture.exists() && !rejected_export.exists());
    for (version, slot, diagnostic) in [(2, 0, "version-3"), (3, 255, "used static material slot")] {
        let mut invalid: Document = serde_json::from_slice(&original).unwrap();
        invalid.version = version;
        invalid.bindings.values_mut().next().unwrap().material_override.as_mut().unwrap().material_slot = slot;
        fs::write(&sidecar, serde_json::to_vec(&invalid).unwrap()).unwrap();
        bad(w.runtime_command(None, 0, "", &failed_capture).output().unwrap(), diagnostic);
        bad(w.export_command(&rejected_export).output().unwrap(), diagnostic);
        assert!(!failed_capture.exists() && !rejected_export.exists(), "invalid material admission created output");
    }
    fs::write(&sidecar, &original).unwrap();
    // Remove the real installed object store, not a fictitious fixture path.
    let objects = w.project.join(".orr/packages/objects");
    let saved = w.temp.path().join("hidden objects");
    fs::rename(&objects, &saved).unwrap();
    let missing = w
        .runtime_command(None, 0, "", &failed_capture)
        .output()
        .unwrap();
    bad(missing, "room active lock");
    bad(
        w.export_command(&rejected_export).output().unwrap(),
        "No such file or directory",
    );
    assert!(!failed_capture.exists() && !rejected_export.exists());
    fs::rename(saved, objects).unwrap();
    bad(
        w.runtime_command(Some(&bundle), 0, "", &bundle.join("forbidden.png"))
            .output()
            .unwrap(),
        "Read-only file system",
    );
    let readonly_output = w.temp.path().join("readonly/export");
    bad(
        w.export_command(&readonly_output).output().unwrap(),
        "Read-only file system",
    );
    assert!(!readonly_output.exists());
    bad(
        w.export_command(&bundle).output().unwrap(),
        "output destination already exists",
    );
    assert_eq!(tree(&bundle), before, "relocated read-only bundle changed");
    assert_eq!(tree(&w.project), project_before, "source inputs changed");
    assert_eq!(
        fs::read_dir(w.temp.path().join("empty")).unwrap().count(),
        0
    );
    if let Some(dest) = std::env::var_os("ORR_ROOM_EXPORT_CAPTURE_DIR") {
        let dest = PathBuf::from(dest);
        fs::create_dir_all(&dest).unwrap();
        copy_tree(&w.temp.path().join("captures"), &dest.join("captures"));
        copy_tree(&w.project, &dest.join("authored-project"));
        fs::copy(bundle.join("orr.export.json"), dest.join("orr.export.json")).unwrap();
    }
}

#[test]
#[ignore = "requires exact production Room tools, real GPU and source-hidden read-only Linux namespace"]
fn room_follow_camera_source_hidden_readonly_export() {
    let w = Work::new();
    let camera_path = w.project.join("room.camera.json");
    let manifest_path = w.project.join("orr.project.json");
    let scene_text = fs::read_to_string(w.project.join("room.scene.yaml")).unwrap();
    let scene = PreparedScene::parse(&scene_text).unwrap();
    let player = actor(scene.frame(), PLAYER);
    let player_guid = scene.index().guid(player).unwrap().to_string();
    let initial = scene.frame().checksum();

    // Work::new owns a v3 static sidecar with per-player/key RGB factors. Keep
    // those exact authored factors while adding only a presentation camera.
    let model_path = w.project.join("room.models.json");
    let model_bytes = fs::read(&model_path).unwrap();
    let models: Document = serde_json::from_slice(&model_bytes).unwrap();
    assert_eq!(models.version, 3);
    assert_eq!(models.bindings.len(), 2);
    let factors: Vec<_> = models
        .bindings
        .values()
        .map(|binding| binding.material_override)
        .collect();
    assert!(factors.iter().all(Option::is_some));
    assert_ne!(factors[0], factors[1]);

    let fixed = orr_sample::room_camera::Document::readable_default();
    let fixed_bytes = fixed.to_bytes().unwrap();
    fs::write(&camera_path, &fixed_bytes).unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["entry"]["camera"] = "room.camera.json".into();
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let admitted = PreparedProject::open(&w.project).unwrap();
    assert_eq!(admitted.scene().frame().checksum(), initial);
    assert_eq!(admitted.camera().unwrap().document, fixed);
    let fixed_capture = w.temp.path().join("captures/fixed-camera.png");
    let fixed_output = good(
        w.runtime_command(None, 0, "", &fixed_capture)
            .output()
            .unwrap(),
    );
    assert!(fixed_output.contains(&format!("room initial checksum: 0x{initial:016x}")));
    let fixed_pixels = pixels(&fixed_capture);

    let mut followed = fixed.clone();
    followed.schema = 2;
    followed.follow = Some(orr_sample::room_camera::Follow {
        player: player_guid.clone(),
        offset: [1.25, -0.5, 0.75],
    });
    let follow_bytes = followed.to_bytes().unwrap();
    fs::write(&camera_path, &follow_bytes).unwrap();
    let admitted = PreparedProject::open(&w.project).unwrap();
    assert_eq!(admitted.scene().frame().checksum(), initial);
    assert_eq!(admitted.camera().unwrap().bytes, follow_bytes);
    assert_eq!(fs::read(&model_path).unwrap(), model_bytes);
    let followed_capture = w.temp.path().join("captures/follow-camera.png");
    let followed_output = good(
        w.runtime_command(None, 0, "", &followed_capture)
            .output()
            .unwrap(),
    );
    assert!(followed_output.contains(&format!("room initial checksum: 0x{initial:016x}")));
    assert_ne!(
        pixels(&followed_capture),
        fixed_pixels,
        "camera-only follow must change actual GPU pixels without changing the simulation checksum"
    );

    // A syntactically valid but unknown GUID, and a GUID reassigned to KEY,
    // must both fail before either runtime capture or exporter creates output.
    let key = actor(scene.frame(), KEY);
    let key_guid = scene.index().guid(key).unwrap().to_string();
    for (invalid_guid, diagnostic) in [
        (
            "e_ffffffffffffffffffffffffffffffff".to_string(),
            "not in the scene index",
        ),
        (key_guid, "sole PLAYER"),
    ] {
        let mut invalid = followed.clone();
        invalid.follow.as_mut().unwrap().player = invalid_guid;
        fs::write(&camera_path, invalid.to_bytes().unwrap()).unwrap();
        let capture = w.temp.path().join("captures/rejected-follow.png");
        bad(
            w.runtime_command(None, 0, "", &capture).output().unwrap(),
            diagnostic,
        );
        let output = w.temp.path().join("rejected follow export");
        bad(w.export_command(&output).output().unwrap(), diagnostic);
        assert!(!capture.exists() && !output.exists());
    }
    fs::write(&camera_path, &follow_bytes).unwrap();

    // Malformed/structurally incomplete sidecars fail admission before the
    // runtime writes a capture or the exporter creates a destination.
    let malformed_cameras: [&[u8]; 3] = [
        b"{malformed camera",
        br#"{"schema":2}"#,
        br#"{"schema":2,"follow":{"player":"not-a-guid","offset":[0,0,0]}}"#,
    ];
    for malformed in malformed_cameras {
        fs::write(&camera_path, malformed).unwrap();
        let capture = w.temp.path().join("captures/rejected-malformed-camera.png");
        bad(
            w.runtime_command(None, 0, "", &capture).output().unwrap(),
            "room camera",
        );
        let output = w.temp.path().join("rejected malformed camera export");
        bad(w.export_command(&output).output().unwrap(), "room camera");
        assert!(!capture.exists() && !output.exists());
    }
    fs::write(&camera_path, &follow_bytes).unwrap();

    let project_before = tree(&w.project);
    let staged = w.temp.path().join("follow-camera export");
    good(w.export_command(&staged).output().unwrap());
    let changed_camera = {
        let mut next = followed.clone();
        next.follow.as_mut().unwrap().offset = [2.25, -0.5, 0.75];
        next
    };
    fs::write(&camera_path, changed_camera.to_bytes().unwrap()).unwrap();
    let changed_export = w.temp.path().join("camera identity export");
    good(w.export_command(&changed_export).output().unwrap());
    let old_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(staged.join("orr.export.json")).unwrap()).unwrap();
    let new_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(changed_export.join("orr.export.json")).unwrap()).unwrap();
    assert_ne!(
        old_manifest["content_digest"],
        new_manifest["content_digest"]
    );
    assert_eq!(
        fs::read(changed_export.join("project/room.camera.json")).unwrap(),
        changed_camera.to_bytes().unwrap()
    );
    fs::remove_dir_all(changed_export).unwrap();
    fs::write(&camera_path, &follow_bytes).unwrap();

    let bundle = w.temp.path().join("relocated follow camera room");
    fs::rename(staged, &bundle).unwrap();
    let bundle_before = tree(&bundle);
    assert_eq!(hash(&bundle.join("bin/room_escape")), hash(&w.runtime));
    assert_eq!(
        fs::read(bundle.join("project/room.camera.json")).unwrap(),
        follow_bytes
    );
    assert_eq!(
        fs::read(bundle.join("project/room.models.json")).unwrap(),
        model_bytes,
        "static material overrides must survive exact export"
    );
    for (path, bytes) in tree(&bundle.join("project")) {
        assert_eq!(
            project_before.get(&path),
            Some(&bytes),
            "export changed {path}"
        );
    }

    // The shipping binary and relocated, read-only export must produce byte-
    // identical checksums, PNGs, and captured source states at initial/move/key.
    let mut initial_image = Vec::new();
    for (name, ticks, held) in [
        ("initial", 0, ""),
        ("movement", 18, "right"),
        ("key", 18, "right,interact"),
    ] {
        let source = w
            .temp
            .path()
            .join("captures")
            .join(format!("{name}-source.png"));
        let exported = w
            .temp
            .path()
            .join("captures")
            .join(format!("{name}-export.png"));
        let a = good(
            w.runtime_command(None, ticks, held, &source)
                .output()
                .unwrap(),
        );
        let b = good(
            w.runtime_command(Some(&bundle), ticks, held, &exported)
                .output()
                .unwrap(),
        );
        assert!(a.contains(&format!("room initial checksum: 0x{initial:016x}")));
        assert!(a.contains("room capture adapter:"));
        assert_eq!(a, b, "production source/export state parity: {name}");
        assert_eq!(
            fs::read(&source).unwrap(),
            fs::read(&exported).unwrap(),
            "PNG parity: {name}"
        );
        let image = pixels(&source);
        if ticks == 0 {
            initial_image = image;
        } else {
            assert_ne!(
                image, initial_image,
                "input should visibly change follow-camera output"
            );
        }
        if name == "key" {
            assert!(a.contains("room key: 1"), "key pickup must occur: {a}");
        }
    }
    assert_eq!(
        tree(&w.project),
        project_before,
        "source project bytes must be restored exactly"
    );
    assert_eq!(fs::read(&model_path).unwrap(), model_bytes);
    assert_eq!(fs::read(&camera_path).unwrap(), follow_bytes);
    let forbidden_capture = bundle.join("forbidden.png");
    bad(
        w.runtime_command(Some(&bundle), 0, "", &forbidden_capture)
            .output()
            .unwrap(),
        "Read-only file system",
    );
    let readonly_export = w.temp.path().join("readonly/export");
    bad(
        w.export_command(&readonly_export).output().unwrap(),
        "Read-only file system",
    );
    assert!(!forbidden_capture.exists() && !readonly_export.exists());
    assert_eq!(
        tree(&bundle),
        bundle_before,
        "read-only export bundle must remain unchanged"
    );
    if let Some(destination) = std::env::var_os("ORR_ROOM_FOLLOW_CAPTURE_DIR") {
        let destination = PathBuf::from(destination);
        fs::create_dir_all(&destination).unwrap();
        copy_tree(
            &w.temp.path().join("captures"),
            &destination.join("static/captures"),
        );
        copy_tree(&w.project, &destination.join("static/authored-project"));
        fs::create_dir_all(destination.join("static/export/project")).unwrap();
        fs::copy(
            bundle.join("orr.export.json"),
            destination.join("static/export/orr.export.json"),
        )
        .unwrap();
        fs::copy(
            bundle.join("project/room.camera.json"),
            destination.join("static/export/project/room.camera.json"),
        )
        .unwrap();
        fs::copy(
            bundle.join("project/room.models.json"),
            destination.join("static/export/project/room.models.json"),
        )
        .unwrap();
    }
    #[cfg(feature = "room-character")]
    {
        // Keep production-binary copies and export staging bounded to one profile.
        drop(w);
        follow_camera_character_export_case();
    }
}

#[cfg(feature = "room-character")]
fn follow_camera_character_export_case() {
    let w = Work::new();
    let character_package = w.source.join("character");
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/room_character_demo"),
        &character_package,
    );
    Project::open_for_install(&w.project, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&character_package))
        .unwrap();
    let lock: orr_package::Lock =
        serde_json::from_slice(&fs::read(w.project.join("orr.packages.lock.json")).unwrap())
            .unwrap();
    let character_digest = &lock
        .packages
        .get("sample-room-character")
        .expect("character package is active")
        .digest;
    let character_object = w
        .project
        .join(".orr/packages/objects")
        .join(character_digest);
    let source_manifest: orr_package::Manifest =
        serde_json::from_slice(&fs::read(character_package.join("orr.package.json")).unwrap())
            .unwrap();
    let installed_manifest: orr_package::Manifest =
        serde_json::from_slice(&fs::read(character_object.join("orr.package.json")).unwrap())
            .unwrap();
    assert_eq!(installed_manifest, source_manifest);
    for file in ["courier.orrmodel.json", "LICENSE.txt", "generate.py"] {
        assert_eq!(
            fs::read(character_object.join(file)).unwrap(),
            fs::read(character_package.join(file)).unwrap(),
            "installed character package bytes changed: {file}"
        );
    }

    let scene =
        PreparedScene::parse(&fs::read_to_string(w.project.join("room.scene.yaml")).unwrap())
            .unwrap();
    let initial = scene.frame().checksum();
    let player_guid = scene
        .index()
        .guid(actor(scene.frame(), PLAYER))
        .unwrap()
        .to_string();
    let key_guid = scene
        .index()
        .guid(actor(scene.frame(), KEY))
        .unwrap()
        .to_string();
    let model_path = w.project.join("room.models.json");
    let mut models: Document = serde_json::from_slice(&fs::read(&model_path).unwrap()).unwrap();
    assert_eq!(models.version, 3);
    let key_static = models.bindings.get(&key_guid).unwrap().clone();
    let player_static = models.bindings.remove(&player_guid).unwrap();
    assert_eq!(key_static.kind, model_bindings::ModelKind::Static);
    let key_factor = key_static.material_override.unwrap();
    assert_eq!(player_static.kind, model_bindings::ModelKind::Static);
    assert!(player_static.material_override.is_some());

    let loaded = model_bindings::load_asset_for_kind(
        &w.project,
        "sample-room-character",
        "courier.orrmodel.json",
        model_bindings::ModelKind::Animated,
    )
    .unwrap();
    let animated_player = Binding::from_animated_asset(
        "sample-room-character".into(),
        "courier.orrmodel.json".into(),
        &loaded,
        model_bindings::AnimationDescriptor {
            clip_index: 0,
            playback: model_bindings::PlaybackMode::Loop,
        },
        LocalTransform::default(),
    )
    .unwrap();
    assert_eq!(animated_player.kind, model_bindings::ModelKind::Animated);
    assert_eq!(animated_player.material_override, None);
    assert_eq!(animated_player.animation.unwrap().clip_index, 0);
    models.bindings.insert(player_guid.clone(), animated_player);
    assert_eq!(models.bindings.len(), 2);
    assert_eq!(
        models.bindings.get(&key_guid).unwrap().material_override,
        Some(key_factor),
        "the static KEY keeps its authored material factor"
    );
    fs::write(&model_path, serde_json::to_vec_pretty(&models).unwrap()).unwrap();
    let model_bytes = fs::read(&model_path).unwrap();

    let character = orr_sample::room_character::Document {
        schema: 1,
        player: player_guid.clone(),
        searching: 0,
        carrying: 1,
        escaped: 2,
        speeds: None,
        crossfade_ticks: None,
    };
    let character_path = w.project.join("room.character.json");
    let character_bytes = character.to_bytes().unwrap();
    fs::write(&character_path, &character_bytes).unwrap();

    let manifest_path = w.project.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["entry"]["character"] = "room.character.json".into();
    let camera_path = w.project.join("room.camera.json");
    let fixed = orr_sample::room_camera::Document::readable_default();
    fs::write(&camera_path, fixed.to_bytes().unwrap()).unwrap();
    manifest["entry"]["camera"] = "room.camera.json".into();
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let open = || {
        PreparedProject::open_with_capabilities(
            &w.project,
            false,
            orr_sample::room_project::CheckpointSupport::Disabled,
            true,
        )
        .unwrap()
    };
    let admitted = open();
    assert_eq!(admitted.scene().frame().checksum(), initial);
    assert_eq!(admitted.camera().unwrap().document, fixed);
    assert_eq!(admitted.character().unwrap().document, character);
    assert_eq!(admitted.character().unwrap().bytes, character_bytes);
    assert_eq!(admitted.models().document, models);
    assert_eq!(
        admitted
            .models()
            .document
            .bindings
            .get(&key_guid)
            .unwrap()
            .material_override,
        Some(key_factor)
    );

    let fixed_capture = w.temp.path().join("captures/character-fixed-camera.png");
    let fixed_output = good(
        w.runtime_command(None, 0, "", &fixed_capture)
            .output()
            .unwrap(),
    );
    assert!(fixed_output.contains(&format!("room initial checksum: 0x{initial:016x}")));
    let fixed_pixels = pixels(&fixed_capture);

    let mut followed = fixed;
    followed.schema = 2;
    followed.follow = Some(orr_sample::room_camera::Follow {
        player: player_guid,
        offset: [1.25, -0.5, 0.75],
    });
    let follow_bytes = followed.to_bytes().unwrap();
    fs::write(&camera_path, &follow_bytes).unwrap();
    let admitted = open();
    assert_eq!(admitted.scene().frame().checksum(), initial);
    assert_eq!(admitted.camera().unwrap().bytes, follow_bytes);
    assert_eq!(admitted.character().unwrap().bytes, character_bytes);
    assert_eq!(admitted.models().document, models);
    assert_eq!(fs::read(&model_path).unwrap(), model_bytes);
    assert_eq!(fs::read(&character_path).unwrap(), character_bytes);

    let followed_capture = w.temp.path().join("captures/character-follow-camera.png");
    let followed_output = good(
        w.runtime_command(None, 0, "", &followed_capture)
            .output()
            .unwrap(),
    );
    assert!(followed_output.contains(&format!("room initial checksum: 0x{initial:016x}")));
    assert_ne!(
        pixels(&followed_capture),
        fixed_pixels,
        "animated PLAYER follow changes actual GPU view without changing the simulation"
    );

    let project_before = tree(&w.project);
    let staged = w.temp.path().join("character follow-camera export");
    good(w.export_command(&staged).output().unwrap());
    let bundle = w.temp.path().join("relocated character follow-camera room");
    fs::rename(&staged, &bundle).unwrap();
    let bundle_before = tree(&bundle);
    assert_eq!(hash(&bundle.join("bin/room_escape")), hash(&w.runtime));
    assert_eq!(
        fs::read(bundle.join("project/room.camera.json")).unwrap(),
        follow_bytes
    );
    assert_eq!(
        fs::read(bundle.join("project/room.models.json")).unwrap(),
        model_bytes
    );
    assert_eq!(
        fs::read(bundle.join("project/room.character.json")).unwrap(),
        character_bytes
    );
    assert_eq!(
        serde_json::from_slice::<Document>(
            &fs::read(bundle.join("project/room.models.json")).unwrap()
        )
        .unwrap()
        .bindings
        .get(&key_guid)
        .unwrap()
        .material_override,
        Some(key_factor),
        "the exported static KEY retains its exact material factor"
    );
    let exported_project = tree(&bundle.join("project"));
    for (path, bytes) in &exported_project {
        assert_eq!(
            project_before.get(path),
            Some(bytes),
            "export changed {path}"
        );
    }
    assert!(exported_project
        .keys()
        .any(|path| path.ends_with("courier.orrmodel.json")));
    assert!(exported_project.keys().any(|path| {
        path == &format!(".orr/packages/objects/{character_digest}/courier.orrmodel.json")
    }));

    let mut initial_image = Vec::new();
    for (name, ticks, held) in [
        ("initial", 0, ""),
        ("movement", 18, "right"),
        ("key", 18, "right,interact"),
    ] {
        let source = w
            .temp
            .path()
            .join("captures")
            .join(format!("character-{name}-source.png"));
        let exported = w
            .temp
            .path()
            .join("captures")
            .join(format!("character-{name}-export.png"));
        let source_output = good(
            w.runtime_command(None, ticks, held, &source)
                .output()
                .unwrap(),
        );
        let exported_output = good(
            w.runtime_command(Some(&bundle), ticks, held, &exported)
                .output()
                .unwrap(),
        );
        assert!(source_output.contains(&format!("room initial checksum: 0x{initial:016x}")));
        assert!(source_output.contains("room capture adapter:"));
        assert_eq!(
            source_output, exported_output,
            "character source/export parity: {name}"
        );
        assert_eq!(fs::read(&source).unwrap(), fs::read(&exported).unwrap());
        let image = pixels(&source);
        assert!(
            image
                .chunks(4)
                .filter(|pixel| *pixel != &image[..4])
                .count()
                > 100
        );
        if ticks == 0 {
            initial_image = image;
        } else {
            assert_ne!(
                image, initial_image,
                "character presentation changed: {name}"
            );
        }
        if name == "key" {
            assert!(
                source_output.contains("room key: 1"),
                "key interaction failed: {source_output}"
            );
        }
    }
    assert_eq!(
        tree(&w.project),
        project_before,
        "character source bytes changed"
    );
    assert_eq!(fs::read(&model_path).unwrap(), model_bytes);
    assert_eq!(fs::read(&character_path).unwrap(), character_bytes);
    assert_eq!(fs::read(&camera_path).unwrap(), follow_bytes);

    let forbidden_capture = bundle.join("character-forbidden.png");
    bad(
        w.runtime_command(Some(&bundle), 0, "", &forbidden_capture)
            .output()
            .unwrap(),
        "Read-only file system",
    );
    let readonly_export = w.temp.path().join("readonly/character-export");
    bad(
        w.export_command(&readonly_export).output().unwrap(),
        "Read-only file system",
    );
    assert!(!forbidden_capture.exists() && !readonly_export.exists());
    assert_eq!(
        tree(&bundle),
        bundle_before,
        "character export bundle changed"
    );

    if let Some(destination) = std::env::var_os("ORR_ROOM_FOLLOW_CAPTURE_DIR") {
        let destination = PathBuf::from(destination).join("character");
        fs::create_dir_all(&destination).unwrap();
        copy_tree(
            &w.temp.path().join("captures"),
            &destination.join("captures"),
        );
        copy_tree(&w.project, &destination.join("authored-project"));
        fs::create_dir_all(destination.join("export/project")).unwrap();
        fs::copy(
            bundle.join("orr.export.json"),
            destination.join("export/orr.export.json"),
        )
        .unwrap();
        for sidecar in [
            "room.camera.json",
            "room.models.json",
            "room.character.json",
        ] {
            fs::copy(
                bundle.join("project").join(sidecar),
                destination.join("export/project").join(sidecar),
            )
            .unwrap();
        }
    }
}
