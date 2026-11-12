use super::*;
use std::os::unix::fs::{symlink, OpenOptionsExt, PermissionsExt};

fn directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
fn write_private(path: &Path, bytes: &[u8]) {
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    f.write_all(bytes).unwrap();
}
fn saved(outcome: Outcome) {
    assert!(matches!(outcome, Outcome::Saved));
}
fn rejected(outcome: Outcome) {
    assert!(matches!(outcome, Outcome::NotSaved(_)));
}
fn pending(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".audio-settings-pending-")
        })
        .collect()
}

#[test]
fn startup_is_read_only_and_explicit_save_restores_both_controls() {
    let dir = directory();
    let mut loaded = Store::open(dir.path(), &[]).unwrap();
    assert_eq!(loaded.preferences, Preferences::default());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    for preferences in [
        Preferences::new(35, false).unwrap(),
        Preferences::new(0, true).unwrap(),
    ] {
        saved(loaded.store.save(&preferences));
        assert_eq!(
            Store::open(dir.path(), &[]).unwrap().preferences,
            preferences
        );
        assert!(pending(dir.path()).is_empty());
    }
    assert_eq!(
        std::fs::metadata(dir.path().join(PRIMARY))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn unsupported_and_malformed_files_remain_untouched_with_save_disabled() {
    for bytes in [
        b"broken".to_vec(),
        vec![b' '; 1025],
        br#"{"schema":2,"profile":"stormglass-audio-v1","gain_percent":5,"muted":false}"#.to_vec(),
    ] {
        let dir = directory();
        let path = dir.path().join(PRIMARY);
        write_private(&path, &bytes);
        let mut loaded = Store::open(dir.path(), &[]).unwrap();
        assert!(!loaded.store.writable());
        assert!(loaded.preferences.muted);
        rejected(loaded.store.save(&Preferences::default()));
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert!(!dir.path().join(LOCK).exists());
    }
}

#[test]
fn unsafe_paths_permissions_links_and_forbidden_roots_are_refused() {
    let dir = directory();
    for path in [
        PathBuf::from("relative"),
        PathBuf::from("/"),
        dir.path().join("."),
        dir.path().join("../other"),
    ] {
        assert!(Store::open(&path, &[]).is_err());
    }
    assert!(Store::open(dir.path(), &[dir.path().to_owned()]).is_err());
    let alias_root = directory();
    symlink(dir.path(), alias_root.path().join("alias")).unwrap();
    assert!(Store::open(&alias_root.path().join("alias"), &[]).is_err());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Store::open(dir.path(), &[]).is_err());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = alias_root.path().join("target");
    write_private(
        &target,
        &serde_json::to_vec(&Preferences::default()).unwrap(),
    );
    symlink(&target, dir.path().join(PRIMARY)).unwrap();
    assert!(!Store::open(dir.path(), &[]).unwrap().store.writable());
    std::fs::remove_file(dir.path().join(PRIMARY)).unwrap();
    std::fs::hard_link(&target, dir.path().join(PRIMARY)).unwrap();
    assert!(!Store::open(dir.path(), &[]).unwrap().store.writable());
}

#[test]
fn stale_writers_and_equal_byte_replacements_preserve_external_file() {
    let dir = directory();
    let mut first = Store::open(dir.path(), &[]).unwrap().store;
    let mut stale = Store::open(dir.path(), &[]).unwrap().store;
    saved(first.save(&Preferences::default()));
    rejected(stale.save(&Preferences::new(20, false).unwrap()));
    let bytes = std::fs::read(dir.path().join(PRIMARY)).unwrap();
    write_private(&dir.path().join("replacement"), &bytes);
    std::fs::rename(dir.path().join("replacement"), dir.path().join(PRIMARY)).unwrap();
    rejected(first.save(&Preferences::new(30, false).unwrap()));
    assert_eq!(std::fs::read(dir.path().join(PRIMARY)).unwrap(), bytes);
}

#[test]
fn busy_lock_is_nonblocking_and_lock_inode_is_stable() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &[]).unwrap().store;
    saved(store.save(&Preferences::default()));
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.path().join(LOCK))
        .unwrap();
    let id = identity(&fs::fstat(&lock).unwrap());
    fs::flock(&lock, FlockOperation::NonBlockingLockExclusive).unwrap();
    rejected(store.save(&Preferences::new(20, false).unwrap()));
    drop(lock);
    saved(store.save(&Preferences::new(20, false).unwrap()));
    link_is(&store.directory, LOCK, id).unwrap();
}

#[test]
fn prepublication_failure_keeps_old_bytes_and_cleans_only_owned_stage() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &[]).unwrap().store;
    saved(store.save(&Preferences::default()));
    let bytes = std::fs::read(dir.path().join(PRIMARY)).unwrap();
    rejected(
        store.save_with(&Preferences::new(20, false).unwrap(), |stage| {
            assert!(stage == Stage::Prepared);
            Err("injected prepublication failure".into())
        }),
    );
    assert_eq!(std::fs::read(dir.path().join(PRIMARY)).unwrap(), bytes);
    assert!(pending(dir.path()).is_empty());
    rejected(
        store.save_with(&Preferences::new(20, false).unwrap(), |stage| {
            if stage == Stage::Prepared {
                let path = pending(dir.path()).pop().unwrap();
                std::fs::rename(&path, dir.path().join("displaced-stage")).unwrap();
                write_private(&path, b"foreign");
            }
            Ok(())
        }),
    );
    assert_eq!(
        std::fs::read(pending(dir.path()).pop().unwrap()).unwrap(),
        b"foreign"
    );
    assert_eq!(std::fs::read(dir.path().join(PRIMARY)).unwrap(), bytes);
}

#[test]
fn postpublication_failure_reports_uncertainty_and_tracks_visible_baseline() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &[]).unwrap().store;
    let preferences = Preferences::new(25, false).unwrap();
    let result = store.save_with(&preferences, |stage| {
        if stage == Stage::Published {
            Err("injected directory sync failure".into())
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Outcome::PublishedUncertain(_)));
    assert_eq!(
        Store::open(dir.path(), &[]).unwrap().preferences,
        preferences
    );
    saved(store.save(&Preferences::default()));
    assert!(pending(dir.path()).is_empty());
}

#[test]
fn ui_draft_is_session_only_until_explicit_request_and_failure_preserves_it() {
    let dir = directory();
    let mut ui = crate::defense_ui::DefenseUi::new();
    crate::defense_audio_settings::install(&mut ui, Store::open(dir.path(), &[]).unwrap());
    ui.audio.gain_percent = 45;
    ui.audio.muted = false;
    crate::defense_audio_settings::process_save(&mut ui);
    assert!(!dir.path().join(PRIMARY).exists());
    ui.audio_settings.save_requested = true;
    crate::defense_audio_settings::process_save(&mut ui);
    assert!(!ui.audio_settings.save_requested);
    assert_eq!(
        Store::open(dir.path(), &[]).unwrap().preferences,
        Preferences::new(45, false).unwrap()
    );
    let replacement = serde_json::to_vec(&Preferences::default()).unwrap();
    write_private(&dir.path().join("external"), &replacement);
    std::fs::rename(dir.path().join("external"), dir.path().join(PRIMARY)).unwrap();
    ui.audio.gain_percent = 65;
    ui.audio_settings.save_requested = true;
    crate::defense_audio_settings::process_save(&mut ui);
    assert!(ui.audio_settings.status.starts_with("Not saved:"));
    assert_eq!((ui.audio.gain_percent, ui.audio.muted), (65, false));
    assert_eq!(
        std::fs::read(dir.path().join(PRIMARY)).unwrap(),
        replacement
    );
    let mut reopened = crate::defense_ui::DefenseUi::new();
    crate::defense_audio_settings::install(&mut reopened, Store::open(dir.path(), &[]).unwrap());
    assert_eq!(
        (reopened.audio.gain_percent, reopened.audio.muted),
        (100, true)
    );
    assert!(!reopened.started);
}
