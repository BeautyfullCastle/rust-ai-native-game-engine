use super::*;

#[test]
fn default_physical_bindings_preserve_existing_controls() {
    let bindings = CollectBindings::default();
    for (action, keys) in [
        (CollectAction::Left, vec![KeyCode::KeyA, KeyCode::ArrowLeft]),
        (
            CollectAction::Right,
            vec![KeyCode::KeyD, KeyCode::ArrowRight],
        ),
        (CollectAction::Up, vec![KeyCode::KeyW, KeyCode::ArrowUp]),
        (CollectAction::Down, vec![KeyCode::KeyS, KeyCode::ArrowDown]),
        (CollectAction::Restart, vec![KeyCode::Space]),
        (CollectAction::Quit, vec![KeyCode::Escape]),
    ] {
        for key in keys {
            assert_eq!(bindings.action_for_key(key), Some(action));
        }
    }
}

#[test]
fn held_bindings_are_ored_without_mutating_physical_state() {
    let bindings = CollectBindings::default();
    let pressed = BTreeSet::from([KeyCode::KeyA, KeyCode::ArrowRight, KeyCode::KeyW]);
    let before = pressed.clone();
    for action in [CollectAction::Left, CollectAction::Right, CollectAction::Up] {
        assert!(bindings.is_held(action, &pressed));
    }
    for action in [
        CollectAction::Down,
        CollectAction::Restart,
        CollectAction::Quit,
    ] {
        assert!(!bindings.is_held(action, &pressed));
    }
    assert_eq!(pressed, before);
    assert!(!bindings.is_held(CollectAction::Left, &BTreeSet::new()));
}

#[test]
fn unbound_physical_keys_do_not_gain_semantic_actions() {
    let bindings = CollectBindings::default();
    for key in [KeyCode::KeyQ, KeyCode::Enter, KeyCode::F1, KeyCode::Numpad4] {
        assert_eq!(bindings.action_for_key(key), None);
    }
}

#[test]
fn default_hints_preserve_existing_window_text() {
    let bindings = CollectBindings::default();
    assert_eq!(bindings.hint(CollectAction::Left), "A/Left");
    assert_eq!(bindings.hint(CollectAction::Restart), "Space");
    assert_eq!(bindings.hint(CollectAction::Quit), "Esc");
    assert_eq!(
        bindings.controls_hint(),
        crate::collect_view::DEFAULT_CONTROLS_HINT
    );
}

#[cfg(feature = "input-actions")]
mod action_maps {
    use super::*;

    fn remapped() -> ActionMap {
        let mut map = default_map();
        for (action, key) in
            map.actions
                .iter_mut()
                .zip([Key::J, Key::L, Key::I, Key::K, Key::R, Key::Q])
        {
            action.bindings = vec![Button::Keyboard { key }];
        }
        map
    }

    fn encoded(map: &ActionMap) -> Vec<u8> {
        let mut bytes = Vec::new();
        map.save(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn default_map_roundtrips_existing_format_and_matches_featureless_defaults() {
        let map = default_map();
        assert_eq!(
            CollectBindings::from_map(&map).unwrap(),
            CollectBindings::default()
        );
        let bytes = encoded(&map);
        assert_eq!(ActionMap::load(bytes.as_slice()).unwrap(), map);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("collect-bindings.json");
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(
            CollectBindings::load_file(&path).unwrap(),
            CollectBindings::default()
        );
    }

    #[test]
    fn remapping_replaces_all_defaults_and_generates_truthful_hints() {
        let map = remapped();
        let before = map.clone();
        let bindings = CollectBindings::from_map(&map).unwrap();
        assert_eq!(map, before, "admission must not mutate the caller's map");
        for (action, key) in ACTIONS.into_iter().zip([
            KeyCode::KeyJ,
            KeyCode::KeyL,
            KeyCode::KeyI,
            KeyCode::KeyK,
            KeyCode::KeyR,
            KeyCode::KeyQ,
        ]) {
            assert_eq!(bindings.action_for_key(key), Some(action));
            assert!(bindings.is_held(action, &BTreeSet::from([key])));
        }
        for key in [
            KeyCode::KeyA,
            KeyCode::KeyD,
            KeyCode::KeyW,
            KeyCode::KeyS,
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
            KeyCode::Space,
            KeyCode::Escape,
        ] {
            assert_eq!(bindings.action_for_key(key), None);
        }
        assert_eq!(
            bindings.controls_hint(),
            "Left J | Right L | Up I | Down K | Restart R | Quit Q"
        );
    }

    #[test]
    fn action_order_is_irrelevant_and_multiple_restart_bindings_are_ored() {
        let mut map = remapped();
        map.actions[4]
            .bindings
            .push(Button::Keyboard { key: Key::Enter });
        let expected = CollectBindings::from_map(&map).unwrap();
        map.actions.reverse();
        let bindings = CollectBindings::from_map(&map).unwrap();
        assert_eq!(bindings, expected);
        for pressed in [
            BTreeSet::from([KeyCode::KeyR]),
            BTreeSet::from([KeyCode::Enter]),
            BTreeSet::from([KeyCode::KeyR, KeyCode::Enter]),
        ] {
            assert!(bindings.is_held(CollectAction::Restart, &pressed));
        }
        assert_eq!(bindings.hint(CollectAction::Restart), "R/Enter");
    }

    #[test]
    fn empty_actions_remain_explicitly_unbound() {
        let mut map = default_map();
        for action in &mut map.actions {
            action.bindings.clear();
        }
        let bindings = CollectBindings::from_map(&map).unwrap();
        for action in ACTIONS {
            assert_eq!(bindings.hint(action), "Unbound");
            assert!(!bindings.is_held(action, &BTreeSet::from([KeyCode::Space])));
        }
        assert_eq!(bindings.action_for_key(KeyCode::Space), None);
        assert_eq!(bindings.controls_hint(),
            "Left Unbound | Right Unbound | Up Unbound | Down Unbound | Restart Unbound | Quit Unbound");
    }

    #[test]
    fn all_shared_keyboard_identities_have_distinct_physical_keys_and_hints() {
        let keys = [
            Key::A,
            Key::B,
            Key::C,
            Key::D,
            Key::E,
            Key::F,
            Key::G,
            Key::H,
            Key::I,
            Key::J,
            Key::K,
            Key::L,
            Key::M,
            Key::N,
            Key::O,
            Key::P,
            Key::Q,
            Key::R,
            Key::S,
            Key::T,
            Key::U,
            Key::V,
            Key::W,
            Key::X,
            Key::Y,
            Key::Z,
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::ArrowUp,
            Key::ArrowDown,
            Key::Space,
            Key::Escape,
            Key::Enter,
            Key::Tab,
        ];
        let expected = [
            "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q",
            "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "Left", "Right", "Up", "Down", "Space",
            "Esc", "Enter", "Tab",
        ];
        let mut physical = BTreeSet::new();
        let mut map = default_map();
        for action in &mut map.actions {
            action.bindings.clear();
        }
        for (key, hint) in keys.into_iter().zip(expected) {
            map.actions[0].bindings = vec![Button::Keyboard { key }];
            let bindings = CollectBindings::from_map(&map).unwrap();
            let code = key_code(key);
            assert!(physical.insert(code), "distinct physical identities");
            assert_eq!(bindings.action_for_key(code), Some(CollectAction::Left));
            assert_eq!(bindings.hint(CollectAction::Left), hint);
        }
        assert_eq!(physical.len(), 34);
    }

    #[test]
    fn missing_extra_unknown_and_duplicate_actions_are_rejected() {
        let mut missing = default_map();
        missing.actions.pop();
        assert!(CollectBindings::from_map(&missing)
            .unwrap_err()
            .contains("exactly"));
        let mut extra = default_map();
        extra.actions.push(Action {
            name: "fire".into(),
            bindings: vec![],
        });
        assert!(CollectBindings::from_map(&extra)
            .unwrap_err()
            .contains("exactly"));
        let mut unknown = default_map();
        unknown.actions[4].name = "fire".into();
        assert!(CollectBindings::from_map(&unknown)
            .unwrap_err()
            .contains("exactly"));
        let mut duplicate = default_map();
        duplicate.actions[4].name = "left".into();
        assert!(CollectBindings::from_map(&duplicate)
            .unwrap_err()
            .contains("duplicate action"));
    }

    #[test]
    fn unsupported_devices_are_rejected_not_dropped() {
        for unsupported in [
            Button::Mouse { button: 0 },
            Button::Gamepad {
                device_id: 0,
                button: 1,
            },
        ] {
            let mut map = default_map();
            map.actions[4].bindings = vec![unsupported];
            assert!(CollectBindings::from_map(&map)
                .unwrap_err()
                .contains("keyboard bindings only"));
        }
    }

    #[test]
    fn duplicate_bindings_within_and_across_actions_are_rejected() {
        let mut across = default_map();
        across.actions[4]
            .bindings
            .push(Button::Keyboard { key: Key::Escape });
        assert!(CollectBindings::from_map(&across)
            .unwrap_err()
            .contains("duplicate physical binding"));
        let mut within = default_map();
        within.actions[4]
            .bindings
            .push(Button::Keyboard { key: Key::Space });
        assert!(CollectBindings::from_map(&within)
            .unwrap_err()
            .contains("duplicate physical binding"));
    }

    #[test]
    fn shared_version_action_and_binding_limits_are_preserved() {
        let mut map = default_map();
        map.version = 2;
        assert!(CollectBindings::from_map(&map)
            .unwrap_err()
            .contains("version"));
        map = default_map();
        map.actions = (0..=orr_input::MAX_ACTIONS)
            .map(|index| Action {
                name: format!("a{index}"),
                bindings: vec![],
            })
            .collect();
        assert!(CollectBindings::from_map(&map)
            .unwrap_err()
            .contains("action count"));
        map = default_map();
        let keys = [
            Key::B,
            Key::C,
            Key::E,
            Key::F,
            Key::G,
            Key::H,
            Key::I,
            Key::J,
            Key::K,
        ];
        map.actions[0].bindings = keys
            .into_iter()
            .map(|key| Button::Keyboard { key })
            .collect();
        assert!(CollectBindings::from_map(&map)
            .unwrap_err()
            .contains("too many bindings"));
        map.actions[0].bindings.pop();
        let bindings = CollectBindings::from_map(&map).unwrap();
        assert_eq!(bindings.hint(CollectAction::Left), "B/C/E/F/G/H/I/J");
    }

    #[test]
    fn malformed_json_unknown_fields_and_unsupported_keys_fail_closed() {
        let valid = String::from_utf8(encoded(&default_map())).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        for invalid in [
            "{".into(),
            "[]".into(),
            valid.replacen('{', "{\"unexpected\":true,", 1),
            valid.replacen("\"key\": \"a\"", "\"key\": \"f1\"", 1),
            valid.replacen("\"device\": \"Keyboard\"", "\"device\": \"Touch\"", 1),
        ] {
            assert_ne!(invalid, valid, "fixture must actually modify the map");
            std::fs::write(&path, invalid).unwrap();
            assert!(CollectBindings::load_file(&path).is_err());
        }
    }

    #[test]
    fn file_loading_retains_shared_byte_boundary_and_rejects_non_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bindings.json");
        let mut bytes = encoded(&default_map());
        bytes.resize(orr_input::MAX_FILE_BYTES, b' ');
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(
            CollectBindings::load_file(&path).unwrap(),
            CollectBindings::default()
        );
        bytes.push(b' ');
        std::fs::write(&path, bytes).unwrap();
        assert!(CollectBindings::load_file(&path)
            .unwrap_err()
            .contains("64 KiB"));
        assert!(CollectBindings::load_file(dir.path())
            .unwrap_err()
            .contains("regular file"));
        assert!(CollectBindings::load_file(dir.path().join("missing.json")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn file_loading_rejects_direct_symlinks_and_preexisting_special_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bindings.json");
        std::fs::write(&path, encoded(&default_map())).unwrap();
        let link = dir.path().join("bindings.link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        let fifo = dir.path().join("bindings.fifo");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        for special in [
            link.as_path(),
            fifo.as_path(),
            std::path::Path::new("/dev/null"),
        ] {
            assert!(CollectBindings::load_file(special)
                .unwrap_err()
                .contains("regular file"));
        }
    }
}
