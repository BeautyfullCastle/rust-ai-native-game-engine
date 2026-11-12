//! Independent failure/boundary acceptance: original and foreign data remain untouched.
use super::*;
fn append(path: &Path, text: &[u8]) {
    let mut bytes = fs::read(path).unwrap();
    bytes.extend_from_slice(text);
    fs::write(path, bytes).unwrap();
}
fn no_stages(root: &Path) {
    assert!(!fs::read_dir(root).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".orr-room-package-upgrade-")));
}
#[test]
fn stale_scene_models_lock_bytes_and_package_payload_refuse_without_writes() {
    for change in ["scene", "models", "lock", "source"] {
        let f = Fixture::new();
        let plan = plan_copy(&f.root, &f.source).unwrap();
        let path = match change {
            "scene" => f.root.join("room.scene.yaml"),
            "models" => f.root.join("room.models.json"),
            "lock" => f.root.join("orr.packages.lock.json"),
            _ => f.source.join("LICENSE.txt"),
        };
        append(&path, b"\n ");
        let before = files(f.temp.path());
        let output = f.output("stale-copy");
        assert!(apply_copy(&plan, &output, true).is_err(), "stale {change}");
        assert!(!output.exists());
        assert_eq!(files(f.temp.path()), before);
        no_stages(f.temp.path());
    }
}
#[test]
fn same_byte_source_directory_replacement_invalidates_observed_identity() {
    let f = Fixture::new();
    let plan = plan_copy(&f.root, &f.source).unwrap();
    let source = files(&f.source);
    let hidden = f.output("prior-source");
    fs::rename(&f.source, &hidden).unwrap();
    fs::create_dir(&f.source).unwrap();
    for (relative, bytes) in source {
        let path = f.source.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let before = files(f.temp.path());
    let output = f.output("identity-copy");
    assert!(apply_copy(&plan, &output, true).is_err());
    assert!(!output.exists());
    assert_eq!(files(f.temp.path()), before);
}
#[test]
fn foreign_output_and_completed_copy_are_never_replaced() {
    let f = Fixture::new();
    let plan = plan_copy(&f.root, &f.source).unwrap();
    let foreign = f.output("foreign");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("sentinel"), b"foreign user data").unwrap();
    let before = files(f.temp.path());
    assert!(apply_copy(&plan, &foreign, true).is_err());
    assert_eq!(files(f.temp.path()), before);
    let output = f.output("accepted-copy");
    apply_copy(&plan, &output, true).unwrap();
    let accepted = files(&output);
    assert!(apply_copy(&plan, &output, true).is_err());
    assert_eq!(files(&output), accepted);
    no_stages(f.temp.path());
}
#[test]
fn output_inside_either_input_or_non_normalized_path_is_refused_before_staging() {
    let f = Fixture::new();
    let plan = plan_copy(&f.root, &f.source).unwrap();
    let before = files(f.temp.path());
    for output in [
        f.root.join("copy"),
        f.source.join("copy"),
        f.root.clone(),
        f.source.clone(),
        PathBuf::from("relative-copy"),
        PathBuf::from(format!("{}/./copy", f.temp.path().display())),
    ] {
        assert!(
            apply_copy(&plan, &output, true).is_err(),
            "{}",
            output.display()
        );
        assert_eq!(files(f.temp.path()), before);
    }
    no_stages(f.temp.path());
}
#[test]
fn unchanged_asset_bytes_still_rebind_every_guid_to_new_package_identity() {
    let f = Fixture::new();
    fs::copy(f.v1.join(MODEL), f.source.join(MODEL)).unwrap();
    let original = files(&f.root);
    let old = open_room_project(&f.root).unwrap();
    let plan = plan_copy(&f.root, &f.source).unwrap();
    let output = f.output("same-payload-copy");
    let report = apply_copy(&plan, &output, true).unwrap();
    assert_eq!(report.bindings.len(), 2);
    let updated = open_room_project(&output).unwrap();
    assert_eq!(updated.models().document.bindings.len(), 2);
    for (guid, next) in &updated.models().document.bindings {
        let prior = &old.models().document.bindings[guid];
        assert_eq!(next.source_hash, prior.source_hash);
        assert_ne!(next.package_digest, prior.package_digest);
        assert_eq!(
            (
                next.kind,
                next.asset.as_str(),
                next.transform,
                next.animation
            ),
            (
                prior.kind,
                prior.asset.as_str(),
                prior.transform,
                prior.animation
            )
        );
        let asset = &updated.models().assets[&(TARGET.into(), MODEL.into())];
        next.validate(asset).unwrap();
    }
    assert!(report
        .bindings
        .iter()
        .all(|b| b.old_source_hash == b.new_source_hash
            && b.old_package_digest != b.new_package_digest));
    assert_eq!(files(&f.root), original);
}
#[test]
fn missing_malformed_and_animated_candidate_assets_never_publish() {
    for change in ["missing", "malformed", "animated"] {
        let f = Fixture::new();
        if change == "missing" {
            let path = f.source.join("orr.package.json");
            let mut manifest: Manifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            manifest.files.remove(MODEL);
            fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            fs::remove_file(f.source.join(MODEL)).unwrap();
        } else if change == "animated" {
            fs::write(
                f.source.join(MODEL),
                include_bytes!("../../../../../assets/room_character_demo/courier.orrmodel.json"),
            )
            .unwrap();
        } else {
            fs::write(f.source.join(MODEL), b"not a model").unwrap();
        }
        orr_package::inspect(&f.source, &room_runtime()).unwrap();
        let before = files(f.temp.path());
        let output = f.output("incompatible-copy");
        match plan_copy(&f.root, &f.source) {
            Ok(plan) => assert!(apply_copy(&plan, &output, true).is_err()),
            Err(_) => {}
        }
        assert!(!output.exists());
        assert_eq!(files(f.temp.path()), before);
        no_stages(f.temp.path());
    }
}
#[test]
fn progress_identity_projects_are_rejected_instead_of_duplicating_saved_identity() {
    let f = Fixture::new();
    let path = f.root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["schema"] = serde_json::json!(3);
    manifest["progress"] = serde_json::json!({"schema":1,"game_id":"6f5896ea-332c-4a6f-8715-6ba24b700918","profile":"room-escape-clear-v1"});
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let before = files(f.temp.path());
    assert!(plan_copy(&f.root, &f.source)
        .err()
        .unwrap()
        .contains("progress"));
    assert_eq!(files(f.temp.path()), before);
    no_stages(f.temp.path());
}
#[cfg(all(
    feature = "room-animated-models",
    not(feature = "package-upgrade-animated")
))]
#[test]
fn selected_animated_bindings_require_explicit_future_support() {
    let f = Fixture::new();
    let bytes = include_bytes!("../../../../../assets/room_character_demo/courier.orrmodel.json");
    for (path, version) in [(&f.v1, "3.0.0"), (&f.source, "4.0.0")] {
        fs::write(path.join(MODEL), bytes).unwrap();
        let file = path.join("orr.package.json");
        let mut manifest: Manifest = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        manifest.version = version.into();
        manifest.capabilities.insert("animation".into());
        fs::write(file, serde_json::to_vec(&manifest).unwrap()).unwrap();
    }
    Project::open(&f.root, room_runtime())
        .unwrap()
        .install(std::slice::from_ref(&f.v1))
        .unwrap();
    let project = Project::open(&f.root, room_runtime()).unwrap();
    let asset = model_bindings::load_asset_from_project(
        &project,
        TARGET,
        MODEL,
        model_bindings::ModelKind::Animated,
    )
    .unwrap();
    let path = f.root.join("room.models.json");
    let mut document: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for b in document.bindings.values_mut() {
        *b = Binding::from_animated_asset(
            TARGET.into(),
            MODEL.into(),
            &asset,
            model_bindings::AnimationDescriptor {
                clip_index: 0,
                playback: model_bindings::PlaybackMode::Loop,
            },
            b.transform,
        )
        .unwrap();
    }
    fs::write(path, serde_json::to_vec(&document).unwrap()).unwrap();
    open_room_project(&f.root).unwrap();
    let before = files(f.temp.path());
    assert!(plan_copy(&f.root, &f.source)
        .err()
        .unwrap()
        .contains("selected animated"));
    assert_eq!(files(f.temp.path()), before);
}
