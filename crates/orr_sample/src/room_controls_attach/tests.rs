use super::*;
use std::os::unix::fs::symlink;
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("한글 Room");
    crate::project_create::create(&crate::project_create::CreateOptions {
        output: root.clone(),
        template: crate::project_create::ROOM_TEMPLATE.into(),
        seed: "attach".into(),
    })
    .unwrap();
    fs::write(
        root.join("custom.json"),
        serde_json::to_vec_pretty(&crate::room_controls::RoomControls::default_map()).unwrap(),
    )
    .unwrap();
    (temp, root)
}
#[test]
fn attaches_preserving_manifest_semantics_and_exact_sidecar_and_backup() {
    let (_temp, root) = fixture();
    let before = fs::read(root.join(MANIFEST)).unwrap();
    let controls = fs::read(root.join("custom.json")).unwrap();
    let result = prepare(&root, "custom.json").unwrap().commit().unwrap();
    assert!(result.durability_warning.is_none());
    assert_eq!(fs::read(result.backup).unwrap(), before);
    let mut after: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(MANIFEST)).unwrap()).unwrap();
    assert_eq!(
        after["entry"]
            .as_object_mut()
            .unwrap()
            .remove("controls")
            .unwrap(),
        "custom.json"
    );
    assert_eq!(
        after,
        serde_json::from_slice::<serde_json::Value>(&before).unwrap()
    );
    assert_eq!(
        crate::room_project::PreparedProject::open(&root)
            .unwrap()
            .controls()
            .unwrap()
            .bytes,
        controls
    );
    assert_eq!(fs::read(root.join("custom.json")).unwrap(), controls);
    assert!(prepare(&root, "custom.json").is_err());
}
#[test]
fn rejects_null_duplicate_wrong_game_and_existing_controls_without_writes() {
    for mutation in ["null", "existing", "game", "duplicate", "schema"] {
        let (_temp, root) = fixture();
        let path = root.join(MANIFEST);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match mutation {
            "null" => value["entry"]["controls"] = serde_json::Value::Null,
            "existing" => value["entry"]["controls"] = "custom.json".into(),
            "game" => value["entry"]["game"] = "arena".into(),
            "schema" => value["schema"] = 3.into(),
            _ => {}
        }
        let mut bytes = serde_json::to_vec(&value).unwrap();
        if mutation == "duplicate" {
            bytes.splice(1..1, b"\"schema\":2,".iter().copied());
        }
        fs::write(&path, &bytes).unwrap();
        assert!(prepare(&root, "custom.json").is_err(), "{mutation}");
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert!(!root.join(BACKUP).exists());
    }
}
#[test]
fn rejects_unsafe_controls_and_aliases() {
    let (_temp, root) = fixture();
    for name in [
        "../custom.json",
        "/tmp/custom.json",
        "orr.project.json",
        "orr.packages.lock.json",
        "orr.project.before-controls.json",
        "room.models.json",
    ] {
        assert!(prepare(&root, name).is_err(), "{name}");
    }
    let controls = root.join("custom.json");
    let bytes = fs::read(&controls).unwrap();
    for invalid in [b"{}".to_vec(), vec![b' '; 65537]] {
        fs::write(&controls, invalid).unwrap();
        assert!(prepare(&root, "custom.json").is_err());
    }
    fs::remove_file(&controls).unwrap();
    let other = root.join("other.json");
    fs::write(&other, bytes).unwrap();
    symlink(&other, &controls).unwrap();
    assert!(prepare(&root, "custom.json").is_err());
    fs::remove_file(&controls).unwrap();
    fs::hard_link(&other, &controls).unwrap();
    assert!(prepare(&root, "custom.json").is_err());
    fs::remove_file(&controls).unwrap();
    rustix::fs::mkfifoat(rustix::fs::CWD, &controls, rustix::fs::Mode::RUSR).unwrap();
    assert!(prepare(&root, "custom.json").is_err());
}
#[test]
fn rejects_backup_collisions_preserving_everything() {
    for kind in ["file", "directory", "symlink"] {
        let (_temp, root) = fixture();
        let before = fs::read(root.join(MANIFEST)).unwrap();
        let backup = root.join(BACKUP);
        match kind {
            "file" => fs::write(&backup, b"prior backup").unwrap(),
            "directory" => fs::create_dir(&backup).unwrap(),
            _ => symlink(root.join(MANIFEST), &backup).unwrap(),
        }
        assert!(prepare(&root, "custom.json").unwrap().commit().is_err());
        assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), before);
        if kind == "file" {
            assert_eq!(fs::read(backup).unwrap(), b"prior backup");
        }
    }
}
#[test]
fn injected_prepublication_failures_keep_original_and_postpublication_is_explicit() {
    for failure in [
        "stage",
        "stage_sync",
        "backup_sync",
        "backup",
        "backup_directory_sync",
        "rename",
        "directory_sync",
    ] {
        let (_temp, root) = fixture();
        let before = fs::read(root.join(MANIFEST)).unwrap();
        let result = prepare(&root, "custom.json").unwrap().commit_with(|point| {
            if point == failure {
                Err(format!("injected {point}"))
            } else {
                Ok(())
            }
        });
        if failure == "directory_sync" {
            assert!(result.unwrap().durability_warning.is_some());
            assert_ne!(fs::read(root.join(MANIFEST)).unwrap(), before);
        } else {
            assert!(result.is_err());
            assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), before);
        }
        if matches!(
            failure,
            "backup_directory_sync" | "rename" | "directory_sync"
        ) {
            assert_eq!(fs::read(root.join(BACKUP)).unwrap(), before);
        } else {
            assert!(!root.join(BACKUP).exists());
        }
    }
}
#[test]
fn conflicts_in_manifest_controls_and_scene_prevent_publication() {
    for phase in ["conflict", "backup_directory_sync"] {
        for filename in [MANIFEST, "custom.json", "room.scene.yaml"] {
            let (_temp, root) = fixture();
            let before = fs::read(root.join(MANIFEST)).unwrap();
            let prepared = prepare(&root, "custom.json").unwrap();
            let changed = root.join(filename);
            let result = prepared.commit_with(|point| {
                if point == phase {
                    let mut bytes = fs::read(&changed).unwrap();
                    bytes.push(b' ');
                    fs::write(&changed, bytes).unwrap();
                }
                Ok(())
            });
            assert!(result.is_err(), "{phase}: {filename}");
            if phase == "backup_directory_sync" {
                assert_eq!(fs::read(root.join(BACKUP)).unwrap(), before);
            } else {
                assert!(!root.join(BACKUP).exists());
            }
            if filename != MANIFEST {
                assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), before);
            }
        }
    }
}

const REVISION_BACKUP: &str = "orr.project.before-controls-v2.json";
fn revision_fixture() -> (tempfile::TempDir, PathBuf) {
    let (temp, root) = fixture();
    prepare(&root, "custom.json").unwrap().commit().unwrap();
    let mut map = crate::room_controls::RoomControls::default_map();
    map.actions
        .iter_mut()
        .find(|a| a.name == "move_left")
        .unwrap()
        .bindings = vec![orr_input::Button::Keyboard {
        key: orr_input::Key::J,
    }];
    fs::write(root.join("v2.json"), serde_json::to_vec(&map).unwrap()).unwrap();
    (temp, root)
}
#[test]
fn explicit_revision_preserves_both_sidecars_and_every_version_backup() {
    let (_temp, root) = revision_fixture();
    let initial_backup = fs::read(root.join(BACKUP)).unwrap();
    let old = fs::read(root.join("custom.json")).unwrap();
    let new = fs::read(root.join("v2.json")).unwrap();
    let before = fs::read(root.join(MANIFEST)).unwrap();
    let old_checksum = crate::room_project::PreparedProject::open(&root)
        .unwrap()
        .scene()
        .frame()
        .checksum();
    let outcome = prepare_update(&root, "v2.json", "custom.json", REVISION_BACKUP)
        .unwrap()
        .commit()
        .unwrap();
    assert!(outcome.durability_warning.is_none());
    assert_eq!(fs::read(&outcome.backup).unwrap(), before);
    let mut expected: serde_json::Value = serde_json::from_slice(&before).unwrap();
    expected["entry"]["controls"] = "v2.json".into();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(root.join(MANIFEST)).unwrap())
            .unwrap(),
        expected
    );
    let admitted = crate::room_project::PreparedProject::open(&root).unwrap();
    assert_eq!(admitted.scene().frame().checksum(), old_checksum);
    let controls = admitted.controls().unwrap();
    assert_eq!(controls.bytes, new);
    assert_eq!(controls.map.input_from_keys([orr_input::Key::J]).move_x, -1);
    assert_eq!(controls.map.input_from_keys([orr_input::Key::A]).move_x, 0);
    let second_before = fs::read(root.join(MANIFEST)).unwrap();
    let second = prepare_update(
        &root,
        "custom.json",
        "v2.json",
        "orr.project.before-controls-v3.json",
    )
    .unwrap()
    .commit()
    .unwrap();
    assert_eq!(fs::read(second.backup).unwrap(), second_before);
    assert_eq!(fs::read(root.join(BACKUP)).unwrap(), initial_backup);
    assert_eq!(fs::read(outcome.backup).unwrap(), before);
    assert_eq!(fs::read(root.join("custom.json")).unwrap(), old);
    assert_eq!(fs::read(root.join("v2.json")).unwrap(), new);
}
#[test]
fn revision_rejects_stale_references_aliases_and_unsafe_names_without_writes() {
    let (_temp, root) = revision_fixture();
    let before = fs::read(root.join(MANIFEST)).unwrap();
    for (new, expected, backup) in [
        ("v2.json", "wrong.json", REVISION_BACKUP),
        ("v2.json", "CUSTOM.json", REVISION_BACKUP),
        ("custom.json", "custom.json", REVISION_BACKUP),
        ("CUSTOM.json", "custom.json", REVISION_BACKUP),
        ("room.models.json", "custom.json", REVISION_BACKUP),
        ("v2.json", "custom.json", "CUSTOM.json"),
        ("v2.json", "custom.json", "V2.json"),
        ("v2.json", "custom.json", "ROOM.MODELS.json"),
        ("v2.json", "custom.json", BACKUP),
        ("v2.json", "custom.json", MANIFEST),
        ("v2.json", "custom.json", "orr.packages.lock.json"),
        ("v2.json", "custom.json", "../backup.json"),
        ("v2.json", "../custom.json", REVISION_BACKUP),
        ("../v2.json", "custom.json", REVISION_BACKUP),
    ] {
        assert!(
            prepare_update(&root, new, expected, backup).is_err(),
            "{new} {expected} {backup}"
        );
        assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), before);
        assert!(!root.join(REVISION_BACKUP).exists());
    }
    assert!(prepare_update(
        &root,
        "v2.json",
        "custom.json",
        &format!("{}.json", "x".repeat(96))
    )
    .is_err());
}
#[test]
fn revision_rejects_missing_null_unsupported_and_corrupt_projects() {
    for mutation in [
        "missing",
        "null",
        "schema",
        "lighting",
        "character",
        "progress",
        "old",
        "new",
    ] {
        let (_temp, root) = revision_fixture();
        let path = root.join(MANIFEST);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match mutation {
            "missing" => {
                manifest["entry"]
                    .as_object_mut()
                    .unwrap()
                    .remove("controls");
            }
            "null" => manifest["entry"]["controls"] = serde_json::Value::Null,
            "schema" => manifest["schema"] = 3.into(),
            "lighting" | "character" => manifest["entry"][mutation] = "invalid.json".into(),
            "progress" => manifest["progress"] = serde_json::json!({"unsupported": true}),
            "old" => fs::write(root.join("custom.json"), b"{}").unwrap(),
            "new" => fs::write(root.join("v2.json"), b"{}").unwrap(),
            _ => unreachable!(),
        }
        let before = serde_json::to_vec(&manifest).unwrap();
        fs::write(&path, &before).unwrap();
        assert!(
            prepare_update(&root, "v2.json", "custom.json", REVISION_BACKUP).is_err(),
            "{mutation}"
        );
        assert_eq!(fs::read(path).unwrap(), before);
        assert!(!root.join(REVISION_BACKUP).exists());
    }
}
#[test]
fn revision_rejects_old_and_new_links_and_backup_collisions() {
    for name in ["custom.json", "v2.json"] {
        for kind in ["hardlink", "symlink"] {
            let (_temp, root) = revision_fixture();
            let before = fs::read(root.join(MANIFEST)).unwrap();
            let path = root.join(name);
            let other = root.join("aliased.json");
            fs::rename(&path, &other).unwrap();
            if kind == "hardlink" {
                fs::hard_link(&other, &path).unwrap();
            } else {
                symlink(&other, &path).unwrap();
            }
            assert!(
                prepare_update(&root, "v2.json", "custom.json", REVISION_BACKUP).is_err(),
                "{name}: {kind}"
            );
            assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), before);
            assert!(!root.join(REVISION_BACKUP).exists());
        }
    }
    for kind in ["file", "directory", "symlink"] {
        let (_temp, root) = revision_fixture();
        let before = fs::read(root.join(MANIFEST)).unwrap();
        let backup = root.join(REVISION_BACKUP);
        match kind {
            "file" => fs::write(&backup, b"older revision").unwrap(),
            "directory" => fs::create_dir(&backup).unwrap(),
            _ => symlink(root.join("custom.json"), &backup).unwrap(),
        }
        assert!(
            prepare_update(&root, "v2.json", "custom.json", REVISION_BACKUP)
                .unwrap()
                .commit()
                .is_err()
        );
        assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), before);
        if kind == "file" {
            assert_eq!(fs::read(backup).unwrap(), b"older revision");
        }
    }
}
#[test]
fn revision_detects_early_and_late_changes_to_both_sidecars_and_project() {
    for phase in ["conflict", "backup_directory_sync"] {
        for name in [MANIFEST, "custom.json", "v2.json", "room.scene.yaml"] {
            let (_temp, root) = revision_fixture();
            let before = fs::read(root.join(MANIFEST)).unwrap();
            let prepared =
                prepare_update(&root, "v2.json", "custom.json", REVISION_BACKUP).unwrap();
            let result = prepared.commit_with(|point| {
                if point == phase {
                    let mut bytes = fs::read(root.join(name)).unwrap();
                    bytes.push(b' ');
                    fs::write(root.join(name), bytes).unwrap();
                }
                Ok(())
            });
            assert!(result.is_err(), "{phase}: {name}");
            if name != MANIFEST {
                assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), before);
            }
            if phase == "backup_directory_sync" {
                assert_eq!(fs::read(root.join(REVISION_BACKUP)).unwrap(), before);
            } else {
                assert!(!root.join(REVISION_BACKUP).exists());
            }
        }
    }
}
#[test]
fn revision_failure_outcomes_preserve_data_and_uncertain_apply_rejects_retry() {
    for failure in [
        "stage",
        "stage_sync",
        "backup_sync",
        "backup",
        "backup_directory_sync",
        "rename",
        "directory_sync",
    ] {
        let (_temp, root) = revision_fixture();
        let before = fs::read(root.join(MANIFEST)).unwrap();
        let old = fs::read(root.join("custom.json")).unwrap();
        let new = fs::read(root.join("v2.json")).unwrap();
        let result = prepare_update(&root, "v2.json", "custom.json", REVISION_BACKUP)
            .unwrap()
            .commit_with(|point| {
                if point == failure {
                    Err(format!("injected {point}"))
                } else {
                    Ok(())
                }
            });
        if failure == "directory_sync" {
            assert!(result.unwrap().durability_warning.is_some());
            assert_eq!(
                crate::room_project::PreparedProject::open(&root)
                    .unwrap()
                    .controls()
                    .unwrap()
                    .bytes,
                new
            );
            assert!(prepare_update(&root, "v2.json", "custom.json", "retry.json").is_err());
            assert!(!root.join("retry.json").exists());
        } else {
            assert!(result.is_err());
            assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), before);
        }
        if matches!(
            failure,
            "backup_directory_sync" | "rename" | "directory_sync"
        ) {
            assert_eq!(fs::read(root.join(REVISION_BACKUP)).unwrap(), before);
        } else {
            assert!(!root.join(REVISION_BACKUP).exists());
        }
        assert_eq!(fs::read(root.join("custom.json")).unwrap(), old);
        assert_eq!(fs::read(root.join("v2.json")).unwrap(), new);
    }
}
