use super::*;
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
};

fn key() -> CheckpointKey {
    let mut game_id = [0x12; 16];
    game_id[6] = 0x42;
    game_id[8] = 0x82;
    CheckpointKey {
        game_id,
        challenge: [0x34; 32],
    }
}
fn fixture() -> (tempfile::TempDir, CheckpointPaths) {
    let temp = tempfile::tempdir().unwrap();
    let paths = CheckpointPaths::from_directory(temp.path().join("checkpoint")).unwrap();
    (temp, paths)
}
fn bytes(collected: bool) -> Vec<u8> {
    serde_json::to_vec(&CheckpointV1::new(&key(), collected)).unwrap()
}
fn place(paths: &CheckpointPaths, bytes: &[u8]) {
    fs::create_dir_all(paths.directory()).unwrap();
    fs::write(paths.directory().join(PRIMARY_NAME), bytes).unwrap();
}
fn open(paths: &CheckpointPaths) -> CheckpointStore {
    CheckpointStore::open(paths.clone(), key()).unwrap()
}

#[test]
fn checkpoint_round_trip_and_durable_reset_are_not_max_merged() {
    let (_temp, paths) = fixture();
    let mut store = open(&paths);
    assert!(!store.key_collected());
    assert!(store.notice().is_none());
    assert_eq!(store.save_key().unwrap(), CommitOutcome::DurablySaved);
    assert!(store.key_collected());
    drop(store);
    let mut store = open(&paths);
    assert!(store.key_collected());
    assert_eq!(store.reset().unwrap(), CommitOutcome::DurablySaved);
    assert!(!store.key_collected());
    drop(store);
    let mut store = open(&paths);
    assert!(!store.key_collected());
    assert_eq!(store.save_key().unwrap(), CommitOutcome::DurablySaved);
}

#[test]
fn concurrent_session_is_readonly_even_after_owner_drops() {
    let (_temp, paths) = fixture();
    let mut owner = open(&paths);
    owner.save_key().unwrap();
    let mut contender = open(&paths);
    assert!(contender.key_collected());
    assert!(contender.notice().unwrap().contains("lock"));
    assert!(contender.reset().is_err());
    owner.reset().unwrap();
    drop(owner);
    assert!(contender.save_key().is_err());
    assert!(!open(&paths).key_collected());
}

#[test]
fn invalid_envelopes_are_preserved_by_save_and_reset() {
    let good = String::from_utf8(bytes(true)).unwrap();
    let invalid = vec![
        b"{".to_vec(),
        b"null".to_vec(),
        b"[]".to_vec(),
        serde_json::json!([1, key().game(), PROFILE, hex(&key().challenge), true])
            .to_string()
            .into_bytes(),
        good.replace("\"schema\":1", "\"schema\":2").into_bytes(),
        good.replace("\"schema\":1", "\"schema\":1,\"schema\":1")
            .into_bytes(),
        good.replace("\"schema\":1", "\"schema\":1,\"other\":true")
            .into_bytes(),
        good.replace("\"key_collected\":true", "\"key_collected\":1")
            .into_bytes(),
        good.replace("\"key_collected\":true", "\"key_collected\":null")
            .into_bytes(),
        good.replace(",\"key_collected\":true", "").into_bytes(),
        good.replace(PROFILE, "other-profile").into_bytes(),
        good.replace(&key().game(), "12121212-1212-1212-8212-121212121212")
            .into_bytes(),
        good.replace(&hex(&key().challenge), &hex(&[0x35; 32]))
            .into_bytes(),
        [bytes(true), b" {}".to_vec()].concat(),
        vec![b' '; MAX_CHECKPOINT_BYTES + 1],
        vec![0xff; 20],
    ];
    for original in invalid {
        let (_temp, paths) = fixture();
        place(&paths, &original);
        let mut store = open(&paths);
        assert!(!store.key_collected());
        assert!(store.notice().is_some());
        assert!(store.save_key().is_err());
        assert!(store.reset().is_err());
        assert_eq!(
            fs::read(paths.directory().join(PRIMARY_NAME)).unwrap(),
            original
        );
    }
}

#[test]
fn key_must_be_uuid_v4() {
    let (_temp, paths) = fixture();
    for index in [6, 8] {
        let mut invalid = key();
        invalid.game_id[index] = 0;
        assert!(CheckpointStore::open(paths.clone(), invalid).is_err());
    }
    assert!(!paths.directory().exists());
}

#[test]
fn environment_paths_are_external_partitioned_and_bounded() {
    let k = key();
    let paths =
        CheckpointPaths::from_environment_values(Some(OsStr::new("/data")), None, &k).unwrap();
    assert_eq!(
        paths.directory(),
        Path::new("/data/orrery/games")
            .join(k.game())
            .join(PROFILE)
            .join(hex(&k.challenge))
    );
    let fallback = CheckpointPaths::from_environment_values(
        Some(OsStr::new("relative")),
        Some(OsStr::new("/home/test")),
        &k,
    )
    .unwrap();
    assert!(fallback.directory().starts_with("/home/test/.local/share"));
    assert!(CheckpointPaths::from_environment_values(None, None, &k).is_err());
    for path in ["/", "relative", "/tmp/../bad", "/tmp/./bad", "/tmp/a\0b"] {
        assert!(CheckpointPaths::from_directory(path).is_err(), "{path:?}");
    }
    assert!(CheckpointPaths::from_directory(format!("/tmp/{}", "x".repeat(256))).is_err());
    assert!(CheckpointPaths::from_directory(format!("/{}", "x/".repeat(129))).is_err());
}

#[test]
fn slot_symlinks_and_hardlinks_are_preserved() {
    for hard in [false, true] {
        let (temp, paths) = fixture();
        fs::create_dir(paths.directory()).unwrap();
        let target = temp.path().join("target");
        fs::write(&target, bytes(true)).unwrap();
        let name = paths.directory().join(PRIMARY_NAME);
        if hard {
            fs::hard_link(&target, &name).unwrap();
        } else {
            symlink(&target, &name).unwrap();
        }
        let mut store = open(&paths);
        assert!(store.notice().is_some());
        assert!(store.save_key().is_err());
        assert!(store.reset().is_err());
        assert_eq!(fs::read(target).unwrap(), bytes(true));
    }
}

#[test]
fn ancestor_symlink_is_never_traversed() {
    let (temp, _) = fixture();
    let target = temp.path().join("target");
    fs::create_dir(&target).unwrap();
    let link = temp.path().join("link");
    symlink(&target, &link).unwrap();
    let paths = CheckpointPaths::from_directory(link.join("checkpoint")).unwrap();
    let mut store = open(&paths);
    assert!(store.notice().is_some());
    assert!(store.save_key().is_err());
    assert!(!target.join("checkpoint").exists());
}

#[test]
fn fifo_slot_and_lock_are_rejected_without_blocking() {
    use rustix::fs::{mkfifoat, Mode, CWD};
    for name in [PRIMARY_NAME, LOCK_NAME] {
        let (_temp, paths) = fixture();
        fs::create_dir(paths.directory()).unwrap();
        mkfifoat(CWD, paths.directory().join(name), Mode::RUSR | Mode::WUSR).unwrap();
        let mut store = open(&paths);
        assert!(store.notice().is_some());
        assert!(store.save_key().is_err());
    }
}

#[test]
fn lock_symlink_is_not_followed() {
    let (temp, paths) = fixture();
    fs::create_dir(paths.directory()).unwrap();
    let target = temp.path().join("target");
    fs::write(&target, b"keep").unwrap();
    symlink(&target, paths.directory().join(LOCK_NAME)).unwrap();
    let mut store = open(&paths);
    assert!(store.save_key().is_err());
    assert_eq!(fs::read(target).unwrap(), b"keep");
}

#[test]
fn readonly_and_shared_writable_paths_do_not_save_even_as_root() {
    for mode in [0o500, 0o777] {
        let (_temp, paths) = fixture();
        fs::create_dir(paths.directory()).unwrap();
        fs::set_permissions(paths.directory(), fs::Permissions::from_mode(mode)).unwrap();
        let mut store = open(&paths);
        assert!(store.save_key().is_err());
        fs::set_permissions(paths.directory(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let (_temp, paths) = fixture();
    place(&paths, &bytes(true));
    fs::set_permissions(
        paths.directory().join(PRIMARY_NAME),
        fs::Permissions::from_mode(0o400),
    )
    .unwrap();
    let mut store = open(&paths);
    assert!(store.key_collected());
    assert!(store.reset().is_err());
}

#[test]
fn failures_before_publication_preserve_old_state_and_cleanup_stage() {
    for point in [
        "primary-write",
        "primary-sync",
        "before-publication",
        "primary-rename",
    ] {
        let (_temp, paths) = fixture();
        let mut store = open(&paths);
        store.save_key().unwrap();
        let original = fs::read(paths.directory().join(PRIMARY_NAME)).unwrap();
        assert!(store
            .commit_with_checkpoint(false, |at| if at == point {
                Err("injected".into())
            } else {
                Ok(())
            })
            .is_err());
        assert!(store.key_collected());
        assert_eq!(
            fs::read(paths.directory().join(PRIMARY_NAME)).unwrap(),
            original
        );
        assert!(fs::read_dir(paths.directory()).unwrap().all(|e| {
            !e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".checkpoint-stage")
        }));
    }
}

#[test]
fn publication_without_confirmed_fsync_is_uncertain_and_disables_retries() {
    for point in ["after-publication", "directory-sync"] {
        let (_temp, paths) = fixture();
        let mut store = open(&paths);
        store.save_key().unwrap();
        let outcome = store
            .commit_with_checkpoint(false, |at| {
                if at == point {
                    Err("injected".into())
                } else {
                    Ok(())
                }
            })
            .unwrap();
        assert!(matches!(outcome, CommitOutcome::DurabilityUncertain(_)));
        assert!(!store.key_collected());
        assert!(store.notice().unwrap().contains("do not retry"));
        assert!(store.save_key().is_err());
        let value: CheckpointV1 =
            serde_json::from_slice(&fs::read(paths.directory().join(PRIMARY_NAME)).unwrap())
                .unwrap();
        assert!(!value.key_collected);
        let mut contender = open(&paths);
        assert!(contender.reset().is_err());
        drop(store);
        assert!(!open(&paths).key_collected());
    }
}

#[test]
fn future_state_arriving_during_staging_is_never_replaced() {
    let (_temp, paths) = fixture();
    let mut store = open(&paths);
    let future = String::from_utf8(bytes(true))
        .unwrap()
        .replace("\"schema\":1", "\"schema\":2")
        .into_bytes();
    assert!(store
        .commit_with_checkpoint(true, |point| {
            if point == "before-publication" {
                fs::write(paths.directory().join(PRIMARY_NAME), &future).unwrap();
            }
            Ok(())
        })
        .is_err());
    assert_eq!(
        fs::read(paths.directory().join(PRIMARY_NAME)).unwrap(),
        future
    );
}

#[test]
fn replaced_session_directory_does_not_receive_checkpoint() {
    let (temp, paths) = fixture();
    let mut store = open(&paths);
    let old = temp.path().join("old");
    fs::rename(paths.directory(), &old).unwrap();
    fs::create_dir(paths.directory()).unwrap();
    assert!(store.save_key().is_err());
    assert!(!old.join(PRIMARY_NAME).exists());
    assert!(!paths.directory().join(PRIMARY_NAME).exists());
}

#[test]
fn replaced_lock_inode_prevents_commit() {
    let (_temp, paths) = fixture();
    let mut store = open(&paths);
    fs::rename(
        paths.directory().join(LOCK_NAME),
        paths.directory().join("old.lock"),
    )
    .unwrap();
    fs::write(paths.directory().join(LOCK_NAME), b"").unwrap();
    assert!(store.save_key().is_err());
    assert!(!paths.directory().join(PRIMARY_NAME).exists());
}

#[test]
fn change_after_publication_is_reported_as_uncertain() {
    let (_temp, paths) = fixture();
    let mut store = open(&paths);
    let future = String::from_utf8(bytes(true))
        .unwrap()
        .replace("\"schema\":1", "\"schema\":2")
        .into_bytes();
    let outcome = store
        .commit_with_checkpoint(true, |point| {
            if point == "after-publication" {
                fs::write(paths.directory().join(PRIMARY_NAME), &future).unwrap();
            }
            Ok(())
        })
        .unwrap();
    assert!(matches!(outcome, CommitOutcome::DurabilityUncertain(_)));
    assert!(store.reset().is_err());
    assert_eq!(
        fs::read(paths.directory().join(PRIMARY_NAME)).unwrap(),
        future
    );
}

#[test]
fn readonly_directory_can_resume_without_creating_or_mutating_files() {
    let (_temp, paths) = fixture();
    place(&paths, &bytes(true));
    fs::set_permissions(paths.directory(), fs::Permissions::from_mode(0o500)).unwrap();
    let mut store = open(&paths);
    assert!(store.key_collected());
    assert!(store.notice().unwrap().contains("read-only"));
    assert!(store.reset().is_err());
    assert!(!paths.directory().join(LOCK_NAME).exists());
    assert_eq!(
        fs::read(paths.directory().join(PRIMARY_NAME)).unwrap(),
        bytes(true)
    );
    fs::set_permissions(paths.directory(), fs::Permissions::from_mode(0o700)).unwrap();
}
