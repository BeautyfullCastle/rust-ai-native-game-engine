use super::*;
use serde_json::json;
fn event(key: Key, pressed: bool, repeat: bool, modifiers: egui::Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat,
        modifiers,
    }
}
fn observe(controls: &mut Controls, events: Vec<Event>) {
    controls.observe(&RawInput {
        events,
        ..Default::default()
    });
}
fn rebind(map: &mut ActionMap, name: &str, key: InputKey) {
    map.actions
        .iter_mut()
        .find(|a| a.name == name)
        .unwrap()
        .bindings = vec![Button::Keyboard { key }];
}
#[test]
fn controls_closed_action_map_rejects_duplicates_reserved_unbound_and_other_devices() {
    validate(&default_map()).unwrap();
    let mut missing = default_map();
    missing.actions.pop();
    assert!(validate(&missing).is_err());
    let mut unknown = default_map();
    unknown.actions[0].name = "fire".into();
    assert!(validate(&unknown).is_err());
    for key in [InputKey::P, InputKey::Escape, InputKey::Enter, InputKey::U] {
        let mut bad = default_map();
        rebind(&mut bad, "summon", key);
        assert!(validate(&bad).is_err());
    }
    for bindings in [
        vec![],
        vec![Button::Mouse { button: 0 }],
        vec![Button::Gamepad {
            device_id: 0,
            button: 0,
        }],
        vec![
            Button::Keyboard { key: InputKey::K },
            Button::Keyboard { key: InputKey::L },
        ],
    ] {
        let mut bad = default_map();
        bad.actions[0].bindings = bindings;
        assert!(validate(&bad).is_err());
    }
}
#[test]
fn controls_strict_document_rejects_future_wrong_profile_unknown_duplicates_and_limits() {
    let value = serde_json::to_value(Document::new(default_map()).unwrap()).unwrap();
    assert_eq!(
        Document::decode(&serde_json::to_vec(&value).unwrap())
            .unwrap()
            .map,
        default_map()
    );
    for (field, v) in [
        ("schema", json!(2)),
        ("profile", json!("stormglass-audio-v1")),
        ("extra", json!(0)),
    ] {
        let mut bad = value.clone();
        bad[field] = v;
        assert!(Document::decode(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    let text = serde_json::to_string(&value).unwrap();
    assert!(Document::decode(format!("{{\"schema\":1,{}", &text[1..]).as_bytes()).is_err());
    assert!(Document::decode(&[b' '; 8193]).is_err());
    let mut bad = value;
    bad["map"]["actions"][0]["extra"] = json!(0);
    assert!(Document::decode(&serde_json::to_vec(&bad).unwrap()).is_err());
}
#[test]
fn controls_capture_requires_new_press_and_cancels_on_focus_loss() {
    let mut c = Controls::default();
    observe(&mut c, vec![event(Key::K, true, false, Default::default())]);
    c.open_panel();
    c.arm(0);
    for repeat in [false, true] {
        observe(
            &mut c,
            vec![event(Key::K, true, repeat, Default::default())],
        );
        c.capture();
        assert_eq!(c.waiting, Some(0));
    }
    observe(
        &mut c,
        vec![event(Key::K, false, false, Default::default())],
    );
    observe(&mut c, vec![event(Key::K, true, false, Default::default())]);
    c.capture();
    assert_eq!(c.hint(0, true), "K");
    c.arm(1);
    observe(
        &mut c,
        vec![
            Event::WindowFocused(false),
            event(Key::L, true, false, Default::default()),
        ],
    );
    c.capture();
    assert!(c.waiting.is_none());
    assert_eq!(c.hint(1, true), "U");
    c.arm(1);
    observe(&mut c, vec![event(Key::L, true, false, Default::default())]);
    c.capture();
    assert_eq!(c.waiting, Some(1));
    observe(
        &mut c,
        vec![
            event(Key::L, false, false, Default::default()),
            event(Key::L, true, false, Default::default()),
        ],
    );
    c.capture();
    assert_eq!(c.hint(1, true), "L");
}
#[test]
fn controls_modified_press_cannot_become_plain_by_releasing_only_modifier() {
    for modified in [
        egui::Modifiers {
            ctrl: true,
            ..Default::default()
        },
        egui::Modifiers {
            alt: true,
            ..Default::default()
        },
        egui::Modifiers {
            command: true,
            ..Default::default()
        },
        egui::Modifiers {
            mac_cmd: true,
            ..Default::default()
        },
    ] {
        let mut c = Controls::default();
        c.open_panel();
        c.arm(0);
        observe(&mut c, vec![event(Key::K, true, false, modified)]);
        c.capture();
        assert_eq!(c.waiting, Some(0));
        observe(&mut c, vec![]);
        c.capture();
        assert_eq!(c.waiting, Some(0));
        observe(
            &mut c,
            vec![
                event(Key::K, false, false, Default::default()),
                event(
                    Key::K,
                    true,
                    false,
                    egui::Modifiers {
                        shift: true,
                        ..Default::default()
                    },
                ),
            ],
        );
        c.capture();
        assert_eq!(c.hint(0, true), "K");
    }
}
#[test]
fn controls_apply_is_transactional_and_neutralizes_held_inputs() {
    let mut c = Controls::default();
    observe(&mut c, vec![event(Key::K, true, false, Default::default())]);
    let active = c.active.clone();
    let held = c.held.clone();
    let blocked = c.blocked.clone();
    rebind(&mut c.draft, "summon", InputKey::U);
    assert!(c.apply().is_err());
    assert_eq!(c.active, active);
    assert_eq!(c.held, held);
    assert_eq!(c.blocked, blocked);
    rebind(&mut c.draft, "summon", InputKey::K);
    c.apply().unwrap();
    assert!(c.blocked(Key::K));
    observe(
        &mut c,
        vec![event(Key::K, false, false, Default::default())],
    );
    assert!(!c.blocked(Key::K));
}
#[test]
fn controls_physical_capture_and_escape_cancel_use_one_identity() {
    let mut c = Controls::default();
    c.open_panel();
    c.arm(0);
    observe(
        &mut c,
        vec![Event::Key {
            key: Key::P,
            physical_key: Some(Key::K),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    c.capture();
    assert_eq!(c.hint(0, true), "K");
    c.arm(1);
    observe(
        &mut c,
        vec![event(Key::Escape, true, false, Default::default())],
    );
    c.capture();
    assert!(c.waiting.is_none());
    assert!(c.open);
    assert_eq!(c.hint(1, true), "U");
}

fn directory() -> (tempfile::TempDir, PathBuf) {
    #[cfg(windows)]
    {
        crate::defense_audio_settings::windows_test_directory()
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.path().to_owned();
        (root, path)
    }
}
fn private_write(path: &Path, bytes: &[u8]) {
    #[cfg(windows)]
    {
        crate::defense_audio_settings::windows_test_write(path, bytes);
    }
    #[cfg(target_os = "linux")]
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.create_new(true).write(true);
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(path).unwrap();
        file.write_all(bytes).unwrap();
    }
}
#[test]
fn controls_startup_apply_are_readonly_and_explicit_save_restores_complete_map() {
    let (_root, path) = directory();
    let mut c = Controls::load(&path, &[]).unwrap();
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
    rebind(&mut c.draft, "summon", InputKey::K);
    c.apply().unwrap();
    process_save(&mut c);
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
    c.save_requested = true;
    process_save(&mut c);
    assert_eq!(Controls::load(&path, &[]).unwrap().active, c.active);
    assert!(!path.join("audio-settings.json").exists());
    assert!(!path.join("audio-settings.lock").exists());
    rebind(&mut c.draft, "upgrade", InputKey::L);
    c.save_requested = true;
    process_save(&mut c);
    assert_eq!(Controls::load(&path, &[]).unwrap().active, c.active);
}
#[test]
fn controls_malformed_future_unknown_and_oversize_files_preserve_bytes_and_disable_save() {
    for bytes in [
        b"broken".to_vec(),
        vec![b' '; 8193],
        br#"{"schema":2,"profile":"stormglass-controls-v1","map":{}}"#.to_vec(),
    ] {
        let (_root, path) = directory();
        private_write(&path.join("controls-settings.json"), &bytes);
        let mut c = Controls::load(&path, &[]).unwrap();
        assert!(!c.writable);
        assert_eq!(c.active, default_map());
        rebind(&mut c.draft, "summon", InputKey::K);
        c.save_requested = true;
        process_save(&mut c);
        assert_eq!(
            std::fs::read(path.join("controls-settings.json")).unwrap(),
            bytes
        );
        assert!(!path.join("controls-settings.lock").exists());
    }
}
#[test]
fn controls_profile_accepts_over_one_kibibyte_and_rejects_cross_profile_publication() {
    let (_root, path) = directory();
    let mut map = default_map();
    rebind(&mut map, "summon", InputKey::K);
    let mut bytes = vec![b' '; 2000];
    bytes.extend(serde_json::to_vec(&Document::new(map.clone()).unwrap()).unwrap());
    private_write(&path.join("controls-settings.json"), &bytes);
    assert_eq!(Controls::load(&path, &[]).unwrap().active, map);
    let (mut audio, _) = Store::open_profile(&path, &[], StorageProfile::Audio).unwrap();
    assert!(matches!(
        audio.save_bytes(serde_json::to_vec(&Document::new(map).unwrap()).unwrap()),
        Outcome::NotSaved(_)
    ));
    let (mut controls, _) = Store::open_profile(&path, &[], StorageProfile::Controls).unwrap();
    assert!(matches!(
        controls.save_bytes(
            serde_json::to_vec(&crate::defense_audio_settings::Preferences::default()).unwrap()
        ),
        Outcome::NotSaved(_)
    ));
    assert!(!path.join("audio-settings.json").exists());
    assert_eq!(
        std::fs::read(path.join("controls-settings.json")).unwrap(),
        bytes
    );
}
#[test]
fn controls_do_not_read_or_overwrite_audio_profile_and_locks_are_independent() {
    let (_root, path) = directory();
    let audio_path = path.join("audio-settings.json");
    private_write(&audio_path, b"unreadable audio sentinel");
    let mut c = Controls::load(&path, &[]).unwrap();
    assert!(c.writable);
    c.save_requested = true;
    process_save(&mut c);
    assert_eq!(
        std::fs::read(&audio_path).unwrap(),
        b"unreadable audio sentinel"
    );
    std::fs::remove_file(&audio_path).unwrap();
    let mut audio = Store::open(&path, &[]).unwrap().store;
    assert!(matches!(
        audio.save(&crate::defense_audio_settings::Preferences::default()),
        Outcome::Saved
    ));
    #[cfg(windows)]
    let lock = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(path.join("audio-settings.lock"))
            .unwrap()
    };
    #[cfg(target_os = "linux")]
    let lock = {
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.join("audio-settings.lock"))
            .unwrap();
        rustix::fs::flock(&f, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
        f
    };
    rebind(&mut c.draft, "summon", InputKey::K);
    c.save_requested = true;
    process_save(&mut c);
    assert_eq!(Controls::load(&path, &[]).unwrap().hint(0, false), "K");
    drop(lock);
}
#[test]
fn controls_stale_save_preserves_disk_and_keeps_applied_session_separate() {
    let (_root, path) = directory();
    let mut a = Controls::load(&path, &[]).unwrap();
    let mut stale = Controls::load(&path, &[]).unwrap();
    rebind(&mut a.draft, "summon", InputKey::K);
    a.save_requested = true;
    process_save(&mut a);
    let original = std::fs::read(path.join("controls-settings.json")).unwrap();
    rebind(&mut stale.draft, "summon", InputKey::L);
    stale.save_requested = true;
    process_save(&mut stale);
    assert!(stale.status.contains("not saved"));
    assert_eq!(stale.hint(0, false), "L");
    assert_eq!(
        std::fs::read(path.join("controls-settings.json")).unwrap(),
        original
    );
    assert_eq!(Controls::load(&path, &[]).unwrap().hint(0, false), "K");
}

#[test]
fn controls_hardlinked_primary_is_readonly_and_shared_target_is_preserved() {
    let (_root, path) = directory();
    let mut map = default_map();
    rebind(&mut map, "summon", InputKey::K);
    let bytes = serde_json::to_vec(&Document::new(map).unwrap()).unwrap();
    private_write(&path.join("external"), &bytes);
    std::fs::hard_link(path.join("external"), path.join("controls-settings.json")).unwrap();
    let mut c = Controls::load(&path, &[]).unwrap();
    assert!(!c.writable);
    assert_eq!(c.active, default_map());
    c.save_requested = true;
    process_save(&mut c);
    assert_eq!(std::fs::read(path.join("external")).unwrap(), bytes);
    assert!(!path.join("controls-settings.lock").exists());
}

#[test]
fn controls_atomic_failure_boundaries_preserve_old_or_track_published_value() {
    let (_root, path) = directory();
    let (mut store, _) = Store::open_profile(&path, &[], StorageProfile::Controls).unwrap();
    let original = serde_json::to_vec(&Document::new(default_map()).unwrap()).unwrap();
    assert!(matches!(store.save_bytes(original.clone()), Outcome::Saved));
    let mut map = default_map();
    rebind(&mut map, "summon", InputKey::K);
    let bytes = serde_json::to_vec(&Document::new(map.clone()).unwrap()).unwrap();
    assert!(matches!(
        store.test_fail_publication(bytes.clone(), false),
        Outcome::NotSaved(_)
    ));
    assert_eq!(
        std::fs::read(path.join("controls-settings.json")).unwrap(),
        original
    );
    assert!(matches!(
        store.test_fail_publication(bytes.clone(), true),
        Outcome::PublishedUncertain(_)
    ));
    assert_eq!(Controls::load(&path, &[]).unwrap().active, map);
    assert_eq!(
        std::fs::read(path.join("controls-settings.json")).unwrap(),
        bytes
    );
    assert!(matches!(store.save_bytes(original), Outcome::Saved));
    assert!(!path.join("audio-settings.json").exists());
}
