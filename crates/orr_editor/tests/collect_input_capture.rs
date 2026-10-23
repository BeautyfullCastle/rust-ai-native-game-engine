//! CPU contracts for physical capture and live draft admission. No native window.
#![cfg(feature = "collect-input-map-editor")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "../src/collect_input_map_authoring.rs"]
mod collect_input_map_authoring;
use collect_input_map_authoring::{App, CaptureState, Panel};
use egui::accesskit::Role;
use egui::{Event, Key as EKey, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_input::{ActionMap, Button, Key};

fn key(key: Key) -> Button {
    Button::Keyboard { key }
}
fn event(
    logical: EKey,
    physical: Option<EKey>,
    pressed: bool,
    repeat: bool,
    modifiers: Modifiers,
) -> Event {
    Event::Key {
        key: logical,
        physical_key: physical,
        pressed,
        repeat,
        modifiers,
    }
}
fn down(k: EKey) -> Event {
    event(k, Some(k), true, false, Modifiers::NONE)
}
fn up(k: EKey) -> Event {
    event(k, Some(k), false, false, Modifiers::NONE)
}
fn listening() -> CaptureState {
    let mut state = CaptureState::default();
    assert_eq!(state.row(), None, "idle capture has no target row");
    state.begin(2, 17);
    assert_eq!(
        state.row(),
        Some(2),
        "listening keeps the selected target row"
    );
    state
}

#[test]
fn all_supported_physical_keys_capture_and_escape_cancels() {
    let keys = [
        (EKey::A, Key::A),
        (EKey::B, Key::B),
        (EKey::C, Key::C),
        (EKey::D, Key::D),
        (EKey::E, Key::E),
        (EKey::F, Key::F),
        (EKey::G, Key::G),
        (EKey::H, Key::H),
        (EKey::I, Key::I),
        (EKey::J, Key::J),
        (EKey::K, Key::K),
        (EKey::L, Key::L),
        (EKey::M, Key::M),
        (EKey::N, Key::N),
        (EKey::O, Key::O),
        (EKey::P, Key::P),
        (EKey::Q, Key::Q),
        (EKey::R, Key::R),
        (EKey::S, Key::S),
        (EKey::T, Key::T),
        (EKey::U, Key::U),
        (EKey::V, Key::V),
        (EKey::W, Key::W),
        (EKey::X, Key::X),
        (EKey::Y, Key::Y),
        (EKey::Z, Key::Z),
        (EKey::ArrowLeft, Key::ArrowLeft),
        (EKey::ArrowRight, Key::ArrowRight),
        (EKey::ArrowUp, Key::ArrowUp),
        (EKey::ArrowDown, Key::ArrowDown),
        (EKey::Space, Key::Space),
        (EKey::Enter, Key::Enter),
        (EKey::Tab, Key::Tab),
    ];
    assert_eq!(keys.len() + 1, 34);
    for (physical, expected) in keys {
        let mut s = listening();
        let out = s.process(&[down(physical)], 17, true);
        assert_eq!(out.captured, Some((2, expected)), "{physical:?}");
        assert!(!s.is_listening());
        assert_eq!(
            s.row(),
            None,
            "completed or cancelled capture has no target row"
        );
        assert_eq!(out.consumed_event_indices, vec![0]);
    }
    let mut s = listening();
    let out = s.process(&[down(EKey::Escape)], 17, true);
    assert!(out.cancelled);
    assert_eq!(out.captured, None);
    assert!(!s.is_listening());
    assert_eq!(
        s.row(),
        None,
        "completed or cancelled capture has no target row"
    );
}

#[test]
fn physical_identity_wins_and_missing_physical_is_refused() {
    for logical in [EKey::Escape, EKey::Tab, EKey::A] {
        let mut s = listening();
        let out = s.process(
            &[event(logical, Some(EKey::J), true, false, Modifiers::NONE)],
            17,
            true,
        );
        assert_eq!(out.captured, Some((2, Key::J)));
        assert!(!out.cancelled);
    }
    for logical in [EKey::J, EKey::Escape] {
        let mut s = listening();
        let out = s.process(
            &[event(logical, None, true, false, Modifiers::NONE)],
            17,
            true,
        );
        assert_eq!(out.captured, None);
        assert!(!out.cancelled);
        assert!(s.is_listening());
        assert!(out.message.is_some());
    }
    let mut s = listening();
    let out = s.process(
        &[event(
            EKey::J,
            Some(EKey::Escape),
            true,
            false,
            Modifiers::NONE,
        )],
        17,
        true,
    );
    assert!(out.cancelled);
    assert_eq!(out.captured, None);
}

#[test]
fn unsupported_and_modifier_chords_never_bind() {
    for e in [
        down(EKey::F1),
        down(EKey::Num1),
        event(EKey::J, Some(EKey::J), true, false, Modifiers::SHIFT),
        event(EKey::J, Some(EKey::J), true, false, Modifiers::CTRL),
        event(EKey::J, Some(EKey::J), true, false, Modifiers::ALT),
        event(EKey::J, Some(EKey::J), true, false, Modifiers::COMMAND),
    ] {
        let mut s = listening();
        let out = s.process(&[e], 17, true);
        assert_eq!(out.captured, None);
        assert!(s.is_listening());
        assert!(out.message.is_some());
    }
}

#[test]
fn held_activation_release_requires_later_frame_and_fresh_press() {
    let mut s = CaptureState::default();
    s.process(&[down(EKey::J)], 17, true);
    s.begin(2, 17);
    assert_eq!(s.process(&[down(EKey::J)], 17, true).captured, None);
    assert_eq!(
        s.process(&[up(EKey::J), down(EKey::K)], 17, true).captured,
        None
    );
    assert_eq!(s.process(&[up(EKey::K)], 17, true).captured, None);
    assert_eq!(
        s.process(&[down(EKey::L)], 17, true).captured,
        Some((2, Key::L))
    );
}

#[test]
fn repeat_and_duplicate_down_need_release_before_capture() {
    let mut s = listening();
    assert_eq!(
        s.process(
            &[event(EKey::J, Some(EKey::J), true, true, Modifiers::NONE)],
            17,
            true
        )
        .captured,
        None
    );
    assert_eq!(s.process(&[down(EKey::J)], 17, true).captured, None);
    assert_eq!(s.process(&[up(EKey::J)], 17, true).captured, None);
    assert_eq!(
        s.process(&[down(EKey::J)], 17, true).captured,
        Some((2, Key::J))
    );
}

#[test]
fn focus_loss_even_regained_in_same_frame_cancels_before_candidate() {
    for events in [
        vec![
            down(EKey::J),
            Event::WindowFocused(false),
            Event::WindowFocused(true),
        ],
        vec![
            Event::WindowFocused(false),
            Event::WindowFocused(true),
            down(EKey::J),
        ],
    ] {
        let mut s = listening();
        let out = s.process(&events, 17, true);
        assert!(out.cancelled);
        assert_eq!(out.captured, None);
        assert!(!s.is_listening());
        assert_eq!(
            s.row(),
            None,
            "completed or cancelled capture has no target row"
        );
    }
    let mut s = listening();
    assert!(s.process(&[down(EKey::J)], 17, false).cancelled);
}

#[test]
fn explicit_cancel_and_context_changes_preserve_no_capture() {
    let mut s = listening();
    assert!(s.cancel().cancelled);
    assert!(!s.is_listening());
    assert_eq!(
        s.row(),
        None,
        "completed or cancelled capture has no target row"
    );
    assert_eq!(s.process(&[down(EKey::J)], 17, true).captured, None);
    let mut s = listening();
    let out = s.process(&[down(EKey::J)], 18, true);
    assert!(out.cancelled);
    assert_eq!(out.captured, None);
    assert!(!s.is_listening());
    assert_eq!(
        s.row(),
        None,
        "completed or cancelled capture has no target row"
    );
}

#[test]
fn text_ime_paste_and_mouse_never_bind_and_consumption_is_sorted_unique() {
    let mut s = listening();
    let events = vec![
        Event::Text("J".into()),
        Event::Paste("K".into()),
        Event::Ime(egui::ImeEvent::Commit("L".into())),
        Event::PointerButton {
            pos: egui::pos2(5.0, 5.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ];
    let out = s.process(&events, 17, true);
    assert_eq!(out.captured, None);
    assert!(s.is_listening());
    let out = s.process(
        &[
            down(EKey::J),
            up(EKey::J),
            down(EKey::K),
            Event::Text("junk".into()),
        ],
        17,
        true,
    );
    assert_eq!(out.captured, Some((2, Key::J)));
    assert!(out.consumed_event_indices.windows(2).all(|w| w[0] < w[1]));
    assert!(out.consumed_event_indices.contains(&0));
}

#[test]
fn live_validation_matches_apply_for_same_and_cross_action_conflicts() {
    for draft in [vec![key(Key::J), key(Key::J)], vec![key(Key::D)]] {
        let mut p = Panel::default();
        let accepted = p.document().clone();
        p.replace_draft(draft.clone()).unwrap();
        let error = p.validate_draft().unwrap_err();
        assert!(error.contains("duplicate physical binding"));
        assert_eq!(p.apply_draft().unwrap_err(), error);
        assert_eq!(p.document(), &accepted);
        assert_eq!(p.draft(), draft);
        assert!(!p.can_undo());
        assert!(!p.can_redo());
    }
}

#[test]
fn invalid_draft_save_and_guards_preserve_accepted_map_and_redo() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Panel::default();
    let initial = p.document().clone();
    p.replace_draft(vec![key(Key::J)]).unwrap();
    p.apply_draft().unwrap();
    let changed = p.document().clone();
    assert!(p.undo().unwrap());
    assert!(p.can_redo());
    p.replace_draft(vec![key(Key::D)]).unwrap();
    assert!(p.validate_draft().is_err());
    let file = dir.path().join("accepted.json");
    p.save_as(&file).unwrap();
    assert_eq!(ActionMap::load_file(&file).unwrap(), initial);
    assert!(p.undo().is_err());
    assert!(p.redo().is_err());
    assert!(p.open(&file).is_err());
    assert!(p.use_defaults().is_err());
    assert!(p.select_action(1).is_err());
    assert!(p.apply_document(changed.clone()).is_err());
    assert_eq!(p.draft(), [key(Key::D)]);
    assert_eq!(p.document(), &initial);
    assert!(p.can_redo());
    p.discard_draft();
    p.validate_draft().unwrap();
    p.apply_draft().unwrap();
    assert!(p.can_redo(), "no-op Apply must not destroy redo");
    assert!(p.redo().unwrap());
    assert_eq!(p.document(), &changed);
}

#[test]
fn live_validation_accepts_exact_eight_and_rejects_ninth_without_mutation() {
    let mut p = Panel::default();
    let initial = p.document().clone();
    let eight = vec![
        Key::B,
        Key::C,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
    ]
    .into_iter()
    .map(key)
    .collect::<Vec<_>>();
    p.replace_draft(eight.clone()).unwrap();
    p.validate_draft().unwrap();
    let mut nine = eight.clone();
    nine.push(key(Key::K));
    assert!(p.replace_draft(nine).is_err());
    assert_eq!(p.draft(), eight);
    assert_eq!(p.document(), &initial);
    assert!(!p.can_undo());
    p.apply_draft().unwrap();
    assert_eq!(p.selected_action().bindings, eight);
    assert!(p.undo().unwrap());
    assert_eq!(p.document(), &initial);
    assert!(p.redo().unwrap());
    assert_eq!(p.selected_action().bindings, eight);
}

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size([900.0, 850.0])
        .with_max_steps(120)
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(|_| App::default());
    settle(&mut h);
    h
}
fn settle(h: &mut Harness<'_, App>) {
    h.try_run().expect("CPU app layout must settle");
}
fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    settle(h);
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "{label}"
    );
    h.get_by_role_and_label(Role::Button, label).click();
    settle(h);
}
fn batch(h: &mut Harness<'_, App>, events: impl IntoIterator<Item = Event>) {
    // Harness::event runs a frame per call; extend raw input for genuine same-frame cases.
    h.input_mut().events.extend(events);
    h.step();
    settle(h);
}

#[test]
fn app_capture_changes_only_draft_and_apply_is_one_undoable_edit() {
    let mut h = harness();
    let initial = h.state().panel().document().clone();
    click(&mut h, "Capture binding 1");
    assert!(h.state().panel().capture_is_listening());
    batch(&mut h, [down(EKey::J), up(EKey::J)]);
    assert!(!h.state().panel().capture_is_listening());
    assert_eq!(h.state().panel().draft()[0], key(Key::J));
    assert_eq!(h.state().panel().document(), &initial);
    assert!(!h.state().panel().can_undo());
    h.state().panel().validate_draft().unwrap();
    click(&mut h, "Apply action");
    let accepted = h.state().panel().document().clone();
    assert_ne!(accepted, initial);
    click(&mut h, "Undo map edit");
    assert_eq!(h.state().panel().document(), &initial);
    assert!(!h.state().panel().can_undo());
    click(&mut h, "Redo map edit");
    assert_eq!(h.state().panel().document(), &accepted);
}

#[test]
fn app_logical_escape_and_tab_with_physical_j_capture_without_widget_side_effects() {
    for logical in [EKey::Escape, EKey::Tab] {
        let mut h = harness();
        let initial = h.state().panel().document().clone();
        h.get_by_role(Role::TextInput).focus();
        settle(&mut h);
        h.get_by_role(Role::TextInput)
            .type_text("unchanged-path.json");
        settle(&mut h);
        let path = h.get_by_role(Role::TextInput).accesskit_node().value();
        click(&mut h, "Capture binding 1");
        batch(
            &mut h,
            [
                event(logical, Some(EKey::J), true, false, Modifiers::NONE),
                Event::Text("must-not-leak".into()),
                Event::Paste("must-not-paste".into()),
                event(logical, Some(EKey::J), false, false, Modifiers::NONE),
            ],
        );
        assert_eq!(h.state().panel().draft()[0], key(Key::J));
        assert_eq!(h.state().panel().document(), &initial);
        assert!(!h.state().panel().can_undo());
        assert_eq!(
            h.get_by_role(Role::TextInput).accesskit_node().value(),
            path
        );
    }
}

#[test]
fn app_same_frame_focus_loss_and_regain_cancel_without_draft_change() {
    let mut h = harness();
    let initial = h.state().panel().document().clone();
    let draft = h.state().panel().draft().to_vec();
    click(&mut h, "Capture binding 1");
    batch(
        &mut h,
        [
            down(EKey::J),
            Event::WindowFocused(false),
            Event::WindowFocused(true),
            up(EKey::J),
        ],
    );
    assert!(!h.state().panel().capture_is_listening());
    assert_eq!(h.state().panel().draft(), draft);
    assert_eq!(h.state().panel().document(), &initial);
    assert!(!h.state().panel().can_undo());
    click(&mut h, "Capture binding 1");
    click(&mut h, "Cancel capture");
    assert!(!h.state().panel().capture_is_listening());
    assert_eq!(h.state().panel().draft(), draft);
}

#[test]
fn app_modal_text_paste_ime_and_pointer_cannot_mutate_underlying_widgets() {
    let mut h = harness();
    let initial = h.state().panel().document().clone();
    let draft = h.state().panel().draft().to_vec();
    h.get_by_role(Role::TextInput).focus();
    settle(&mut h);
    h.get_by_role(Role::TextInput)
        .type_text("protected-path.json");
    settle(&mut h);
    let path = h.get_by_role(Role::TextInput).accesskit_node().value();
    let pos = h
        .get_by_role_and_label(Role::Button, "Clear action draft")
        .rect()
        .center();
    click(&mut h, "Capture binding 1");
    batch(
        &mut h,
        [
            Event::Text("text".into()),
            Event::Paste("paste".into()),
            Event::Ime(egui::ImeEvent::Commit("ime".into())),
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
            Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    assert!(h.state().panel().capture_is_listening());
    assert_eq!(h.state().panel().draft(), draft);
    assert_eq!(h.state().panel().document(), &initial);
    assert!(!h.state().panel().can_undo());
    click(&mut h, "Cancel capture");
    assert_eq!(
        h.get_by_role(Role::TextInput).accesskit_node().value(),
        path
    );
}

#[test]
fn app_captured_conflict_is_live_and_discard_restores_accepted_row() {
    let mut h = harness();
    let initial = h.state().panel().document().clone();
    let draft = h.state().panel().draft().to_vec();
    click(&mut h, "Capture binding 1");
    batch(&mut h, [down(EKey::D), up(EKey::D)]);
    assert_eq!(h.state().panel().draft()[0], key(Key::D));
    assert!(h
        .state()
        .panel()
        .validate_draft()
        .unwrap_err()
        .contains("duplicate physical binding"));
    assert_eq!(h.state().panel().document(), &initial);
    assert!(!h.state().panel().can_undo());
    click(&mut h, "Apply action");
    assert!(h
        .state()
        .panel()
        .message()
        .contains("duplicate physical binding"));
    assert_eq!(h.state().panel().document(), &initial);
    click(&mut h, "Discard draft");
    assert_eq!(h.state().panel().draft(), draft);
    h.state().panel().validate_draft().unwrap();
    assert!(!h.state().panel().can_undo());
}

#[test]
fn app_missing_physical_escape_refuses_without_closing_modal() {
    let mut h = harness();
    let draft = h.state().panel().draft().to_vec();
    click(&mut h, "Capture binding 1");
    batch(
        &mut h,
        [event(EKey::Escape, None, true, false, Modifiers::NONE)],
    );
    assert!(h.state().panel().capture_is_listening());
    assert_eq!(h.state().panel().draft(), draft);
    assert!(h
        .state()
        .panel()
        .capture_message()
        .contains("Physical key identity is unavailable"));
    batch(
        &mut h,
        [
            event(EKey::J, Some(EKey::Escape), true, false, Modifiers::NONE),
            up(EKey::Escape),
        ],
    );
    assert!(!h.state().panel().capture_is_listening());
    assert_eq!(h.state().panel().draft(), draft);
}
