//! CPU regressions for production generic numeric widgets, without custom panels.
#![allow(clippy::float_arithmetic)]
use egui::{accesskit::Role, Id, Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::inspector::{show_component, InspEvent};
use orr_reflect::{FieldDesc, IntKind, TypeDesc, Value};

struct State {
    target: i128,
    visible: bool,
    enabled: bool,
    edits: Vec<InspEvent>,
}
fn harness() -> Harness<'static, State> {
    Harness::builder().with_size([500.0, 300.0]).build_ui_state(
        |ui, state: &mut State| {
            if !state.visible {
                ui.label("Inspector hidden");
                return;
            }
            let desc = TypeDesc::structure(
                8,
                vec![
                    FieldDesc::new("before", 0, TypeDesc::int(IntKind::U32)),
                    FieldDesc::new("target", 4, TypeDesc::int(IntKind::U32)),
                ],
            );
            let value = Value::Struct(vec![
                ("before".into(), Value::Int(7)),
                ("target".into(), Value::Int(state.target)),
            ]);
            let mut events = Vec::new();
            ui.add_enabled_ui(state.enabled, |ui| {
                show_component(ui, Id::new("numbers"), &desc, &value, &mut events)
            });
            for event in &events {
                if let InspEvent::Set {
                    path,
                    value: Value::Int(value),
                } = event
                {
                    if path == "target" {
                        state.target = *value;
                    }
                }
            }
            state.edits.extend(events);
        },
        State {
            target: 10,
            visible: true,
            enabled: true,
            edits: Vec::new(),
        },
    )
}
fn draft(h: &mut Harness<'_, State>, original: &str, next: &str) {
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some(original))
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(1);
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some(original))
        .type_text(next);
    h.run_steps(2);
    assert!(h.state().edits.is_empty());
}
#[test]
fn enter_commits_once() {
    let mut h = harness();
    draft(&mut h, "10", "20");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().target, 20);
    assert_eq!(h.state().edits.len(), 1);
    h.run_steps(3);
    assert_eq!(h.state().edits.len(), 1);
}
#[test]
fn programmatic_focus_to_earlier_field_commits_once() {
    let mut h = harness();
    draft(&mut h, "10", "20");
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("7"))
        .focus();
    h.run_steps(3);
    assert_eq!(h.state().target, 20);
    assert_eq!(h.state().edits.len(), 1);
    h.run_steps(3);
    assert_eq!(h.state().edits.len(), 1);
}
#[test]
fn pointer_focus_to_earlier_field_commits_once() {
    let mut h = harness();
    draft(&mut h, "10", "20");
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("7"))
        .click();
    h.run_steps(3);
    assert_eq!(h.state().target, 20);
    assert_eq!(h.state().edits.len(), 1);
}
#[test]
fn escape_cancels_and_refocus_keeps_canonical_value() {
    let mut h = harness();
    draft(&mut h, "10", "20");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().target, 10);
    assert!(h.state().edits.is_empty());
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("10"))
        .focus();
    h.run_steps(2);
    assert!(h
        .query_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("20"))
        .is_none());
}

#[test]
fn disabled_pending_draft_never_commits() {
    let mut h = harness();
    draft(&mut h, "10", "20");
    h.state_mut().enabled = false;
    h.run_steps(3);
    assert_eq!(h.state().target, 10);
    assert!(h.state().edits.is_empty());
    h.state_mut().enabled = true;
    h.run_steps(2);
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("10"))
        .focus();
    h.run_steps(2);
    assert!(h
        .query_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("20"))
        .is_none());
}
#[test]
fn hidden_for_one_frame_discards_draft_without_commit_on_restore() {
    let mut h = harness();
    draft(&mut h, "10", "20");
    h.state_mut().visible = false;
    h.run_steps(1);
    h.state_mut().visible = true;
    h.run_steps(2);
    assert_eq!(h.state().target, 10);
    assert!(h.state().edits.is_empty());
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("10"))
        .focus();
    h.run_steps(2);
    assert!(h
        .query_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("20"))
        .is_none());
}
#[test]
fn tab_commits_once() {
    let mut h = harness();
    draft(&mut h, "10", "20");
    h.key_press(Key::Tab);
    h.run_steps(3);
    assert_eq!(h.state().target, 20);
    assert_eq!(h.state().edits.len(), 1);
}
