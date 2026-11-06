use super::*;
pub(crate) fn private_directory() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private");
    let security = Security::new().unwrap();
    let attributes = security.attributes();
    let name = security::wide(path.as_os_str());
    // SAFETY: valid NUL-terminated path and live private descriptor.
    assert_ne!(
        unsafe { CreateDirectoryW(name.as_ptr(), &attributes) },
        0,
        "{}",
        security::last()
    );
    (root, path)
}
pub(crate) fn write_private(path: &Path, bytes: &[u8]) {
    let mut file = Security::new().unwrap().create(path).unwrap();
    file.write_all(bytes).unwrap();
}
fn saved(outcome: Outcome) {
    assert!(matches!(outcome, Outcome::Saved), "{outcome:?}");
}
fn rejected(outcome: Outcome) {
    assert!(matches!(outcome, Outcome::NotSaved(_)), "{outcome:?}");
}
fn pending(path: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(path)
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
fn windows_startup_writes_nothing_and_two_saves_reopen_exact_controls() {
    let (_root, path) = private_directory();
    let mut loaded = Store::open(&path, &[]).unwrap();
    assert_eq!(loaded.preferences, Preferences::default());
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
    for preference in [
        Preferences::new(35, false).unwrap(),
        Preferences::new(0, true).unwrap(),
    ] {
        saved(loaded.store.save(&preference));
        assert_eq!(Store::open(&path, &[]).unwrap().preferences, preference);
        assert!(pending(&path).is_empty());
    }
}

#[test]
fn windows_malformed_future_oversized_files_remain_byte_exact_and_readonly() {
    for bytes in [
        b"broken".to_vec(),
        vec![b' '; 1025],
        br#"{"schema":2,"profile":"stormglass-audio-v1","gain_percent":5,"muted":false}"#.to_vec(),
    ] {
        let (_root, path) = private_directory();
        write_private(&path.join(PRIMARY), &bytes);
        let mut loaded = Store::open(&path, &[]).unwrap();
        assert!(!loaded.store.writable());
        assert!(loaded.preferences.muted);
        rejected(loaded.store.save(&Preferences::default()));
        assert_eq!(std::fs::read(path.join(PRIMARY)).unwrap(), bytes);
        assert!(!path.join(LOCK).exists());
    }
}

#[test]
fn windows_rejects_ambiguous_paths_and_forbidden_root_identity() {
    for path in [
        "relative",
        "C:relative",
        "C:\\",
        "\\\\server\\share\\private",
        "\\\\?\\C:\\private",
        "C:\\a\\..\\b",
        "C:\\a\\.\\b",
        "C:\\a.",
        "C:\\a ",
        "C:\\NUL.txt",
        "C:\\COM1",
        "C:\\LPT¹",
        "C:\\a:stream",
        "C:/private",
        "C:\\a\\",
        "C:\\a\\\\b",
    ] {
        assert!(validate_path(Path::new(path)).is_err(), "{path}");
    }
    let (_root, path) = private_directory();
    assert!(Store::open(&path, std::slice::from_ref(&path)).is_err());
    let upper = PathBuf::from(path.to_str().unwrap().to_uppercase());
    assert!(Store::open(&upper, std::slice::from_ref(&path)).is_err());
}

#[test]
fn windows_ancestor_rename_is_blocked_until_store_closes() {
    let (root, path) = private_directory();
    let store = Store::open(&path, &[]).unwrap().store;
    let renamed = root.path().join("renamed");
    assert!(std::fs::rename(&path, &renamed).is_err());
    drop(store);
    std::fs::rename(&path, &renamed).unwrap();
}

#[test]
fn windows_stale_writer_and_equal_byte_replacement_do_not_overwrite() {
    let (_root, path) = private_directory();
    let mut current = Store::open(&path, &[]).unwrap().store;
    let mut stale = Store::open(&path, &[]).unwrap().store;
    saved(current.save(&Preferences::default()));
    rejected(stale.save(&Preferences::new(20, false).unwrap()));
    let bytes = std::fs::read(path.join(PRIMARY)).unwrap();
    write_private(&path.join("replacement"), &bytes);
    std::fs::rename(path.join("replacement"), path.join(PRIMARY)).unwrap();
    rejected(current.save(&Preferences::new(30, false).unwrap()));
    assert_eq!(std::fs::read(path.join(PRIMARY)).unwrap(), bytes);
}

#[test]
fn windows_busy_lock_and_locked_primary_preserve_bytes_and_lock_identity() {
    let (_root, path) = private_directory();
    let mut store = Store::open(&path, &[]).unwrap().store;
    saved(store.save(&Preferences::default()));
    let bytes = std::fs::read(path.join(PRIMARY)).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path.join(LOCK))
        .unwrap();
    let id = identity(&info(&lock).unwrap());
    rejected(store.save(&Preferences::new(20, false).unwrap()));
    drop(lock);
    let primary = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path.join(PRIMARY))
        .unwrap();
    rejected(store.save(&Preferences::new(20, false).unwrap()));
    drop(primary);
    assert_eq!(std::fs::read(path.join(PRIMARY)).unwrap(), bytes);
    saved(store.save(&Preferences::new(20, false).unwrap()));
    let lock = File::open(path.join(LOCK)).unwrap();
    assert_eq!(identity(&info(&lock).unwrap()), id);
}

#[test]
fn windows_prepublication_failure_keeps_old_primary_and_cleans_owned_stage() {
    let (_root, path) = private_directory();
    let mut store = Store::open(&path, &[]).unwrap().store;
    saved(store.save(&Preferences::default()));
    let bytes = std::fs::read(path.join(PRIMARY)).unwrap();
    rejected(
        store.save_with(&Preferences::new(20, false).unwrap(), |stage| {
            assert!(stage == Stage::Prepared);
            Err("injected failure before rename".into())
        }),
    );
    assert_eq!(std::fs::read(path.join(PRIMARY)).unwrap(), bytes);
    assert!(pending(&path).is_empty());
}

#[test]
fn windows_stage_substitution_is_preserved_and_never_published_or_deleted() {
    let (_root, path) = private_directory();
    let mut store = Store::open(&path, &[]).unwrap().store;
    rejected(store.save_with(&Preferences::default(), |stage| {
        if stage == Stage::Prepared {
            let stage = pending(&path).pop().unwrap();
            std::fs::rename(&stage, path.join("displaced")).unwrap();
            write_private(&stage, b"foreign");
        }
        Ok(())
    }));
    assert_eq!(
        std::fs::read(pending(&path).pop().unwrap()).unwrap(),
        b"foreign"
    );
    assert!(!path.join(PRIMARY).exists());
}

#[test]
fn windows_locked_stage_fails_without_changing_previous_primary() {
    let (_root, path) = private_directory();
    let mut store = Store::open(&path, &[]).unwrap().store;
    saved(store.save(&Preferences::default()));
    let bytes = std::fs::read(path.join(PRIMARY)).unwrap();
    let mut held = None;
    rejected(
        store.save_with(&Preferences::new(20, false).unwrap(), |stage| {
            if stage == Stage::Prepared {
                held = Some(
                    OpenOptions::new()
                        .read(true)
                        .share_mode(0)
                        .open(pending(&path).pop().unwrap())
                        .unwrap(),
                );
            }
            Ok(())
        }),
    );
    assert_eq!(std::fs::read(path.join(PRIMARY)).unwrap(), bytes);
    drop(held);
    // Inaccessible staging was left alone; a later explicit save uses a fresh name.
    saved(store.save(&Preferences::new(20, false).unwrap()));
}

#[test]
fn windows_postpublication_failure_reports_uncertainty_and_keeps_visible_baseline() {
    let (_root, path) = private_directory();
    let mut store = Store::open(&path, &[]).unwrap().store;
    let preference = Preferences::new(25, false).unwrap();
    assert!(matches!(
        store.save_with(&preference, |stage| {
            if stage == Stage::Published {
                Err("injected postpublication failure".into())
            } else {
                Ok(())
            }
        }),
        Outcome::PublishedUncertain(_)
    ));
    assert_eq!(Store::open(&path, &[]).unwrap().preferences, preference);
    saved(store.save(&Preferences::default()));
}

#[test]
fn windows_hardlink_primary_and_lock_are_rejected_without_modifying_target() {
    let (_root, path) = private_directory();
    let bytes = serde_json::to_vec(&Preferences::default()).unwrap();
    write_private(&path.join("target"), &bytes);
    std::fs::hard_link(path.join("target"), path.join(PRIMARY)).unwrap();
    assert!(!Store::open(&path, &[]).unwrap().store.writable());
    std::fs::remove_file(path.join(PRIMARY)).unwrap();
    write_private(&path.join("lock-target"), b"");
    std::fs::hard_link(path.join("lock-target"), path.join(LOCK)).unwrap();
    let mut store = Store::open(&path, &[]).unwrap().store;
    rejected(store.save(&Preferences::default()));
    assert_eq!(std::fs::read(path.join("target")).unwrap(), bytes);
    assert!(!path.join(PRIMARY).exists());
}

#[test]
fn windows_public_and_null_dacls_are_refused_for_directory_and_primary() {
    for acl in ["D:P(A;;FA;;;WD)", "D:NO_ACCESS_CONTROL"] {
        let security = Security::test_acl(acl);
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("public");
        let attributes = security.attributes();
        let name = security::wide(dir.as_os_str());
        // SAFETY: test-owned directory and live descriptor.
        assert_ne!(unsafe { CreateDirectoryW(name.as_ptr(), &attributes) }, 0);
        assert!(Store::open(&dir, &[]).is_err());
        let (_root, path) = private_directory();
        let mut file = security.create(&path.join(PRIMARY)).unwrap();
        let bytes = serde_json::to_vec(&Preferences::default()).unwrap();
        file.write_all(&bytes).unwrap();
        drop(file);
        let loaded = Store::open(&path, &[]).unwrap();
        assert!(!loaded.store.writable());
        assert_eq!(std::fs::read(path.join(PRIMARY)).unwrap(), bytes);
    }
}

#[test]
fn windows_directory_junction_and_primary_junction_are_refused() {
    let (root, path) = private_directory();
    let junction = root.path().join("junction");
    let output = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(Store::open(&junction, &[]).is_err());
    std::fs::remove_dir(&junction).unwrap();
    let primary = path.join(PRIMARY);
    let output = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&primary)
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!Store::open(&path, &[]).unwrap().store.writable());
    std::fs::remove_dir(primary).unwrap();
}

#[test]
fn windows_reader_denies_delete_sharing_and_publication_preserves_previous_file() {
    let (_root, path) = private_directory();
    let mut store = Store::open(&path, &[]).unwrap().store;
    saved(store.save(&Preferences::default()));
    let bytes = std::fs::read(path.join(PRIMARY)).unwrap();
    let mut held = None;
    rejected(
        store.save_with(&Preferences::new(20, false).unwrap(), |stage| {
            if stage == Stage::Prepared {
                held = Some(
                    OpenOptions::new()
                        .read(true)
                        .share_mode(FILE_SHARE_READ)
                        .open(path.join(PRIMARY))
                        .unwrap(),
                );
            }
            Ok(())
        }),
    );
    assert_eq!(std::fs::read(path.join(PRIMARY)).unwrap(), bytes);
    assert!(pending(&path).is_empty());
    drop(held);
    saved(store.save(&Preferences::new(20, false).unwrap()));
}

#[test]
fn windows_two_process_save_then_restore() {
    const CHILD_PATH: &str = "ORR_AUDIO_SETTINGS_TEST_CHILD_PATH";
    const CHILD_MODE: &str = "ORR_AUDIO_SETTINGS_TEST_CHILD_MODE";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        let mut loaded = Store::open(Path::new(&path), &[]).unwrap();
        if std::env::var(CHILD_MODE).unwrap() == "write" {
            saved(loaded.store.save(&Preferences::new(70, false).unwrap()));
        } else {
            assert_eq!(loaded.preferences, Preferences::new(70, false).unwrap());
        }
        return;
    }
    let (_root, path) = private_directory();
    for mode in ["write", "read"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "defense_audio_settings::windows::tests::windows_two_process_save_then_restore",
                "--test-threads=1",
            ])
            .env(CHILD_PATH, &path)
            .env(CHILD_MODE, mode)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child {mode}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn windows_rename_error_after_actual_publication_is_detected_without_rollback() {
    let (_root, path) = private_directory();
    let mut store = Store::open(&path, &[]).unwrap().store;
    let preferences = Preferences::new(80, false).unwrap();
    let outcome = store.save_with_move(
        &preferences,
        |_| Ok(()),
        |from, to| {
            move_primary(from, to)?;
            Err("injected rename error after namespace publication".into())
        },
    );
    assert!(
        matches!(outcome, Outcome::PublishedUncertain(_)),
        "{outcome:?}"
    );
    assert_eq!(Store::open(&path, &[]).unwrap().preferences, preferences);
    saved(store.save(&Preferences::default()));
}

#[test]
fn windows_unrecognized_rename_outcome_disables_further_saves_and_preserves_external_value() {
    let (_root, path) = private_directory();
    let mut store = Store::open(&path, &[]).unwrap().store;
    saved(store.save(&Preferences::default()));
    let external = Preferences::new(90, false).unwrap();
    write_private(
        &path.join("external"),
        &serde_json::to_vec(&external).unwrap(),
    );
    let outcome = store.save_with_move(
        &Preferences::new(20, false).unwrap(),
        |_| Ok(()),
        |_, to| {
            std::fs::rename(path.join("external"), to).map_err(error)?;
            Err("injected unrecognized publication outcome".into())
        },
    );
    assert!(matches!(outcome, Outcome::Uncertain(_)), "{outcome:?}");
    assert!(!store.writable());
    rejected(store.save(&Preferences::default()));
    assert_eq!(Store::open(&path, &[]).unwrap().preferences, external);
}

#[test]
fn windows_disabled_output_still_allows_explicit_settings_controls_without_game_commands() {
    let (_root, path) = private_directory();
    let mut ui = crate::defense_ui::DefenseUi::new();
    super::super::install(&mut ui, Store::open(&path, &[]).unwrap());
    let sim = orr_sim::Simulation::<orr_games::coop_defense_game::DefenseGame>::new(
        Default::default(),
        60,
        1,
    );
    for (index, x) in [580.0, 690.0, 840.0].into_iter().enumerate() {
        let pos = egui::pos2(x, 30.0);
        let events = [true, false]
            .into_iter()
            .map(|pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            })
            .collect();
        let (mut output, command) = ui.show(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 940.0),
                )),
                events,
                ..Default::default()
            },
            sim.frame(),
        );
        output.textures_delta.clear();
        assert!(command.is_none());
        assert!(!ui.started);
        assert!(!ui.audio.enabled);
        super::super::process_save(&mut ui);
        if index < 2 {
            assert!(!path.join(PRIMARY).exists());
        }
    }
    assert_eq!(
        Store::open(&path, &[]).unwrap().preferences,
        Preferences::new(90, false).unwrap()
    );
}
