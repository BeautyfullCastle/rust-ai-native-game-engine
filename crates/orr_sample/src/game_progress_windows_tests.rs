//! The Windows progress transport: real files, real share-mode locks.
use super::*;
use std::fs;

fn key() -> ProgressKey {
    let mut id = [1; 16];
    id[6] = 0x41;
    id[8] = 0x81;
    ProgressKey {
        game_id: id,
        challenge: [3; 32],
        goal: 10,
    }
}

fn sandbox() -> (tempfile::TempDir, ProgressStore) {
    let root = tempfile::tempdir().unwrap();
    let paths = ProgressPaths::from_directory(root.path().join("progress")).unwrap();
    let store = ProgressStore::open(paths, key()).unwrap();
    (root, store)
}

fn reopen(store: &ProgressStore) -> ProgressStore {
    ProgressStore::open(store.paths.clone(), store.key.clone()).unwrap()
}

fn raw(score: u32) -> Vec<u8> {
    serde_json::to_vec(&ProgressV1::new(&key(), score)).unwrap()
}

fn names(store: &ProgressStore) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(store.paths.directory())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn loading_a_missing_store_writes_nothing() {
    let (_root, store) = sandbox();
    assert_eq!(store.best(), 0);
    assert!(store.notice().is_none());
    assert!(!store.paths.directory().exists());
}

#[test]
fn a_commit_publishes_both_copies_and_the_best_only_grows() {
    let (_root, mut store) = sandbox();
    assert_eq!(store.submit_completed(4).unwrap(), CommitOutcome::DurablySaved);
    assert_eq!(store.best(), 4);
    assert_eq!(names(&store), [PRIMARY_NAME, BACKUP_NAME, LOCK_NAME]);
    assert_eq!(reopen(&store).best(), 4);
    // A lower score keeps the best. A higher one replaces it.
    assert_eq!(store.submit_completed(2).unwrap(), CommitOutcome::DurablySaved);
    assert_eq!(reopen(&store).best(), 4);
    store.submit_completed(7).unwrap();
    assert_eq!(reopen(&store).best(), 7);
    // The old best stays in the backup until the next commit.
    let backup = fs::read(store.paths.directory().join(BACKUP_NAME)).unwrap();
    assert_eq!(backup, [raw(4).as_slice(), b"\n"].concat());
    assert!(store.submit_completed(11).is_err(), "goal is 10");
    assert!(!names(&store).iter().any(|name| name.starts_with(".progress-stage-")));
}

#[test]
fn a_corrupt_primary_is_recovered_from_the_backup() {
    let (_root, mut store) = sandbox();
    store.submit_completed(5).unwrap();
    fs::write(store.paths.directory().join(PRIMARY_NAME), b"{ not json").unwrap();
    let recovered = reopen(&store);
    assert_eq!(recovered.best(), 5);
    assert!(recovered.notice().unwrap().contains("recovered matching backup"));
    let mut recovered = recovered;
    recovered.submit_completed(6).unwrap();
    assert_eq!(reopen(&store).best(), 6);
    assert!(reopen(&store).notice().is_none());
}

#[test]
fn bytes_from_a_newer_version_are_preserved_read_only() {
    let (_root, mut store) = sandbox();
    store.submit_completed(3).unwrap();
    let future = br#"{"version":2,"best_collected":9}"#;
    fs::write(store.paths.directory().join(PRIMARY_NAME), future).unwrap();
    let mut loaded = reopen(&store);
    assert!(loaded.notice().unwrap().contains("unsupported progress version"));
    assert!(loaded.submit_completed(8).is_err());
    assert_eq!(fs::read(store.paths.directory().join(PRIMARY_NAME)).unwrap(), future);
}

#[test]
fn a_held_writer_lock_defers_loading_and_refuses_commits() {
    let (_root, mut store) = sandbox();
    store.submit_completed(3).unwrap();
    let directory = store.paths.directory().to_path_buf();
    // Another process that writes holds the lock file with no sharing at all.
    let writer = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(directory.join(LOCK_NAME))
            .unwrap()
    };
    let busy = reopen(&store);
    assert!(busy.notice().unwrap().contains("another writer holds the lock"));
    let error = store.submit_completed(5).unwrap_err();
    assert!(error.contains("locked or locking is unavailable"), "{error}");
    drop(writer);
    assert_eq!(reopen(&store).best(), 3);
    store.submit_completed(5).unwrap();
    assert_eq!(reopen(&store).best(), 5);
}

#[test]
fn a_failure_before_publication_changes_nothing_and_leaves_no_stage() {
    let (_root, mut store) = sandbox();
    store.submit_completed(3).unwrap();
    let before = (
        fs::read(store.paths.directory().join(PRIMARY_NAME)).unwrap(),
        fs::read(store.paths.directory().join(BACKUP_NAME)).unwrap(),
    );
    let error = store
        .submit_with_checkpoint(6, |point| {
            if point == "before-publication" {
                Err("injected".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error, "injected");
    assert_eq!(
        before,
        (
            fs::read(store.paths.directory().join(PRIMARY_NAME)).unwrap(),
            fs::read(store.paths.directory().join(BACKUP_NAME)).unwrap(),
        )
    );
    assert!(!names(&store).iter().any(|name| name.starts_with(".progress-stage-")));
    assert_eq!(reopen(&store).best(), 3);
}

#[test]
fn a_failure_after_the_primary_is_published_is_reported_as_uncertain() {
    let (_root, mut store) = sandbox();
    let outcome = store
        .submit_with_checkpoint(2, |point| {
            if point == "backup-rename" {
                Err("injected".into())
            } else {
                Ok(())
            }
        })
        .unwrap();
    assert!(matches!(outcome, CommitOutcome::DurabilityUncertain(_)));
    assert_eq!(reopen(&store).best(), 2);
    assert!(store.submit_completed(3).is_err(), "no automatic retry after uncertainty");
}

#[test]
fn directories_with_reparse_points_and_unsafe_names_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    // A directory junction made with the system tool: a reparse point.
    let junction = root.path().join("junction");
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&real)
        .output()
        .unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let paths = ProgressPaths::from_directory(junction.join("progress")).unwrap();
    let store = ProgressStore::open(paths, key()).unwrap();
    assert!(store.notice().unwrap().contains("unsafe or unavailable progress directory"));
    for bad in ["relative", "C:relative", "C:\\", "C:\\a\\..\\b", "C:\\a\\.\\b", "C:\\a\\b:stream", "C:\\a\\b."] {
        assert!(ProgressPaths::from_directory(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_default_location_is_under_local_app_data() {
    let k = key();
    assert!(ProgressPaths::from_environment_values(None, None, &k).is_err());
    let paths = ProgressPaths::from_environment_values(
        Some(OsStr::new("relative")),
        Some(OsStr::new("C:\\Users\\alice")),
        &k,
    )
    .unwrap();
    assert_eq!(
        paths.directory(),
        Path::new("C:\\Users\\alice\\AppData\\Local\\orrery\\games")
            .join(k.game())
            .join(PROFILE)
            .join(hex(&k.challenge))
    );
    let explicit = ProgressPaths::from_environment_values(
        Some(OsStr::new("D:\\Data")),
        Some(OsStr::new("C:\\Users\\alice")),
        &k,
    )
    .unwrap();
    assert!(explicit.directory().starts_with("D:\\Data"));
}

#[cfg(feature = "room-progress")]
#[test]
fn a_room_completion_uses_the_same_transport() {
    let mut id = [2; 16];
    id[6] = 0x41;
    id[8] = 0x81;
    let key = room::RoomClearKey { game_id: id, challenge: [5; 32] };
    let root = tempfile::tempdir().unwrap();
    let paths = ProgressPaths::from_directory(root.path().join("room")).unwrap();
    let mut store = room::RoomClearStore::open(paths.clone(), key.clone()).unwrap();
    assert!(!store.cleared());
    assert_eq!(store.mark_cleared().unwrap(), CommitOutcome::DurablySaved);
    assert!(room::RoomClearStore::open(paths, key).unwrap().cleared());
    assert!(root.path().join("room").join("completion.json").is_file());
}
