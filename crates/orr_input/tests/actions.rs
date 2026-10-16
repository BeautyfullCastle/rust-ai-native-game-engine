use orr_input::{Action, ActionMap, Button, Input, Key, MAX_FILE_BYTES};
fn key(key: Key) -> Button {
    Button::Keyboard { key }
}
fn map() -> ActionMap {
    ActionMap {
        version: 1,
        actions: vec![
            Action {
                name: "left".into(),
                bindings: vec![key(Key::A), key(Key::ArrowLeft)],
            },
            Action {
                name: "right".into(),
                bindings: vec![key(Key::D)],
            },
            Action {
                name: "fire".into(),
                bindings: vec![
                    Button::Mouse { button: 0 },
                    Button::Gamepad {
                        device_id: 1,
                        button: 0,
                    },
                ],
            },
        ],
    }
}
#[test]
fn edges_wait_for_tick_not_render_and_taps_coalesce() {
    let mut input = Input::new(map()).unwrap();
    input.button(key(Key::A), true, false, false);
    input.button(key(Key::A), false, false, false);
    for _ in 0..100 {
        assert!(input.state("left").pressed);
    }
    let sample = input.sample_tick();
    assert!(sample["left"].pressed && sample["left"].released && !sample["left"].held);
    assert!(!input.sample_tick()["left"].pressed);
}
#[test]
fn repeats_multiple_bindings_and_opposing_axes() {
    let mut input = Input::new(map()).unwrap();
    input.button(key(Key::A), true, false, false);
    input.sample_tick();
    input.button(key(Key::A), true, true, false);
    input.button(key(Key::ArrowLeft), true, false, false);
    assert!(!input.state("left").pressed);
    input.button(key(Key::A), false, false, false);
    assert!(input.state("left").held);
    input.button(key(Key::D), true, false, false);
    assert_eq!(input.axis("left", "right"), 0);
    input.button(key(Key::ArrowLeft), false, false, false);
    assert_eq!(input.axis("left", "right"), 1);
}
#[test]
fn focus_and_ui_clear_held_and_pending_presses() {
    let mut input = Input::new(map()).unwrap();
    input.button(key(Key::A), true, false, false);
    input.set_focused(false);
    assert!(!input.state("left").held && !input.state("left").pressed);
    assert!(input.state("left").released);
    input.set_focused(true);
    input.button(key(Key::A), true, true, false);
    assert!(!input.state("left").held);
    input.button(key(Key::A), true, false, false);
    input.set_blocked(true);
    input.button(key(Key::D), true, false, false);
    input.set_blocked(false);
    assert_eq!(input.axis("left", "right"), 0);
    input.button(key(Key::D), true, true, false);
    assert!(!input.state("right").held);
}
#[test]
fn consumed_release_and_mouse_press_do_not_leak() {
    let mut input = Input::new(map()).unwrap();
    input.button(key(Key::A), true, false, false);
    input.button(key(Key::A), false, false, true);
    assert!(!input.state("left").held);
    input.button(Button::Mouse { button: 0 }, true, false, true);
    assert!(!input.state("fire").held && !input.state("fire").pressed);
}
#[test]
fn gamepad_disconnect_preserves_other_binding() {
    let mut input = Input::new(map()).unwrap();
    input.button(
        Button::Gamepad {
            device_id: 1,
            button: 0,
        },
        true,
        false,
        false,
    );
    input.button(Button::Mouse { button: 0 }, true, false, false);
    input.disconnect_gamepad(1);
    assert!(input.state("fire").held);
    input.button(Button::Mouse { button: 0 }, false, false, false);
    assert!(!input.state("fire").held);
}
#[test]
fn persistence_and_transactional_rebind() {
    let mut bytes = Vec::new();
    map().save(&mut bytes).unwrap();
    assert_eq!(ActionMap::load(bytes.as_slice()).unwrap(), map());
    let mut input = Input::new(map()).unwrap();
    input.button(key(Key::A), true, false, false);
    let mut bad = map();
    bad.actions[1].bindings.push(key(Key::A));
    assert!(input.replace_map(bad).is_err());
    assert!(input.state("left").held);
    let mut next = map();
    next.actions[0].bindings = vec![key(Key::Q)];
    input.replace_map(next).unwrap();
    assert!(!input.state("left").held);
    input.button(key(Key::A), true, false, false);
    assert!(!input.state("left").held);
    input.button(key(Key::Q), true, false, false);
    assert!(input.state("left").held);
}
#[test]
fn rejects_version_unknown_fields_duplicates_limits_and_malformed() {
    let mut bad = map();
    bad.version = 2;
    assert!(bad.validate().is_err());
    bad = map();
    bad.actions.push(bad.actions[0].clone());
    assert!(bad.validate().is_err());
    bad = map();
    bad.actions[0].bindings.push(key(Key::A));
    assert!(bad.validate().is_err());
    bad = map();
    bad.actions[0].name = "x".repeat(65);
    assert!(bad.validate().is_err());
    bad = map();
    bad.actions[0].bindings = vec![key(Key::A); 9];
    assert!(bad.validate().is_err());
    assert!(ActionMap::load(&vec![b' '; MAX_FILE_BYTES + 1][..]).is_err());
    assert!(ActionMap::load(&b"{\"version\":1,\"actions\":[],\"extra\":true}"[..]).is_err());
    assert!(ActionMap::load(&b"not json"[..]).is_err());
    let mut untouched = vec![42];
    bad.save(&mut untouched).unwrap_err();
    assert_eq!(untouched, vec![42]);
}
#[test]
fn atomic_new_file_roundtrip_and_failure_preservation() {
    let root = std::env::temp_dir().join(format!("orr-input-test-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("bindings.json");
    map().save_new(&path).unwrap();
    let original = std::fs::read(&path).unwrap();
    assert_eq!(ActionMap::load(original.as_slice()).unwrap(), map());
    let mut changed = map();
    changed.actions[0].bindings.clear();
    assert!(changed.save_new(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(changed.save_new(root.join("missing/child.json")).is_err());
    assert_eq!(
        std::fs::read_dir(&root).unwrap().count(),
        1,
        "no staging files leaked"
    );
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn disconnect_cancels_pending_device_press_and_boundaries_are_checked() {
    let mut input = Input::new(map()).unwrap();
    input.button(
        Button::Gamepad {
            device_id: 1,
            button: 0,
        },
        true,
        false,
        false,
    );
    input.disconnect_gamepad(1);
    let state = input.sample_tick()["fire"];
    assert!(!state.held && !state.pressed && state.released);
    let mut bad = map();
    bad.actions[0].bindings = vec![Button::Mouse { button: 8 }];
    assert!(bad.validate().is_err());
    bad.actions[0].bindings = vec![Button::Gamepad {
        device_id: 0,
        button: 32,
    }];
    assert!(bad.validate().is_err());
    bad.actions = (0..65)
        .map(|i| Action {
            name: format!("a{i}"),
            bindings: vec![],
        })
        .collect();
    assert!(bad.validate().is_err());
}

#[test]
fn idle_device_disconnect_preserves_other_devices_queued_tap() {
    let mut input = Input::new(map()).unwrap();
    input.button(Button::Mouse { button: 0 }, true, false, false);
    input.button(Button::Mouse { button: 0 }, false, false, false);
    input.disconnect_gamepad(1);
    let state = input.sample_tick()["fire"];
    assert!(state.pressed && state.released && !state.held);
}
#[test]
fn file_loader_checks_type_size_and_roundtrips() {
    let root = std::env::temp_dir().join(format!("orr-input-file-test-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("bindings.json");
    map().save_new(&path).unwrap();
    assert_eq!(ActionMap::load_file(&path).unwrap(), map());
    assert!(ActionMap::load_file(&root)
        .unwrap_err()
        .contains("regular file"));
    std::fs::write(&path, vec![b' '; MAX_FILE_BYTES + 1]).unwrap();
    assert!(ActionMap::load_file(&path).unwrap_err().contains("64 KiB"));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(root).unwrap();
}
#[cfg(unix)]
#[test]
fn file_loader_rejects_fifo_device_and_symlink_before_open() {
    let root = std::env::temp_dir().join(format!("orr-input-special-test-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let fifo = root.join("bindings.fifo");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    assert!(ActionMap::load_file(&fifo)
        .unwrap_err()
        .contains("regular file"));
    assert!(ActionMap::load_file("/dev/null")
        .unwrap_err()
        .contains("regular file"));
    let path = root.join("bindings.json");
    map().save_new(&path).unwrap();
    let link = root.join("bindings.link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(ActionMap::load_file(&link)
        .unwrap_err()
        .contains("regular file"));
    for p in [fifo, path, link] {
        std::fs::remove_file(p).unwrap();
    }
    std::fs::remove_dir(root).unwrap();
}
