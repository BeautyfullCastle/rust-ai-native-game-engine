mod common;

use common::*;
use orr_edit::{EditError, Op, Origin};
use orr_reflect::Value;

fn set(guid: &orr_reflect::Guid, component: &str, path: &str, value: Value) -> Op {
    Op::SetField { guid: guid.clone(), component: component.into(), path: path.into(), value }
}

/// Applies `op`, checks the state changed, undoes (exact state back), redoes
/// (same state as after the first apply) and undoes again.
fn check_roundtrip(doc: &mut orr_edit::EditorDoc, op: Op) {
    let before = state(doc);
    let hist = doc.history().len();
    let applied = doc.apply(op.clone(), Origin::User).unwrap_or_else(|e| panic!("{op:?}: {e}"));
    assert!(applied.changed, "{op:?} changed nothing");
    let after = state(doc);
    assert_ne!(before, after, "{op:?}");
    assert_synced(doc);
    assert_eq!(doc.history().len(), hist + 1);
    doc.undo().unwrap();
    assert_eq!(state(doc), before, "undo of {op:?}");
    assert_synced(doc);
    doc.redo().unwrap();
    assert_eq!(state(doc), after, "redo of {op:?}");
    doc.undo().unwrap();
    assert_eq!(state(doc), before, "second undo of {op:?}");
    doc.redo().unwrap(); // leave the edit applied
}

#[test]
fn every_op_kind_undoes_and_redoes_exactly() {
    let mut doc = demo_doc();
    let body = guid_named(&doc, "body_01");
    let paddle = guid_named(&doc, "paddle_0");
    let floor = guid_named(&doc, "floor");

    check_roundtrip(&mut doc, set(&body, "orr_physics::Body", "pos.x", fixed(3)));
    check_roundtrip(&mut doc, set(&body, "orr_physics::Body", "pos", vec2(1, 9)));
    check_roundtrip(&mut doc, set(&body, "orr_physics::Body", "kind", Value::Enum("static".into())));
    check_roundtrip(&mut doc, set(&body, "orr_physics::Collider", "friction", fixed(2)));
    check_roundtrip(&mut doc, Op::RemoveComponent { guid: paddle.clone(), component: "PaddleTag".into() });
    check_roundtrip(&mut doc, Op::AddComponent { guid: floor.clone(), component: "PaddleTag".into(), value: None });
    let tag = Value::Struct(vec![("slot".into(), Value::Int(1))]);
    check_roundtrip(&mut doc, Op::AddComponent { guid: body.clone(), component: "PaddleTag".into(), value: Some(tag.clone()) });
    check_roundtrip(&mut doc, Op::Rename { guid: body.clone(), name: Some("hero".into()) });
    check_roundtrip(&mut doc, Op::Rename { guid: body.clone(), name: None });
    check_roundtrip(&mut doc, set_singleton("Scene", "max_entities", Value::Int(500)));
    check_roundtrip(&mut doc, set_singleton("orr_physics::PhysicsState", "config.gravity", vec2(0, -3)));
    check_roundtrip(&mut doc, Op::RemoveSingleton { singleton: "Scene".into() });

    // Spawn with a chosen GUID, then with a made one; despawn both.
    let g = orr_reflect::Guid::parse("e_deadbeef").unwrap();
    let spawn = Op::SpawnEntity {
        guid: Some(g.clone()),
        name: Some("crate".into()),
        components: vec![("PaddleTag".into(), tag.clone())],
    };
    check_roundtrip(&mut doc, spawn);
    check_roundtrip(&mut doc, Op::DespawnEntity { guid: g });
    let made = doc.apply(Op::SpawnEntity { guid: None, name: None, components: vec![] }, Origin::User).unwrap();
    let made = made.guid.expect("a made GUID");
    assert!(doc.scene().entities.contains_key(&made));
    check_roundtrip(&mut doc, Op::DespawnEntity { guid: made });
    check_roundtrip(&mut doc, Op::DespawnEntity { guid: body });
}

fn set_singleton(name: &str, path: &str, value: Value) -> Op {
    Op::SetSingletonField { singleton: name.into(), path: path.into(), value }
}

#[test]
fn undo_everything_returns_to_the_loaded_text() {
    let mut doc = demo_doc();
    let text = doc.to_yaml();
    let start = doc.checksum();
    let body = guid_named(&doc, "body_02");
    doc.apply(set(&body, "orr_physics::Body", "pos.y", fixed(30)), Origin::User).unwrap();
    doc.apply(Op::Rename { guid: body.clone(), name: Some("x".into()) }, Origin::User).unwrap();
    doc.apply(Op::DespawnEntity { guid: body }, Origin::User).unwrap();
    assert!(doc.is_dirty());
    while doc.can_undo() {
        doc.undo().unwrap();
    }
    assert!(!doc.is_dirty());
    assert_eq!(doc.to_yaml(), text);
    assert_eq!(doc.checksum(), start);
    assert_eq!(doc.scene().to_yaml(), text, "regenerated text matches too");
}

#[test]
fn transaction_is_one_undo_step() {
    let mut doc = demo_doc();
    let before = state(&doc);
    let a = guid_named(&doc, "body_01");
    let b = guid_named(&doc, "body_02");
    doc.begin_tx("nudge two bodies", Origin::User).unwrap();
    doc.apply(set(&a, "orr_physics::Body", "pos.x", fixed(1)), Origin::User).unwrap();
    doc.apply(set(&b, "orr_physics::Body", "pos.x", fixed(2)), Origin::User).unwrap();
    doc.apply(Op::Rename { guid: a.clone(), name: Some("a".into()) }, Origin::User).unwrap();
    assert!(doc.in_tx());
    assert!(doc.history().is_empty(), "an open tx is not in the history yet");
    assert_eq!(doc.undo(), Err(EditError::TxOpen));
    doc.commit_tx().unwrap();
    let h = doc.history();
    assert_eq!(h.len(), 1);
    assert_eq!((h[0].label.as_str(), h[0].op_count), ("nudge two bodies", 3));
    let after = state(&doc);
    doc.undo().unwrap();
    assert_eq!(state(&doc), before);
    doc.redo().unwrap();
    assert_eq!(state(&doc), after);
}

#[test]
fn rollback_restores_everything() {
    let mut doc = demo_doc();
    let before = state(&doc);
    let a = guid_named(&doc, "body_01");
    doc.begin_tx("oops", Origin::User).unwrap();
    doc.apply(set(&a, "orr_physics::Body", "pos.x", fixed(1)), Origin::User).unwrap();
    doc.apply(Op::DespawnEntity { guid: a.clone() }, Origin::User).unwrap();
    doc.apply(Op::SpawnEntity { guid: None, name: None, components: vec![] }, Origin::User).unwrap();
    assert_ne!(state(&doc), before);
    doc.rollback_tx().unwrap();
    assert_eq!(state(&doc), before);
    assert!(doc.history().is_empty() && !doc.can_undo() && !doc.in_tx());
    assert_synced(&doc);
    assert_eq!(doc.commit_tx(), Err(EditError::NoTx));
}

#[test]
fn consecutive_sets_of_one_field_coalesce_in_a_tx() {
    let mut doc = demo_doc();
    let before = state(&doc);
    let a = guid_named(&doc, "body_01");
    doc.begin_tx("drag", Origin::User).unwrap();
    for x in 1..=20 {
        doc.apply(set(&a, "orr_physics::Body", "pos.x", fixed(x)), Origin::User).unwrap();
    }
    doc.commit_tx().unwrap();
    assert_eq!(doc.history()[0].op_count, 1);
    assert_synced(&doc);
    let after = state(&doc);
    doc.undo().unwrap();
    assert_eq!(state(&doc), before, "one undo takes back the whole drag");
    doc.redo().unwrap();
    assert_eq!(state(&doc), after, "redo lands on the last value");

    // A different field or entity in between breaks the run.
    doc.begin_tx("two fields", Origin::User).unwrap();
    doc.apply(set(&a, "orr_physics::Body", "pos.x", fixed(50)), Origin::User).unwrap();
    doc.apply(set(&a, "orr_physics::Body", "pos.y", fixed(50)), Origin::User).unwrap();
    doc.apply(set(&a, "orr_physics::Body", "pos.x", fixed(51)), Origin::User).unwrap();
    doc.commit_tx().unwrap();
    assert_eq!(doc.history()[1].op_count, 3);
}

#[test]
fn empty_transaction_and_noop_edits_leave_no_history() {
    let mut doc = demo_doc();
    doc.begin_tx("nothing", Origin::User).unwrap();
    doc.commit_tx().unwrap();
    let a = guid_named(&doc, "body_01");
    let current = doc.view().field(&a.clone().into(), "orr_physics::Body", "pos.x").unwrap();
    let r = doc.apply(set(&a, "orr_physics::Body", "pos.x", current), Origin::User).unwrap();
    assert!(!r.changed);
    assert!(doc.history().is_empty());
    assert!(!doc.is_dirty());
}

#[test]
fn invalid_ops_change_nothing() {
    let mut doc = demo_doc();
    let a = guid_named(&doc, "body_01");
    let ghost = orr_reflect::Guid::parse("e_0badf00d").unwrap();
    let body = "orr_physics::Body";
    let bad: Vec<Op> = vec![
        set(&ghost, body, "pos.x", fixed(1)),
        set(&a, "Nope", "x", fixed(1)),
        set(&a, "Scene", "half_w", fixed(1)), // a singleton is not a component
        set(&a, body, "pos.q", fixed(1)),
        set(&a, body, "pos.x", Value::Bool(true)),
        set(&a, body, "kind", Value::Enum("sideways".into())),
        set(&a, "orr_physics::Collider", "friction", fixed(-5)), // range 0..=100
        set(&a, "orr_physics::Collider", "friction", fixed(101)),
        set(&a, "PaddleTag", "slot", Value::Int(1)), // body_01 has no PaddleTag
        Op::AddComponent { guid: a.clone(), component: body.into(), value: None }, // already has it
        Op::AddComponent { guid: ghost.clone(), component: body.into(), value: None },
        Op::AddComponent { guid: a.clone(), component: "PaddleTag".into(), value: Some(Value::Bool(true)) },
        Op::RemoveComponent { guid: a.clone(), component: "PaddleTag".into() },
        Op::RemoveComponent { guid: ghost.clone(), component: body.into() },
        Op::SpawnEntity { guid: Some(a.clone()), name: None, components: vec![] }, // GUID in use
        Op::SpawnEntity { guid: None, name: None, components: vec![("Nope".into(), Value::Bool(true))] },
        Op::SpawnEntity {
            guid: None,
            name: None,
            components: vec![("PaddleTag".into(), Value::Struct(vec![])), ("PaddleTag".into(), Value::Struct(vec![]))],
        },
        Op::DespawnEntity { guid: ghost.clone() },
        Op::Rename { guid: ghost.clone(), name: Some("x".into()) },
        set_singleton("Nope", "", Value::Bool(true)),
        set_singleton("orr_physics::Body", "x", Value::Bool(true)), // a component, not a singleton
        set_singleton("Scene", "max_entities", Value::Bool(true)),
        Op::RemoveSingleton { singleton: "Nope".into() },
        set(&a, body, "pos", Value::EntityGuid(Some("e_00000001".into()))), // wrong kind of value
    ];
    let before = state(&doc);
    for op in bad {
        let r = doc.apply(op.clone(), Origin::Agent("t".into()));
        assert!(r.is_err(), "{op:?} should be refused");
        assert_eq!(state(&doc), before, "{op:?} changed something");
        assert!(doc.history().is_empty() && !doc.is_dirty());
    }
    assert_synced(&doc);
    // A refused op inside a tx leaves the tx usable and its earlier ops intact.
    doc.begin_tx("t", Origin::User).unwrap();
    doc.apply(set(&a, body, "pos.x", fixed(4)), Origin::User).unwrap();
    let mid = state(&doc);
    assert!(doc.apply(set(&a, body, "pos.x", Value::Bool(false)), Origin::User).is_err());
    assert_eq!(state(&doc), mid);
    doc.rollback_tx().unwrap();
    assert_eq!(state(&doc), before);
}

#[test]
fn batch_is_all_or_nothing() {
    let mut doc = demo_doc();
    let a = guid_named(&doc, "body_01");
    let before = state(&doc);
    let ops = vec![set(&a, "orr_physics::Body", "pos.x", fixed(9)), set(&a, "orr_physics::Body", "pos.x", Value::Bool(true))];
    assert!(doc.apply_batch("bad batch", ops, Origin::Agent("bot".into())).is_err());
    assert_eq!(state(&doc), before);
    assert!(doc.history().is_empty() && !doc.in_tx());

    let ops = vec![
        set(&a, "orr_physics::Body", "pos.x", fixed(9)),
        Op::Rename { guid: a.clone(), name: Some("moved".into()) },
        Op::SpawnEntity { guid: None, name: Some("new".into()), components: vec![] },
    ];
    let out = doc.apply_batch("good batch", ops, Origin::Agent("bot".into())).unwrap();
    assert_eq!(out.len(), 3);
    assert!(out[2].guid.is_some());
    assert_eq!(doc.history().len(), 1);
    doc.undo().unwrap();
    assert_eq!(state(&doc), before);
}

#[test]
fn history_records_origin_and_undone_state() {
    let mut doc = demo_doc();
    let a = guid_named(&doc, "body_01");
    let bot = Origin::Agent("planner".into());
    doc.apply(set(&a, "orr_physics::Body", "pos.x", fixed(1)), Origin::User).unwrap();
    doc.apply_batch("agent batch", vec![Op::Rename { guid: a.clone(), name: Some("r".into()) }], bot.clone()).unwrap();
    doc.apply(set(&a, "orr_physics::Body", "pos.y", fixed(2)), bot.clone()).unwrap();
    let h = doc.history();
    assert_eq!(h.iter().map(|e| e.origin.clone()).collect::<Vec<_>>(), vec![Origin::User, bot.clone(), bot.clone()]);
    assert!(h[0].label.contains("Body.pos.x") && h[0].label.contains(a.as_str()), "{}", h[0].label);
    assert_eq!(h[1].label, "agent batch");
    assert!(h.iter().all(|e| !e.undone));
    assert!(h.windows(2).all(|w| w[0].id < w[1].id));

    doc.undo().unwrap();
    let h = doc.history();
    assert_eq!(h.len(), 3);
    assert!(h[2].undone && !h[1].undone);
    assert_eq!(h[2].origin, bot);

    // A new edit clears the redo stack.
    doc.apply(set(&a, "orr_physics::Body", "pos.y", fixed(3)), Origin::User).unwrap();
    assert!(!doc.can_redo());
    assert_eq!(doc.redo(), Err(EditError::NothingToRedo));
    assert_eq!(doc.history().len(), 3);
    assert_eq!(doc.history()[2].origin, Origin::User);
}

#[test]
fn a_transaction_of_one_origin_blocks_the_other() {
    let mut doc = demo_doc();
    let a = guid_named(&doc, "body_01");
    doc.begin_tx("drag", Origin::User).unwrap();
    let err = doc.apply(set(&a, "orr_physics::Body", "pos.x", fixed(1)), Origin::Agent("x".into())).unwrap_err();
    assert!(matches!(err, EditError::TxBusy { .. }));
    assert_eq!(doc.begin_tx("again", Origin::User), Err(EditError::TxOpen));
    doc.rollback_tx().unwrap();
    assert!(doc.apply(set(&a, "orr_physics::Body", "pos.x", fixed(1)), Origin::Agent("x".into())).is_ok());
}

#[test]
fn undo_on_empty_history_is_an_error() {
    let mut doc = empty_doc();
    assert_eq!(doc.undo(), Err(EditError::NothingToUndo));
    assert_eq!(doc.redo(), Err(EditError::NothingToRedo));
}

#[test]
fn dirty_flag_follows_the_saved_point() {
    let mut doc = demo_doc();
    let a = guid_named(&doc, "body_01");
    assert!(!doc.is_dirty());
    doc.apply(Op::Rename { guid: a.clone(), name: Some("q".into()) }, Origin::User).unwrap();
    assert!(doc.is_dirty());
    let saved = doc.save_yaml();
    assert!(!doc.is_dirty());
    assert_eq!(doc.to_yaml(), saved);
    doc.undo().unwrap();
    assert!(doc.is_dirty(), "undo moved away from the saved state");
    doc.redo().unwrap();
    assert!(!doc.is_dirty());
    assert_eq!(saved, doc.scene().to_yaml());
}

#[test]
fn comments_survive_edits() {
    let src = "\
# header comment
schema: orr.scene/1
entities:
  # the first one
  e_00000001:
    name: one
    # tag comment
    PaddleTag: { slot: 1 }
  e_00000002:
    name: two
";
    let mut doc = orr_edit::EditorDoc::from_yaml(src, types(), orr_sim::Simulation::<orr_sample::physics_game::PhysGame>::build_registry(), 1)
        .unwrap();
    assert_eq!(doc.to_yaml(), src, "unedited text comes back byte for byte");
    let g = orr_reflect::Guid::parse("e_00000001").unwrap();
    doc.apply(set(&g, "PaddleTag", "slot", Value::Int(3)), Origin::User).unwrap();
    let out = doc.to_yaml();
    assert!(out.contains("# header comment") && out.contains("# the first one") && out.contains("# tag comment"), "{out}");
    assert!(out.contains("slot: 3"), "{out}");
    // Comments of a removed entity go away with it.
    doc.apply(Op::DespawnEntity { guid: g }, Origin::User).unwrap();
    let out = doc.to_yaml();
    assert!(!out.contains("the first one") && out.contains("# header comment"), "{out}");
}

#[test]
fn load_yaml_replaces_the_document_and_rejects_bad_text() {
    let mut doc = demo_doc();
    let before = state(&doc);
    assert!(doc.load_yaml("schema: orr.scene/1\nentities:\n  nope: {}\n").is_err());
    assert_eq!(state(&doc), before);
    doc.load_yaml("schema: orr.scene/1\nentities:\n  e_00000009:\n    name: solo\n").unwrap();
    assert_eq!(doc.view().entities().len(), 1);
    assert!(doc.history().is_empty() && !doc.is_dirty());
}

#[test]
fn registry_mismatch_is_refused() {
    let mut reg = orr_reflect::TypeRegistry::new();
    reg.register_component::<orr_sample::physics_game::PaddleTag>("WrongName");
    let r = orr_edit::EditorDoc::new(reg, orr_sim::Simulation::<orr_sample::physics_game::PhysGame>::build_registry(), 1);
    assert!(matches!(r, Err(EditError::RegistryMismatch(_))));
}

#[test]
fn queries_list_entities_components_and_schema() {
    let doc = demo_doc();
    let v = doc.view();
    let all = v.entities();
    assert_eq!(all.len(), 49);
    let a = all.iter().find(|e| e.name.as_deref() == Some("body_01")).unwrap();
    assert_eq!(a.components, vec!["orr_physics::Body".to_string(), "orr_physics::Collider".to_string()]);
    assert!(a.guid.is_some());
    let comps = v.components(&a.guid.clone().unwrap().into()).unwrap();
    assert_eq!(comps.len(), 2);
    assert!(matches!(comps[0].1, Value::Struct(_)));
    let p = v.field(&a.entity.into(), "orr_physics::Body", "pos.x").unwrap();
    assert!(matches!(p, Value::Fixed(_)));
    assert!(v.singletons().iter().any(|(n, _)| n == "Scene"));
    assert!(matches!(v.singleton("Scene", "max_entities").unwrap(), Value::Int(20000)));
    assert!(v.json_schema().contains("orr_physics::Body"));
    assert!(v.type_schema("PaddleTag").is_some());
    assert!(v.entity(&orr_edit::Target::Guid(orr_reflect::Guid::parse("e_0badf00d").unwrap())).is_err());
    // The scene form of the doc and the query agree.
    let scene_val = &doc.scene().entities[a.guid.as_ref().unwrap()].components[0].1;
    assert_eq!(&comps[0].1, scene_val);
}

/// The scene format has no empty names: an empty one is refused (it used to
/// be accepted and made the saved scene unloadable).
#[test]
fn an_empty_entity_name_is_refused() {
    let mut doc = demo_doc();
    let body = guid_named(&doc, "body_01");
    let before = doc.to_yaml();
    let e = doc.apply(Op::Rename { guid: body.clone(), name: Some(String::new()) }, Origin::User).unwrap_err();
    assert!(matches!(e, EditError::Invalid(_)), "{e}");
    let e = doc.apply(Op::SpawnEntity { guid: None, name: Some(String::new()), components: Vec::new() }, Origin::User).unwrap_err();
    assert!(matches!(e, EditError::Invalid(_)), "{e}");
    assert_eq!(doc.to_yaml(), before);
    assert!(doc.history().is_empty());
    doc.apply(Op::Rename { guid: body, name: None }, Origin::User).unwrap();
}
