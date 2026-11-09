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
