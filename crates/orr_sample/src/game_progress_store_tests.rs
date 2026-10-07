use super::*;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
};
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
fn bytes(score: u32) -> Vec<u8> {
    serde_json::to_vec(&ProgressV1::new(&key(), score)).unwrap()
}
fn put(store: &ProgressStore, name: &str, bytes: &[u8]) {
    fs::create_dir_all(store.paths.directory()).unwrap();
    fs::write(store.paths.directory().join(name), bytes).unwrap();
}
fn copies(store: &ProgressStore) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    (
        fs::read(store.paths.directory().join(PRIMARY_NAME)).ok(),
        fs::read(store.paths.directory().join(BACKUP_NAME)).ok(),
    )
}
#[test]
fn pure_discovery_and_missing_load_do_not_write() {
    let (_root, store) = sandbox();
    assert_eq!(store.best(), 0);
    assert!(!store.paths.directory().exists());
    let k = key();
    assert!(ProgressPaths::from_environment_values(None, None, &k).is_err());
    let p = ProgressPaths::from_environment_values(
        Some(OsStr::new("relative")),
        Some(OsStr::new("/home/alice")),
        &k,
    )
    .unwrap();
    assert_eq!(
        p.directory(),
        Path::new("/home/alice/.local/share/orrery/games")
            .join(k.game())
            .join(PROFILE)
            .join(hex(&k.challenge))
    );
    for path in ["relative", "/", "/tmp/../x", "/tmp/./x"] {
        assert!(ProgressPaths::from_directory(path).is_err());
    }
}
// The parallel suite launches subprocesses. A fork can briefly inherit an
// unrelated test's locked open-file description until CLOEXEC takes effect.
// Retry ONLY the prepublication EAGAIN lock result for sequential assertions;
// production stays fail-fast, and uncertain publication is never retried.
fn submit_after_transient_lock(
    store: &mut ProgressStore,
    score: u32,
) -> Result<CommitOutcome, String> {
    for attempt in 0..100 {
        match store.submit_completed(score) {
            Err(reason)
                if reason.contains("progress are locked or locking is unavailable")
                    && reason.contains("os error 11")
                    && attempt < 99 =>
            {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            result => return result,
        }
    }
    unreachable!("last attempt returns its result")
}
#[test]
fn scores_are_bounded_and_stale_writers_merge_maximum() {
    let (_root, mut a) = sandbox();
    let mut b = reopen(&a);
    assert!(a.submit_completed(11).is_err());
    assert!(!a.paths.directory().exists());
    assert_eq!(
        submit_after_transient_lock(&mut a, 8).unwrap(),
        CommitOutcome::DurablySaved
    );
    assert_eq!(
        submit_after_transient_lock(&mut b, 3).unwrap(),
        CommitOutcome::DurablySaved
    );
    assert_eq!(b.best(), 8);
    assert_eq!(reopen(&a).best(), 8);
    assert_eq!(
        submit_after_transient_lock(&mut a, 2).unwrap(),
        CommitOutcome::DurablySaved
    );
    assert_eq!(a.best(), 8);
}
#[test]
fn backup_recovery_preserves_good_backup() {
    let (_root, store) = sandbox();
    put(&store, PRIMARY_NAME, b"broken");
    put(&store, BACKUP_NAME, &bytes(5));
    let mut recovered = reopen(&store);
    assert_eq!(recovered.best(), 5);
    assert!(recovered.notice().is_some());
    recovered.submit_completed(6).unwrap();
    assert_eq!(reopen(&store).best(), 6);
    assert_eq!(
        fs::read(store.paths.directory().join(BACKUP_NAME)).unwrap(),
        bytes(5)
    );
}
#[test]
fn bad_future_mismatched_and_oversize_copies_are_preserved() {
    for raw in [
        b"broken".to_vec(),
        br#"{"version":2,"anything":true}"#.to_vec(),
        vec![b' '; 4097],
        bytes(11),
        br#"{"version":0}"#.to_vec(),
    ] {
        let (_root, store) = sandbox();
        put(&store, PRIMARY_NAME, &raw);
        let before = copies(&store);
        let mut loaded = reopen(&store);
        assert!(loaded.submit_completed(2).is_err());
        assert_eq!(copies(&store), before);
    }
    for name in [PRIMARY_NAME, BACKUP_NAME] {
        let (_root, store) = sandbox();
        put(&store, PRIMARY_NAME, &bytes(5));
        let mut wrong = ProgressV1::new(&key(), 4);
        wrong.challenge = "ff".repeat(32);
        put(&store, name, &serde_json::to_vec(&wrong).unwrap());
        let before = copies(&store);
        let mut loaded = reopen(&store);
        assert!(loaded.submit_completed(7).is_err());
        assert_eq!(copies(&store), before);
        put(&store, name, br#"{"version":99,"unknown":"field"}"#);
        let before = copies(&store);
        assert!(reopen(&store).submit_completed(8).is_err());
        assert_eq!(copies(&store), before);
    }
}
#[test]
fn strict_envelope_rejects_unknown_duplicate_and_non_integer_values() {
    for raw in [
        String::from_utf8(bytes(4))
            .unwrap()
            .replace("\"best_collected\":4", "\"best_collected\":4,\"extra\":0"),
        String::from_utf8(bytes(4)).unwrap().replace(
            "\"best_collected\":4",
            "\"best_collected\":4,\"best_collected\":3",
        ),
        String::from_utf8(bytes(4))
            .unwrap()
            .replace("\"best_collected\":4", "\"best_collected\":4.0"),
    ] {
        let (_root, store) = sandbox();
        put(&store, PRIMARY_NAME, raw.as_bytes());
        assert!(reopen(&store).submit_completed(5).is_err());
    }
}
#[test]
fn prepublication_failures_leave_bytes_and_loaded_best_unchanged() {
    for point in [
        "primary-write",
        "primary-sync",
        "backup-write",
        "backup-sync",
        "before-publication",
        "primary-rename",
    ] {
        let (_root, mut store) = sandbox();
        store.submit_completed(3).unwrap();
        let before = copies(&store);
        assert!(
            store
                .submit_with_checkpoint(7, |p| if p == point {
                    Err("injected".into())
                } else {
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(store.best(), 3);
        assert_eq!(copies(&store), before);
        assert!(
            !fs::read_dir(store.paths.directory()).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("stage"))
        );
    }
}
#[test]
fn postpublication_failure_keeps_published_best_and_disallows_blind_retry() {
    for point in ["after-publication", "backup-rename", "directory-sync"] {
        let (_root, mut store) = sandbox();
        store.submit_completed(3).unwrap();
        assert!(matches!(
            store
                .submit_with_checkpoint(7, |p| if p == point {
                    Err("injected".into())
                } else {
                    Ok(())
                })
                .unwrap(),
            CommitOutcome::DurabilityUncertain(_)
        ));
        assert_eq!(store.best(), 7);
        assert_eq!(reopen(&store).best(), 7);
        assert!(store.submit_completed(8).is_err());
    }
}
#[test]
fn stable_nonblocking_lock_and_reread_before_merge() {
    let (_root, mut a) = sandbox();
    a.submit_completed(2).unwrap();
    let inode = fs::metadata(a.paths.directory().join(LOCK_NAME))
        .unwrap()
        .ino();
    let mut b = reopen(&a);
    a.submit_with_checkpoint(6, |point| {
        if point == "primary-write" {
            assert!(b.submit_completed(9).is_err());
        }
        Ok(())
    })
    .unwrap();
    b.submit_completed(4).unwrap();
    assert_eq!(b.best(), 6);
    assert_eq!(
        fs::metadata(a.paths.directory().join(LOCK_NAME))
            .unwrap()
            .ino(),
        inode
    );
}
#[test]
fn links_special_files_and_unwritable_files_are_refused() {
    for name in [PRIMARY_NAME, BACKUP_NAME, LOCK_NAME] {
        for kind in ["symlink", "hardlink", "fifo", "readonly"] {
            let (root, store) = sandbox();
            fs::create_dir_all(store.paths.directory()).unwrap();
            let outside = root.path().join("outside");
            fs::write(&outside, bytes(3)).unwrap();
            let target = store.paths.directory().join(name);
            match kind {
                "symlink" => symlink(&outside, &target).unwrap(),
                "hardlink" => fs::hard_link(&outside, &target).unwrap(),
                "fifo" => rustix::fs::mkfifoat(
                    rustix::fs::CWD,
                    &target,
                    rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
                )
                .unwrap(),
                _ => {
                    fs::write(&target, bytes(3)).unwrap();
                    fs::set_permissions(&target, fs::Permissions::from_mode(0o400)).unwrap();
                }
            }
            assert!(reopen(&store).submit_completed(5).is_err(), "{name} {kind}");
            assert_eq!(fs::read(&outside).unwrap(), bytes(3));
        }
    }
}
#[test]
fn changed_public_names_and_ancestor_symlinks_block_publication() {
    let (root, mut store) = sandbox();
    store.submit_completed(2).unwrap();
    let path = store.paths.directory().join(BACKUP_NAME);
    assert!(
        store
            .submit_with_checkpoint(8, |point| {
                if point == "before-publication" {
                    fs::write(&path, br#"{"version":8}"#).unwrap();
                }
                Ok(())
            })
            .is_err()
    );
    assert_eq!(reopen(&store).best(), 2);
    assert_eq!(fs::read(path).unwrap(), br#"{"version":8}"#);
    let alias = root.path().join("alias");
    symlink(store.paths.directory(), &alias).unwrap();
    let mut aliased =
        ProgressStore::open(ProgressPaths::from_directory(alias).unwrap(), key()).unwrap();
    assert!(aliased.submit_completed(6).is_err());
}

#[test]
fn every_unsupported_discriminator_preserves_even_with_matching_backup() {
    for version in [
        "0",
        "2",
        "1.0",
        "1.5",
        "-1",
        "null",
        "\"1\"",
        "{}",
        "18446744073709551616",
        "1e999",
    ] {
        let (_root, store) = sandbox();
        put(
            &store,
            PRIMARY_NAME,
            format!("{{\"version\":{version}}}").as_bytes(),
        );
        put(&store, BACKUP_NAME, &bytes(4));
        let before = copies(&store);
        assert!(reopen(&store).submit_completed(5).is_err(), "{version}");
        assert_eq!(copies(&store), before);
    }
    let (_root, store) = sandbox();
    put(&store, PRIMARY_NAME, br#"{"version":1,"version":2}"#);
    put(&store, BACKUP_NAME, &bytes(4));
    assert!(reopen(&store).submit_completed(5).is_err());
}
#[test]
fn insecure_ancestor_and_invalid_key_are_rejected() {
    let (root, store) = sandbox();
    let ancestor = root.path().join("shared");
    fs::create_dir(&ancestor).unwrap();
    fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o777)).unwrap();
    let paths = ProgressPaths::from_directory(ancestor.join("progress")).unwrap();
    assert!(
        ProgressStore::open(paths, key())
            .unwrap()
            .submit_completed(3)
            .is_err()
    );
    for goal in [0, 33, u32::MAX] {
        let mut k = key();
        k.goal = goal;
        assert!(ProgressStore::open(store.paths.clone(), k).is_err());
    }
    let mut k = key();
    k.game_id[6] = 0x11;
    assert!(ProgressStore::open(store.paths, k).is_err());
}

#[test]
fn duplicate_identity_fields_never_recover_over_ambiguous_bytes() {
    for name in [PRIMARY_NAME, BACKUP_NAME] {
        for (field, correct) in [
            ("game_id", key().game()),
            ("profile", PROFILE.to_owned()),
            ("challenge", hex(&key().challenge)),
        ] {
            for reverse in [false, true] {
                let (_root, store) = sandbox();
                put(&store, PRIMARY_NAME, &bytes(3));
                put(&store, BACKUP_NAME, &bytes(3));
                let original = String::from_utf8(bytes(4)).unwrap();
                let replacement = if reverse {
                    format!("\"{field}\":\"{correct}\",\"{field}\":\"wrong\"")
                } else {
                    format!("\"{field}\":\"wrong\",\"{field}\":\"{correct}\"")
                };
                let ambiguous =
                    original.replace(&format!("\"{field}\":\"{correct}\""), &replacement);
                put(&store, name, ambiguous.as_bytes());
                let before = copies(&store);
                assert!(reopen(&store).submit_completed(8).is_err());
                assert_eq!(copies(&store), before);
            }
        }
    }
}

#[test]
#[ignore = "child fixture invoked explicitly by concurrent_processes_merge_without_lost_highscore"]
fn concurrent_process_child() {
    let directory =
        std::env::var_os("ORR_PROGRESS_CHILD_DIRECTORY").expect("child fixture requires directory");
    let score = std::env::var("ORR_PROGRESS_CHILD_SCORE")
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let mut store = ProgressStore::open(
        ProgressPaths::from_directory(PathBuf::from(directory)).unwrap(),
        key(),
    )
    .unwrap();
    for _ in 0..100 {
        match store.submit_completed(score) {
            Ok(CommitOutcome::DurablySaved) => return,
            Ok(CommitOutcome::DurabilityUncertain(reason)) => panic!("{reason}"),
            Err(reason) if reason.contains("locked") => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            Err(reason) => panic!("{reason}"),
        }
    }
    panic!("test writers did not release the lock");
}
#[test]
fn concurrent_processes_merge_without_lost_highscore() {
    let (_root, mut store) = sandbox();
    store.submit_completed(1).unwrap();
    let exe = std::env::current_exe().unwrap();
    let mut children = [4, 9].map(|score| {
        std::process::Command::new(&exe)
            .args([
                "--ignored",
                "--exact",
                "game_progress::tests::concurrent_process_child",
                "--nocapture",
            ])
            .env("ORR_PROGRESS_CHILD_DIRECTORY", store.paths.directory())
            .env("ORR_PROGRESS_CHILD_SCORE", score.to_string())
            .spawn()
            .unwrap()
    });
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    assert_eq!(reopen(&store).best(), 9);
}
