//! Entity fields of the generic inspector: a combo box of the scene's entities.
use egui::{accesskit::Role, Id};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::inspector::{set_entity_choices, show_component, InspEvent};
use orr_reflect::{FieldDesc, TypeDesc, Value};

struct State {
    body: Option<String>,
    edits: Vec<InspEvent>,
}

fn harness(body: Option<&str>) -> Harness<'static, State> {
    Harness::builder().with_size([500.0, 300.0]).build_ui_state(
        |ui, state: &mut State| {
            set_entity_choices(ui.ctx(), vec![("e_00000001".into(), "pivot".into()), ("e_00000002".into(), "door".into())]);
            let desc = TypeDesc::structure(8, vec![FieldDesc::new("a", 0, TypeDesc::entity())]);
            let value = Value::Struct(vec![("a".into(), Value::EntityGuid(state.body.clone()))]);
            let mut events = Vec::new();
            show_component(ui, Id::new("joint"), &desc, &value, &mut events);
            for event in &events {
                if let InspEvent::Set { path, value: Value::EntityGuid(g) } = event {
                    assert_eq!(path, "a");
                    state.body = g.clone();
                }
            }
            state.edits.extend(events);
        },
        State { body: body.map(str::to_string), edits: Vec::new() },
    )
}

/// The text the entity field's combo box shows.
fn combo(h: &Harness<'_, State>) -> String {
    h.get_by_role(Role::ComboBox).value().unwrap_or_default()
}

#[test]
fn an_entity_field_shows_the_label_and_picks_another_entity() {
    let mut h = harness(Some("e_00000001"));
    h.run();
    assert_eq!(combo(&h), "pivot", "the field shows the entity's label");
    h.get_by_role(Role::ComboBox).click();
    h.run();
    h.get_by_label("door").click();
    h.run();
    assert_eq!(h.state().body.as_deref(), Some("e_00000002"));
    assert_eq!(h.state().edits.len(), 1, "one pick is one edit");
}

#[test]
fn an_entity_field_can_be_cleared_to_none() {
    let mut h = harness(Some("e_00000002"));
    h.run();
    h.get_by_role(Role::ComboBox).click();
    h.run();
    h.get_by_label("none").click();
    h.run();
    assert_eq!(h.state().body, None);
    // Picking the entity it already has changes nothing.
    let mut h = harness(None);
    h.run();
    assert_eq!(combo(&h), "none");
    h.get_by_role(Role::ComboBox).click();
    h.run();
    h.get_by_label("none").click();
    h.run();
    assert!(h.state().edits.is_empty());
}
