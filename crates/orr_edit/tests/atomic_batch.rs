mod common;

use common::*;
use orr_edit::{Op, Origin};
use orr_reflect::Value;

fn rename(guid: &orr_reflect::Guid, name: &str) -> Op {
    Op::Rename {
        guid: guid.clone(),
        name: Some(name.into()),
    }
}

fn invalid_position(guid: &orr_reflect::Guid) -> Op {
    Op::SetField {
        guid: guid.clone(),
        component: "orr_physics::Body".into(),
        path: "pos.x".into(),
        value: Value::Bool(true),
    }
}

#[test]
fn rejected_batch_preserves_nonempty_redo_and_all_history_metadata() {
    let mut doc = demo_doc();
    let mut control = demo_doc();
    let first = guid_named(&doc, "body_01");
    let second = guid_named(&doc, "body_02");
    let origin = Origin::Agent("before-batch".into());

    for target in [&mut doc, &mut control] {
        target
            .apply(rename(&first, "first edit"), Origin::User)
            .unwrap();
        target
            .apply(rename(&second, "redo entry"), origin.clone())
            .unwrap();
        target.undo().unwrap();
    }

    let before_scene = state(&doc);
    let before_history = doc.history();
    let before_revision = doc.revision();
    let before_dirty = doc.is_dirty();
    let before_can_undo = doc.can_undo();
    let before_can_redo = doc.can_redo();
    assert!(before_can_undo && before_can_redo);
    assert_eq!(before_history.len(), 2);
    assert!(!before_history[0].undone && before_history[1].undone);

    let rejected = doc.apply_atomic_batch(
        "must be atomic",
        vec![
            Op::SpawnEntity {
                guid: None,
                name: Some("staged prefix".into()),
                components: vec![],
            },
            invalid_position(&first),
        ],
        Origin::Agent("batch".into()),
    );
    assert!(rejected.is_err());

    assert_eq!(state(&doc), before_scene);
    assert_eq!(doc.history(), before_history);
    assert_eq!(doc.revision(), before_revision);
    assert_eq!(doc.is_dirty(), before_dirty);
    assert_eq!(doc.can_undo(), before_can_undo);
    assert_eq!(doc.can_redo(), before_can_redo);
    assert!(!doc.in_tx());
    assert_synced(&doc);

    // The redo entry survived the rejected batch and remains executable.
    doc.redo().unwrap();
    control.redo().unwrap();
    assert_eq!(state(&doc), state(&control));
    assert_eq!(doc.history(), control.history());

    // The failed staged spawn did not consume the next scene GUID either.
    let spawn = |d: &mut orr_edit::EditorDoc| {
        d.apply(
            Op::SpawnEntity {
                guid: None,
                name: Some("after rejection".into()),
                components: vec![],
            },
            Origin::User,
        )
        .unwrap()
    };
    assert_eq!(spawn(&mut doc).guid, spawn(&mut control).guid);
    assert_eq!(state(&doc), state(&control));
    assert_eq!(doc.history(), control.history());
}

#[test]
fn rejected_position_prefix_preserves_nonempty_redo() {
    let mut doc = demo_doc();
    let mut control = demo_doc();
    let first = guid_named(&doc, "body_01");
    let second = guid_named(&doc, "body_02");
    let origin = Origin::Agent("before-position-batch".into());

    for target in [&mut doc, &mut control] {
        target
            .apply(rename(&first, "first edit"), Origin::User)
            .unwrap();
        target
            .apply(rename(&second, "redo entry"), origin.clone())
            .unwrap();
        target.undo().unwrap();
    }

    let before_scene = state(&doc);
    let before_history = doc.history();
    let before_revision = doc.revision();
    assert!(doc.can_undo() && doc.can_redo());

    let rejected = doc.apply_atomic_batch(
        "position then invalid suffix",
        vec![
            Op::SetField {
                guid: first.clone(),
                component: "orr_physics::Body".into(),
                path: "pos".into(),
                value: vec2(123, 45),
            },
            invalid_position(&first),
        ],
        Origin::Agent("position-batch".into()),
    );
    assert!(rejected.is_err());

    assert_eq!(state(&doc), before_scene);
    assert_eq!(doc.history(), before_history);
    assert_eq!(doc.revision(), before_revision);
    assert!(doc.can_undo() && doc.can_redo());
    assert!(!doc.in_tx());
    assert_synced(&doc);

    // The pre-existing redo remains the same operation and still applies.
    doc.redo().unwrap();
    control.redo().unwrap();
    assert_eq!(state(&doc), state(&control));
    assert_eq!(doc.history(), control.history());
}

#[test]
fn successful_batch_is_one_undo_and_redo_entry() {
    let mut doc = demo_doc();
    let first = guid_named(&doc, "body_01");
    let second = guid_named(&doc, "body_02");
    let before = state(&doc);

    let applied = doc
        .apply_atomic_batch(
            "rename pair",
            vec![rename(&first, "one"), rename(&second, "two")],
            Origin::Agent("batch".into()),
        )
        .unwrap();
    assert_eq!(applied.len(), 2);
    assert!(applied.iter().all(|a| a.changed));
    assert_ne!(state(&doc), before);
    assert_eq!(doc.history().len(), 1);
    assert_eq!(doc.history()[0].label, "rename pair");
    assert_eq!(doc.history()[0].origin, Origin::Agent("batch".into()));
    assert_eq!(doc.history()[0].op_count, 2);
    assert!(doc.can_undo() && !doc.can_redo());

    let after = state(&doc);
    doc.undo().unwrap();
    assert_eq!(state(&doc), before);
    assert!(doc.history()[0].undone);
    doc.redo().unwrap();
    assert_eq!(state(&doc), after);
    assert!(!doc.history()[0].undone);
}

#[test]
fn noop_batch_does_not_clear_redo_or_add_history() {
    let mut doc = demo_doc();
    let body = guid_named(&doc, "body_01");
    let after_edit = {
        doc.apply(rename(&body, "renamed"), Origin::User).unwrap();
        state(&doc)
    };
    doc.undo().unwrap();

    let before_scene = state(&doc);
    let before_history = doc.history();
    let before_revision = doc.revision();
    assert!(doc.can_redo());

    let applied = doc
        .apply_atomic_batch(
            "noop",
            vec![rename(&body, "body_01")],
            Origin::Agent("batch".into()),
        )
        .unwrap();
    assert_eq!(applied.len(), 1);
    assert!(!applied[0].changed);
    assert_eq!(state(&doc), before_scene);
    assert_eq!(doc.history(), before_history);
    assert_eq!(doc.revision(), before_revision);
    assert!(!doc.can_undo() && doc.can_redo());

    doc.redo().unwrap();
    assert_eq!(state(&doc), after_edit);
}

#[test]
fn isolated_batch_api_preserves_revision_while_legacy_batch_keeps_rollback_versioning() {
    let body = guid_named(&demo_doc(), "body_01");
    let mut legacy = demo_doc();
    let legacy_before = state(&legacy);
    let legacy_revision = legacy.revision();
    assert!(legacy
        .apply_batch(
            "legacy rejected batch",
            vec![invalid_position(&body)],
            Origin::User
        )
        .is_err());
    assert_eq!(state(&legacy), legacy_before);
    assert!(
        legacy.revision() > legacy_revision,
        "legacy rollback continues to invalidate verification state"
    );

    let mut isolated = demo_doc();
    let isolated_before = state(&isolated);
    let isolated_revision = isolated.revision();
    assert!(isolated
        .apply_atomic_batch(
            "isolated rejected batch",
            vec![invalid_position(&body)],
            Origin::User
        )
        .is_err());
    assert_eq!(state(&isolated), isolated_before);
    assert_eq!(
        isolated.revision(),
        isolated_revision,
        "atomic rejection preserves its complete pre-call state"
    );
}
