//! Generate a new room before admission, edits, real runtime and relocated export.
//! This covers the closed room loop; it does not claim general camera/HUD/high-score support.
//! ORR_ROOM_CREATOR=/absolute/orr_new_arena
//! ORR_ROOM_RUNTIME=/absolute/room_escape ORR_ROOM_EXPORTER=/absolute/orr_export_room
//! ORR_REQUIRE_GPU=1 ORR_REQUIRE_PROJECT_ISOLATION=1 cargo test -p orr_sample
//! --features room-project,project-create,project-export --test new_room_project -- --ignored --exact
//! room::generated_room_real_export_source_hidden_gpu_workflow
#![cfg(all(
    feature = "project-create",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types)]

#[cfg(not(feature = "room-project"))]
#[test]
fn room_template_requires_explicit_feature() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().canonicalize().unwrap().join("not published");
    let options = orr_sample::project_create::CreateOptions {
        output: target.clone(),
        template: "room-escape-3d-v1".into(),
        seed: "disabled-room".into(),
    };
    assert!(orr_sample::project_create::create(&options).is_err());
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_orr_new_arena"))
        .arg("--output")
        .arg(&target)
        .args(["--template", "room-escape-3d-v1", "--seed", "disabled-room"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!target.exists());
}

#[cfg(feature = "room-project")]
mod room {
    use orr_fp::{FP, FPVec3};
    use orr_model_bindings::model_bindings::Document;
    use orr_reflect::{Scene, Value};
    use orr_sample::{
        project_create::{CreateOptions, create},
        room_game::{EXIT, KEY, PLAYER},
        room_project::{self, PreparedProject, PreparedScene},
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

    #[test]
    fn generated_room_cli_api_determinism_complete_package_and_no_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let cwd = base.join("empty cwd");
        fs::create_dir(&cwd).unwrap();
        let cli = base.join("CLI generated room");
        good(
            Command::new(env!("CARGO_BIN_EXE_orr_new_arena"))
                .current_dir(&cwd)
                .arg("--output")
                .arg(&cli)
                .args([
                    "--template",
                    "room-escape-3d-v1",
                    "--seed",
                    "room-acceptance",
                ])
                .output()
                .unwrap(),
        );
        let options = CreateOptions {
            output: base.join("API generated room"),
            template: "room-escape-3d-v1".into(),
            seed: "room-acceptance".into(),
        };
        let report = create(&options).unwrap();
        assert_eq!(tree(&cli), tree(&report.output), "CLI/API byte determinism");
        assert_eq!(report.template, options.template);
        assert_eq!(report.seed, options.seed);
        assert_eq!(report.entity_guids.len(), 7);
        let prepared = PreparedProject::open(&cli).unwrap();
        assert_eq!(prepared.scene().frame().checksum(), report.initial_checksum);
        for guid in &report.entity_guids {
            assert!(prepared.scene().index().entity(guid).is_some());
        }
        assert_eq!(prepared.models().document.bindings.len(), 2);
        let frame_before = prepared.scene().frame().to_bytes();
        let placements = orr_sample::room_view::placements(
            orr_bridge::FrameView::of(prepared.scene().frame()),
            prepared.scene().index(),
            prepared.models(),
        )
        .unwrap();
        assert_eq!(placements.len(), 2);
        assert!(
            placements
                .iter()
                .flat_map(|placement| &placement.model.source().primitives)
                .flat_map(|primitive| &primitive.vertices)
                .any(|vertex| vertex.uv[0] > 0.0 || vertex.uv[1] > 0.0),
            "real imported UV vertices"
        );
        assert_eq!(
            prepared.scene().frame().to_bytes(),
            frame_before,
            "presentation stays read-only"
        );
        let installed = tree(&cli);
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
        for name in [
            "orr.package.json",
            "foreground.glb",
            "background.glb",
            "LICENSE.txt",
            "generate.py",
        ] {
            let matches: Vec<_> = installed
                .iter()
                .filter(|(path, _)| {
                    path.starts_with(".orr/packages/objects/")
                        && Path::new(path).file_name().unwrap() == name
                })
                .collect();
            assert_eq!(
                matches.len(),
                1,
                "exactly one installed source-closure file: {name}"
            );
            if name == "orr.package.json" {
                let installed_manifest: orr_package::Manifest =
                    serde_json::from_slice(&fs::read(cli.join(matches[0].0)).unwrap()).unwrap();
                let source_manifest: orr_package::Manifest =
                    serde_json::from_slice(&fs::read(source.join(name)).unwrap()).unwrap();
                assert_eq!(
                    serde_json::to_value(installed_manifest).unwrap(),
                    serde_json::to_value(source_manifest).unwrap(),
                    "canonical package manifest"
                );
            } else {
                assert_eq!(
                    matches[0].1.1,
                    hash(&source.join(name)),
                    "full package bytes: {name}"
                );
            }
        }
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(cli.join("orr.project.json")).unwrap()).unwrap();
        assert_eq!(manifest["schema"], 2);
        assert_eq!(manifest["entry"]["scene"], "room.scene.yaml");
        assert_eq!(manifest["entry"]["models"], "room.models.json");
        assert!(
            manifest.get("progress").is_none(),
            "room template must not claim high-score persistence"
        );
        let other = create(&CreateOptions {
            output: base.join("different seed"),
            seed: "other-room-acceptance".into(),
            ..options.clone()
        })
        .unwrap();
        assert!(
            report
                .entity_guids
                .iter()
                .all(|guid| !other.entity_guids.contains(guid))
        );
        assert_eq!(
            other.initial_checksum, report.initial_checksum,
            "seed namespaces authored GUIDs without replacing room simulation"
        );
        let before = tree(&options.output);
        assert!(create(&options).is_err());
        assert_eq!(tree(&options.output), before);
        assert_eq!(fs::read_dir(&cwd).unwrap().count(), 0);
    }

    #[test]
    fn room_cli_rejects_progress_game_identity_without_publishing() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().canonicalize().unwrap().join("not published");
        let result = Command::new(env!("CARGO_BIN_EXE_orr_new_arena"))
            .arg("--output")
            .arg(&target)
            .args([
                "--template",
                "room-escape-3d-v1",
                "--seed",
                "rejected-game-id",
                "--game-id",
                "0c599a2a-6209-49e5-8b53-cb90f23e4955",
            ])
            .output()
            .unwrap();
        assert!(
            !result.status.success(),
            "Room must reject a progress identity"
        );
        assert!(!target.exists(), "rejected Room creation must not publish");
    }

    struct Work {
        temp: tempfile::TempDir,
        project: PathBuf,
        workspace: PathBuf,
        runtime: PathBuf,
        creator: PathBuf,
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
                .join("../..")
                .canonicalize()
                .unwrap();
            assert!(
                !base.starts_with(&workspace),
                "test fixture must live outside hidden workspace"
            );
            for name in [
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
            let mut originals = Vec::new();
            let mut copied = Vec::new();
            for (key, name) in [
                ("ORR_ROOM_RUNTIME", "room_escape"),
                ("ORR_ROOM_EXPORTER", "orr_export_room"),
                ("ORR_ROOM_CREATOR", "orr_new_arena"),
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
            let work = Self {
                temp,
                project: project.clone(),
                workspace,
                runtime: copied[0].clone(),
                exporter: copied[1].clone(),
                creator: copied[2].clone(),
                originals,
            };
            good(
                work.isolated(&work.creator, None)
                    .arg("--output")
                    .arg(&project)
                    .args([
                        "--template",
                        "room-escape-3d-v1",
                        "--seed",
                        "new-room-export-acceptance",
                    ])
                    .output()
                    .unwrap(),
            );
            assert!(
                fs::read_dir(work.temp.path().join("empty"))
                    .unwrap()
                    .next()
                    .is_none()
            );
            let original_yaml = fs::read_to_string(project.join("room.scene.yaml")).unwrap();
            let original = PreparedScene::parse(&original_yaml).unwrap();
            let mut scene = Scene::parse(&original_yaml, &room_project::types()).unwrap();
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
            let admitted = PreparedScene::parse(&yaml).unwrap();
            assert_ne!(admitted.frame().checksum(), original.frame().checksum());
            // Retain the creator's real installed package and GUID-keyed model sidecar.
            PreparedProject::open(&project).unwrap();
            work
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
            } else if self.project.exists() {
                command
                    .arg("--ro-bind")
                    .arg(&self.project)
                    .arg(&self.project);
            }
            command.arg("--");
            if bundle.is_some() {
                // Verify the namespace from inside it before launching production
                // code. A malformed hide setup fails instead of weakening proof.
                command.args(["/bin/sh", "-c", "test ! -e \"$1\" && test ! -e \"$2\" && test ! -e \"$3\" && test ! -s \"$4\" && test ! -s \"$5\" && test ! -s \"$6\" || exit 91; shift 6; exec \"$@\"", "sh"])
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"))
                .arg(self.project.join("orr.project.json"))
                .arg(&self.runtime)
                .arg(&self.originals[0])
                .arg(&self.originals[1])
                .arg(&self.originals[2]);
            } else {
                // The creator and source runtime both execute with the whole source
                // workspace and original executable paths verified inaccessible.
                command.args(["/bin/sh", "-c", "test ! -e \"$1\" && test ! -s \"$2\" && test ! -s \"$3\" && test ! -s \"$4\" || exit 91; shift 4; exec \"$@\"", "sh"])
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"))
                .arg(&self.originals[0])
                .arg(&self.originals[1])
                .arg(&self.originals[2]);
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
    fn generated_room_real_export_source_hidden_gpu_workflow() {
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
        assert!(
            exported_project
                .keys()
                .any(|p| p.ends_with("foreground.glb"))
        );
        assert!(
            exported_project
                .keys()
                .any(|p| p.ends_with("orr.package.json"))
        );
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
}
