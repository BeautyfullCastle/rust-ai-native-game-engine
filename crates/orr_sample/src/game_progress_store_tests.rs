use super::*;
use std::{
    fs,
    os::unix::fs::{symlink, MetadataExt, PermissionsExt},
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
    let loaded = load_after_transient_lock(&store.paths, &store.key, || {});
    ProgressStore {
        paths: store.paths.clone(),
        key: store.key.clone(),
        loaded,
    }
}
fn load_after_transient_lock(
    paths: &ProgressPaths,
    key: &ProgressKey,
    mut checkpoint: impl FnMut(),
) -> Loaded {
    for _ in 0..100 {
        let mut reached_checkpoint = false;
        let loaded = linux::load_with_checkpoint(paths, key, || {
            reached_checkpoint = true;
            checkpoint();
        });
        // A parallel fork can retain a released writer's open-file description
        // until exec. Retry only actual busy loads that admitted no snapshot.
        if !reached_checkpoint && matches!(loaded.write_state, WriteState::Deferred) {
            std::thread::sleep(std::time::Duration::from_millis(1));
            continue;
        }
        return loaded;
    }
    panic!("test writer did not release the load lock");
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
    existing_directory_load_does_not_create_a_lock();
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
// The API deliberately uses a nonblocking lock. Concurrent subprocess launches
// elsewhere in a parallel test process can temporarily keep an inherited locked
// open-file description alive until exec. Never assume a positive fixture commit
// is uncontended. This helper retries only the exact lock-stage EAGAIN, proves
// that no checkpoint ran and no saved bytes/in-memory best changed, and bounds
// its retries. All other failures and publication uncertainty return immediately.
fn submit_checkpoint_after_transient_lock(
    store: &mut ProgressStore,
    score: u32,
    checkpoint: impl FnMut(&str) -> Result<(), String>,
) -> Result<CommitOutcome, String> {
    submit_checkpoint_with_busy(store, score, checkpoint, || {})
}
fn submit_checkpoint_with_busy(
    store: &mut ProgressStore,
    score: u32,
    mut checkpoint: impl FnMut(&str) -> Result<(), String>,
    mut on_busy: impl FnMut(),
) -> Result<CommitOutcome, String> {
    for attempt in 0..100 {
        let before = copies(store);
        let best = store.best();
        let mut reached_checkpoint = false;
        let result = store.submit_with_checkpoint(score, |point| {
            reached_checkpoint = true;
            checkpoint(point)
        });
        match result {
            Err(reason)
                if reason.contains("progress are locked or locking is unavailable")
                    && reason.contains("os error 11")
                    && attempt < 99 =>
            {
                assert!(
                    !reached_checkpoint,
                    "lock failure must precede every checkpoint"
                );
                assert_eq!(
                    copies(store),
                    before,
                    "busy lock must not publish saved data"
                );
                assert_eq!(store.best(), best);
                on_busy();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            result => return result,
        }
    }
    unreachable!("last attempt returns its result")
}
fn submit_after_transient_lock(
    store: &mut ProgressStore,
    score: u32,
) -> Result<CommitOutcome, String> {
    submit_checkpoint_after_transient_lock(store, score, |_| Ok(()))
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
    submit_after_transient_lock(&mut recovered, 6).unwrap();
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
        submit_after_transient_lock(&mut store, 3).unwrap();
        let before = copies(&store);
        assert_eq!(
            submit_checkpoint_after_transient_lock(&mut store, 7, |p| if p == point {
                Err("injected".into())
            } else {
                Ok(())
            })
            .unwrap_err(),
            "injected"
        );
        assert_eq!(store.best(), 3);
        assert_eq!(copies(&store), before);
        assert!(!fs::read_dir(store.paths.directory()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("stage")));
    }
}
#[test]
fn postpublication_failure_keeps_published_best_and_disallows_blind_retry() {
    for point in ["after-publication", "backup-rename", "directory-sync"] {
        let (_root, mut store) = sandbox();
        submit_after_transient_lock(&mut store, 3).unwrap();
        assert!(matches!(
            submit_checkpoint_after_transient_lock(&mut store, 7, |p| if p == point {
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
    opening_during_publication_defers_then_merges();
    shared_load_lock_excludes_writer_until_snapshot_finishes();
    first_writer_lock_appearance_discards_unlocked_snapshot();
    let (_root, mut a) = sandbox();
    submit_after_transient_lock(&mut a, 2).unwrap();
    let inode = fs::metadata(a.paths.directory().join(LOCK_NAME))
        .unwrap()
        .ino();
    let mut b = reopen(&a);
    submit_checkpoint_after_transient_lock(&mut a, 6, |point| {
        if point == "primary-write" {
            assert!(b.submit_completed(9).is_err());
        }
        Ok(())
    })
    .unwrap();
    submit_after_transient_lock(&mut b, 4).unwrap();
    assert_eq!(b.best(), 6);
    assert_eq!(
        fs::metadata(a.paths.directory().join(LOCK_NAME))
            .unwrap()
            .ino(),
        inode
    );
    // Deterministically exercise the helper's prepublication busy branch. This
    // proves retry eligibility without relying on the hypothesized fork timing.
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(a.paths.directory().join(LOCK_NAME))
        .unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
    let (release_signal, observed_busy) = std::sync::mpsc::channel();
    let release = std::thread::spawn(move || {
        observed_busy
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("lock must remain held until helper observes EAGAIN");
        drop(lock);
    });
    let mut release_signal = Some(release_signal);
    let mut entered = 0;
    assert_eq!(
        submit_checkpoint_with_busy(
            &mut b,
            7,
            |point| {
                if point == "primary-write" {
                    entered += 1;
                }
                Ok(())
            },
            || {
                if let Some(signal) = release_signal.take() {
                    signal.send(()).unwrap();
                }
            }
        )
        .unwrap(),
        CommitOutcome::DurablySaved
    );
    release.join().unwrap();
    assert!(
        release_signal.is_none(),
        "must exercise the busy retry branch"
    );
    assert_eq!(entered, 1);
    assert_eq!(b.best(), 7);
}
#[test]
fn links_special_files_and_unwritable_files_are_refused() {
    readonly_lock_preserves_saved_best_without_allowing_commit();
    deferred_load_still_rejects_unsafe_or_future_storage();
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
    replacement_of_read_lock_inode_is_terminal();
    let (root, mut store) = sandbox();
    submit_after_transient_lock(&mut store, 2).unwrap();
    let path = store.paths.directory().join(BACKUP_NAME);
    assert!(
        submit_checkpoint_after_transient_lock(&mut store, 8, |point| {
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
    assert!(ProgressStore::open(paths, key())
        .unwrap()
        .submit_completed(3)
        .is_err());
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
    submit_after_transient_lock(&mut store, 1).unwrap();
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
// This deterministic invariant test uses the real existing commit checkpoint.
// It reproduces unguarded load admission, not the unrecorded CI stat predicate.
fn opening_during_publication_defers_then_merges() {
    let (_root, mut writer) = sandbox();
    submit_after_transient_lock(&mut writer, 2).unwrap();
    let paths = writer.paths.clone();
    let mut reader = None;
    submit_checkpoint_after_transient_lock(&mut writer, 6, |point| {
        if point == "after-publication" {
            let before = (
                fs::read(paths.directory().join(PRIMARY_NAME)).unwrap(),
                fs::read(paths.directory().join(BACKUP_NAME)).unwrap(),
            );
            let opened = ProgressStore::open(paths.clone(), key()).unwrap();
            assert!(
                opened
                    .notice()
                    .is_some_and(|notice| notice.contains("load deferred")),
                "a loader must not admit snapshots during an exclusive publication lock"
            );
            assert_eq!(
                opened.best(),
                0,
                "deferred load must not admit partial snapshots"
            );
            assert_eq!(
                fs::read(paths.directory().join(PRIMARY_NAME)).unwrap(),
                before.0
            );
            assert_eq!(
                fs::read(paths.directory().join(BACKUP_NAME)).unwrap(),
                before.1
            );
            reader = Some(opened);
        }
        Ok(())
    })
    .unwrap();
    let mut reader = reader.expect("publication checkpoint reached");
    assert_eq!(
        submit_after_transient_lock(&mut reader, 4).unwrap(),
        CommitOutcome::DurablySaved
    );
    assert_eq!(reader.best(), 6);
    assert!(reader.notice().is_none());
}

fn existing_directory_load_does_not_create_a_lock() {
    let (_root, store) = sandbox();
    fs::create_dir_all(store.paths.directory()).unwrap();
    let empty = reopen(&store);
    assert_eq!(empty.best(), 0);
    assert_eq!(fs::read_dir(store.paths.directory()).unwrap().count(), 0);
    put(&store, PRIMARY_NAME, &bytes(3));
    let loaded = reopen(&store);
    assert_eq!(loaded.best(), 3);
    assert_eq!(fs::read_dir(store.paths.directory()).unwrap().count(), 1);
    assert!(!store.paths.directory().join(LOCK_NAME).exists());
}

fn first_writer_lock_appearance_discards_unlocked_snapshot() {
    let (_root, mut writer) = sandbox();
    fs::create_dir_all(writer.paths.directory()).unwrap();
    let paths = writer.paths.clone();
    let loaded = load_after_transient_lock(&paths, &key(), || {
        submit_after_transient_lock(&mut writer, 8).unwrap();
    });
    assert!(matches!(loaded.write_state, WriteState::Deferred));
    assert_eq!(loaded.best, 0);
    let mut reader = ProgressStore {
        paths,
        key: key(),
        loaded,
    };
    submit_after_transient_lock(&mut reader, 3).unwrap();
    assert_eq!(reader.best(), 8);
}

fn shared_load_lock_excludes_writer_until_snapshot_finishes() {
    let (_root, mut writer) = sandbox();
    submit_after_transient_lock(&mut writer, 2).unwrap();
    let paths = writer.paths.clone();
    let before = copies(&writer);
    let loaded = load_after_transient_lock(&paths, &key(), || {
        let mut reached_checkpoint = false;
        let failure = writer
            .submit_with_checkpoint(9, |_| {
                reached_checkpoint = true;
                Ok(())
            })
            .unwrap_err();
        assert!(failure.contains("progress are locked"), "{failure}");
        assert!(failure.contains("os error 11"), "{failure}");
        assert!(!reached_checkpoint);
        assert_eq!(copies(&writer), before);
    });
    assert!(matches!(loaded.write_state, WriteState::Ready));
    assert_eq!(loaded.best, 2);
    submit_after_transient_lock(&mut writer, 9).unwrap();
    assert_eq!(writer.best(), 9);
}

fn replacement_of_read_lock_inode_is_terminal() {
    let (_root, mut writer) = sandbox();
    submit_after_transient_lock(&mut writer, 2).unwrap();
    let paths = writer.paths.clone();
    let loaded = load_after_transient_lock(&paths, &key(), || {
        let replacement = paths.directory().join("replacement-lock");
        fs::write(&replacement, []).unwrap();
        fs::rename(replacement, paths.directory().join(LOCK_NAME)).unwrap();
    });
    assert!(matches!(loaded.write_state, WriteState::ReadOnly));
    assert!(loaded
        .notice
        .as_deref()
        .unwrap()
        .contains("lock changed while loading"));
    let mut reader = ProgressStore {
        paths,
        key: key(),
        loaded,
    };
    let before = copies(&reader);
    assert!(reader.submit_completed(9).is_err());
    assert_eq!(copies(&reader), before);
}

fn deferred_load_still_rejects_unsafe_or_future_storage() {
    for kind in [
        "hardlink",
        "symlink",
        "fifo",
        "readonly",
        "other-writable",
        "future",
    ] {
        let (root, mut writer) = sandbox();
        submit_after_transient_lock(&mut writer, 2).unwrap();
        let lock = fs::File::open(writer.paths.directory().join(LOCK_NAME)).unwrap();
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
        let mut reader = ProgressStore::open(writer.paths.clone(), key()).unwrap();
        assert!(matches!(reader.loaded.write_state, WriteState::Deferred));
        drop(lock);
        let primary = writer.paths.directory().join(PRIMARY_NAME);
        let outside = root.path().join("outside");
        fs::write(&outside, bytes(3)).unwrap();
        match kind {
            "hardlink" => fs::hard_link(&primary, root.path().join("alias")).unwrap(),
            "symlink" => {
                fs::remove_file(&primary).unwrap();
                symlink(&outside, &primary).unwrap();
            }
            "fifo" => {
                fs::remove_file(&primary).unwrap();
                rustix::fs::mkfifoat(
                    rustix::fs::CWD,
                    &primary,
                    rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
                )
                .unwrap();
            }
            "readonly" => fs::set_permissions(&primary, fs::Permissions::from_mode(0o400)).unwrap(),
            "other-writable" => {
                fs::set_permissions(&primary, fs::Permissions::from_mode(0o620)).unwrap()
            }
            "future" => fs::write(&primary, br#"{"version":2}"#).unwrap(),
            _ => unreachable!(),
        }
        let backup = fs::read(writer.paths.directory().join(BACKUP_NAME)).unwrap();
        let mut reached_checkpoint = false;
        let mut failure = None;
        for attempt in 0..100 {
            let result = reader.submit_with_checkpoint(9, |_| {
                reached_checkpoint = true;
                Ok(())
            });
            assert!(!reached_checkpoint, "{kind}");
            match result {
                Err(reason)
                    if reason.contains("progress are locked or locking is unavailable")
                        && reason.contains("os error 11")
                        && attempt < 99 =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                result => {
                    failure = Some(result.expect_err(kind));
                    break;
                }
            }
        }
        let failure = failure.expect("last attempt returned its error");
        let expected = match kind {
            "readonly" => "highscore.json is read-only",
            "future" => "unsupported progress version",
            _ => "highscore.json must be a user-owned regular file",
        };
        assert!(failure.contains(expected), "{kind}: {failure}");
        assert!(!reached_checkpoint, "{kind}");
        assert_eq!(
            fs::read(writer.paths.directory().join(BACKUP_NAME)).unwrap(),
            backup,
            "{kind}"
        );
        assert_eq!(fs::read(&outside).unwrap(), bytes(3), "{kind}");
        assert_eq!(
            fs::read_dir(writer.paths.directory()).unwrap().count(),
            3,
            "{kind}"
        );
    }
}

fn readonly_lock_preserves_saved_best_without_allowing_commit() {
    let (_root, mut writer) = sandbox();
    submit_after_transient_lock(&mut writer, 7).unwrap();
    fs::set_permissions(
        writer.paths.directory().join(LOCK_NAME),
        fs::Permissions::from_mode(0o400),
    )
    .unwrap();
    let mut reader = reopen(&writer);
    assert_eq!(reader.best(), 7);
    assert!(matches!(reader.loaded.write_state, WriteState::ReadOnly));
    let before = copies(&reader);
    assert!(reader.submit_completed(9).is_err());
    assert_eq!(copies(&reader), before);
}
