//! Production Room lighting acceptance with genuine source-hidden, read-only exports.
//! Supply ORR_ROOM_RUNTIME and ORR_ROOM_EXPORTER as absolute built binary paths,
//! and set ORR_REQUIRE_GPU=1 and ORR_REQUIRE_PROJECT_ISOLATION=1 before explicitly
//! running the ignored test. No adapter or namespace failure is a passing skip.
#![cfg(all(
    feature = "room-lighting",
    feature = "project-create",
    feature = "project-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types)]

use orr_fp::{FPVec3, FP};
use orr_reflect::{Scene, Value};
use orr_sample::{
    project_create::{create, CreateOptions, ROOM_TEMPLATE},
    room_game::{RoomActor, EXIT, KEY, PLAYER},
    room_lighting::Document,
    room_project::{self, CheckpointSupport, PreparedProject, PreparedScene},
};
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
    assert!(!output.status.success(), "unexpected success: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(diagnostic),
        "expected {diagnostic:?}: {stderr}"
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

fn open(root: &Path) -> PreparedProject {
    PreparedProject::open_with_presentation(
        root,
        false,
        CheckpointSupport::Disabled,
        cfg!(feature = "room-character"),
        true,
    )
    .unwrap()
}

struct Work {
    temp: tempfile::TempDir,
    project: PathBuf,
    workspace: PathBuf,
    runtime: PathBuf,
    exporter: PathBuf,
    originals: Vec<PathBuf>,
}

impl Work {
    fn new(template: &str) -> Self {
        for key in ["ORR_REQUIRE_GPU", "ORR_REQUIRE_PROJECT_ISOLATION"] {
            assert_eq!(
                std::env::var(key).as_deref(),
                Ok("1"),
                "acceptance requires {key}=1"
            );
        }
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        // Match room_export.rs: conceal the owning workspace, not just a fixture.
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .unwrap();
        assert!(
            !base.starts_with(&workspace),
            "fixture must be outside hidden sources"
        );
        for name in [
            "tools",
            "empty",
            "captures",
            "evidence",
            "home",
            "xdg-data",
            "xdg-config",
            "xdg-cache",
            "xdg-runtime",
            "readonly",
        ] {
            fs::create_dir(base.join(name)).unwrap();
        }
        let project = base.join("generated Room");
        create(&CreateOptions {
            output: project.clone(),
            template: template.into(),
            seed: "room-lighting-export-acceptance".into(),
        })
        .unwrap();

        // Retain the real generated packages, GUIDs and presentation sidecars.
        // Author positions different from setup so a runtime fallback cannot
        // accidentally pass the initial checksum or captured-pixel assertions.
        let scene_path = project.join("room.scene.yaml");
        let original = fs::read_to_string(&scene_path).unwrap();
        let canonical = PreparedScene::parse(&original).unwrap();
        let mut scene = Scene::parse(&original, &room_project::types()).unwrap();
        let mut changed = 0;
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
            let Value::Struct(fields) = &mut entity
                .components
                .iter_mut()
                .find(|(name, _)| name == "orr_physics3d::Body")
                .unwrap()
                .1
            else {
                panic!("Room Body must be reflected struct")
            };
            fields.iter_mut().find(|(name, _)| name == "pos").unwrap().1 =
                Value::Vec3(FPVec3::new(FP::from_int(x), FP::HALF, FP::from_int(-1)));
            changed += 1;
        }
        assert_eq!(changed, 3);
        fs::write(&scene_path, scene.to_yaml()).unwrap();
        assert_ne!(
            open(&project).scene().frame().checksum(),
            canonical.frame().checksum()
        );

        let mut originals = Vec::new();
        let mut copied = Vec::new();
        for (key, name) in [
            ("ORR_ROOM_RUNTIME", "room_escape"),
            ("ORR_ROOM_EXPORTER", "orr_export_room"),
        ] {
            let path = PathBuf::from(
                std::env::var_os(key).unwrap_or_else(|| panic!("acceptance requires {key}")),
            );
            assert!(
                path.is_absolute() && path.is_file(),
                "{key} must be an absolute built executable"
            );
            let path = path.canonicalize().unwrap();
            assert!(
                !path.starts_with(&base),
                "original tool cannot be inside fixture"
            );
            let target = base.join("tools").join(name);
            fs::copy(&path, &target).unwrap();
            assert_eq!(hash(&path), hash(&target));
            originals.push(path);
            copied.push(target);
        }
        Self {
            temp,
            project,
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
        // Check the actual namespace before exec, including original binaries
        // outside the workspace. Any failed hiding assertion exits with 91.
        if bundle.is_some() {
            command.args(["/bin/sh", "-c", "test ! -e \"$1\" && test ! -e \"$2\" && test ! -e \"$3\" && test ! -s \"$4\" && test ! -s \"$5\" || exit 91; shift 5; exec \"$@\"", "sh"])
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"))
                .arg(self.project.join("orr.project.json"))
                .arg(&self.runtime).arg(&self.originals[0]).arg(&self.originals[1]);
        } else {
            command.args(["/bin/sh", "-c", "test ! -e \"$1\" && test ! -s \"$2\" && test ! -s \"$3\" || exit 91; shift 3; exec \"$@\"", "sh"])
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"))
                .arg(&self.originals[0]).arg(&self.originals[1]);
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
        let mut command = self.isolated(&exe, bundle);
        if bundle.is_none() {
            command.arg("--project").arg(&self.project);
        }
        command
            .args(["--headless", "--ticks", &ticks.to_string(), "--capture"])
            .arg(capture);
        if !held.is_empty() {
            command.args(["--hold", held]);
        }
        command
    }

    fn export_command(&self, destination: &Path) -> Command {
        let mut command = self.isolated(&self.exporter, None);
        command
            .arg("--project")
            .arg(&self.project)
            .arg("--runtime")
            .arg(&self.runtime)
            .args([
                "--runtime-sha256",
                &hash(&self.runtime),
                "--trusted-runtime",
                "--output",
            ])
            .arg(destination);
        command
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

fn changed_pixels(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

#[test]
#[ignore = "requires exact production tools, real GPU and source-hidden read-only Linux namespace"]
fn room_lighting_source_hidden_readonly_export() {
    lighting_export_case(ROOM_TEMPLATE, "static");
    #[cfg(feature = "room-character")]
    lighting_export_case(
        orr_sample::project_create::ROOM_CHARACTER_TEMPLATE,
        "character",
    );
}

fn lighting_export_case(template: &str, profile: &str) {
    let w = Work::new(template);
    let initial_project = open(&w.project);
    assert!(initial_project.lighting().is_none());
    let initial_frame = initial_project.scene().frame().to_bytes();
    let initial = initial_project.scene().frame().checksum();
    let installed_before = tree(&w.project.join(".orr"));
    let original_project = tree(&w.project);
    let models = fs::read(w.project.join("room.models.json")).unwrap();
    let camera = fs::read(w.project.join("room.camera.json")).unwrap();
    #[cfg(feature = "room-character")]
    assert_eq!(
        initial_project.character().is_some(),
        profile == "character"
    );

    let absent_capture = w.temp.path().join("captures/absent.png");
    let absent_output = good(
        w.runtime_command(None, 0, "", &absent_capture)
            .output()
            .unwrap(),
    );
    let absent_image = pixels(&absent_capture);
    assert!(absent_output.contains(&format!("room initial checksum: 0x{initial:016x}")));
    assert!(absent_output.contains("room capture adapter:"));
    assert_eq!(tree(&w.project), original_project);

    let manifest_path = w.project.join("orr.project.json");
    let original_manifest = fs::read(&manifest_path).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&original_manifest).unwrap();
    manifest["entry"]["lighting"] = "room.lighting.json".into();
    let declared_manifest = serde_json::to_vec_pretty(&manifest).unwrap();
    fs::write(&manifest_path, &declared_manifest).unwrap();
    let lighting_path = w.project.join("room.lighting.json");
    let enabled = Document::enabled_default();
    let mut moved = enabled.clone();
    moved.point_light.as_mut().unwrap().position = [-3.0, 2.0, 3.0];
    let mut baseline_outputs = BTreeMap::new();
    let mut initial_images = BTreeMap::new();
    let mut content_digests = BTreeMap::new();

    for (name, document) in [
        ("off", Document::default()),
        ("on", enabled.clone()),
        ("moved", moved),
    ] {
        let lighting_bytes = document.to_bytes().unwrap();
        assert_eq!(
            orr_render::PointLightSettings::from_bytes(&lighting_bytes).unwrap(),
            document.settings()
        );
        fs::write(&lighting_path, &lighting_bytes).unwrap();
        let project = open(&w.project);
        assert_eq!(project.lighting().unwrap().bytes, lighting_bytes);
        assert_eq!(project.point_light_settings(), document.settings());
        assert_eq!(project.scene().frame().to_bytes(), initial_frame);
        assert_eq!(project.scene().frame().checksum(), initial);
        let source_before = tree(&w.project);
        let staged = w.temp.path().join(format!("{name} staging export"));
        let export_output = good(w.export_command(&staged).output().unwrap());
        let bundle = w.temp.path().join(format!("relocated {name} Room"));
        fs::rename(staged, &bundle).unwrap();
        let bundle_before = tree(&bundle);
        assert_eq!(hash(&bundle.join("bin/room_escape")), hash(&w.runtime));
        assert_eq!(
            fs::read(bundle.join("project/room.lighting.json")).unwrap(),
            lighting_bytes
        );
        assert_eq!(
            fs::read(bundle.join("project/room.models.json")).unwrap(),
            models
        );
        assert_eq!(
            fs::read(bundle.join("project/room.camera.json")).unwrap(),
            camera
        );
        assert_eq!(tree(&bundle.join("project/.orr")), installed_before);
        #[cfg(feature = "room-character")]
        if profile == "character" {
            assert_eq!(
                fs::read(bundle.join("project/room.character.json")).unwrap(),
                fs::read(w.project.join("room.character.json")).unwrap()
            );
            assert!(tree(&bundle.join("project"))
                .keys()
                .any(|path| path.ends_with("courier.orrmodel.json")));
        }
        for (path, bytes) in tree(&bundle.join("project")) {
            assert_eq!(
                source_before.get(&path),
                Some(&bytes),
                "export changed {path}"
            );
        }
        let exported_manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(bundle.join("orr.export.json")).unwrap()).unwrap();
        let content_digest = exported_manifest["content_digest"]
            .as_str()
            .expect("export content digest");
        assert_eq!(content_digest.len(), 64);
        content_digests.insert(name, content_digest.to_string());

        for (state, ticks, held) in [
            ("initial", 0, ""),
            ("movement", 18, "right"),
            ("key", 18, "right,interact"),
        ] {
            let source_capture = w
                .temp
                .path()
                .join("captures")
                .join(format!("{name}-{state}-source.png"));
            let export_capture = w
                .temp
                .path()
                .join("captures")
                .join(format!("{name}-{state}-export.png"));
            let source_output = good(
                w.runtime_command(None, ticks, held, &source_capture)
                    .output()
                    .unwrap(),
            );
            let exported_output = good(
                w.runtime_command(Some(&bundle), ticks, held, &export_capture)
                    .output()
                    .unwrap(),
            );
            assert!(source_output.contains(&format!("room initial checksum: 0x{initial:016x}")));
            assert!(
                source_output.contains("room capture adapter:"),
                "GPU capture must really execute"
            );
            assert_eq!(
                source_output, exported_output,
                "production source/export state parity: {profile}/{name}/{state}"
            );
            assert_eq!(
                fs::read(&source_capture).unwrap(),
                fs::read(&export_capture).unwrap(),
                "exact exported PNG parity: {profile}/{name}/{state}"
            );
            if name == "off" {
                baseline_outputs.insert(state, source_output.clone());
            } else {
                assert_eq!(
                    baseline_outputs.get(state).unwrap(),
                    &source_output,
                    "presentation-only lighting must preserve initial and stepped checksums/state"
                );
            }
            let image = pixels(&source_capture);
            assert!(
                image
                    .chunks_exact(4)
                    .filter(|pixel| *pixel != &image[..4])
                    .count()
                    > 100,
                "capture must contain a rendered Room"
            );
            if state == "initial" {
                initial_images.insert(name, image);
            } else {
                assert_ne!(
                    image, initial_images[name],
                    "actual movement must remain visible"
                );
            }
            if state == "key" {
                assert!(
                    source_output.contains("room key: 1"),
                    "real key interaction must occur: {source_output}"
                );
            }
            fs::write(
                w.temp
                    .path()
                    .join("evidence")
                    .join(format!("{name}-{state}.log")),
                source_output,
            )
            .unwrap();
        }

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
            tree(&w.project),
            source_before,
            "runtime/exporter mutated authored source"
        );
        assert_eq!(
            tree(&bundle),
            bundle_before,
            "read-only exported bundle changed"
        );
        assert_eq!(
            tree(&w.project.join(".orr")),
            installed_before,
            "installed immutable bytes changed"
        );
        let evidence = w.temp.path().join("evidence").join(name);
        fs::create_dir(&evidence).unwrap();
        fs::write(evidence.join("export.log"), export_output).unwrap();
        fs::copy(
            bundle.join("orr.export.json"),
            evidence.join("orr.export.json"),
        )
        .unwrap();
        fs::copy(
            bundle.join("project/room.lighting.json"),
            evidence.join("room.lighting.json"),
        )
        .unwrap();
        // Keep fixture disk use bounded to one exported production binary.
        fs::remove_dir_all(bundle).unwrap();
    }

    assert_eq!(
        initial_images["off"], absent_image,
        "explicit off must preserve legacy pixels"
    );
    assert_eq!(baseline_outputs["initial"], absent_output);
    assert!(
        changed_pixels(&initial_images["off"], &initial_images["on"]) > 20,
        "enabled lighting must visibly change production pixels"
    );
    assert!(
        changed_pixels(&initial_images["on"], &initial_images["moved"]) > 20,
        "moving the light must visibly change production pixels"
    );
    for (left, right) in [("off", "on"), ("on", "moved"), ("off", "moved")] {
        assert_ne!(
            content_digests[left], content_digests[right],
            "authored lighting must participate in export identity"
        );
    }

    // All fail before capture/export publication; there is no silent default.
    for (malformed, diagnostic) in [
        (b"{malformed lighting".as_slice(), "lighting"),
        (br#"{"version":1}"#, "lighting"),
        (br#"{"version":2,"point_light":null}"#, "lighting"),
        (br#"{"version":1,"point_light":null,"shadow":true}"#, "lighting"),
        (br#"{"version":1,"point_light":{"position":[0,4,0],"color":[1,1,1],"intensity":3,"range":0}}"#, "point light"),
    ] {
        fs::write(&lighting_path, malformed).unwrap();
        let capture = w.temp.path().join("captures/rejected.png");
        let export = w.temp.path().join("rejected export");
        bad(w.runtime_command(None, 0, "", &capture).output().unwrap(), diagnostic);
        bad(w.export_command(&export).output().unwrap(), diagnostic);
        assert!(!capture.exists() && !export.exists());
    }
    fs::write(&lighting_path, enabled.to_bytes().unwrap()).unwrap();
    let restored_capture = w.temp.path().join("captures/reopened-on.png");
    good(
        w.runtime_command(None, 0, "", &restored_capture)
            .output()
            .unwrap(),
    );
    assert_eq!(
        pixels(&restored_capture),
        initial_images["on"],
        "reopen restores exact saved lighting pixels"
    );

    // Prove the generated PLAYER model itself contributes production pixels,
    // including the skinned courier when the character capability is selected.
    // A scene containing only procedural walls cannot satisfy this assertion.
    let player = initial_project
        .scene()
        .frame()
        .entities()
        .find(|&entity| {
            initial_project
                .scene()
                .frame()
                .get::<RoomActor>(entity)
                .is_some_and(|actor| actor.kind == PLAYER)
        })
        .unwrap();
    let player_guid = initial_project
        .scene()
        .index()
        .guid(player)
        .unwrap()
        .to_string();
    let mut hidden_models: orr_model_bindings::model_bindings::Document =
        serde_json::from_slice(&models).unwrap();
    hidden_models
        .bindings
        .get_mut(&player_guid)
        .unwrap()
        .transform
        .translation = [1000.0; 3];
    fs::write(
        w.project.join("room.models.json"),
        serde_json::to_vec(&hidden_models).unwrap(),
    )
    .unwrap();
    let hidden_capture = w.temp.path().join("captures/hidden-player-model.png");
    let hidden_output = good(
        w.runtime_command(None, 0, "", &hidden_capture)
            .output()
            .unwrap(),
    );
    assert_eq!(
        hidden_output, baseline_outputs["initial"],
        "model presentation cannot change state/checksum"
    );
    assert!(
        changed_pixels(&pixels(&hidden_capture), &initial_images["on"]) > 20,
        "generated PLAYER model must visibly contribute: {profile}"
    );
    fs::write(w.project.join("room.models.json"), &models).unwrap();

    assert_eq!(open(&w.project).scene().frame().to_bytes(), initial_frame);
    assert_eq!(tree(&w.project.join(".orr")), installed_before);
    assert_eq!(fs::read(&manifest_path).unwrap(), declared_manifest);
    assert_eq!(
        fs::read(w.project.join("room.models.json")).unwrap(),
        models
    );
    assert_eq!(
        fs::read(w.project.join("room.camera.json")).unwrap(),
        camera
    );
    // Only the explicitly authored manifest descriptor and new root sidecar
    // differ from the generated-and-edited fixture, including failed admissions.
    let mut restored_tree = tree(&w.project);
    restored_tree.remove("room.lighting.json");
    restored_tree.insert(
        "orr.project.json".into(),
        original_project["orr.project.json"].clone(),
    );
    assert_eq!(restored_tree, original_project);

    if let Some(destination) = std::env::var_os("ORR_ROOM_LIGHTING_CAPTURE_DIR") {
        let destination = PathBuf::from(destination).join(profile);
        fs::create_dir_all(&destination).unwrap();
        copy_tree(
            &w.temp.path().join("captures"),
            &destination.join("captures"),
        );
        copy_tree(
            &w.temp.path().join("evidence"),
            &destination.join("evidence"),
        );
        copy_tree(&w.project, &destination.join("authored-project"));
    }
}
