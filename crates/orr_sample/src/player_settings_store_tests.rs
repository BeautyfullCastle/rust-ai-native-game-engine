use super::*;
use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    process::Command,
};

fn sandbox() -> (tempfile::TempDir, SettingsStore) {
    let root = tempfile::tempdir().unwrap();
    let store =
        SettingsStore::new(SettingsPaths::from_directory(root.path().join("controls")).unwrap());
    (root, store)
}

fn mouse() -> PlayerSettingsV1 {
    PlayerSettingsV1 {
        fire_binding: FireBinding::LeftMouse,
        ..PlayerSettingsV1::default()
    }
}

fn bytes(settings: &PlayerSettingsV1) -> Vec<u8> {
    serde_json::to_vec(settings).unwrap()
}

fn put(store: &SettingsStore, primary: &[u8], backup: Option<&[u8]>) {
    fs::create_dir_all(store.paths().directory()).unwrap();
    fs::write(store.paths().primary(), primary).unwrap();
    if let Some(backup) = backup {
        fs::write(store.paths().backup(), backup).unwrap();
    }
}

fn files(store: &SettingsStore) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    (
        fs::read(store.paths().primary()).ok(),
        fs::read(store.paths().backup()).ok(),
    )
}

#[test]
fn defaults_and_full_action_map_are_closed_and_valid() {
    let settings = PlayerSettingsV1::default();
    assert_eq!(settings.schema, 1);
    assert_eq!(settings.profile, PROFILE);
    assert_eq!(
        settings.action_map().unwrap(),
        crate::arena_input::default_map()
    );
    let map = mouse().action_map().unwrap();
    let fire = map
        .actions
        .iter()
        .find(|action| action.name == "fire")
        .unwrap();
    assert_eq!(fire.bindings, vec![Button::Mouse { button: 0 }]);
    crate::arena_input::validate_map(&map).unwrap();
    for action in map.actions.iter().filter(|action| action.name != "fire") {
        assert_eq!(
            Some(action),
            crate::arena_input::default_map()
                .actions
                .iter()
                .find(|default| default.name == action.name)
        );
    }
    let json = String::from_utf8(bytes(&mouse())).unwrap();
    assert_eq!(
        json,
        r#"{"schema":1,"profile":"arena-controls-v1","fire_binding":"left_mouse"}"#
    );
}

#[test]
fn discovery_is_pure_absolute_and_never_falls_back_to_working_directory() {
    let (_root, store) = sandbox();
    let xdg = store.paths().directory().join("xdg");
    let home = store.paths().directory().join("home");
    let paths =
        SettingsPaths::from_environment_values(Some(xdg.as_os_str()), Some(home.as_os_str()))
            .unwrap();
    assert_eq!(paths.directory(), xdg.join("orrery/arena-controls-v1"));
    assert!(!store.paths().directory().exists());
    let fallback = SettingsPaths::from_environment_values(
        Some(OsStr::new("relative")),
        Some(home.as_os_str()),
    )
    .unwrap();
    assert_eq!(
        fallback.directory(),
        home.join(".config/orrery/arena-controls-v1")
    );
    for (xdg, home) in [
        (None, None),
        (Some("relative"), Some("relative")),
        (Some(""), None),
    ] {
        assert!(
            SettingsPaths::from_environment_values(xdg.map(OsStr::new), home.map(OsStr::new))
                .is_err()
        );
    }
    for path in [
        "",
        ".",
        "relative",
        "/",
        "/tmp/../config",
        "/tmp/./config",
        "/tmp/invalid\0path",
    ] {
        assert!(SettingsPaths::from_directory(path).is_err(), "{path:?}");
    }
    assert!(SettingsPaths::from_directory(format!("/tmp/{}", "x".repeat(256))).is_err());
    assert!(
        SettingsPaths::from_environment_values(
            Some(OsStr::new("/tmp/../unsafe")),
            Some(home.as_os_str())
        )
        .is_err()
    );
}

#[test]
fn missing_load_and_defaults_do_not_write_anything() {
    let (root, store) = sandbox();
    let loaded = store.load();
    assert_eq!(loaded.state, LoadState::Missing);
    assert_eq!(loaded.settings, PlayerSettingsV1::default());
    assert!(loaded.writable());
    assert!(loaded.notice.is_none());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn successful_commit_reloads_and_retains_last_good_primary_as_backup() {
    let (_root, store) = sandbox();
    assert_eq!(
        store.commit(&PlayerSettingsV1::default()),
        CommitOutcome::Published
    );
    assert_eq!(store.load().state, LoadState::Primary);
    let first = fs::read(store.paths().primary()).unwrap();
    assert_eq!(fs::read(store.paths().backup()).unwrap(), first);
    let lock_inode = fs::metadata(store.paths().lock()).unwrap().ino();
    assert_eq!(store.commit(&mouse()), CommitOutcome::Published);
    assert_eq!(store.load().settings, mouse());
    assert_eq!(fs::read(store.paths().backup()).unwrap(), first);
    assert_eq!(
        fs::metadata(store.paths().lock()).unwrap().ino(),
        lock_inode
    );
    let names: Vec<_> = fs::read_dir(store.paths().directory())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 3, "{names:?}");
    assert_eq!(
        fs::metadata(store.paths().primary()).unwrap().mode() & 0o777,
        0o600
    );
}

#[test]
fn strict_v1_rejects_duplicates_unknowns_nulls_and_invalid_binding() {
    let malformed = [
        r#"{"schema":1,"profile":"arena-controls-v1","fire_binding":"space","unknown":true}"#,
        r#"{"schema":1,"schema":1,"profile":"arena-controls-v1","fire_binding":"space"}"#,
        r#"{"schema":1,"profile":"arena-controls-v1","profile":"arena-controls-v1","fire_binding":"space"}"#,
        r#"{"schema":1,"profile":"arena-controls-v1","fire_binding":"space","fire_binding":"left_mouse"}"#,
        r#"{"schema":null,"profile":"arena-controls-v1","fire_binding":"space"}"#,
        r#"{"schema":1,"profile":null,"fire_binding":"space"}"#,
        r#"{"schema":1,"profile":"arena-controls-v1","fire_binding":null}"#,
        r#"{"schema":1,"profile":"arena-controls-v1","fire_binding":"right_mouse"}"#,
        r#"{"schema":1,"profile":"arena-controls-v1","fire_binding":{"device":"keyboard","key":"space"}}"#,
        r#"{"schema":1,"profile":"arena-controls-v1"}"#,
        r#"{"schema":1.0,"profile":"arena-controls-v1","fire_binding":"space"}"#,
        r#"{"schema":1,"profile":"arena-controls-v1","fire_binding":"space"} trailing"#,
        r#"{"schema":1,"profile":"arena-controls-v1","fire_binding":"spa"#,
        "null",
        "[]",
        "",
        "garbage",
    ];
    for malformed in malformed {
        let (_root, store) = sandbox();
        put(&store, malformed.as_bytes(), None);
        let original = files(&store);
        let loaded = store.load();
        assert_eq!(loaded.state, LoadState::Defaults, "{malformed}");
        assert!(loaded.notice.is_some());
        assert_eq!(loaded.settings, PlayerSettingsV1::default());
        assert_eq!(files(&store), original);
        assert!(!store.paths().lock().exists());
    }
}

#[test]
fn future_or_unknown_profile_is_read_only_even_with_valid_backup() {
    for future in [
        r#"{"schema":2,"profile":"arena-controls-v1","new_controls":{}}"#,
        r#"{"schema":2}"#,
        r#"{"schema":2,"profile":{"id":"arena-controls-v2"},"fire_binding":{}}"#,
        r#"{"schema":4294967296,"profile":"arena-controls-v1","new_controls":{}}"#,
        r#"{"schema":18446744073709551616,"profile":"arena-controls-v1","new_controls":{}}"#,
        r#"{"schema":0,"profile":"arena-controls-v1","fire_binding":"space"}"#,
        r#"{"schema":1,"profile":"another-game-v1","fire_binding":"space"}"#,
    ] {
        let (_root, store) = sandbox();
        put(&store, future.as_bytes(), Some(&bytes(&mouse())));
        let original = files(&store);
        let loaded = store.load();
        assert_eq!(loaded.state, LoadState::ReadOnly, "{future}");
        assert!(!loaded.writable());
        assert!(loaded.notice.is_some());
        assert_ne!(
            loaded.settings,
            mouse(),
            "do not mask unsupported primary with backup"
        );
        assert!(matches!(
            store.commit(&mouse()),
            CommitOutcome::NotPublished(_)
        ));
        assert_eq!(files(&store), original);
    }
}

#[test]
fn future_backup_is_not_overwritten_by_a_valid_primary() {
    let (_root, store) = sandbox();
    put(
        &store,
        &bytes(&mouse()),
        Some(br#"{"schema":99,"profile":"arena-controls-v1"}"#),
    );
    let original = files(&store);
    assert_eq!(store.load().state, LoadState::ReadOnly);
    assert_eq!(store.load().settings, mouse());
    assert!(matches!(
        store.commit(&PlayerSettingsV1::default()),
        CommitOutcome::NotPublished(_)
    ));
    assert_eq!(files(&store), original);
}

#[test]
fn corrupt_primary_recovers_and_never_rotates_corrupt_bytes_into_backup() {
    let (_root, store) = sandbox();
    let backup = bytes(&mouse());
    put(&store, b"{truncated", Some(&backup));
    let loaded = store.load();
    assert_eq!(loaded.state, LoadState::RecoveredBackup);
    assert_eq!(loaded.settings, mouse());
    assert!(loaded.notice.is_some());
    assert_eq!(fs::read(store.paths().primary()).unwrap(), b"{truncated");
    assert_eq!(
        store.commit(&PlayerSettingsV1::default()),
        CommitOutcome::Published
    );
    assert_eq!(fs::read(store.paths().backup()).unwrap(), backup);
    assert_eq!(store.load().settings, PlayerSettingsV1::default());
}

#[test]
fn missing_primary_recovers_backup_without_automatic_repair() {
    let (_root, store) = sandbox();
    fs::create_dir_all(store.paths().directory()).unwrap();
    fs::write(store.paths().backup(), bytes(&mouse())).unwrap();
    assert_eq!(store.load().state, LoadState::RecoveredBackup);
    assert_eq!(store.load().settings, mouse());
    assert!(!store.paths().primary().exists());
    assert!(!store.paths().lock().exists());
}

#[test]
fn oversized_files_are_bounded_and_preserved_read_only() {
    let (_root, store) = sandbox();
    put(
        &store,
        &vec![b' '; MAX_SETTINGS_BYTES + 1],
        Some(&bytes(&mouse())),
    );
    let original = files(&store);
    assert_eq!(store.load().state, LoadState::ReadOnly);
    assert!(matches!(
        store.commit(&mouse()),
        CommitOutcome::NotPublished(_)
    ));
    assert_eq!(files(&store), original);
    // A sparse file must not be allocated/read according to its advertised size.
    fs::File::create(store.paths().primary())
        .unwrap()
        .set_len(1024 * 1024 * 1024)
        .unwrap();
    assert_eq!(store.load().state, LoadState::ReadOnly);
}

#[test]
fn symlinked_directory_ancestor_and_public_files_are_refused() {
    let (root, store) = sandbox();
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, store.paths().directory()).unwrap();
    assert_eq!(store.load().state, LoadState::ReadOnly);
    assert!(matches!(
        store.commit(&mouse()),
        CommitOutcome::NotPublished(_)
    ));
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    let nested = SettingsStore::new(
        SettingsPaths::from_directory(store.paths().directory().join("nested")).unwrap(),
    );
    assert_eq!(nested.load().state, LoadState::ReadOnly);
    assert!(matches!(
        nested.commit(&mouse()),
        CommitOutcome::NotPublished(_)
    ));
    fs::remove_file(store.paths().directory()).unwrap();
    fs::create_dir(store.paths().directory()).unwrap();
    let sentinel = outside.join("sentinel");
    fs::write(&sentinel, b"do not touch").unwrap();
    for target in [
        store.paths().primary(),
        store.paths().backup(),
        store.paths().lock(),
    ] {
        // Earlier failed commits intentionally retain the stable lock inode.
        // Remove only that sandbox fixture before testing a malicious lock link.
        if target == store.paths().lock() && target.exists() {
            fs::remove_file(&target).unwrap();
        }
        symlink(&sentinel, &target).unwrap();
        assert!(
            matches!(store.commit(&mouse()), CommitOutcome::NotPublished(_)),
            "{target:?}"
        );
        assert_eq!(fs::read(&sentinel).unwrap(), b"do not touch");
        fs::remove_file(&target).unwrap();
    }
}

#[test]
fn fifo_socket_directory_and_hardlink_are_never_read_or_replaced() {
    let (root, store) = sandbox();
    fs::create_dir(store.paths().directory()).unwrap();
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        store.paths().primary(),
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    assert_eq!(store.load().state, LoadState::ReadOnly);
    assert!(matches!(
        store.commit(&mouse()),
        CommitOutcome::NotPublished(_)
    ));
    fs::remove_file(store.paths().primary()).unwrap();
    // A socket filesystem node is enough to test admission. No listening
    // socket, network operation, or sandbox exception is needed.
    rustix::fs::mknodat(
        rustix::fs::CWD,
        store.paths().primary(),
        rustix::fs::FileType::Socket,
        rustix::fs::Mode::RUSR,
        0,
    )
    .unwrap();
    assert_eq!(store.load().state, LoadState::ReadOnly);
    assert!(matches!(
        store.commit(&mouse()),
        CommitOutcome::NotPublished(_)
    ));
    fs::remove_file(store.paths().primary()).unwrap();
    fs::create_dir(store.paths().primary()).unwrap();
    assert_eq!(store.load().state, LoadState::ReadOnly);
    assert!(matches!(
        store.commit(&mouse()),
        CommitOutcome::NotPublished(_)
    ));
    fs::remove_dir(store.paths().primary()).unwrap();
    let other = root.path().join("other.json");
    fs::write(&other, bytes(&mouse())).unwrap();
    fs::hard_link(&other, store.paths().primary()).unwrap();
    assert_eq!(store.load().state, LoadState::ReadOnly);
    assert!(matches!(
        store.commit(&mouse()),
        CommitOutcome::NotPublished(_)
    ));
    assert_eq!(fs::read(other).unwrap(), bytes(&mouse()));
}

#[test]
fn read_only_directory_keeps_loaded_value_and_refuses_save_even_as_root() {
    let (_root, store) = sandbox();
    put(&store, &bytes(&mouse()), None);
    let original = files(&store);
    fs::set_permissions(store.paths().directory(), fs::Permissions::from_mode(0o500)).unwrap();
    let loaded = store.load();
    assert_eq!(loaded.settings, mouse());
    assert_eq!(loaded.state, LoadState::ReadOnly);
    assert!(matches!(
        store.commit(&PlayerSettingsV1::default()),
        CommitOutcome::NotPublished(_)
    ));
    assert_eq!(files(&store), original);
    fs::set_permissions(store.paths().directory(), fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn lock_contention_is_nonblocking_and_stable_inode_survives_failure() {
    let (_root, store) = sandbox();
    assert_eq!(store.commit(&mouse()), CommitOutcome::Published);
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(store.paths().lock())
        .unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
    let inode = lock.metadata().unwrap().ino();
    let original = files(&store);
    assert!(matches!(
        store.commit(&PlayerSettingsV1::default()),
        CommitOutcome::NotPublished(_)
    ));
    assert_eq!(files(&store), original);
    assert_eq!(fs::metadata(store.paths().lock()).unwrap().ino(), inode);
    drop(lock);
    assert_eq!(
        store.commit(&PlayerSettingsV1::default()),
        CommitOutcome::Published
    );
    assert_eq!(fs::metadata(store.paths().lock()).unwrap().ino(), inode);
}

#[test]
fn every_prepublication_failure_preserves_primary_and_backup() {
    for point in [
        "primary-write",
        "primary-sync",
        "backup-write",
        "backup-sync",
        "before-publication",
        "primary-rename",
    ] {
        let (_root, store) = sandbox();
        put(
            &store,
            &bytes(&PlayerSettingsV1::default()),
            Some(&bytes(&mouse())),
        );
        let original = files(&store);
        let result = linux::commit(store.paths(), &mouse(), |at| {
            if at == point {
                Err(format!("injected {point}"))
            } else {
                Ok(())
            }
        });
        assert!(
            matches!(result, CommitOutcome::NotPublished(_)),
            "{point}: {result:?}"
        );
        assert_eq!(files(&store), original, "{point}");
        assert_eq!(fs::read_dir(store.paths().directory()).unwrap().count(), 3);
    }
}

#[test]
fn postpublication_failures_explicitly_report_new_primary_without_retry() {
    for point in ["after-publication", "backup-rename", "directory-sync"] {
        let (_root, store) = sandbox();
        put(
            &store,
            &bytes(&PlayerSettingsV1::default()),
            Some(&bytes(&mouse())),
        );
        let result = linux::commit(store.paths(), &mouse(), |at| {
            if at == point {
                Err(format!("injected {point}"))
            } else {
                Ok(())
            }
        });
        assert!(
            matches!(result, CommitOutcome::PublishedDurabilityUncertain(_)),
            "{point}: {result:?}"
        );
        assert_eq!(store.load().settings, mouse(), "{point}");
        let backup: PlayerSettingsV1 =
            serde_json::from_slice(&fs::read(store.paths().backup()).unwrap()).unwrap();
        assert_eq!(
            backup,
            if point == "directory-sync" {
                PlayerSettingsV1::default()
            } else {
                mouse()
            }
        );
    }
}

#[test]
fn corrupt_recovery_failure_cannot_mutate_the_valid_backup() {
    for point in [
        "primary-write",
        "primary-sync",
        "before-publication",
        "after-publication",
        "directory-sync",
    ] {
        let (_root, store) = sandbox();
        let backup = bytes(&mouse());
        put(&store, b"broken", Some(&backup));
        let result = linux::commit(store.paths(), &PlayerSettingsV1::default(), |at| {
            if at == point {
                Err(format!("injected {point}"))
            } else {
                Ok(())
            }
        });
        assert!(!matches!(result, CommitOutcome::Published));
        assert_eq!(fs::read(store.paths().backup()).unwrap(), backup);
    }
}

#[test]
fn invalid_candidate_cannot_create_directory_or_touch_existing_files() {
    let (root, store) = sandbox();
    for settings in [
        PlayerSettingsV1 {
            schema: 2,
            ..mouse()
        },
        PlayerSettingsV1 {
            profile: "other".into(),
            ..mouse()
        },
    ] {
        assert!(matches!(
            store.commit(&settings),
            CommitOutcome::NotPublished(_)
        ));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

#[test]
fn path_swap_before_publication_is_refused_without_following_symlink() {
    let (root, store) = sandbox();
    put(
        &store,
        &bytes(&PlayerSettingsV1::default()),
        Some(&bytes(&mouse())),
    );
    let original = files(&store);
    let moved = root.path().join("moved");
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let result = linux::commit(store.paths(), &mouse(), |point| {
        if point == "before-publication" {
            fs::rename(store.paths().directory(), &moved).unwrap();
            symlink(&outside, store.paths().directory()).unwrap();
        }
        Ok(())
    });
    assert!(matches!(result, CommitOutcome::NotPublished(_)));
    assert_eq!(
        (
            fs::read(moved.join(PRIMARY_NAME)).ok(),
            fs::read(moved.join(BACKUP_NAME)).ok()
        ),
        original
    );
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}

// A separate process exits without unwinding, so no stage Drop cleanup occurs.
// All child paths are explicit sandbox paths, never user configuration paths.
#[test]
fn interruption_worker() {
    let Some(directory) = std::env::var_os("ORR_SETTINGS_INTERRUPT_DIRECTORY") else {
        return;
    };
    let point = std::env::var("ORR_SETTINGS_INTERRUPT_POINT").unwrap();
    let paths = SettingsPaths::from_directory(directory).unwrap();
    let _ = linux::commit(&paths, &mouse(), |at| {
        if at == point {
            std::io::stdout().flush().unwrap();
            std::process::exit(73);
        }
        Ok(())
    });
    panic!("interruption point was not reached");
}

#[test]
fn process_interruption_around_publication_keeps_a_valid_recoverable_state() {
    for point in ["before-publication", "after-publication", "directory-sync"] {
        let (_root, store) = sandbox();
        put(
            &store,
            &bytes(&PlayerSettingsV1::default()),
            Some(&bytes(&PlayerSettingsV1::default())),
        );
        let status = Command::new(std::env::current_exe().unwrap())
            .env_clear()
            .args([
                "--exact",
                "player_settings::tests::interruption_worker",
                "--nocapture",
            ])
            .env(
                "ORR_SETTINGS_INTERRUPT_DIRECTORY",
                store.paths().directory(),
            )
            .env("ORR_SETTINGS_INTERRUPT_POINT", point)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(73), "{point}");
        let loaded = store.load();
        assert_eq!(loaded.state, LoadState::Primary);
        assert_eq!(
            loaded.settings,
            if point == "before-publication" {
                PlayerSettingsV1::default()
            } else {
                mouse()
            }
        );
        let backup: PlayerSettingsV1 =
            serde_json::from_slice(&fs::read(store.paths().backup()).unwrap()).unwrap();
        assert_eq!(backup, PlayerSettingsV1::default());
        // Process death releases the kernel lock; stale unique stages do not block.
        assert_eq!(store.commit(&mouse()), CommitOutcome::Published);
    }
}

#[test]
fn read_only_primary_backup_and_lock_are_preserved() {
    for read_only in [PRIMARY_NAME, BACKUP_NAME, LOCK_NAME] {
        let (_root, store) = sandbox();
        assert_eq!(store.commit(&mouse()), CommitOutcome::Published);
        let path = store.paths().directory().join(read_only);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        let original = files(&store);
        let loaded = store.load();
        assert_eq!(loaded.state, LoadState::ReadOnly, "{read_only}");
        assert_eq!(loaded.settings, mouse());
        assert!(matches!(
            store.commit(&PlayerSettingsV1::default()),
            CommitOutcome::NotPublished(_)
        ));
        assert_eq!(files(&store), original);
    }
}

#[test]
fn group_or_world_writable_profile_directory_is_refused() {
    let (_root, store) = sandbox();
    put(&store, &bytes(&mouse()), None);
    for mode in [0o770, 0o707] {
        fs::set_permissions(store.paths().directory(), fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(store.load().state, LoadState::ReadOnly);
        assert!(matches!(
            store.commit(&PlayerSettingsV1::default()),
            CommitOutcome::NotPublished(_)
        ));
        assert_eq!(fs::read(store.paths().primary()).unwrap(), bytes(&mouse()));
    }
    fs::set_permissions(store.paths().directory(), fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn settings_changed_to_future_after_load_are_rechecked_under_lock() {
    let (_root, store) = sandbox();
    put(
        &store,
        &bytes(&PlayerSettingsV1::default()),
        Some(&bytes(&mouse())),
    );
    assert!(store.load().writable());
    let future = br#"{"schema":1234,"profile":"arena-controls-v1","new_shape":[]}"#;
    fs::write(store.paths().primary(), future).unwrap();
    let original = files(&store);
    assert!(matches!(
        store.commit(&mouse()),
        CommitOutcome::NotPublished(_)
    ));
    assert_eq!(files(&store), original);
}

#[test]
fn bounded_profile_bytes_never_allow_unsafe_candidate_serialization() {
    let (_root, store) = sandbox();
    let too_long = PlayerSettingsV1 {
        profile: "x".repeat(MAX_SETTINGS_BYTES * 2),
        ..mouse()
    };
    assert!(matches!(
        store.commit(&too_long),
        CommitOutcome::NotPublished(_)
    ));
    assert!(!store.paths().directory().exists());
}

#[test]
fn future_primary_or_backup_arriving_during_staging_is_never_overwritten() {
    for target in [PRIMARY_NAME, BACKUP_NAME] {
        let (root, store) = sandbox();
        put(
            &store,
            &bytes(&PlayerSettingsV1::default()),
            Some(&bytes(&mouse())),
        );
        let future = br#"{"schema":2,"profile":{"id":"future"},"new_binding":{}}"#;
        let replacement = root.path().join("replacement");
        fs::write(&replacement, future).unwrap();
        let result = linux::commit(store.paths(), &mouse(), |point| {
            if point == "before-publication" {
                fs::rename(&replacement, store.paths().directory().join(target)).unwrap();
            }
            Ok(())
        });
        assert!(
            matches!(result, CommitOutcome::NotPublished(_)),
            "{target}: {result:?}"
        );
        assert_eq!(
            fs::read(store.paths().directory().join(target)).unwrap(),
            future
        );
        let unchanged = if target == PRIMARY_NAME {
            BACKUP_NAME
        } else {
            PRIMARY_NAME
        };
        let expected = if unchanged == PRIMARY_NAME {
            PlayerSettingsV1::default()
        } else {
            mouse()
        };
        assert_eq!(
            fs::read(store.paths().directory().join(unchanged)).unwrap(),
            bytes(&expected)
        );
        assert_eq!(store.load().state, LoadState::ReadOnly);
    }
}

#[test]
fn changed_malformed_bytes_and_same_bytes_new_inode_abort_publication() {
    let (_root, store) = sandbox();
    put(&store, b"broken1", Some(&bytes(&mouse())));
    let result = linux::commit(store.paths(), &PlayerSettingsV1::default(), |point| {
        if point == "before-publication" {
            fs::write(store.paths().primary(), b"broken2").unwrap();
        }
        Ok(())
    });
    assert!(matches!(result, CommitOutcome::NotPublished(_)));
    assert_eq!(fs::read(store.paths().primary()).unwrap(), b"broken2");
    assert_eq!(fs::read(store.paths().backup()).unwrap(), bytes(&mouse()));
    let replacement = store.paths().directory().join("replacement");
    fs::write(&replacement, b"broken2").unwrap();
    let result = linux::commit(store.paths(), &PlayerSettingsV1::default(), |point| {
        if point == "before-publication" {
            fs::rename(&replacement, store.paths().primary()).unwrap();
        }
        Ok(())
    });
    assert!(matches!(result, CommitOutcome::NotPublished(_)));
    assert_eq!(fs::read(store.paths().primary()).unwrap(), b"broken2");
    assert_eq!(fs::read(store.paths().backup()).unwrap(), bytes(&mouse()));
}

#[test]
fn replacing_stable_lock_during_staging_aborts_publication() {
    let (_root, store) = sandbox();
    put(
        &store,
        &bytes(&PlayerSettingsV1::default()),
        Some(&bytes(&mouse())),
    );
    let original = files(&store);
    let result = linux::commit(store.paths(), &mouse(), |point| {
        if point == "before-publication" {
            fs::remove_file(store.paths().lock()).unwrap();
            fs::write(store.paths().lock(), b"replacement lock").unwrap();
        }
        Ok(())
    });
    assert!(matches!(result, CommitOutcome::NotPublished(_)));
    assert_eq!(files(&store), original);
}
