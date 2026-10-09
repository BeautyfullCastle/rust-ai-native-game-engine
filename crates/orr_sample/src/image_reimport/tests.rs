use super::*;
use serde_json::{json, Value};

const PACKAGE: &str = "sample-sprites";
const DOCUMENT: &str = "sprites.json";
const IMAGE: &str = "lantern_keeper.png";
const RAW: &str = "lantern_keeper.rgba";
const LOCK: &str = "orr.packages.lock.json";

struct Fixture {
    temp: tempfile::TempDir,
    source: PathBuf,
    options: Options,
}
impl Fixture {
    fn new(collect: bool) -> Self {
        Self::custom(collect, |_| {})
    }
    fn custom(collect: bool, edit: impl FnOnce(&Path)) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let source = temp.path().join("sprite-source");
        fs::create_dir(&project).unwrap();
        copy_package(&assets().join("sprite_demo"), &source);
        edit(&source);
        let mut manifest = json!({"schema":2,"engine":"*","entry":{
            "game":if collect {"collect-dodge-v1"} else {"arena"},
            "scene":"scene.yaml","sprites":"sprites.json"}});
        if collect {
            manifest["schema"] = 3.into();
            manifest["progress"] = json!({"schema":1,"game_id":"12345678-1234-4234-8234-123456789abc","profile":"collect-dodge-highscore-v1"});
        }
        write_json(&project.join("orr.project.json"), &manifest);
        let scene: &[u8] = if collect {
            include_bytes!("../../../../scenes/collect_dodge_v1.scene.yaml")
        } else {
            include_bytes!("../../../../assets/saved_arena_project/arena.scene.yaml")
        };
        fs::write(project.join("scene.yaml"), scene).unwrap();
        write_json(
            &project.join("sprites.json"),
            &json!({"version":2,"scene":"scene.yaml","project":".","bindings":{
            "e_00000001":{"package":PACKAGE,"document":DOCUMENT,"source":{"Locomotion":{"idle":"idle","walk":"walk"}},"units_per_pixel":2.0}},"camera_follow":"e_00000001"}),
        );
        let image = temp.path().join("replacement.png");
        fs::write(&image, png(64, 16, false)).unwrap();
        let fixture = Self {
            temp,
            source,
            options: Options {
                project,
                package: PACKAGE.into(),
                document: DOCUMENT.into(),
                image,
                version: "1.0.1".into(),
                consumer: Consumer { collect, ui: false },
            },
        };
        fixture
            .manager()
            .install(std::slice::from_ref(&fixture.source))
            .unwrap();
        fixture
    }
    fn manager(&self) -> Project {
        let profile = if self.options.consumer.collect {
            Profile::Collect
        } else {
            Profile::Arena
        };
        Project::open(&self.options.project, profile.runtime()).unwrap()
    }
    fn lock_bytes(&self) -> Vec<u8> {
        fs::read(self.options.project.join(LOCK)).unwrap()
    }
    fn reject(&self) {
        let before = self.lock_bytes();
        assert!(prepare(&self.options).is_err());
        assert_eq!(self.lock_bytes(), before);
    }
}
fn assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}
fn copy_package(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}
fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn edit_manifest(source: &Path, edit: impl FnOnce(&mut Value)) {
    let path = source.join("orr.package.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    edit(&mut value);
    write_json(&path, &value);
}
fn png(width: u32, height: u32, animated: bool) -> Vec<u8> {
    let mut output = Vec::new();
    {
        let mut encoder = png17::Encoder::new(&mut output, width, height);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        if animated {
            encoder.set_animated(1, 0).unwrap();
        }
        let mut writer = encoder.write_header().unwrap();
        let pixels: Vec<_> = (0..width * height)
            .flat_map(|i| [(i % 251) as u8, 91, 203, 255])
            .collect();
        writer.write_image_data(&pixels).unwrap();
        writer.finish().unwrap();
    }
    output
}

#[test]
fn changed_pixels_commit_preserves_authored_identity_and_cache_outlives_stage() {
    for collect in [false, true] {
        let fixture = Fixture::new(collect);
        let original = fixture.manager().verify().unwrap();
        let old = &original.packages[PACKAGE];
        let old_object = fixture
            .options
            .project
            .join(".orr/packages/objects")
            .join(&old.digest);
        let protected: Vec<_> = [
            fixture.options.project.join("orr.project.json"),
            fixture.options.project.join("scene.yaml"),
            fixture.options.project.join("sprites.json"),
            fixture.source.join(IMAGE),
            old_object.join(IMAGE),
            old_object.join(RAW),
        ]
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
        assert_ne!(
            fs::read(fixture.source.join(IMAGE)).unwrap(),
            fs::read(&fixture.options.image).unwrap()
        );
        let transaction = prepare(&fixture.options).unwrap();
        assert_eq!(
            fixture.manager().verify().unwrap(),
            original,
            "prepare must not activate"
        );
        assert_eq!(
            transaction.scene_path(),
            fixture.options.project.join("scene.yaml")
        );
        assert_eq!(
            transaction.sidecar_path(),
            fixture.options.project.join("sprites.json")
        );
        let profile = if collect {
            Profile::Collect
        } else {
            Profile::Arena
        };
        assert_eq!(
            transaction.initial_checksum(),
            Prepared::open(&fixture.options.project, profile)
                .unwrap()
                .checksum()
        );
        let expected = decode_atlas(&fs::read(&fixture.options.image).unwrap(), 64, 16).unwrap();
        let cached_assets = transaction.assets().clone();
        let cached_document = transaction.document().clone();
        let checksum = transaction.initial_checksum();
        let candidate = transaction.candidate_lock().clone();
        assert_eq!(candidate.packages[PACKAGE].manifest.version, "1.0.1");
        assert_ne!(candidate.packages[PACKAGE].digest, old.digest);
        let stage_path = transaction._stage.path().to_path_buf();
        assert_eq!(transaction.commit().unwrap(), candidate);
        assert!(
            !stage_path.exists(),
            "transaction owns and removes its private stage"
        );
        assert_eq!(
            cached_assets[&(PACKAGE.into(), DOCUMENT.into())].rgba,
            expected
        );
        assert_eq!(
            cached_document,
            Document::from_bytes(&fs::read(fixture.options.project.join("sprites.json")).unwrap())
                .unwrap()
        );
        assert_eq!(
            fixture.manager().read_asset(PACKAGE, RAW).unwrap(),
            expected
        );
        assert_eq!(
            fixture.manager().read_asset(PACKAGE, IMAGE).unwrap(),
            fs::read(&fixture.options.image).unwrap()
        );
        assert_eq!(
            fixture.manager().read_asset(PACKAGE, DOCUMENT).unwrap(),
            fs::read(fixture.source.join(DOCUMENT)).unwrap()
        );
        assert_eq!(fixture.manager().verify().unwrap(), candidate);
        assert_eq!(
            Prepared::open(&fixture.options.project, profile)
                .unwrap()
                .checksum(),
            checksum
        );
        for (path, bytes) in protected {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
}

#[test]
fn drop_without_commit_leaves_lock_and_owned_preview_valid() {
    let fixture = Fixture::new(false);
    let before = fixture.lock_bytes();
    let transaction = prepare(&fixture.options).unwrap();
    let stage_path = transaction._stage.path().to_path_buf();
    let cache = transaction.assets().clone();
    drop(transaction);
    assert!(!stage_path.exists());
    assert_eq!(fixture.lock_bytes(), before);
    assert_eq!(
        cache[&(PACKAGE.into(), DOCUMENT.into())].rgba,
        decode_atlas(&fs::read(&fixture.options.image).unwrap(), 64, 16).unwrap()
    );
}

#[test]
fn malformed_truncated_wrong_size_and_animated_images_fail_without_activation() {
    let valid = png(64, 16, false);
    for image in [
        b"not an image".to_vec(),
        valid[..valid.len() / 2].to_vec(),
        png(32, 16, false),
        png(64, 16, true),
    ] {
        let fixture = Fixture::new(false);
        fs::write(&fixture.options.image, image).unwrap();
        fixture.reject();
    }
}

#[test]
fn versions_must_increase_semver_precedence_not_only_build_metadata() {
    for version in ["1.0.0", "0.9.9", "1.0.0+different", "1.0.0-rc.1", "banana"] {
        let mut fixture = Fixture::new(false);
        fixture.options.version = version.into();
        fixture.reject();
    }
}

#[test]
fn package_shape_rejects_extra_files_and_non_png_atlas() {
    let extra = Fixture::custom(false, |source| {
        fs::write(source.join("extra.txt"), b"extra").unwrap();
        edit_manifest(source, |value| {
            value["files"]
                .as_array_mut()
                .unwrap()
                .push("extra.txt".into())
        });
    });
    extra.reject();
    let non_png = Fixture::custom(false, |source| {
        fs::rename(source.join(IMAGE), source.join("atlas.bin")).unwrap();
        let document = source.join(DOCUMENT);
        let text = fs::read_to_string(&document)
            .unwrap()
            .replace(IMAGE, "atlas.bin");
        fs::write(document, text).unwrap();
        edit_manifest(source, |value| {
            for file in value["files"].as_array_mut().unwrap() {
                if file == IMAGE {
                    *file = "atlas.bin".into();
                }
            }
        });
    });
    non_png.reject();
}

#[test]
fn stale_authored_or_replacement_bytes_cannot_activate() {
    for target in [
        "orr.project.json",
        "scene.yaml",
        "sprites.json",
        "replacement",
        "active-image",
    ] {
        let fixture = Fixture::new(false);
        let transaction = prepare(&fixture.options).unwrap();
        let before = fixture.lock_bytes();
        let path = match target {
            "replacement" => fixture.options.image.clone(),
            "active-image" => fixture
                .options
                .project
                .join(".orr/packages/objects")
                .join(&fixture.manager().verify().unwrap().packages[PACKAGE].digest)
                .join(IMAGE),
            other => fixture.options.project.join(other),
        };
        let mut bytes = fs::read(&path).unwrap();
        bytes.push(b' ');
        fs::write(&path, &bytes).unwrap();
        assert!(transaction.commit().is_err(), "stale {target} must fail");
        assert_eq!(fixture.lock_bytes(), before);
        assert_eq!(
            fs::read(path).unwrap(),
            bytes,
            "must not roll back another writer"
        );
    }
}

#[test]
fn unrelated_package_lock_change_blocks_the_whole_lock_cas() {
    let fixture = Fixture::new(false);
    let transaction = prepare(&fixture.options).unwrap();
    let extra = fixture.temp.path().join("unrelated");
    fs::create_dir(&extra).unwrap();
    fs::write(extra.join("text.txt"), b"other content").unwrap();
    write_json(
        &extra.join("orr.package.json"),
        &json!({"schema":1,"name":"unrelated","version":"1.0.0","engine":"*","capabilities":[],"dependencies":{},"files":["text.txt"]}),
    );
    fixture.manager().install(&[extra]).unwrap();
    let intervening = fixture.lock_bytes();
    assert!(transaction.commit().is_err());
    assert_eq!(fixture.lock_bytes(), intervening);
    assert_eq!(
        fixture.manager().verify().unwrap().packages[PACKAGE]
            .manifest
            .version,
        "1.0.0"
    );
}

#[test]
fn packages_with_dependencies_fail_even_when_dependency_is_valid() {
    let mut fixture = Fixture::new(false);
    let dependency = fixture.temp.path().join("dependency");
    fs::create_dir(&dependency).unwrap();
    fs::write(dependency.join("text.txt"), b"dependency").unwrap();
    write_json(
        &dependency.join("orr.package.json"),
        &json!({"schema":1,"name":"dependency","version":"1.0.0","engine":"*","capabilities":[],"dependencies":{},"files":["text.txt"]}),
    );
    edit_manifest(&fixture.source, |value| {
        value["version"] = "1.0.1".into();
        value["dependencies"] = json!({"dependency":"1.0.0"});
    });
    fixture
        .manager()
        .install(&[fixture.source.clone(), dependency])
        .unwrap();
    fixture.options.version = "1.0.2".into();
    fixture.reject();
}

#[test]
fn same_layout_package_without_rgba_is_supported_without_adding_files() {
    let fixture = Fixture::custom(false, |source| {
        fs::remove_file(source.join(RAW)).unwrap();
        edit_manifest(source, |value| {
            value["files"]
                .as_array_mut()
                .unwrap()
                .retain(|file| file != RAW)
        });
    });
    let committed = prepare(&fixture.options).unwrap().commit().unwrap();
    assert!(!committed.packages[PACKAGE].manifest.files.contains(RAW));
    assert_eq!(committed.packages[PACKAGE].manifest.files.len(), 3);
}

#[cfg(feature = "game-ui")]
fn add_ui(fixture: &Fixture) {
    let path = fixture.options.project.join("orr.project.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut ui = json!({"profile": "arena-korean-v1", "font":{"package":"korean-game-ui","asset":"OrreryKoreanUI.otf"}});
    if fixture.options.consumer.collect {
        ui["profile"] = "collect-authored-v1".into();
        ui["document"] = "ui.json".into();
        #[cfg(feature = "collect-ui")]
        fs::write(
            fixture.options.project.join("ui.json"),
            crate::authored_ui::Document::default_collect()
                .to_bytes()
                .unwrap(),
        )
        .unwrap();
    }
    manifest["entry"]["ui"] = ui;
    write_json(&path, &manifest);
    fixture
        .manager()
        .install(&[assets().join("game_ui_font").canonicalize().unwrap()])
        .unwrap();
}

#[cfg(feature = "game-ui")]
#[test]
fn actual_ui_consumers_preserve_font_lock_and_reject_unsupported_route() {
    for collect in [false, true] {
        if collect && !cfg!(feature = "collect-ui") {
            continue;
        }
        let mut fixture = Fixture::new(collect);
        add_ui(&fixture);
        fixture.reject();
        fixture.options.consumer.ui = true;
        let original = fixture.manager().verify().unwrap();
        let font = fixture
            .manager()
            .read_asset("korean-game-ui", "OrreryKoreanUI.otf")
            .unwrap();
        let transaction = prepare(&fixture.options).unwrap();
        let candidate = transaction.commit().unwrap();
        assert_eq!(
            candidate.packages["korean-game-ui"],
            original.packages["korean-game-ui"]
        );
        assert_eq!(
            fixture
                .manager()
                .read_asset("korean-game-ui", "OrreryKoreanUI.otf")
                .unwrap(),
            font
        );
    }
}

#[cfg(feature = "game-ui")]
#[test]
fn stale_font_and_authored_ui_document_are_part_of_transaction_closure() {
    for target in ["font", "ui.json"] {
        if target == "ui.json" && !cfg!(feature = "collect-ui") {
            continue;
        }
        let mut fixture = Fixture::new(target == "ui.json");
        add_ui(&fixture);
        fixture.options.consumer.ui = true;
        let transaction = prepare(&fixture.options).unwrap();
        let before = fixture.lock_bytes();
        let path = if target == "font" {
            fixture
                .options
                .project
                .join(".orr/packages/objects")
                .join(&fixture.manager().verify().unwrap().packages["korean-game-ui"].digest)
                .join("OrreryKoreanUI.otf")
        } else {
            fixture.options.project.join(target)
        };
        let mut bytes = fs::read(&path).unwrap();
        bytes.push(b' ');
        fs::write(&path, &bytes).unwrap();
        assert!(transaction.commit().is_err());
        assert_eq!(fixture.lock_bytes(), before);
    }
}

/// Run explicitly with ORR_COLLECT_RUNTIME and ORR_COLLECT_EXPORTER naming
/// trusted production binaries built with collect-sprites and collect-progress.
/// Requires functional bubblewrap namespaces and a real software GPU; no fallback.
#[test]
#[ignore = "requires production Collect runtime/exporter, mandatory source hiding, read-only bundle and software GPU"]
fn reimport_real_pixels_export_relocates_with_sources_hidden_and_read_only() {
    use std::process::{Command, Output};
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
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                let kind = entry.file_type().unwrap();
                assert!(!kind.is_symlink());
                if kind.is_dir() {
                    visit(root, &entry.path(), out);
                } else {
                    assert!(kind.is_file());
                    out.insert(
                        entry.path().strip_prefix(root).unwrap().to_path_buf(),
                        fs::read(entry.path()).unwrap(),
                    );
                }
            }
        }
        let mut result = BTreeMap::new();
        visit(root, root, &mut result);
        result
    }
    fn run(runtime: &Path, project: &Path, capture: &Path, cwd: &Path) -> String {
        good(
            Command::new(runtime)
                .arg("--project")
                .arg(project)
                .args(["--headless", "--ticks", "0", "--capture"])
                .arg(capture)
                .current_dir(cwd)
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY")
                .output()
                .unwrap(),
        )
    }
    let fixture = Fixture::new(true);
    let base = fixture.temp.path().canonicalize().unwrap();
    let tools = base.join("trusted tools");
    let empty = base.join("empty cwd");
    fs::create_dir(&tools).unwrap();
    fs::create_dir(&empty).unwrap();
    let mut copied = Vec::new();
    for (key, name) in [
        ("ORR_COLLECT_RUNTIME", "collect_dodge"),
        ("ORR_COLLECT_EXPORTER", "orr_export_collect"),
    ] {
        let source = PathBuf::from(
            std::env::var_os(key).unwrap_or_else(|| panic!("mandatory acceptance needs {key}")),
        )
        .canonicalize()
        .unwrap();
        let destination = tools.join(name);
        fs::copy(source, &destination).unwrap();
        copied.push(destination);
    }
    let runtime = &copied[0];
    let exporter = &copied[1];
    let original_lock = fixture.manager().verify().unwrap();
    let old_object = fixture
        .options
        .project
        .join(".orr/packages/objects")
        .join(&original_lock.packages[PACKAGE].digest);
    let old_package = snapshot(&old_object);
    let source_package = snapshot(&fixture.source);
    let old_capture = base.join("before.png");
    let new_capture = base.join("after.png");
    let old_stdout = run(runtime, &fixture.options.project, &old_capture, &empty);
    assert!(old_stdout.contains("software: true"));
    let transaction = prepare(&fixture.options).unwrap();
    let expected_checksum = transaction.initial_checksum();
    transaction.commit().unwrap();
    fs::remove_file(&fixture.options.image).unwrap();
    assert!(!fixture.options.image.exists(), "replacement input is unavailable after commit");
    let new_stdout = run(runtime, &fixture.options.project, &new_capture, &empty);
    assert_eq!(
        old_stdout, new_stdout,
        "image reimport must preserve real runtime simulation output"
    );
    assert!(new_stdout.contains(&format!(
        "collect initial checksum: 0x{expected_checksum:016x}"
    )));
    assert_ne!(
        fs::read(&old_capture).unwrap(),
        fs::read(&new_capture).unwrap(),
        "actual compositor output must show the changed atlas"
    );
    assert_eq!(snapshot(&old_object), old_package);
    assert_eq!(snapshot(&fixture.source), source_package);
    let committed_project = snapshot(&fixture.options.project);
    let exported = base.join("original export");
    let runtime_hash = crate::project_export::admission::hash(&fs::read(runtime).unwrap());
    good(
        Command::new(exporter)
            .arg("--project")
            .arg(&fixture.options.project)
            .arg("--runtime")
            .arg(runtime)
            .args([
                "--runtime-sha256",
                &runtime_hash,
                "--trusted-runtime",
                "--output",
            ])
            .arg(&exported)
            .current_dir(&empty)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap(),
    );
    let relocated = base.join("relocated read only game");
    fs::rename(&exported, &relocated).unwrap();
    assert!(!exported.exists());
    let bundle_bytes = snapshot(&relocated);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let hidden = std::env::var_os("ORR_EXPORT_HIDE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository.parent().unwrap().to_path_buf())
        .canonicalize()
        .unwrap();
    assert!(repository.starts_with(&hidden));
    assert!(
        !base.starts_with(&hidden),
        "acceptance data must be outside hidden workspace"
    );
    let isolated_capture = base.join("relocated.png");
    let mut isolated = Command::new("bwrap");
    isolated.args(["--die-with-parent", "--ro-bind", "/", "/", "--dev-bind", "/dev/null", "/dev/null", "--bind"])
        .arg(&base).arg(&base)
        .arg("--tmpfs").arg(&hidden)
        .arg("--tmpfs").arg(&fixture.options.project)
        .arg("--tmpfs").arg(&fixture.source)
        .arg("--tmpfs").arg(&tools)
        .arg("--ro-bind").arg(&relocated).arg(&relocated)
        .args(["--", "/bin/sh", "-c", "test ! -e \"$1\" && test ! -e \"$2\" && test ! -e \"$3\" || exit 91; shift 3; exec \"$@\"", "sh"])
        .arg(repository.join("Cargo.toml"))
        .arg(fixture.options.project.join("orr.project.json"))
        .arg(runtime)
        .arg(relocated.join("run-collect-dodge"))
        .args(["--headless", "--ticks", "0", "--capture"]).arg(&isolated_capture)
        .current_dir(&empty).env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
    assert_eq!(
        new_stdout,
        good(
            isolated
                .output()
                .expect("mandatory acceptance requires bubblewrap")
        )
    );
    assert_eq!(
        fs::read(&new_capture).unwrap(),
        fs::read(&isolated_capture).unwrap()
    );
    assert_eq!(
        snapshot(&relocated),
        bundle_bytes,
        "read-only exported bundle must remain unchanged"
    );
    assert_eq!(snapshot(&fixture.options.project), committed_project);
    assert_eq!(snapshot(&old_object), old_package);
    if let Some(destination) = std::env::var_os("ORR_COLLECT_CAPTURES") {
        let destination = PathBuf::from(destination);
        fs::create_dir_all(&destination).unwrap();
        for (path, name) in [
            (&old_capture, "reimport-before.png"),
            (&new_capture, "reimport-after.png"),
            (&isolated_capture, "reimport-relocated.png"),
        ] {
            fs::copy(path, destination.join(name)).unwrap();
        }
    }
}
