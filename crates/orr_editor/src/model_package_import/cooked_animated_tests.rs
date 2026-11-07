use super::*;

fn unopened_request() -> Request {
    Request {
        source: PathBuf::from("unopened-source.animated.json"),
        license: PathBuf::from("unopened-license"),
        destination: PathBuf::from("unopened-output"),
        name: "cooked-animation".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Animated,
        source_format: SourceFormat::CookedAnimated,
        license_acknowledged: true,
    }
}
#[test]
fn cooked_animation_requires_animated_kind_before_any_path_access() {
    let mut request = unopened_request();
    request.kind = ModelKind::Static;
    assert!(import_package(request)
        .err()
        .unwrap()
        .contains("requires Animated model mode"));
}
#[cfg(not(feature = "room-animated-models"))]
#[test]
fn cooked_animation_requires_feature_before_any_input_or_destination_access() {
    assert!(import_package(unopened_request())
        .err()
        .unwrap()
        .contains("requires room-animated-models"));
}
#[cfg(feature = "room-animated-models")]
#[test]
fn cooked_animation_decodes_complete_payload_and_requires_real_clips() {
    let bytes = include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
    validate_cooked_animated(bytes).unwrap();
    assert!(validate_cooked_animated(b"not json").is_err());
    let original: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let mut empty = original.clone();
    empty["clips"] = serde_json::json!([]);
    assert!(
        validate_cooked_animated(&serde_json::to_vec(&empty).unwrap())
            .unwrap_err()
            .contains("at least one animation clip")
    );
    let mut external = original.clone();
    external["images"][0] = serde_json::json!({"uri":"../unopened-image.png"});
    assert!(validate_cooked_animated(&serde_json::to_vec(&external).unwrap()).is_err());
    let mut bad_joint = original;
    bad_joint["primitives"][0]["vertices"][0]["joints"][0] = serde_json::json!(999);
    assert!(validate_cooked_animated(&serde_json::to_vec(&bad_joint).unwrap()).is_err());
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
mod publication {
    use super::*;
    const SOURCE: &[u8] =
        include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
    const LICENSE: &[u8] = b"Explicit user supplied license text\n";
    fn request(root: &Path, filename: &str, source: &[u8]) -> Request {
        let input = root.join("input");
        fs::create_dir_all(&input).unwrap();
        let path = input.join(filename);
        fs::write(&path, source).unwrap();
        let license = input.join("LICENSE.txt");
        fs::write(&license, LICENSE).unwrap();
        Request {
            source: path,
            license,
            destination: root.join("cooked-animation"),
            ..unopened_request()
        }
    }
    #[test]
    fn imports_verbatim_reproducible_self_contained_package_then_installs_and_loads_it() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let mut provenance: serde_json::Value = serde_json::from_slice(SOURCE).unwrap();
        provenance["dependencies"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"uri":"history/missing-source.glb","sha256":"a".repeat(64)}));
        let source = serde_json::to_vec(&provenance).unwrap();
        let first = request(&root, "Courier.ANIMATED.JSON", &source);
        let source_path = first.source.clone();
        let license_path = first.license.clone();
        let report = import_package(first).unwrap();
        assert_eq!(report.asset, "model.animated.json");
        assert_eq!(report.kind, ModelKind::Animated);
        assert_eq!(report.source_sha256, digest(&source));
        assert_eq!(report.license_sha256, digest(LICENSE));
        assert_eq!(fs::read(&source_path).unwrap(), source);
        assert_eq!(fs::read(&license_path).unwrap(), LICENSE);
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), source);
        assert_eq!(fs::read(report.root.join("LICENSE.txt")).unwrap(), LICENSE);
        let runtime = orr_sample::room_project::compiled_runtime();
        let inspected = orr_package::inspect(&report.root, &runtime).unwrap();
        assert_eq!(
            inspected.manifest.capabilities,
            BTreeSet::from(["models".into(), "animation".into()])
        );
        assert!(inspected.manifest.dependencies.is_empty());
        assert_eq!(
            inspected.manifest.files,
            BTreeSet::from(["model.animated.json".into(), "LICENSE.txt".into()])
        );
        let mut second = request(&root, "second.animated.json", &source);
        second.destination = root.join("second package");
        let second = import_package(second).unwrap();
        let inspected_second = orr_package::inspect(&second.root, &runtime).unwrap();
        assert_eq!(inspected.digest, inspected_second.digest);
        let project_root = root.join("project");
        fs::create_dir(&project_root).unwrap();
        fs::write(
            project_root.join("orr.project.json"),
            serde_json::to_vec(
                &serde_json::json!({"schema":1,"engine":format!("={}",runtime.engine_version)}),
            )
            .unwrap(),
        )
        .unwrap();
        let project = orr_package::Project::open(&project_root, runtime).unwrap();
        project.install(std::slice::from_ref(&report.root)).unwrap();
        project.verify().unwrap();
        fs::rename(&report.root, root.join("original package hidden")).unwrap();
        fs::remove_dir_all(root.join("input")).unwrap();
        let loaded = crate::model_bindings::load_asset_from_project(
            &project,
            "cooked-animation",
            "model.animated.json",
            ModelKind::Animated,
        )
        .unwrap();
        assert_eq!(loaded.source_hash(), digest(&source));
        let model = loaded.animated_model().unwrap();
        assert_eq!(model.source().clips.len(), 3);
        let pose = model.sample_clip(0, 0.5).unwrap();
        model.bounds(&pose).unwrap();
    }
    #[test]
    fn invalid_suffix_payload_license_and_existing_destination_never_replace_user_data() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        for name in ["source.json", "source.glb", "source.animated.json.bak"] {
            let request = request(&root, name, SOURCE);
            let output = request.destination.clone();
            assert!(import_package(request)
                .err()
                .unwrap()
                .contains(".animated.json"));
            assert!(!output.exists());
        }
        let invalid = request(&root, "source.animated.json", b"invalid json");
        let output = invalid.destination.clone();
        assert!(import_package(invalid).is_err());
        assert!(!output.exists());
        for license in [b" \n".as_slice(), [0xff].as_slice()] {
            let request = request(&root, "source.animated.json", SOURCE);
            fs::write(&request.license, license).unwrap();
            assert!(import_package(request).is_err());
            assert!(!output.exists());
        }
        let existing = request(&root, "source.animated.json", SOURCE);
        fs::create_dir(&existing.destination).unwrap();
        fs::write(existing.destination.join("sentinel"), b"existing package").unwrap();
        assert!(import_package(existing).is_err());
        assert_eq!(
            fs::read(output.join("sentinel")).unwrap(),
            b"existing package"
        );
        assert_eq!(fs::read_dir(output).unwrap().count(), 1);
        assert!(!fs::read_dir(&root).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".orr-model-import-")));
    }
    #[test]
    fn staged_cooked_payload_or_license_tamper_rejects_and_collision_preserves_foreign_output() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let stage = root.join("stage");
        fs::create_dir(&stage).unwrap();
        let destination = root.join("published");
        let manifest = serde_json::to_vec(
            &package_manifest(&unopened_request(), "model.animated.json").unwrap(),
        )
        .unwrap();
        for changed in ["model.animated.json", "LICENSE.txt"] {
            fs::write(stage.join("model.animated.json"), SOURCE).unwrap();
            fs::write(stage.join("LICENSE.txt"), LICENSE).unwrap();
            fs::write(stage.join("orr.package.json"), &manifest).unwrap();
            fs::write(stage.join(changed), b"tampered").unwrap();
            assert!(publish_directory(
                &stage,
                &destination,
                "model.animated.json",
                SOURCE,
                LICENSE,
                &manifest
            )
            .unwrap_err()
            .contains("changed before publication"));
            assert!(!destination.exists());
        }
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("sentinel"), b"other writer").unwrap();
        assert!(publish_directory(
            &stage,
            &destination,
            "model.animated.json",
            SOURCE,
            LICENSE,
            &manifest
        )
        .is_err());
        assert_eq!(
            fs::read(destination.join("sentinel")).unwrap(),
            b"other writer"
        );
        assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
    }
}
