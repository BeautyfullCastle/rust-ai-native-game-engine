use orr_input::{
    Action, Button, HeldActionInput, HeldActionProfile, HeldActions, Key, MAX_HELD_STATES,
};

fn action(name: &str, keys: &[Key]) -> Action {
    Action {
        name: name.into(),
        bindings: keys
            .iter()
            .map(|key| Button::Keyboard { key: *key })
            .collect(),
    }
}
fn profile() -> HeldActionProfile {
    HeldActionProfile::new(vec![
        action("jump", &[Key::Space, Key::J]),
        action("dash", &[Key::K]),
    ])
    .unwrap()
}
#[test]
fn complete_bounded_domain_and_invalid_samples() {
    for count in 0..=4 {
        let keys = [Key::J, Key::K, Key::L, Key::Q];
        let profile = HeldActionProfile::new(
            (0..count)
                .map(|i| action(&format!("a{i}"), &[keys[i]]))
                .collect(),
        )
        .unwrap();
        assert_eq!(profile.state_count(), 9 * (1 << count));
        let states: Vec<_> = profile.states().collect();
        assert_eq!(states.len(), profile.state_count());
        for (index, state) in states.iter().enumerate() {
            assert_eq!(profile.index(*state), Some(index));
        }
        assert_eq!(
            states[profile.index(HeldActions::default()).unwrap()],
            HeldActions::default()
        );
        for x in [i8::MIN, -2, 2, i8::MAX] {
            assert_eq!(
                profile.index(HeldActions {
                    x,
                    ..Default::default()
                }),
                None
            );
            assert_eq!(
                profile.index(HeldActions {
                    y: x,
                    ..Default::default()
                }),
                None
            );
        }
        for actions in (1u16 << count)..=255 {
            assert_eq!(
                profile.index(HeldActions {
                    actions: actions as u8,
                    ..Default::default()
                }),
                None
            );
        }
    }
    assert_eq!(MAX_HELD_STATES, 144);
}
#[test]
fn admission_rejects_ambiguous_unbounded_or_unusable_bindings() {
    for key in [
        Key::Escape,
        Key::Enter,
        Key::Tab,
        Key::W,
        Key::A,
        Key::S,
        Key::D,
        Key::ArrowLeft,
        Key::ArrowRight,
        Key::ArrowUp,
        Key::ArrowDown,
    ] {
        assert!(HeldActionProfile::new(vec![action("test", &[key])]).is_err());
    }
    for name in ["", "a b", "é", &"x".repeat(65)] {
        assert!(HeldActionProfile::new(vec![action(name, &[Key::J])]).is_err());
    }
    for actions in [
        vec![action("same", &[Key::J]), action("same", &[Key::K])],
        vec![action("a", &[Key::J]), action("b", &[Key::J])],
        vec![action("a", &[Key::J, Key::J])],
        vec![action("a", &[])],
        vec![action(
            "a",
            &[
                Key::B,
                Key::C,
                Key::E,
                Key::F,
                Key::G,
                Key::H,
                Key::I,
                Key::J,
                Key::K,
            ],
        )],
        (0..5)
            .map(|i| action(&format!("a{i}"), &[Key::J]))
            .collect(),
        vec![Action {
            name: "a".into(),
            bindings: vec![Button::Mouse { button: 0 }],
        }],
        vec![Action {
            name: "a".into(),
            bindings: vec![Button::Gamepad {
                device_id: 0,
                button: 0,
            }],
        }],
    ] {
        assert!(HeldActionProfile::new(actions).is_err());
    }
}
#[test]
fn simultaneous_actions_axes_and_alternate_bindings() {
    let p = profile();
    assert!(p.contains_key(Key::W));
    assert!(p.contains_key(Key::J));
    assert!(!p.contains_key(Key::Escape));
    assert!(!p.contains_key(Key::Q));
    let mut input = HeldActionInput::new(p);
    for key in [Key::D, Key::W, Key::Space, Key::J, Key::K] {
        input.key(key, true, false, false);
    }
    assert_eq!(
        input.held(),
        HeldActions {
            x: 1,
            y: 1,
            actions: 3
        }
    );
    input.key(Key::Space, false, false, true);
    assert_eq!(input.held().actions, 3);
    input.key(Key::J, false, false, true);
    assert_eq!(input.held().actions, 2);
    input.key(Key::A, true, false, false);
    input.key(Key::S, true, false, false);
    assert_eq!(
        input.held(),
        HeldActions {
            x: 0,
            y: 0,
            actions: 2
        }
    );
    input.key(Key::K, true, false, true);
    assert_eq!(input.held().actions, 0);
}
#[test]
fn focus_and_ui_clear_never_resurrect_held_controls() {
    let mut input = HeldActionInput::new(profile());
    for block in [false, true] {
        for key in [Key::W, Key::J, Key::K] {
            input.key(key, true, false, false);
        }
        assert_eq!(input.held().actions, 3);
        if block {
            input.set_blocked(true);
        } else {
            input.set_focused(false);
        }
        assert_eq!(input.held(), HeldActions::default());
        for key in [Key::W, Key::J, Key::K] {
            input.key(key, true, false, false);
        }
        assert_eq!(input.held(), HeldActions::default());
        if block {
            input.set_blocked(false);
        } else {
            input.set_focused(true);
        }
        for key in [Key::W, Key::J, Key::K] {
            input.key(key, true, true, false);
        }
        assert_eq!(input.held(), HeldActions::default());
        input.key(Key::J, true, false, false);
        assert_eq!(input.held().actions, 1);
    }
}
#[test]
fn user_names_do_not_alias_internal_axis_or_action_names() {
    let p = HeldActionProfile::new(vec![
        action("left", &[Key::J]),
        action("action_0", &[Key::K]),
    ])
    .unwrap();
    let mut input = HeldActionInput::new(p);
    input.key(Key::J, true, false, false);
    assert_eq!(
        input.held(),
        HeldActions {
            actions: 1,
            ..Default::default()
        }
    );
    input.key(Key::K, true, false, false);
    assert_eq!(
        input.held(),
        HeldActions {
            actions: 3,
            ..Default::default()
        }
    );
}

#[test]
fn profile_owns_exact_ordered_identity() {
    let mut source = vec![
        action("jump", &[Key::J, Key::Space]),
        action("dash", &[Key::K]),
    ];
    let profile = HeldActionProfile::new(source.clone()).unwrap();
    source[0].name = "changed".into();
    source[0].bindings.clear();
    assert_eq!(profile.actions()[0], action("jump", &[Key::J, Key::Space]));
    assert_eq!(profile, profile.clone());
    assert_ne!(
        profile,
        HeldActionProfile::new(vec![
            action("dash", &[Key::K]),
            action("jump", &[Key::J, Key::Space])
        ])
        .unwrap()
    );
    assert_ne!(
        profile,
        HeldActionProfile::new(vec![
            action("jump", &[Key::Space, Key::J]),
            action("dash", &[Key::K])
        ])
        .unwrap()
    );
}

#[test]
fn controls_legend_preserves_order_alternatives_and_domain() {
    for actions in [
        vec![],
        vec![action("jump", &[Key::Space, Key::J])],
        vec![
            action("jump", &[Key::Space, Key::J]),
            action("fire", &[Key::F]),
            action("interact", &[Key::E]),
            action("dash", &[Key::Q]),
        ],
    ] {
        let profile = HeldActionProfile::new(actions).unwrap();
        let before = profile.clone();
        let states: Vec<_> = profile.states().collect();
        let expected = match profile.actions().len() {
            0 => "WASD/arrows move",
            1 => "WASD/arrows move; jump: Space/J",
            4 => "WASD/arrows move; jump: Space/J; fire: F; interact: E; dash: Q",
            _ => unreachable!(),
        };
        assert_eq!(profile.controls_legend(), expected);
        assert_eq!(profile.controls_legend(), expected);
        assert_eq!(profile, before);
        assert_eq!(profile.states().collect::<Vec<_>>(), states);
        for (index, state) in states.into_iter().enumerate() {
            assert_eq!(profile.index(state), Some(index));
        }
    }
}

#[test]
fn controls_legend_keeps_maximum_names_and_all_available_bindings() {
    // Only 23 non-reserved physical keys exist: four sets of eight cannot
    // coexist without illegal duplicate bindings. Fill two sets to eight and
    // distribute every remaining available key over the other two actions.
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
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::T,
        Key::U,
        Key::V,
        Key::X,
        Key::Y,
        Key::Z,
        Key::Space,
    ];
    let names: Vec<_> = ['a', 'b', 'c', 'd']
        .into_iter()
        .map(|c| c.to_string().repeat(64))
        .collect();
    let profile = HeldActionProfile::new(vec![
        action(&names[0], &keys[..8]),
        action(&names[1], &keys[8..16]),
        action(&names[2], &keys[16..22]),
        action(&names[3], &keys[22..]),
    ])
    .unwrap();
    let legend = profile.controls_legend();
    assert_eq!(legend, format!(
        "WASD/arrows move; {}: B/C/E/F/G/H/I/J; {}: K/L/M/N/O/P/Q/R; {}: T/U/V/X/Y/Z; {}: Space",
        names[0], names[1], names[2], names[3]
    ));
    assert!(legend.len() > 256); // The legacy hint cap must not truncate it.
    assert!(legend.len() <= 15 + 4 * (4 + 64 + 8 * 5 + 7));
    assert!(legend.is_ascii());
    assert!(!legend.contains(['\n', '\r']));
    assert!(!legend.contains("restart"));
}
