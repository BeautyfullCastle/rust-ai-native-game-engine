#![cfg(feature = "animated-models")]

use orr_editor::animated_bindings::{
    self, Binding, Bindings, Document, PlaybackMode, PlaybackSettings,
};
use orr_package::{Project, Runtime};
use orr_reflect::Guid;
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap()
}

fn project_runtime() -> Runtime {
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("animation".into());
    runtime
}

fn install(source: &Path, project_root: &Path) {
    let installer =
        Project::open_for_install(project_root, Runtime::content_only().engine_version).unwrap();
    installer.install(&[source.to_path_buf()]).unwrap();
}

fn copy_fixture(destination: &Path) -> PathBuf {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(fixture_dir()).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
    }
    destination.to_path_buf()
}

fn installed_asset(root: &Path) -> animated_bindings::LoadedAsset {
    animated_bindings::load_asset(root, "sample-animation", "animated.glb").unwrap()
}

#[test]
fn imports_verified_glb_cooks_reloads_and_lists_clip_slots() {
    let temp = tempfile::tempdir().unwrap();
    let source = fixture_dir();
    install(&source, temp.path());

    let loaded = installed_asset(temp.path());
    assert_eq!(loaded.package(), "sample-animation");
    assert_eq!(loaded.asset(), "animated.glb");
    assert_eq!(loaded.package_digest().len(), 64);
    assert_eq!(loaded.source_hash().len(), 64);
    assert!(!loaded.clips().is_empty());
    assert!(loaded
        .clips()
        .iter()
        .enumerate()
        .all(|(index, clip)| clip.index as usize == index && clip.duration.is_finite()));
    assert_eq!(loaded.model().source().clips.len(), loaded.clips().len());
    // A serialized cooked round-trip is also accepted by the same immutable loader.
    let cooked = loaded.model().to_bytes().unwrap();
    let reloaded = orr_model::animation::AnimatedModel::from_bytes(&cooked).unwrap();
    assert_eq!(reloaded.source().clips.len(), loaded.clips().len());
}

#[test]
fn assignment_is_guid_keyed_transactional_and_reopens_without_transient_history() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = installed_asset(temp.path());
    let path = temp.path().join("scene.animations.json");
    let mut bindings = Bindings::create(path.clone(), "scene.yaml".into(), ".".into()).unwrap();
    let ids = [
        Guid::parse("e_00000001").unwrap(),
        Guid::parse("e_00000002").unwrap(),
    ];
    let binding = Binding::from_asset(
        loaded.package().to_owned(),
        loaded.asset().to_owned(),
        &loaded,
        loaded.clips()[0].index,
        PlaybackSettings {
            mode: PlaybackMode::Loop,
            speed: 1.5,
        },
    )
    .unwrap();

    bindings.assign_validated(&ids, &binding, &loaded).unwrap();
    assert_eq!(bindings.document().bindings.len(), 2);
    bindings.undo();
    assert!(bindings.document().bindings.is_empty());
    bindings.redo();
    assert_eq!(bindings.document().bindings[&ids[0].to_string()], binding);
    bindings.save().unwrap();
    assert!(!bindings.dirty());

    let mut reopened = Bindings::open(path).unwrap();
    assert_eq!(reopened.document(), bindings.document());
    assert!(!reopened.dirty());
    reopened.undo();
    assert_eq!(reopened.document(), bindings.document());
    std::fs::write(temp.path().join("scene.yaml"), b"scene").unwrap();
    assert!(reopened.matches_scene(&temp.path().join("scene.yaml").display().to_string()));
}

#[test]
fn invalid_asset_identity_clip_and_stale_hash_fail_before_mutating_document() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = installed_asset(temp.path());
    let path = temp.path().join("scene.animations.json");
    let mut bindings = Bindings::create(path, "scene.yaml".into(), ".".into()).unwrap();
    let id = Guid::parse("e_00000001").unwrap();
    let before = bindings.document().clone();

    assert!(Binding::from_asset(
        "wrong-package".into(),
        loaded.asset().to_owned(),
        &loaded,
        0,
        PlaybackSettings::default(),
    )
    .is_err());
    assert!(Binding::from_asset(
        loaded.package().to_owned(),
        loaded.asset().to_owned(),
        &loaded,
        u32::MAX,
        PlaybackSettings::default(),
    )
    .is_err());

    let mut invalid_clip = Binding::from_asset(
        loaded.package().to_owned(),
        loaded.asset().to_owned(),
        &loaded,
        loaded.clips()[0].index,
        PlaybackSettings::default(),
    )
    .unwrap();
    invalid_clip.clip_index = u32::MAX;
    assert!(bindings
        .assign_validated(&[id], &invalid_clip, &loaded)
        .is_err());
    assert_eq!(bindings.document(), &before);

    let mut stale = invalid_clip.clone();
    stale.clip_index = loaded.clips()[0].index;
    stale.source_hash = "0".repeat(64);
    assert!(stale
        .validate(&loaded)
        .unwrap_err()
        .contains("stale animation binding"));
    stale.source_hash = loaded.source_hash().to_owned();
    stale.package_digest = "0".repeat(64);
    assert!(stale
        .validate(&loaded)
        .unwrap_err()
        .contains("package changed"));
    assert_eq!(bindings.document(), &before);
}

#[test]
fn remove_and_reinstall_reports_removed_and_stale_content_without_retargeting() {
    let temp = tempfile::tempdir().unwrap();
    let package_source = copy_fixture(&temp.path().join("package-source"));
    let root = temp.path().join("project");
    std::fs::create_dir(&root).unwrap();
    install(&package_source, &root);
    let original = installed_asset(&root);
    let binding = Binding::from_asset(
        original.package().to_owned(),
        original.asset().to_owned(),
        &original,
        original.clips()[0].index,
        PlaybackSettings::default(),
    )
    .unwrap();

    Project::open(&root, project_runtime())
        .unwrap()
        .remove("sample-animation")
        .unwrap();
    assert!(animated_bindings::load_binding(&root, &binding)
        .unwrap_err()
        .contains("package was removed"));

    install(&package_source, &root);
    let same = installed_asset(&root);
    assert_eq!(original.package_digest(), same.package_digest());
    binding.validate(&same).unwrap();

    std::fs::write(
        package_source.join("LICENSE.txt"),
        b"changed declared package content\n",
    )
    .unwrap();
    Project::open(&root, project_runtime())
        .unwrap()
        .remove("sample-animation")
        .unwrap();
    install(&package_source, &root);
    let changed = installed_asset(&root);
    assert_eq!(original.source_hash(), changed.source_hash());
    assert_ne!(original.package_digest(), changed.package_digest());
    assert!(binding
        .validate(&changed)
        .unwrap_err()
        .contains("package changed"));
}

#[test]
fn remove_is_one_undo_transaction_and_save_failure_keeps_dirty_state_and_history() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = installed_asset(temp.path());
    let path = temp.path().join("scene.animations.json");
    let mut bindings = Bindings::create(path.clone(), "scene.yaml".into(), ".".into()).unwrap();
    let ids = [
        Guid::parse("e_00000001").unwrap(),
        Guid::parse("e_00000002").unwrap(),
    ];
    let binding = Binding::from_asset(
        loaded.package().to_owned(),
        loaded.asset().to_owned(),
        &loaded,
        loaded.clips()[0].index,
        PlaybackSettings::default(),
    )
    .unwrap();
    bindings.assign_validated(&ids, &binding, &loaded).unwrap();
    bindings.remove(&ids).unwrap();
    assert!(bindings.document().bindings.is_empty());
    bindings.undo();
    assert_eq!(bindings.document().bindings.len(), 2);

    // Make atomic replacement fail at persist. This must not mark data clean or
    // consume the undo record, and the pre-existing destination object survives.
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("sentinel"), b"destination stays intact").unwrap();
    assert!(bindings.save().is_err());
    assert!(bindings.dirty());
    assert!(path.is_dir());
    assert_eq!(
        std::fs::read(path.join("sentinel")).unwrap(),
        b"destination stays intact"
    );
    bindings.undo();
    assert!(bindings.document().bindings.is_empty());
}

#[test]
fn oversized_save_preserves_existing_destination_dirty_state_and_redo_history() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = installed_asset(temp.path());
    let path = temp.path().join("scene.animations.json");
    let mut bindings = Bindings::create(path.clone(), "scene.yaml".into(), ".".into()).unwrap();
    bindings.save().unwrap();
    let baseline = std::fs::read(&path).unwrap();

    let binding = Binding::from_asset(
        loaded.package().to_owned(),
        loaded.asset().to_owned(),
        &loaded,
        loaded.clips()[0].index,
        PlaybackSettings::default(),
    )
    .unwrap();
    let guids = (0..4096_u32)
        .map(|n| Guid::parse(&format!("e_{n:08x}")).unwrap())
        .collect::<Vec<_>>();
    bindings
        .assign_validated(&guids, &binding, &loaded)
        .unwrap();
    let encoded = serde_json::to_vec_pretty(bindings.document()).unwrap();
    assert!(encoded.len() > 1024 * 1024);

    assert!(bindings.save().unwrap_err().contains("byte limit"));
    assert_eq!(std::fs::read(&path).unwrap(), baseline);
    assert!(bindings.dirty());

    bindings.undo();
    assert!(bindings.document().bindings.is_empty());
    assert!(!bindings.dirty());
    bindings.redo();
    assert_eq!(bindings.document().bindings.len(), 4096);
    assert!(bindings.dirty());
    assert_eq!(std::fs::read(&path).unwrap(), baseline);
}

#[cfg(unix)]
#[test]
fn sidecar_open_rejects_symlinks_and_fifo_without_waiting() {
    use std::os::unix::fs::symlink;
    use std::process::Command;

    let temp = tempfile::tempdir().unwrap();
    let regular = temp.path().join("regular.json");
    std::fs::write(
        &regular,
        br#"{"version":1,"scene":"scene.yaml","project":".","bindings":{}}"#,
    )
    .unwrap();
    let alias = temp.path().join("alias.json");
    symlink(&regular, &alias).unwrap();
    assert!(Bindings::open(alias)
        .err()
        .unwrap()
        .contains("regular file"));

    let fifo = temp.path().join("sidecar.fifo");
    let result = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(result.success());
    assert!(Bindings::open(fifo).err().unwrap().contains("regular file"));
}

#[test]
fn sidecar_parser_rejects_oversize_unknown_version_unknown_fields_and_bad_values() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bad.animations.json");
    let valid = serde_json::to_value(Document {
        version: 1,
        scene: "scene.yaml".into(),
        project: ".".into(),
        bindings: Default::default(),
    })
    .unwrap();

    std::fs::write(&path, vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert!(Bindings::open(path.clone())
        .err()
        .unwrap()
        .contains("byte limit"));

    let mutations: [fn(serde_json::Value) -> serde_json::Value; 6] = [
        |mut value: serde_json::Value| {
            value["extra"] = serde_json::json!(true);
            value
        },
        |mut value: serde_json::Value| {
            value["version"] = serde_json::json!(2);
            value
        },
        |mut value: serde_json::Value| {
            value["scene"] = serde_json::json!("/remote/scene.yaml");
            value
        },
        |mut value: serde_json::Value| {
            value["bindings"] = serde_json::json!({
                "e_not-a-guid": {
                    "package": "sample-animation", "asset": "animated.glb",
                    "package_digest": "a".repeat(64), "source_hash": "b".repeat(64),
                    "clip_index": 0,
                    "playback": {"mode": "loop", "speed": 1.0}
                }
            });
            value
        },
        |mut value: serde_json::Value| {
            value["bindings"] = serde_json::json!({
                "e_00000001": {
                    "package": "sample-animation", "asset": "../animated.glb",
                    "package_digest": "a".repeat(64), "source_hash": "b".repeat(64),
                    "clip_index": 0,
                    "playback": {"mode": "loop", "speed": 1.0}
                }
            });
            value
        },
        |mut value: serde_json::Value| {
            value["bindings"] = serde_json::json!({
                "e_00000001": {
                    "package": "sample-animation", "asset": "animated.glb",
                    "package_digest": "a".repeat(64), "source_hash": "b".repeat(64),
                    "clip_index": 0,
                    "playback": {"mode": "loop", "speed": 9.0}
                }
            });
            value
        },
    ];
    for mutate in mutations {
        std::fs::write(&path, serde_json::to_vec(&mutate(valid.clone())).unwrap()).unwrap();
        assert!(Bindings::open(path.clone()).is_err());
    }
    let duplicate = format!(
        r#"{{"version":1,"scene":"scene.yaml","project":".","bindings":{{
          "e_00000001":{{"package":"sample-animation","asset":"animated.glb","package_digest":"{}","source_hash":"{}","clip_index":0,"playback":{{"mode":"loop","speed":1.0}}}},
          "e_00000001":{{"package":"sample-animation","asset":"animated.glb","package_digest":"{}","source_hash":"{}","clip_index":0,"playback":{{"mode":"loop","speed":1.0}}}}
        }}}}"#,
        "a".repeat(64),
        "b".repeat(64),
        "a".repeat(64),
        "b".repeat(64),
    );
    std::fs::write(&path, duplicate).unwrap();
    assert!(Bindings::open(path.clone())
        .err()
        .unwrap()
        .to_string()
        .contains("duplicate animation binding GUID"));
    std::fs::write(
        &path,
        b"{\"version\":1,\"scene\":\"scene.yaml\",\"project\":\".\",\"bindings\":{},\"speed\":NaN}",
    )
    .unwrap();
    assert!(Bindings::open(path).is_err());
}

#[test]
fn assignment_history_is_bounded_and_invalid_paths_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = installed_asset(temp.path());
    assert!(Bindings::create(temp.path().join("bad"), "../scene.yaml".into(), ".".into()).is_ok());
    assert!(Bindings::create(
        temp.path().join("abs"),
        "scene.yaml".into(),
        "/remote".into()
    )
    .is_err());
    assert_eq!(
        animated_bindings::resolve_project(temp.path(), "..").unwrap(),
        temp.path().parent().unwrap()
    );
    assert!(animated_bindings::resolve_project(temp.path(), "/remote").is_err());

    let mut bindings = Bindings::create(
        temp.path().join("history.animations.json"),
        "scene.yaml".into(),
        ".".into(),
    )
    .unwrap();
    let binding = Binding::from_asset(
        loaded.package().to_owned(),
        loaded.asset().to_owned(),
        &loaded,
        loaded.clips()[0].index,
        PlaybackSettings::default(),
    )
    .unwrap();
    for n in 0..130_u32 {
        let guid = Guid::parse(&format!("e_{n:08x}")).unwrap();
        bindings
            .assign_validated(&[guid], &binding, &loaded)
            .unwrap();
    }
    for _ in 0..128 {
        bindings.undo();
    }
    assert_eq!(bindings.document().bindings.len(), 2);
}

#[test]
fn project_resolution_accepts_canonical_bases_and_checks_every_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let sidecars = root.join("sidecars");
    let project = root.join("project");
    std::fs::create_dir(&sidecars).unwrap();
    std::fs::create_dir(&project).unwrap();
    // On Windows canonicalize yields a verbatim path such as \\?\\C:\\... .
    // The prefix must not be queried as a bare directory before its RootDir.
    assert_eq!(
        animated_bindings::resolve_project(&sidecars, "../project").unwrap(),
        project
    );
    assert_eq!(
        animated_bindings::resolve_project(&root, ".").unwrap(),
        root
    );
    let volume_root = root.ancestors().last().unwrap();
    assert!(animated_bindings::resolve_project(volume_root, "..").is_err());
    std::fs::create_dir(project.join("nested")).unwrap();
    assert_eq!(
        animated_bindings::resolve_project(&sidecars, "../project/nested").unwrap(),
        project.join("nested")
    );
    assert!(animated_bindings::resolve_project(&sidecars, "../missing").is_err());
    std::fs::write(root.join("file"), b"not a directory").unwrap();
    assert!(animated_bindings::resolve_project(&sidecars, "../file").is_err());
    #[cfg(unix)]
    {
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&project, &alias).unwrap();
        assert!(animated_bindings::resolve_project(&sidecars, "../alias").is_err());
        assert!(animated_bindings::resolve_project(&alias, "../project").is_err());
    }
}
