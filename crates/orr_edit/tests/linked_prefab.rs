#![cfg(feature = "linked-prefabs")]

// Run with `--features linked-prefabs,orr_sample/collect-dodge` so these
// integration checks exercise the actual game's closed admission contract.
use std::sync::Arc;

use orr_ecs::Frame;
use orr_edit::{BakeAdmission, EditError, EditorDoc, HistoryEntry, Op, Origin};
use orr_fp::{FPVec2, FP};
use orr_reflect::{Guid, Scene, SceneIndex, Value};
use orr_sample::{collect_game::CollectDodgeV1, collect_project};
use orr_sim::Simulation;

const INITIAL: &str = include_str!("../../../scenes/collect_dodge_v1.scene.yaml");
const SOURCE: &str = "prefabs/never-created-hazard.scene.yaml";
const ACTOR: &str = "CollectDodgeV1::Actor";

struct CollectAdmission;
impl BakeAdmission for CollectAdmission {
    fn admit_source(&self, text: &str) -> Result<(), EditError> {
        if text.len() as u64 > collect_project::MAX_BYTES {
            return Err(EditError::Invalid(
                "source exceeds Collect scene bound".into(),
            ));
        }
        Ok(())
    }

    fn admit(&self, scene: &Scene, frame: &mut Frame, index: &SceneIndex) -> Result<(), EditError> {
        collect_project::admit_initial(scene, frame, index).map_err(EditError::Invalid)
    }
}

fn open(text: &str) -> EditorDoc {
    EditorDoc::from_yaml_with_admission(
        text,
        collect_project::types(),
        Simulation::<CollectDodgeV1>::build_registry(),
        collect_project::SEED,
        Some(Arc::new(CollectAdmission)),
    )
    .unwrap()
}

fn id(value: u32) -> Guid {
    Guid::from_u32(value)
}

fn point(x: i32, y: i32) -> FPVec2 {
    FPVec2 {
        x: FP::from_int(x),
        y: FP::from_int(y),
    }
}

fn field<'a>(scene: &'a Scene, guid: &Guid, name: &str) -> &'a Value {
    scene.entities[guid].components[0].1.field(name).unwrap()
}

fn change(source: &str, guid: &Guid, name: &str, value: Value) -> String {
    let mut scene = Scene::parse(source, &collect_project::types()).unwrap();
    let Value::Struct(fields) = &mut scene.entities.get_mut(guid).unwrap().components[0].1 else {
        panic!("Actor must be a struct");
    };
    *fields.iter_mut().find(|(key, _)| key == name).unwrap() = (name.into(), value);
    scene.to_yaml()
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    scene: Scene,
    frame: Vec<u8>,
    yaml: String,
    revision: u64,
    history: Vec<HistoryEntry>,
    dirty: bool,
    redo: bool,
}

fn snapshot(doc: &EditorDoc) -> Snapshot {
    Snapshot {
        scene: doc.scene().clone(),
        frame: doc.frame().to_bytes(),
        yaml: doc.to_yaml(),
        revision: doc.revision(),
        history: doc.history(),
        dirty: doc.is_dirty(),
        redo: doc.can_redo(),
    }
}

fn hazard(doc: &EditorDoc) -> String {
    doc.capture_linked_collect_source(&[id(4)]).unwrap()
}

fn instantiate(doc: &mut EditorDoc, source: &str) -> (Guid, Guid) {
    let instance = doc
        .instantiate_linked_collect(SOURCE, source, Origin::User)
        .unwrap();
    let target = instance.guids[&id(4)].clone();
    (instance.guids.values().next().unwrap().clone(), target)
}

#[test]
fn repeated_groups_allocate_contiguous_ordinals_and_one_history_entry_each() {
    let mut doc = open(INITIAL);
    let source = doc.capture_linked_collect_source(&[id(2), id(4)]).unwrap();
    let before = doc.frame().to_bytes();
    let first = doc
        .instantiate_linked_collect(SOURCE, &source, Origin::User)
        .unwrap();
    assert_eq!(doc.history().len(), 1);
    let second = doc
        .instantiate_linked_collect(SOURCE, &source, Origin::User)
        .unwrap();
    assert_eq!(doc.history().len(), 2);
    for (instance, collectible, hazard) in [(&first, 2, 1), (&second, 3, 2)] {
        assert_eq!(
            field(doc.scene(), &instance.guids[&id(2)], "ordinal"),
            &Value::Int(collectible)
        );
        assert_eq!(
            field(doc.scene(), &instance.guids[&id(4)], "ordinal"),
            &Value::Int(hazard)
        );
    }
    assert!(first
        .guids
        .values()
        .all(|g| !second.guids.values().any(|other| other == g)));
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.frame().to_bytes(), before);
    assert!(doc.scene().prefab_links.is_empty());
    doc.redo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.scene().prefab_links.len(), 2);
}

#[test]
fn explicit_position_override_survives_update_while_velocity_updates() {
    let mut doc = open(INITIAL);
    let source = hazard(&doc);
    let (key, target) = instantiate(&mut doc, &source);
    doc.override_linked_collect_position(&key, &id(4), point(7, 8), Origin::User)
        .unwrap();
    let old_digest = doc.scene().prefab_links[&key].digest.clone();
    let updated = change(&source, &id(4), "position", Value::Vec2(point(2, 3)));
    let updated = change(&updated, &id(4), "velocity", Value::Vec2(point(1, 0)));
    doc.update_linked_collect(&key, SOURCE, &old_digest, &updated, Origin::User)
        .unwrap();
    assert_eq!(
        field(doc.scene(), &target, "position"),
        &Value::Vec2(point(7, 8))
    );
    assert_eq!(
        field(doc.scene(), &target, "velocity"),
        &Value::Vec2(point(1, 0))
    );
    assert_ne!(doc.scene().prefab_links[&key].digest, old_digest);
    assert_eq!(doc.history().len(), 3);
    doc.revert_linked_collect_position(&key, &id(4), Origin::User)
        .unwrap();
    assert_eq!(
        field(doc.scene(), &target, "position"),
        &Value::Vec2(point(2, 3))
    );
    assert!(doc.scene().prefab_links[&key].position_overrides.is_empty());
}

#[test]
fn metadata_only_update_is_dirty_undoable_and_retains_provenance() {
    let mut doc = open(INITIAL);
    let source = hazard(&doc);
    let (key, target) = instantiate(&mut doc, &source);
    doc.override_linked_collect_position(&key, &id(4), point(7, 8), Origin::User)
        .unwrap();
    let saved = doc.save_yaml();
    let old_link = doc.scene().prefab_links[&key].clone();
    let frame = doc.frame().to_bytes();
    let revision = doc.revision();
    let updated = change(&source, &id(4), "position", Value::Vec2(point(3, 4)));
    doc.update_linked_collect(
        &key,
        SOURCE,
        &old_link.digest,
        &updated,
        Origin::Agent("prefab-test".into()),
    )
    .unwrap();
    assert_eq!(doc.frame().to_bytes(), frame);
    assert!(doc.revision() > revision);
    assert!(doc.is_dirty());
    assert_eq!(doc.history().len(), 3);
    assert_eq!(
        doc.history().last().unwrap().origin,
        Origin::Agent("prefab-test".into())
    );
    let new_link = doc.scene().prefab_links[&key].clone();
    assert_ne!(old_link.digest, new_link.digest);
    doc.undo().unwrap();
    assert_eq!(doc.scene().prefab_links[&key], old_link);
    assert_eq!(doc.to_yaml(), saved);
    assert!(!doc.is_dirty());
    doc.redo().unwrap();
    assert_eq!(doc.scene().prefab_links[&key], new_link);
    assert_eq!(
        field(doc.scene(), &target, "position"),
        &Value::Vec2(point(7, 8))
    );
}

#[test]
fn rejected_source_changes_preserve_complete_document_and_redo() {
    let mut doc = open(INITIAL);
    let source = hazard(&doc);
    let (key, _) = instantiate(&mut doc, &source);
    doc.override_linked_collect_position(&key, &id(4), point(7, 8), Origin::User)
        .unwrap();
    doc.undo().unwrap();
    assert!(doc.can_redo());
    doc.save_yaml();
    let digest = doc.scene().prefab_links[&key].digest.clone();
    let before = snapshot(&doc);
    let mut missing = Scene::parse(&source, &collect_project::types()).unwrap();
    let mut added = missing.clone();
    added
        .entities
        .insert(id(100), added.entities[&id(4)].clone());
    let mut component_removed = missing.clone();
    component_removed
        .entities
        .get_mut(&id(4))
        .unwrap()
        .components
        .clear();
    missing.entities.clear();
    let bad_sources = [
        change(&source, &id(4), "kind", Value::Int(1)),
        change(&source, &id(4), "ordinal", Value::Int(1)),
        change(&source, &id(4), "velocity", Value::Vec2(point(2, 0))),
        missing.to_yaml(),
        added.to_yaml(),
        component_removed.to_yaml(),
    ];
    for bad in bad_sources {
        assert!(doc
            .update_linked_collect(&key, SOURCE, &digest, &bad, Origin::User)
            .is_err());
        assert_eq!(snapshot(&doc), before);
    }
    assert!(doc
        .update_linked_collect(
            &key,
            "prefabs/wrong.scene.yaml",
            &digest,
            &source,
            Origin::User
        )
        .is_err());
    assert_eq!(snapshot(&doc), before);
    assert!(doc
        .update_linked_collect(&key, SOURCE, "stale-digest", &source, Origin::User)
        .is_err());
    assert_eq!(snapshot(&doc), before);
    doc.redo().unwrap();
}

#[test]
fn updating_one_instance_preserves_the_other_instance_and_source_entities() {
    let mut doc = open(INITIAL);
    let source = hazard(&doc);
    let (first, _) = instantiate(&mut doc, &source);
    let (second, second_target) = instantiate(&mut doc, &source);
    let source_entity = doc.scene().entities[&id(4)].clone();
    let second_entity = doc.scene().entities[&second_target].clone();
    let second_link = doc.scene().prefab_links[&second].clone();
    let digest = doc.scene().prefab_links[&first].digest.clone();
    let updated = change(&source, &id(4), "position", Value::Vec2(point(3, 4)));
    doc.update_linked_collect(&first, SOURCE, &digest, &updated, Origin::User)
        .unwrap();
    assert_eq!(doc.scene().entities[&id(4)], source_entity);
    assert_eq!(doc.scene().entities[&second_target], second_entity);
    assert_eq!(doc.scene().prefab_links[&second], second_link);
}

#[test]
fn untracked_position_and_structural_edits_are_rejected_atomically() {
    let mut doc = open(INITIAL);
    let source = hazard(&doc);
    let (_, target) = instantiate(&mut doc, &source);
    let before = snapshot(&doc);
    for (path, value) in [
        ("position", Value::Vec2(point(7, 8))),
        ("ordinal", Value::Int(5)),
    ] {
        assert!(doc
            .apply(
                Op::SetField {
                    guid: target.clone(),
                    component: ACTOR.into(),
                    path: path.into(),
                    value
                },
                Origin::User
            )
            .is_err());
        assert_eq!(snapshot(&doc), before);
    }
}

#[test]
fn save_reopen_uses_embedded_source_without_filesystem_authority() {
    let mut doc = open(INITIAL);
    let source = hazard(&doc);
    let (key, _) = instantiate(&mut doc, &source);
    doc.override_linked_collect_position(&key, &id(4), point(7, 8), Origin::User)
        .unwrap();
    let text = doc.save_yaml();
    assert!(text.contains("schema: orr.scene/2"));
    let mut reopened = open(&text);
    assert_eq!(reopened.scene(), doc.scene());
    assert_eq!(reopened.frame().to_bytes(), doc.frame().to_bytes());
    assert_eq!(reopened.save_yaml(), text);
    assert!(!reopened.is_dirty());
    let digest = reopened.scene().prefab_links[&key].digest.clone();
    let changed = change(&source, &id(4), "velocity", Value::Vec2(point(1, 0)));
    reopened
        .update_linked_collect(&key, SOURCE, &digest, &changed, Origin::User)
        .unwrap();
    reopened.undo().unwrap();
    assert_eq!(reopened.to_yaml(), text);
}

#[test]
fn instance_and_source_caps_are_atomic_and_player_or_nested_capture_is_rejected() {
    let mut doc = open(INITIAL);
    let source = hazard(&doc);
    assert!(doc.capture_linked_collect_source(&[id(1)]).is_err());
    assert!(doc.capture_linked_collect_source(&[]).is_err());
    let (_, target) = instantiate(&mut doc, &source);
    assert!(doc.capture_linked_collect_source(&[target]).is_err());
    for _ in 1..8 {
        instantiate(&mut doc, &source);
    }
    let before = snapshot(&doc);
    assert!(doc
        .instantiate_linked_collect(SOURCE, &source, Origin::User)
        .is_err());
    assert_eq!(snapshot(&doc), before);

    let mut fresh = open(INITIAL);
    let before = snapshot(&fresh);
    let too_large = format!("# {}\n{source}", "x".repeat(32 * 1024));
    assert!(fresh
        .instantiate_linked_collect(SOURCE, &too_large, Origin::User)
        .is_err());
    assert_eq!(snapshot(&fresh), before);
    let mut too_many = Scene::parse(&source, &collect_project::types()).unwrap();
    let entity = too_many.entities[&id(4)].clone();
    for n in 100..108 {
        too_many.entities.insert(id(n), entity.clone());
    }
    assert!(fresh
        .instantiate_linked_collect(SOURCE, &too_many.to_yaml(), Origin::User)
        .is_err());
    assert_eq!(snapshot(&fresh), before);
}

#[test]
fn schema_one_bytes_are_preserved_when_link_feature_is_enabled_but_unused() {
    let mut doc = open(INITIAL);
    assert_eq!(doc.to_yaml(), INITIAL);
    assert_eq!(doc.save_yaml(), INITIAL);
    assert!(doc.scene().prefab_links.is_empty());
    assert!(doc.scene().to_yaml().contains("schema: orr.scene/1"));
    let source = hazard(&doc);
    instantiate(&mut doc, &source);
    doc.undo().unwrap();
    assert_eq!(doc.to_yaml(), INITIAL);
    assert!(doc.scene().prefab_links.is_empty());
}

#[test]
fn no_policy_rejected_edit_preserves_revision_and_redo() {
    let mut doc = EditorDoc::from_yaml(
        INITIAL,
        collect_project::types(),
        Simulation::<CollectDodgeV1>::build_registry(),
        collect_project::SEED,
    )
    .unwrap();
    let source = hazard(&doc);
    let (key, target) = instantiate(&mut doc, &source);
    doc.override_linked_collect_position(&key, &id(4), point(7, 8), Origin::User)
        .unwrap();
    doc.undo().unwrap();
    doc.save_yaml();
    let before = snapshot(&doc);
    assert!(doc
        .apply(
            Op::SetField {
                guid: target,
                component: ACTOR.into(),
                path: "position".into(),
                value: Value::Vec2(point(3, 4)),
            },
            Origin::User
        )
        .is_err());
    assert_eq!(snapshot(&doc), before);
    doc.redo().unwrap();
}

#[test]
fn explicit_override_marker_allows_subsequent_ordinary_position_edits_only() {
    let mut doc = open(INITIAL);
    let source = hazard(&doc);
    let (key, target) = instantiate(&mut doc, &source);
    doc.override_linked_collect_position(&key, &id(4), point(7, 8), Origin::User)
        .unwrap();
    // The explicit marker authorizes subsequent field-local position editing;
    // it does not permit ordinary changes to the rest of the linked Actor.
    doc.apply(
        Op::SetField {
            guid: target.clone(),
            component: ACTOR.into(),
            path: "position".into(),
            value: Value::Vec2(point(3, 4)),
        },
        Origin::User,
    )
    .unwrap();
    let before = snapshot(&doc);
    assert!(doc
        .apply(
            Op::SetField {
                guid: target.clone(),
                component: ACTOR.into(),
                path: "velocity".into(),
                value: Value::Vec2(point(1, 0)),
            },
            Origin::User
        )
        .is_err());
    assert_eq!(snapshot(&doc), before);
    let digest = doc.scene().prefab_links[&key].digest.clone();
    let updated = change(&source, &id(4), "position", Value::Vec2(point(5, 6)));
    doc.update_linked_collect(&key, SOURCE, &digest, &updated, Origin::User)
        .unwrap();
    assert_eq!(
        field(doc.scene(), &target, "position"),
        &Value::Vec2(point(3, 4))
    );
}

#[test]
fn no_policy_rejected_batch_restores_valid_prefix_revision_and_redo() {
    let mut doc = EditorDoc::from_yaml(
        INITIAL,
        collect_project::types(),
        Simulation::<CollectDodgeV1>::build_registry(),
        collect_project::SEED,
    )
    .unwrap();
    let source = hazard(&doc);
    let (key, target) = instantiate(&mut doc, &source);
    doc.override_linked_collect_position(&key, &id(4), point(7, 8), Origin::User)
        .unwrap();
    doc.undo().unwrap();
    doc.save_yaml();
    let before = snapshot(&doc);
    assert!(doc
        .apply_batch(
            "rejected linked suffix",
            vec![
                Op::Rename {
                    guid: id(1),
                    name: Some("temporary valid prefix".into())
                },
                Op::SetField {
                    guid: target,
                    component: ACTOR.into(),
                    path: "position".into(),
                    value: Value::Vec2(point(3, 4)),
                },
            ],
            Origin::User
        )
        .is_err());
    assert_eq!(snapshot(&doc), before);
    doc.redo().unwrap();
}
