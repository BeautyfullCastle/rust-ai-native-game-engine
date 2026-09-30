mod common;

use common::*;
use orr_edit::{unified_diff, EditError, EditorDoc, Op, Origin, ProposalId};
use orr_reflect::Value;

const BODY: &str = "orr_physics::Body";

fn agent() -> Origin {
    Origin::Agent("bot".into())
}

fn set(guid: &orr_reflect::Guid, component: &str, path: &str, value: Value) -> Op {
    Op::SetField { guid: guid.clone(), component: component.into(), path: path.into(), value }
}

fn body_pos(view: orr_edit::View<'_>, guid: &orr_reflect::Guid) -> Value {
    view.field(&orr_edit::Target::Guid(guid.clone()), BODY, "pos").unwrap()
}

#[test]
fn staged_ops_show_in_the_preview_and_not_in_the_document() {
    let mut doc = demo_doc();
    let before = state(&doc);
    let hist = doc.history().len();
    let b1 = guid_named(&doc, "body_01");
    let orig_pos = body_pos(doc.view(), &b1);

    let id = doc.propose("lift body_01", agent()).unwrap();
    assert_eq!(doc.proposal_info(id).unwrap().op_count, 0);
    let applied = doc.proposal_apply(id, set(&b1, BODY, "pos", vec2(3, 9))).unwrap();
    assert!(applied.changed);
    // A no-op is not staged.
    assert!(!doc.proposal_apply(id, set(&b1, BODY, "pos", vec2(3, 9))).unwrap().changed);
    assert_eq!(doc.proposal_ops(id).unwrap().len(), 1);

    assert_eq!(body_pos(doc.proposal_preview(id).unwrap(), &b1), vec2(3, 9));
    assert_eq!(body_pos(doc.view(), &b1), orig_pos, "the document frame is untouched");
    assert_ne!(doc.proposal_preview(id).unwrap().checksum(), doc.checksum());
    assert_eq!(state(&doc), before);
    assert_eq!(doc.history().len(), hist, "staging records no history");
    assert!(!doc.is_dirty());

    // An invalid op is refused and stages nothing.
    assert!(doc.proposal_apply(id, set(&b1, BODY, "kind", Value::Enum("nope".into()))).is_err());
    assert!(matches!(doc.proposal_apply(ProposalId(999), Op::Rename { guid: b1.clone(), name: None }), Err(EditError::UnknownProposal(999))));
    assert_eq!(doc.proposal_ops(id).unwrap().len(), 1);
}

#[test]
fn diff_text_is_exact_for_a_known_edit() {
    let mut doc = demo_doc();
    let b1 = guid_named(&doc, "body_01");
    let id = doc.propose("lift", agent()).unwrap();
    doc.proposal_apply(id, set(&b1, BODY, "pos", vec2(3, 9))).unwrap();
    doc.proposal_apply(id, Op::Rename { guid: b1.clone(), name: Some("hero".into()) }).unwrap();
    let d = doc.proposal_diff(id).unwrap();
    let expected = "\
--- base
+++ proposal
@@ -195,9 +195,9 @@
       mask: 4294967295
       flags: []
   e_0000000a:
-    name: body_01
+    name: hero
     orr_physics::Body:
-      pos: [-18.30666, 19.97716]
+      pos: [3, 9]
       angle: 1.93665
       vel: [-0.39624, 0]
       omega: 0
";
    assert_eq!(d.text, expected);
    assert!(d.text.contains("+    name: hero\n"));

    let lines = d.summary.lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with(&format!("~ rename {b1}: body_01 -> hero")), "{lines:?}");
    assert!(lines[1].starts_with(&format!("~ {b1} {BODY}.pos: [")) && lines[1].ends_with("-> [3, 9]"), "{lines:?}");
}

#[test]
fn unified_diff_hunks_and_edge_cases() {
    assert_eq!(unified_diff("a\nb\n", "a\nb\n", "x", "y", 3), "");
    let a = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n";
    let b = "1\n2\n3\nfour\n5\n6\n7\n8\n9\n10\n11\n12\nextra\n";
    let expected = "--- x\n+++ y\n@@ -1,7 +1,7 @@\n 1\n 2\n 3\n-4\n+four\n 5\n 6\n 7\n@@ -10,3 +10,4 @@\n 10\n 11\n 12\n+extra\n";
    assert_eq!(unified_diff(a, b, "x", "y", 3), expected);
    // Pure insert into an empty text, and a single-line count without ",1".
    assert_eq!(unified_diff("", "a\n", "x", "y", 3), "--- x\n+++ y\n@@ -0,0 +1 @@\n+a\n");
    assert_eq!(unified_diff("a\n", "", "x", "y", 3), "--- x\n+++ y\n@@ -1 +0,0 @@\n-a\n");
}

#[test]
fn summary_lists_added_removed_renamed_and_changed() {
    let mut doc = demo_doc();
    let (b2, b3, floor) = (guid_named(&doc, "body_02"), guid_named(&doc, "body_03"), guid_named(&doc, "floor"));
    let id = doc.propose("mixed", agent()).unwrap();
    doc.proposal_apply(id, Op::DespawnEntity { guid: b2.clone() }).unwrap();
    doc.proposal_apply(id, set(&b3, "orr_physics::Collider", "friction", fixed(2))).unwrap();
    doc.proposal_apply(id, Op::AddComponent { guid: floor.clone(), component: "PaddleTag".into(), value: None }).unwrap();
    doc.proposal_apply(id, Op::SetSingletonField { singleton: "Scene".into(), path: "max_entities".into(), value: Value::Int(500) })
        .unwrap();
    let spawned = doc
        .proposal_apply(id, Op::SpawnEntity { guid: None, name: Some("new".into()), components: vec![("PaddleTag".into(), Value::Struct(vec![("slot".into(), Value::Int(1))]))] })
        .unwrap()
        .guid
        .expect("a guid is assigned at staging");
    let s = doc.proposal_diff(id).unwrap().summary;
    assert_eq!(s.entities_removed.len(), 1);
    assert_eq!(s.entities_removed[0].guid, b2);
    assert_eq!(s.entities_added.len(), 1);
    assert_eq!(s.entities_added[0].guid, spawned);
    assert_eq!(s.components_added, vec![(floor, "PaddleTag".to_string())]);
    assert_eq!(s.fields_changed.len(), 2);
    let f = s.fields_changed.iter().find(|f| f.entity.is_some()).unwrap();
    assert_eq!((f.component.as_str(), f.path.as_str()), ("orr_physics::Collider", "friction"));
    assert_eq!(f.new, fixed(2));
    let sing = s.fields_changed.iter().find(|f| f.entity.is_none()).unwrap();
    assert_eq!((sing.component.as_str(), sing.path.as_str(), &sing.new), ("Scene", "max_entities", &Value::Int(500)));
    assert!(!s.is_empty());
}

#[test]
fn accept_is_one_undo_entry_with_the_agent_origin() {
    let mut doc = demo_doc();
    let before = state(&doc);
    let (b1, b2) = (guid_named(&doc, "body_01"), guid_named(&doc, "body_02"));
    let id = doc.propose("rearrange", agent()).unwrap();
    doc.proposal_apply(id, set(&b1, BODY, "pos", vec2(3, 9))).unwrap();
    doc.proposal_apply(id, set(&b2, BODY, "pos", vec2(-3, 9))).unwrap();
    doc.proposal_apply(id, Op::Rename { guid: b1.clone(), name: Some("hero".into()) }).unwrap();
    let staged_checksum = doc.proposal_preview(id).unwrap().checksum();

    let acc = doc.accept(id).unwrap();
    assert_eq!(acc.applied.len(), 3);
    assert!(acc.history_id.is_some());
    let h = doc.history();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].label, "rearrange");
    assert_eq!(h[0].origin, agent());
    assert_eq!(h[0].op_count, 3);
    assert_eq!(Some(h[0].id), acc.history_id);
    assert_eq!(doc.checksum(), staged_checksum, "the document equals the preview");
    assert!(doc.is_dirty());
    assert_synced(&doc);
    assert!(doc.list_proposals().is_empty(), "an accepted proposal is gone");
    assert!(matches!(doc.accept(id), Err(EditError::UnknownProposal(_))));

    doc.undo().unwrap();
    assert_eq!(state(&doc), before, "undo of an accepted proposal is a normal undo");
    doc.redo().unwrap();
    assert_eq!(doc.checksum(), staged_checksum);
}

#[test]
fn person_edits_meanwhile_and_a_clean_accept_still_works() {
    let mut doc = demo_doc();
    let (b1, b2, b3) = (guid_named(&doc, "body_01"), guid_named(&doc, "body_02"), guid_named(&doc, "body_03"));
    let id = doc.propose("lift body_01", agent()).unwrap();
    doc.proposal_apply(id, set(&b1, BODY, "pos", vec2(3, 9))).unwrap();
    assert!(!doc.proposal_info(id).unwrap().stale);

    // The person edits other things meanwhile.
    doc.apply(set(&b2, BODY, "pos", vec2(1, 20)), Origin::User).unwrap();
    doc.apply(Op::Rename { guid: b3.clone(), name: Some("mine".into()) }, Origin::User).unwrap();
    assert!(doc.proposal_info(id).unwrap().stale);
    doc.proposal_check(id).unwrap();

    doc.accept(id).unwrap();
    assert_eq!(body_pos(doc.view(), &b1), vec2(3, 9));
    assert_eq!(body_pos(doc.view(), &b2), vec2(1, 20), "the person's edit is kept");
    let h = doc.history();
    assert_eq!(h.len(), 3);
    assert_eq!(h[2].origin, agent());
    assert_eq!(h[0].origin, Origin::User);
    doc.undo().unwrap();
    let pristine = demo_doc();
    assert_eq!(body_pos(doc.view(), &b1), body_pos(pristine.view(), &b1));
    assert_eq!(body_pos(doc.view(), &b2), vec2(1, 20));
}

#[test]
fn a_conflicting_accept_fails_and_changes_nothing() {
    let mut doc = demo_doc();
    let (b1, b2) = (guid_named(&doc, "body_01"), guid_named(&doc, "body_02"));
    let id = doc.propose("edit both", agent()).unwrap();
    doc.proposal_apply(id, set(&b1, BODY, "pos", vec2(3, 9))).unwrap();
    doc.proposal_apply(id, set(&b2, BODY, "pos", vec2(-3, 9))).unwrap();

    // The person despawns body_02 meanwhile.
    doc.apply(Op::DespawnEntity { guid: b2.clone() }, Origin::User).unwrap();
    let before = state(&doc);
    let hist = doc.history();

    let err = doc.accept(id).unwrap_err();
    match &err {
        EditError::ProposalConflict { proposal, op_index, cause } => {
            assert_eq!((*proposal, *op_index), (id.0, 1));
            assert!(matches!(**cause, EditError::UnknownEntity(_)), "{cause:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(err.to_string().contains("conflicts"), "{err}");
    assert!(matches!(doc.proposal_check(id), Err(EditError::ProposalConflict { .. })));
    assert_eq!(state(&doc), before, "nothing changed, not even the first op");
    assert_eq!(doc.history(), hist);
    assert!(!doc.in_tx());
    assert_synced(&doc);
    assert_eq!(doc.list_proposals().len(), 1, "the proposal is kept");
    doc.reject(id).unwrap();
    assert!(doc.list_proposals().is_empty());
}

#[test]
fn spawned_guids_are_stable_and_unique_across_proposals() {
    let mut doc = demo_doc();
    let spawn = || Op::SpawnEntity { guid: None, name: Some("s".into()), components: vec![] };
    let (p1, p2) = (doc.propose("one", agent()).unwrap(), doc.propose("two", agent()).unwrap());
    let g1 = doc.proposal_apply(p1, spawn()).unwrap().guid.unwrap();
    let g2 = doc.proposal_apply(p2, spawn()).unwrap().guid.unwrap();
    assert_ne!(g1, g2);
    assert!(doc.proposal_preview(p1).unwrap().entity_of(&g1).is_some());
    doc.accept(p1).unwrap();
    doc.accept(p2).unwrap();
    assert!(doc.view().entity_of(&g1).is_some() && doc.view().entity_of(&g2).is_some());
}

#[test]
fn reject_list_tx_and_load() {
    let mut doc = demo_doc();
    let b1 = guid_named(&doc, "body_01");
    let p1 = doc.propose("a", agent()).unwrap();
    let p2 = doc.propose("b", Origin::Agent("other".into())).unwrap();
    doc.proposal_apply(p2, Op::Rename { guid: b1.clone(), name: Some("x".into()) }).unwrap();
    let list = doc.list_proposals();
    assert_eq!(list.iter().map(|i| (i.id, i.label.as_str(), i.op_count)).collect::<Vec<_>>(), vec![(p1, "a", 0), (p2, "b", 1)]);
    assert_eq!(list[1].origin, Origin::Agent("other".into()));

    // An empty proposal accepts to nothing.
    let acc = doc.accept(p1).unwrap();
    assert_eq!((acc.applied.len(), acc.history_id), (0, None));
    assert!(doc.history().is_empty());

    // Not while a transaction is open.
    doc.begin_tx("person", Origin::User).unwrap();
    assert!(matches!(doc.accept(p2), Err(EditError::TxOpen)));
    doc.rollback_tx().unwrap();

    doc.reject(p2).unwrap();
    assert!(matches!(doc.reject(p2), Err(EditError::UnknownProposal(_))));
    assert!(matches!(doc.proposal_diff(p2), Err(EditError::UnknownProposal(_))));

    // load_yaml drops proposals; ids are not reused.
    let p3 = doc.propose("c", agent()).unwrap();
    assert!(p3.0 > p2.0);
    let text = demo_text();
    doc.load_yaml(&text).unwrap();
    assert!(doc.list_proposals().is_empty());
    assert!(matches!(doc.proposal_ops(p3), Err(EditError::UnknownProposal(_))));
    let p4 = doc.propose("d", agent()).unwrap();
    assert!(p4.0 > p3.0);
}

#[test]
fn an_empty_document_can_propose_too() {
    let mut doc: EditorDoc = empty_doc();
    let id = doc.propose("first entity", agent()).unwrap();
    doc.proposal_apply(id, Op::SpawnEntity { guid: None, name: Some("a".into()), components: vec![] }).unwrap();
    let d = doc.proposal_diff(id).unwrap();
    assert!(d.text.starts_with("--- base\n+++ proposal\n"), "{}", d.text);
    assert_eq!(d.summary.entities_added.len(), 1);
    doc.accept(id).unwrap();
    assert_eq!(doc.view().entities().len(), 1);
}
