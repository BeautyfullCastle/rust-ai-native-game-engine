use super::*;
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
};
fn key() -> RoomClearKey {
    let mut game_id = [7; 16];
    game_id[6] = 0x47;
    game_id[8] = 0x87;
    RoomClearKey {
        game_id,
        challenge: [3; 32],
    }
}
fn setup() -> (tempfile::TempDir, ProgressPaths) {
    let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let paths = ProgressPaths::from_directory(dir.path().join("save")).unwrap();
    (dir, paths)
}
fn encoded(cleared: bool) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&CompletionV1::new(&key(), cleared)).unwrap();
    bytes.push(b'\n');
    bytes
}
#[test]
fn room_bool_format_namespace_and_collect_bytes_remain_distinct() {
    let (dir, paths) = setup();
    let mut room = RoomClearStore::open(paths.clone(), key()).unwrap();
    assert!(!room.cleared());
    assert!(!paths.directory().exists());
    assert_eq!(room.mark_cleared().unwrap(), CommitOutcome::DurablySaved);
    assert!(room.cleared());
    assert_eq!(
        fs::read(paths.directory().join("completion.json")).unwrap(),
        encoded(true)
    );
    assert!(!paths.directory().join(PRIMARY_NAME).exists());
    let collect_key = ProgressKey {
        game_id: key().game_id,
        challenge: key().challenge,
        goal: 5,
    };
    let collect_paths =
        ProgressPaths::from_environment_values(Some(dir.path().as_os_str()), None, &collect_key)
            .unwrap();
    let room_paths = key()
        .paths_from_environment_values(Some(dir.path().as_os_str()), None)
        .unwrap();
    assert_ne!(collect_paths, room_paths);
    assert!(room_paths
        .directory()
        .to_string_lossy()
        .contains("/room-escape-clear-v1/"));
    let mut collect = ProgressStore::open(collect_paths.clone(), collect_key.clone()).unwrap();
    collect.submit_completed(3).unwrap();
    let expected = format!("{{\"version\":1,\"game_id\":\"{}\",\"profile\":\"collect-dodge-highscore-v1\",\"challenge\":\"{}\",\"best_collected\":3}}\n", collect_key.game(), hex(&collect_key.challenge));
    assert_eq!(
        fs::read(collect_paths.directory().join(PRIMARY_NAME)).unwrap(),
        expected.as_bytes()
    );
}
#[test]
fn room_rejects_invalid_identity_and_internal_values() {
    let (_, paths) = setup();
    let mut invalid = key();
    invalid.game_id = [0; 16];
    assert!(RoomClearStore::open(paths, invalid).is_err());
    assert!(StoreCodec::Room(&key()).merge(2, 1).is_err());
    assert!(StoreCodec::Room(&key()).encode(2).is_err());
    assert!(key().paths_from_environment_values(None, None).is_err());
}
#[test]
fn strict_bool_and_cross_profile_bytes_are_preserved_read_only() {
    for mode in 0..6 {
        let (_dir, paths) = setup();
        fs::create_dir(paths.directory()).unwrap();
        let mut value = serde_json::to_value(CompletionV1::new(&key(), true)).unwrap();
        match mode {
            0 => value["cleared"] = 1.into(),
            1 => {
                value.as_object_mut().unwrap().remove("cleared");
            }
            2 => value["best_collected"] = 1.into(),
            3 => value["profile"] = super::super::PROFILE.into(),
            4 => value["version"] = 2.into(),
            _ => value["challenge"] = "00".repeat(32).into(),
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        let path = paths.directory().join("completion.json");
        fs::write(&path, &bytes).unwrap();
        let mut store = RoomClearStore::open(paths.clone(), key()).unwrap();
        assert!(!store.cleared());
        assert!(store.notice().is_some());
        assert!(store.mark_cleared().is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}
#[test]
fn matching_backup_recovers_and_unsupported_backup_blocks_writes() {
    let (_dir, paths) = setup();
    fs::create_dir(paths.directory()).unwrap();
    fs::write(paths.directory().join("completion.json"), b"broken").unwrap();
    fs::write(paths.directory().join("completion.json.bak"), encoded(true)).unwrap();
    let mut store = RoomClearStore::open(paths.clone(), key()).unwrap();
    assert!(store.cleared());
    assert!(store.notice().unwrap().contains("backup"));
    store.mark_cleared().unwrap();
    assert_eq!(
        fs::read(paths.directory().join("completion.json.bak")).unwrap(),
        encoded(true)
    );
    let mut incompatible: serde_json::Value = serde_json::from_slice(&encoded(true)).unwrap();
    incompatible["profile"] = "collect-dodge-highscore-v1".into();
    fs::write(
        paths.directory().join("completion.json.bak"),
        serde_json::to_vec(&incompatible).unwrap(),
    )
    .unwrap();
    let mut reopened = RoomClearStore::open(paths, key()).unwrap();
    assert!(reopened.cleared());
    assert!(reopened.mark_cleared().is_err());
}
#[test]
fn stale_sessions_merge_clear_and_contended_lock_defers_without_overwrite() {
    use rustix::fs::{flock, FlockOperation};
    let (_dir, paths) = setup();
    let mut a = RoomClearStore::open(paths.clone(), key()).unwrap();
    let mut b = RoomClearStore::open(paths.clone(), key()).unwrap();
    a.mark_cleared().unwrap();
    b.mark_cleared().unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(paths.directory().join("completion.lock"))
        .unwrap();
    flock(&lock, FlockOperation::NonBlockingLockExclusive).unwrap();
    let before = fs::read(paths.directory().join("completion.json")).unwrap();
    let mut blocked = RoomClearStore::open(paths.clone(), key()).unwrap();
    assert!(!blocked.cleared());
    assert!(blocked.notice().unwrap().contains("deferred"));
    assert!(blocked.mark_cleared().is_err());
    assert_eq!(
        fs::read(paths.directory().join("completion.json")).unwrap(),
        before
    );
    drop(lock);
    blocked.mark_cleared().unwrap();
    assert!(blocked.cleared());
    // A false transport candidate cannot downgrade a concurrently saved clear.
    assert_eq!(
        linux::commit_codec(&paths, StoreCodec::Room(&key()), 0, |_| Ok(()))
            .unwrap()
            .0,
        1
    );
}
#[test]
fn failures_before_publication_preserve_and_after_publication_disable_retry() {
    for point in [
        "primary-sync",
        "primary-rename",
        "after-publication",
        "directory-sync",
    ] {
        let (_dir, paths) = setup();
        let mut store = RoomClearStore::open(paths.clone(), key()).unwrap();
        let result = store.mark_with_checkpoint(|phase| {
            if phase == point {
                Err("injected".into())
            } else {
                Ok(())
            }
        });
        if matches!(point, "after-publication" | "directory-sync") {
            assert!(matches!(
                result.unwrap(),
                CommitOutcome::DurabilityUncertain(_)
            ));
            assert!(store.cleared());
            assert!(store.mark_cleared().is_err());
            assert_eq!(
                fs::read(paths.directory().join("completion.json")).unwrap(),
                encoded(true)
            );
        } else {
            assert!(result.is_err());
            assert!(!paths.directory().join("completion.json").exists());
        }
        assert!(fs::read_dir(paths.directory()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".progress-stage-")));
    }
}
#[test]
fn symlink_and_readonly_files_never_become_write_authority() {
    let (dir, paths) = setup();
    fs::create_dir(paths.directory()).unwrap();
    let outside = dir.path().join("outside");
    fs::write(&outside, encoded(false)).unwrap();
    let primary = paths.directory().join("completion.json");
    symlink(&outside, &primary).unwrap();
    let mut store = RoomClearStore::open(paths.clone(), key()).unwrap();
    assert!(store.mark_cleared().is_err());
    assert_eq!(fs::read(&outside).unwrap(), encoded(false));
    fs::remove_file(&primary).unwrap();
    fs::write(&primary, encoded(false)).unwrap();
    fs::set_permissions(&primary, fs::Permissions::from_mode(0o400)).unwrap();
    let mut store = RoomClearStore::open(paths, key()).unwrap();
    assert!(store.mark_cleared().is_err());
    assert_eq!(fs::read(primary).unwrap(), encoded(false));
}

#[test]
fn duplicate_identity_and_room_record_in_collect_namespace_are_preserved() {
    let (_dir, paths) = setup();
    fs::create_dir(paths.directory()).unwrap();
    let bytes = String::from_utf8(encoded(true)).unwrap();
    let duplicate = bytes.replacen("\"version\":1", "\"version\":1,\"version\":1", 1);
    let primary = paths.directory().join("completion.json");
    fs::write(&primary, &duplicate).unwrap();
    let mut room = RoomClearStore::open(paths.clone(), key()).unwrap();
    assert!(room.mark_cleared().is_err());
    assert_eq!(fs::read(&primary).unwrap(), duplicate.as_bytes());
    fs::write(paths.directory().join(PRIMARY_NAME), encoded(true)).unwrap();
    let collect_key = ProgressKey {
        game_id: key().game_id,
        challenge: key().challenge,
        goal: 5,
    };
    let mut collect = ProgressStore::open(paths.clone(), collect_key).unwrap();
    assert!(collect.submit_completed(1).is_err());
    assert_eq!(
        fs::read(paths.directory().join(PRIMARY_NAME)).unwrap(),
        encoded(true)
    );
}

#[test]
fn room_lock_replacement_during_staging_refuses_publication() {
    let (_dir, paths) = setup();
    let mut store = RoomClearStore::open(paths.clone(), key()).unwrap();
    let result = store.mark_with_checkpoint(|phase| {
        if phase == "primary-sync" {
            let path = paths.directory().join("completion.lock");
            fs::rename(&path, paths.directory().join("retained-old-lock")).unwrap();
            fs::write(path, b"").unwrap();
        }
        Ok(())
    });
    assert!(result.unwrap_err().contains("lock changed"));
    assert!(!paths.directory().join("completion.json").exists());
}

#[test]
fn room_backup_publish_failure_is_uncertain_and_retains_primary_clear() {
    let (_dir, paths) = setup();
    fs::create_dir(paths.directory()).unwrap();
    fs::write(paths.directory().join("completion.json"), encoded(false)).unwrap();
    let mut store = RoomClearStore::open(paths.clone(), key()).unwrap();
    let result = store
        .mark_with_checkpoint(|phase| {
            if phase == "backup-rename" {
                Err("injected backup publication failure".into())
            } else {
                Ok(())
            }
        })
        .unwrap();
    assert!(matches!(result, CommitOutcome::DurabilityUncertain(_)));
    assert!(store.cleared());
    assert!(store.mark_cleared().is_err());
    assert_eq!(
        fs::read(paths.directory().join("completion.json")).unwrap(),
        encoded(true)
    );
}
