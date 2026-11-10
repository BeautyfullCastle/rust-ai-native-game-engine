#![cfg(feature = "custom-inspector")]
use egui::{Id, Ui};
use orr_editor::{custom_inspector::*, inspector::InspEvent};
use orr_reflect::{FieldDesc, IntKind, TypeDesc, Value};

fn key() -> SchemaKey {
    SchemaKey {
        type_name: "example::Health".into(),
        schema_json: "schema-42".into(),
    }
}
fn desc() -> TypeDesc {
    TypeDesc::structure(
        4,
        vec![FieldDesc::new(
            "hp",
            0,
            TypeDesc::int(IntKind::U32).with_range_str("0..=100"),
        )],
    )
}
fn snapshot() -> Snapshot {
    Snapshot::new(
        key(),
        desc(),
        Value::Struct(vec![("hp".into(), Value::Int(50))]),
    )
    .unwrap()
}
fn draw(_: &mut Ui, _: Id, _: &Snapshot) -> Option<FieldEdit> {
    Some(FieldEdit {
        field: "hp".into(),
        value: Value::Int(75),
    })
}
fn forbidden(_: &mut Ui, _: Id, _: &Snapshot) -> Option<FieldEdit> {
    panic!("disabled callback invoked")
}
fn extension(id: &'static str, priority: i32) -> Extension {
    Extension {
        id,
        schema: key(),
        priority,
        api_version: 1,
        draw,
    }
}
fn run(registry: &Registry, snapshot: &Snapshot, editable: bool) -> Vec<InspEvent> {
    let ctx = egui::Context::default();
    let mut events = Vec::new();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        registry.show(ui, Id::new("health"), snapshot, editable, &mut events)
    });
    output.textures_delta.clear();
    events
}

#[test]
fn exact_schema_and_type_or_generic_fallback() {
    let mut r = Registry::default();
    assert!(r.resolve(&key()).is_none());
    r.register(extension("health", 0)).unwrap();
    assert_eq!(r.resolve(&key()).unwrap().id, "health");
    assert!(r
        .resolve(&SchemaKey {
            schema_json: "schema-43".into(),
            ..key()
        })
        .is_none());
    assert!(r
        .resolve(&SchemaKey {
            type_name: "other::Health".into(),
            ..key()
        })
        .is_none());
    assert!(run(&Registry::default(), &snapshot(), true).is_empty());
}

#[test]
fn priority_is_independent_of_registration_order() {
    for reverse in [false, true] {
        let mut r = Registry::default();
        let mut entries = [extension("low", -1), extension("high", 10)];
        if reverse {
            entries.reverse();
        }
        for e in entries {
            r.register(e).unwrap();
        }
        assert_eq!(r.resolve(&key()).unwrap().id, "high");
    }
}

#[test]
fn duplicate_and_unsupported_registration_leave_original_intact() {
    let mut r = Registry::default();
    r.register(extension("health", 0)).unwrap();
    assert!(r.register(extension("health", 100)).is_err());
    assert!(r.register(extension("other", 0)).is_err());
    let mut future = extension("future", 100);
    future.api_version = 2;
    assert!(r.register(future).is_err());
    assert!(r.register(extension("", 100)).is_err());
    let mut empty_type = extension("empty-type", 100);
    empty_type.schema.type_name.clear();
    assert!(r.register(empty_type).is_err());
    assert_eq!(r.resolve(&key()).unwrap().id, "health");
}

#[test]
fn checked_edit_preserves_owned_snapshot() {
    let s = snapshot();
    let before = s.value().clone();
    assert_eq!(
        s.checked_edit(FieldEdit {
            field: "hp".into(),
            value: Value::Int(75)
        })
        .unwrap(),
        InspEvent::Set {
            path: "hp".into(),
            value: Value::Int(75)
        }
    );
    assert_eq!(s.value(), &before);
}

#[test]
fn invalid_intents_reject_without_mutation() {
    let s = snapshot();
    let before = s.value().clone();
    for (field, value) in [
        ("hp", Value::Int(-1)),
        ("hp", Value::Int(101)),
        ("hp", Value::Bool(true)),
        ("missing", Value::Int(2)),
        ("hp.x", Value::Int(2)),
        ("hp[0]", Value::Int(2)),
        ("", Value::Int(2)),
    ] {
        assert!(s
            .checked_edit(FieldEdit {
                field: field.into(),
                value
            })
            .is_err());
        assert_eq!(s.value(), &before);
    }
}

#[test]
fn malformed_snapshots_and_structural_edits_reject() {
    assert!(Snapshot::new(key(), desc(), Value::Struct(vec![])).is_err());
    assert!(Snapshot::new(
        key(),
        desc(),
        Value::Struct(vec![("hp".into(), Value::Int(101))])
    )
    .is_err());
    assert!(Snapshot::new(
        SchemaKey {
            type_name: String::new(),
            ..key()
        },
        desc(),
        snapshot().value().clone()
    )
    .is_err());
    let d = TypeDesc::structure(
        4,
        vec![FieldDesc::new(
            "items",
            0,
            TypeDesc::array(TypeDesc::int(IntKind::U32), 1),
        )],
    );
    let s = Snapshot::new(
        key(),
        d,
        Value::Struct(vec![("items".into(), Value::Array(vec![Value::Int(1)]))]),
    )
    .unwrap();
    assert!(s
        .checked_edit(FieldEdit {
            field: "items".into(),
            value: Value::Array(vec![Value::Int(2)])
        })
        .is_err());
}

#[test]
fn custom_draw_emits_normal_event_and_disabled_skips_callback() {
    let mut r = Registry::default();
    r.register(extension("health", 0)).unwrap();
    assert_eq!(
        run(&r, &snapshot(), true),
        vec![InspEvent::Set {
            path: "hp".into(),
            value: Value::Int(75)
        }]
    );
    let mut disabled = extension("disabled", 10);
    disabled.draw = forbidden;
    r.register(disabled).unwrap();
    assert!(run(&r, &snapshot(), false).is_empty());
    assert!(run(&Registry::default(), &snapshot(), false).is_empty());
}

#[test]
fn invalid_callback_output_is_only_an_error_event() {
    let mut bad = extension("bad", 0);
    bad.draw = |_, _, _| {
        Some(FieldEdit {
            field: "hp".into(),
            value: Value::Int(1000),
        })
    };
    let mut r = Registry::default();
    r.register(bad).unwrap();
    assert!(matches!(
        run(&r, &snapshot(), true).as_slice(),
        [InspEvent::Error(_)]
    ));
}

#[test]
fn duplicate_snapshot_fields_are_rejected_recursively() {
    let duplicate = Value::Struct(vec![
        ("hp".into(), Value::Int(50)),
        ("hp".into(), Value::Bool(true)),
    ]);
    assert!(Snapshot::new(key(), desc(), duplicate.clone()).is_err());
    let nested = TypeDesc::structure(
        4,
        vec![FieldDesc::new("children", 0, TypeDesc::array(desc(), 1))],
    );
    assert!(Snapshot::new(
        key(),
        nested,
        Value::Struct(vec![("children".into(), Value::Array(vec![duplicate]))])
    )
    .is_err());
}

#[test]
fn disabled_parent_ui_never_invokes_extension() {
    let mut r = Registry::default();
    let mut e = extension("disabled-parent", 0);
    e.draw = forbidden;
    r.register(e).unwrap();
    let mut events = Vec::new();
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.disable();
        r.show(
            ui,
            Id::new("disabled-parent"),
            &snapshot(),
            true,
            &mut events,
        );
    });
    output.textures_delta.clear();
    assert!(events.is_empty());
}
