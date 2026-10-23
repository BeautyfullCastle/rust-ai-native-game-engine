//! Independent boundary tests for immutable finite held-action profiles.
use orr_input::{Action, Button, HeldActionInput, HeldActionProfile, HeldActions, Key};

fn action(name: &str, keys: &[Key]) -> Action {
    Action {
        name: name.into(),
        bindings: keys.iter().map(|&key| Button::Keyboard { key }).collect(),
    }
}
fn profile() -> HeldActionProfile {
    HeldActionProfile::new(vec![
        action("left", &[Key::J, Key::U]),
        action("action_0", &[Key::K]),
        action("third", &[Key::L]),
        action("fourth", &[Key::I]),
    ])
    .unwrap()
}

#[test]
fn full_domain_is_bijective_and_invalid_public_values_never_alias() {
    let profile = profile();
    let states: Vec<_> = profile.states().collect();
    assert_eq!(profile.state_count(), 144);
    assert_eq!(states.len(), 144);
    for (index, state) in states.iter().copied().enumerate() {
        assert_eq!(profile.index(state), Some(index));
    }
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
    for actions in 16..=u8::MAX {
        assert_eq!(
            profile.index(HeldActions {
                actions,
                ..Default::default()
            }),
            None
        );
    }
    let empty = HeldActionProfile::new(vec![]).unwrap();
    assert_eq!(empty.state_count(), 9);
    assert_eq!(
        empty.index(HeldActions {
            actions: 1,
            ..Default::default()
        }),
        None
    );
    let mut input = HeldActionInput::new(empty);
    input.key(Key::Space, true, false, false);
    assert_eq!(input.held(), HeldActions::default());
}

#[test]
fn all_bits_clear_across_each_focus_and_block_boundary_without_repeat_resurrection() {
    for focus in [true, false] {
        let mut input = HeldActionInput::new(profile());
        for key in [Key::J, Key::K, Key::L, Key::I, Key::D, Key::W] {
            input.key(key, true, false, false);
        }
        assert_eq!(input.held().actions, 15);
        if focus {
            input.set_focused(false);
        } else {
            input.set_blocked(true);
        }
        assert_eq!(input.held(), HeldActions::default());
        for key in [Key::J, Key::K, Key::L, Key::I, Key::D, Key::W] {
            input.key(key, true, false, false);
        }
        if focus {
            input.set_focused(true);
        } else {
            input.set_blocked(false);
        }
        for key in [Key::J, Key::K, Key::L, Key::I, Key::D, Key::W] {
            input.key(key, true, true, false);
        }
        assert_eq!(input.held(), HeldActions::default());
        input.key(Key::L, true, false, false);
        assert_eq!(input.held().actions, 4);
    }
}

#[test]
fn consumed_and_secondary_binding_releases_preserve_only_real_held_contributions() {
    let mut input = HeldActionInput::new(profile());
    input.key(Key::J, true, false, false);
    input.key(Key::U, true, false, false);
    input.key(Key::K, true, false, false);
    input.key(Key::J, false, true, true);
    assert_eq!(input.held().actions, 3);
    input.key(Key::U, true, false, true);
    assert_eq!(input.held().actions, 2);
    input.key(Key::K, false, false, true);
    assert_eq!(input.held(), HeldActions::default());
    input.key(Key::A, true, false, false);
    input.key(Key::D, true, false, false);
    assert_eq!(input.held().x, 0);
    input.key(Key::D, false, false, true);
    assert_eq!(input.held().x, -1);
    input.key(Key::A, true, false, true);
    assert_eq!(input.held(), HeldActions::default());
}

#[test]
fn stable_order_is_identity_and_every_reserved_or_duplicate_binding_is_rejected() {
    let first = profile();
    let mut reordered = first.actions().to_vec();
    reordered.swap(0, 1);
    let second = HeldActionProfile::new(reordered).unwrap();
    let mut a = HeldActionInput::new(first);
    let mut b = HeldActionInput::new(second);
    a.key(Key::J, true, false, false);
    b.key(Key::J, true, false, false);
    assert_eq!(a.held().actions, 1);
    assert_eq!(b.held().actions, 2);
    for key in [
        Key::Escape,
        Key::Enter,
        Key::Tab,
        Key::W,
        Key::A,
        Key::S,
        Key::D,
        Key::ArrowUp,
        Key::ArrowDown,
        Key::ArrowLeft,
        Key::ArrowRight,
    ] {
        assert!(HeldActionProfile::new(vec![action("valid", &[key])]).is_err());
    }
    assert!(HeldActionProfile::new(vec![action("a", &[Key::J, Key::J])]).is_err());
    assert!(HeldActionProfile::new(vec![action("a", &[Key::J]), action("b", &[Key::J])]).is_err());
    assert!(HeldActionProfile::new(vec![action("a", &[Key::J]), action("a", &[Key::K])]).is_err());
    assert!(HeldActionProfile::new(vec![action("empty", &[])]).is_err());
    for button in [
        Button::Mouse { button: 0 },
        Button::Gamepad {
            device_id: 0,
            button: 0,
        },
    ] {
        assert!(HeldActionProfile::new(vec![Action {
            name: "valid".into(),
            bindings: vec![button]
        }])
        .is_err());
    }
}
