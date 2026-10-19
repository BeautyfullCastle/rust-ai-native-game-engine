use super::*;
use orr_input::{Button, Key as MapKey};
use orr_sample::collect_app::input_actions::default_map;

fn event(key: Key, pressed: bool) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn alternate() -> CollectBindings {
    let mut map = default_map();
    for action in &mut map.actions {
        let keys = match action.name.as_str() {
            "left" => vec![MapKey::J, MapKey::H],
            "right" => vec![MapKey::L],
            "up" => vec![MapKey::I],
            "down" => vec![MapKey::K],
            "restart" => vec![MapKey::R, MapKey::Enter],
            "quit" => vec![MapKey::Q],
            _ => unreachable!(),
        };
        action.bindings = keys
            .into_iter()
            .map(|key| Button::Keyboard { key })
            .collect();
    }
    CollectBindings::from_map(&map).unwrap()
}

#[test]
fn all_supported_keys_have_distinct_physical_mapping() {
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
    assert_eq!(
        keys.into_iter()
            .filter_map(physical_code)
            .collect::<BTreeSet<_>>()
            .len(),
        34
    );
    assert_eq!(physical_code(Key::F1), None);
}

#[test]
fn aliases_are_ored_and_opposing_directions_cancel() {
    let mut state = CollectControls {
        bindings: alternate(),
        ..Default::default()
    };
    assert_eq!(
        state
            .route(
                &[
                    event(Key::J, true),
                    event(Key::H, true),
                    event(Key::L, true)
                ],
                true
            )
            .keys
            .x,
        0
    );
    assert_eq!(state.route(&[event(Key::J, false)], true).keys.x, 0);
    assert_eq!(state.route(&[event(Key::H, false)], true).keys.x, 1);
    assert_eq!(
        state
            .route(&[event(Key::I, true), event(Key::K, true)], true)
            .keys
            .y,
        0
    );
    assert_eq!(state.route(&[event(Key::K, false)], true).keys.y, 1);
}

#[test]
fn fresh_restart_aliases_are_ordered_without_repeat_or_hold_retrigger() {
    let mut state = CollectControls {
        bindings: alternate(),
        ..Default::default()
    };
    assert_eq!(
        state
            .route(&[event(Key::R, true), event(Key::Enter, true)], true)
            .restart_transitions,
        [false, true, false, true]
    );
    assert!(state
        .route(&[event(Key::R, true)], true)
        .restart_transitions
        .is_empty());
    let mut repeat = event(Key::R, true);
    if let Event::Key { repeat, .. } = &mut repeat {
        *repeat = true;
    }
    assert!(state.route(&[repeat], true).restart_transitions.is_empty());
    let frame = state.route(&[event(Key::R, false), event(Key::R, true)], true);
    assert_eq!(frame.restart_transitions, [false, true]);
    assert!(!frame.keys.fire);
    assert!(state.route(&[], true).restart_transitions.is_empty());
}

#[test]
fn physical_identity_does_not_leak_logical_defaults() {
    let mut state = CollectControls {
        bindings: alternate(),
        ..Default::default()
    };
    let mut movement = event(Key::D, true);
    if let Event::Key { physical_key, .. } = &mut movement {
        *physical_key = Some(Key::L);
    }
    let mut restart = event(Key::Space, true);
    if let Event::Key { physical_key, .. } = &mut restart {
        *physical_key = Some(Key::R);
    }
    let frame = state.route(
        &[
            movement,
            restart,
            event(Key::D, true),
            event(Key::Space, true),
        ],
        true,
    );
    assert_eq!(frame.keys.x, 1);
    assert_eq!(frame.consumed, [0, 1]);
    assert_eq!(frame.restart_transitions, [false, true]);
}

#[test]
fn quit_and_escape_release_and_cancel_frame_restarts() {
    for quit in [Key::Q, Key::Escape] {
        let mut state = CollectControls {
            bindings: alternate(),
            ..Default::default()
        };
        let frame = state.route(
            &[event(Key::L, true), event(Key::R, true), event(quit, true)],
            true,
        );
        assert!(frame.release);
        assert_eq!(frame.keys, Keys::default());
        assert!(frame.restart_transitions.is_empty());
        assert_eq!(frame.consumed, [0, 1, 2]);
        assert_eq!(state.route(&[], true).keys, Keys::default());
    }
}

#[test]
fn disabled_frames_and_reset_quarantine_held_keys_until_release() {
    let mut state = CollectControls::default();
    state.route(&[event(Key::D, true), event(Key::Space, true)], false);
    assert_eq!(state.route(&[], true).keys, Keys::default());
    assert!(state
        .route(&[event(Key::Space, true)], true)
        .restart_transitions
        .is_empty());
    let frame = state.route(
        &[
            event(Key::D, false),
            event(Key::Space, false),
            event(Key::D, true),
            event(Key::Space, true),
        ],
        true,
    );
    assert_eq!(frame.keys.x, 1);
    assert_eq!(frame.restart_transitions, [false, true]);
    state.use_defaults();
    assert_eq!(state.route(&[], true).keys, Keys::default());
    state.route(&[], false); // focus, dialog, Stop, seek, or lease-loss boundary
    assert_eq!(
        state.route(&[event(Key::D, true)], true).keys,
        Keys::default()
    );
    assert_eq!(
        state
            .route(&[event(Key::D, false), event(Key::D, true)], true)
            .keys
            .x,
        1
    );
}

#[test]
fn epoch_replacement_resets_map_but_same_epoch_authoring_preserves_it() {
    let mut state = CollectControls::default();
    state.sync_session(7, true);
    state.bindings = alternate();
    state.path = "controls.json".into();
    state.route(&[event(Key::L, true)], true);
    state.sync_session(7, true);
    assert_eq!(state.bindings, alternate());
    assert_eq!(state.route(&[], true).keys.x, 1);
    state.sync_session(8, true);
    assert_eq!(state.bindings, CollectBindings::default());
    assert!(state.path.is_empty());
    assert_eq!(state.route(&[], true).keys, Keys::default());
    state.bindings = alternate();
    state.sync_session(8, false);
    assert_eq!(state.bindings, CollectBindings::default());
}

#[test]
fn invalid_missing_and_escape_configs_retain_accepted_map() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("map.json");
    let mut state = CollectControls {
        bindings: alternate(),
        path: path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    std::fs::write(&path, b"{not JSON}").unwrap();
    assert!(!state.load());
    assert!(state.error.is_some());
    assert_eq!(state.bindings, alternate());
    let mut map = default_map();
    map.actions
        .iter_mut()
        .find(|action| action.name == "quit")
        .unwrap()
        .bindings
        .clear();
    map.actions
        .iter_mut()
        .find(|action| action.name == "restart")
        .unwrap()
        .bindings = vec![Button::Keyboard {
        key: MapKey::Escape,
    }];
    std::fs::write(&path, serde_json::to_vec(&map).unwrap()).unwrap();
    assert!(!state.load());
    assert!(state.error.as_deref().unwrap().contains("Escape"));
    assert_eq!(state.bindings, alternate());
    std::fs::remove_file(&path).unwrap();
    assert!(!state.load());
    assert!(state.error.is_some());
    assert_eq!(state.bindings, alternate());
    std::fs::write(&path, serde_json::to_vec(&default_map()).unwrap()).unwrap();
    assert!(state.load());
    assert!(state.error.is_none());
    assert_eq!(state.bindings, CollectBindings::default());
}

#[test]
fn pre_pump_release_peek_is_non_consuming_and_obeys_quarantine() {
    let mut state = CollectControls {
        bindings: alternate(),
        ..Default::default()
    };
    state.route(&[event(Key::Q, true)], false);
    assert!(!state.release_requested(&[event(Key::Q, true)], true));
    let events = [
        event(Key::Q, false),
        event(Key::R, true),
        event(Key::Q, true),
    ];
    assert!(state.release_requested(&events, true));
    assert!(state.release_requested(&events, true));
    let frame = state.route(&events, true);
    assert!(frame.release);
    assert!(frame.restart_transitions.is_empty());
    assert!(!state.release_requested(&[], true));
    assert!(!state.release_requested(&[event(Key::Escape, true)], false));
}

#[test]
fn physical_tab_is_action_when_bound_and_release_when_unbound() {
    let mut state = CollectControls::default();
    assert!(state.release_requested(&[event(Key::Tab, true)], true));
    assert!(state.route(&[event(Key::Tab, true)], true).release);

    let mut map = default_map();
    map.actions
        .iter_mut()
        .find(|action| action.name == "right")
        .unwrap()
        .bindings = vec![Button::Keyboard { key: MapKey::Tab }];
    let mut state = CollectControls {
        bindings: CollectBindings::from_map(&map).unwrap(),
        ..Default::default()
    };
    assert!(!state.release_requested(&[event(Key::Tab, true)], true));
    let frame = state.route(&[event(Key::Tab, true)], true);
    assert!(!frame.release);
    assert_eq!(frame.keys.x, 1);
    assert_eq!(frame.consumed, [0]);
}

#[test]
fn logical_escape_tab_do_not_override_physical_action_and_missing_physical_falls_back() {
    for logical in [Key::Escape, Key::Tab] {
        let mut state = CollectControls {
            bindings: alternate(),
            ..Default::default()
        };
        let mut input = event(logical, true);
        if let Event::Key { physical_key, .. } = &mut input {
            *physical_key = Some(Key::L);
        }
        assert!(!state.release_requested(&[input.clone()], true));
        let frame = state.route(&[input], true);
        assert!(!frame.release);
        assert_eq!(frame.keys.x, 1);
        assert_eq!(frame.consumed, [0]);
    }
    let mut state = CollectControls::default();
    let mut input = event(Key::Escape, true);
    if let Event::Key { physical_key, .. } = &mut input {
        *physical_key = None;
    }
    assert!(state.release_requested(&[input.clone()], true));
    assert!(state.route(&[input], true).release);
}
