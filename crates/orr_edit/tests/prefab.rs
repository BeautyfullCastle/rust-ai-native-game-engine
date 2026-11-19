//! Generic prefab instances on the 2D physics game: create, instantiate,
//! overrides, apply, update from a changed prefab, nesting and save/reopen.
#![cfg(feature = "linked-prefabs")]

mod common;

use common::*;
use orr_edit::{EditorDoc, OverrideKind, Op, Origin};
use orr_fp::FP;
use orr_reflect::{Guid, Scene, Value};
use orr_sample::physics_game::PhysGame;
use orr_sim::Simulation;

const JOINTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes/joints_2d_demo.scene.yaml");
const BODY: &str = "orr_physics::Body";
const COLLIDER: &str = "orr_physics::Collider";
const JOINT: &str = "orr_physics::Joint";

fn doc() -> EditorDoc {
    let text = std::fs::read_to_string(JOINTS).unwrap();
    EditorDoc::from_yaml(&text, types(), Simulation::<PhysGame>::build_registry(), SEED).unwrap()
}

fn reopen(doc: &mut EditorDoc) -> EditorDoc {
    EditorDoc::from_yaml(&doc.save_yaml(), types(), Simulation::<PhysGame>::build_registry(), SEED).unwrap()
}

/// The motor: a static hub, a propeller and the joint between them.
fn motor(doc: &EditorDoc) -> Vec<Guid> {
    ["motor_hub", "motor_propeller", "motor_pin"].iter().map(|n| guid_named(doc, n)).collect()
}

fn field(doc: &EditorDoc, guid: &Guid, component: &str, path: &str) -> Value {
    doc.scene().entities[guid].components.iter().find(|(n, _)| n == component).map(|(_, v)| v).and_then(|v| {
        let mut v = v;
        for part in path.split('.').filter(|p| !p.is_empty()) {
            v = v.field(part)?;
        }
        Some(v.clone())
    }).unwrap_or_else(|| panic!("{guid} has no {component}.{path}"))
}

fn set(doc: &mut EditorDoc, guid: &Guid, component: &str, path: &str, value: Value) {
    doc.apply(Op::SetField { guid: guid.clone(), component: component.into(), path: path.into(), value }, Origin::User).unwrap();
}

/// Creates the prefab file from the motor and returns its text.
fn motor_prefab(doc: &EditorDoc) -> String {
    doc.create_prefab_source(&motor(doc)).unwrap().text
}

fn members_of(doc: &EditorDoc, root: &Guid) -> Vec<(String, Guid)> {
    let info = doc.prefab_instances().unwrap().into_iter().find(|i| &i.root == root).unwrap();
    info.members.into_iter().map(|(s, l, _)| (s.to_string(), l)).collect()
}

#[test]
fn create_remaps_to_fragment_local_ids_and_leaves_the_scene_alone() {
    let doc = doc();
    let before = state(&doc);
    let source = doc.create_prefab_source(&motor(&doc)).unwrap();
    assert_eq!(state(&doc), before, "creating a prefab text changes nothing");
    let parsed = Scene::parse(&source.text, &types()).unwrap();
    let ids: Vec<String> = parsed.entities.keys().map(ToString::to_string).collect();
    assert_eq!(ids, ["e_00000001", "e_00000002", "e_00000003"]);
    // The joint is the third entity (hub, propeller, pin in GUID order) and
    // refers to the local ids of the two bodies, not to the scene ids.
    let joint = &parsed.entities[&Guid::from_u32(3)];
    let text = format!("{:?}", joint.components);
    assert!(text.contains("e_00000001") && text.contains("e_00000002"), "{text}");
    assert!(!text.contains("e_00000007") && !text.contains("e_00000008"), "{text}");
    assert_eq!(source.guids[&Guid::from_u32(3)], guid_named(&doc, "motor_pin"));
}

#[test]
fn a_reference_to_an_outside_entity_is_refused_with_a_clear_message() {
    let doc = doc();
    let error = doc.create_prefab_source(&[guid_named(&doc, "motor_pin")]).unwrap_err().to_string();
    assert!(error.contains("cannot create a prefab") && error.contains("outside"), "{error}");
    assert!(doc.create_prefab_source(&[]).is_err());
}

#[test]
fn instantiate_gives_fresh_ids_remapped_references_and_one_undo_step() {
    let mut doc = doc();
    let text = motor_prefab(&doc);
    let entities = doc.scene().entities.len();
    let history = doc.history().len();
    let (root, map) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    assert_eq!(doc.scene().entities.len(), entities + 3);
    assert_eq!(doc.history().len(), history + 1);
    let hub = &map[&Guid::from_u32(1)];
    let propeller = &map[&Guid::from_u32(2)];
    let pin = &map[&Guid::from_u32(3)];
    assert!(map.values().all(|g| !motor(&doc).contains(g)), "fresh ids");
    assert_eq!(field(&doc, pin, JOINT, "a"), Value::EntityGuid(Some(hub.to_string())));
    assert_eq!(field(&doc, pin, JOINT, "b"), Value::EntityGuid(Some(propeller.to_string())));
    assert!(doc.prefab_overrides(&root).unwrap().is_empty());
    assert_synced(&doc);

    doc.undo().unwrap();
    assert_eq!(doc.scene().entities.len(), entities);
    assert!(doc.prefab_instances().unwrap().is_empty());
    doc.redo().unwrap();
    assert_eq!(doc.prefab_instances().unwrap().len(), 1);
    assert_eq!(field(&doc, pin, JOINT, "a"), Value::EntityGuid(Some(hub.to_string())));
}

#[test]
fn an_edited_field_is_an_override_and_revert_restores_the_prefab_value() {
    let mut doc = doc();
    let text = motor_prefab(&doc);
    let (root, map) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    let prop = map[&Guid::from_u32(2)].clone();
    let original = field(&doc, &prop, BODY, "pos");
    set(&mut doc, &prop, BODY, "pos", vec2(5, 6));
    set(&mut doc, &prop, COLLIDER, "friction", fixed(3));
    let overrides = doc.prefab_overrides(&root).unwrap();
    assert_eq!(overrides.len(), 2);
    assert!(overrides.iter().all(|o| o.entity == prop));
    assert!(overrides.iter().any(|o| matches!(&o.kind, OverrideKind::Field { component, path, .. } if component == BODY && path == "pos")));

    let history = doc.history().len();
    assert_eq!(doc.revert_prefab_field(&prop, BODY, "pos", Origin::User).unwrap(), 1);
    assert_eq!(doc.history().len(), history + 1, "one undo step");
    assert_eq!(field(&doc, &prop, BODY, "pos"), original);
    assert_eq!(doc.prefab_overrides(&root).unwrap().len(), 1);
    doc.undo().unwrap();
    assert_eq!(field(&doc, &prop, BODY, "pos"), vec2(5, 6));
    assert_eq!(doc.prefab_overrides(&root).unwrap().len(), 2);
    doc.redo().unwrap();

    // Reverting a field that is not overridden is refused.
    assert!(doc.revert_prefab_field(&prop, BODY, "pos", Origin::User).is_err());
    // Revert all restores the rest in one step.
    let history = doc.history().len();
    assert_eq!(doc.revert_prefab_instance(&prop, Origin::User).unwrap(), 1);
    assert_eq!(doc.history().len(), history + 1);
    assert!(doc.prefab_overrides(&root).unwrap().is_empty());
    assert_eq!(field(&doc, &prop, COLLIDER, "friction"), field(&doc, &guid_named(&doc, "motor_propeller"), COLLIDER, "friction"));
    doc.undo().unwrap();
    assert_eq!(doc.prefab_overrides(&root).unwrap().len(), 1);
}

#[test]
fn deleting_a_member_and_adding_a_component_are_overrides_that_revert() {
    let mut doc = doc();
    let text = motor_prefab(&doc);
    let (root, map) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    let pin = map[&Guid::from_u32(3)].clone();
    let hub = map[&Guid::from_u32(1)].clone();
    doc.apply(Op::DespawnEntity { guid: pin.clone() }, Origin::User).unwrap();
    doc.apply(Op::Rename { guid: hub.clone(), name: Some("renamed".into()) }, Origin::User).unwrap();
    let kinds: Vec<_> = doc.prefab_overrides(&root).unwrap().into_iter().map(|o| o.kind).collect();
    assert!(kinds.contains(&OverrideKind::EntityRemoved));
    assert!(kinds.iter().any(|k| matches!(k, OverrideKind::Renamed { .. })));
    assert_eq!(doc.revert_prefab_instance(&hub, Origin::User).unwrap(), 2);
    assert!(doc.prefab_overrides(&root).unwrap().is_empty());
    assert_eq!(field(&doc, &pin, JOINT, "a"), Value::EntityGuid(Some(hub.to_string())));
    assert_synced(&doc);
}

#[test]
fn apply_writes_overrides_into_the_prefab_and_other_instances_follow() {
    let mut doc = doc();
    let text = motor_prefab(&doc);
    let (a, map_a) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    let (b, map_b) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    let pin_a = map_a[&Guid::from_u32(3)].clone();
    let pin_b = map_b[&Guid::from_u32(3)].clone();
    let prop_b = map_b[&Guid::from_u32(2)].clone();
    set(&mut doc, &pin_a, JOINT, "motor_speed", fixed(9));
    set(&mut doc, &pin_b, JOINT, "max_motor_torque", fixed(77)); // B's own override
    set(&mut doc, &prop_b, COLLIDER, "friction", fixed(1)); // B's own override
    let (source, new_text) = doc.prefab_text_with_overrides(&pin_a).unwrap();
    assert_eq!(source, "motor.prefab.yaml");
    assert_ne!(new_text, text);

    let history = doc.history().len();
    let report = doc.update_prefab_instances(&source, &new_text, Origin::User).unwrap();
    assert_eq!(doc.history().len(), history + 1, "one undo step for all instances");
    assert_eq!(report.updated.len(), 2);
    assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    // A keeps its (now prefab) value and has no overrides left.
    assert!(doc.prefab_overrides(&a).unwrap().is_empty());
    assert_eq!(field(&doc, &pin_a, JOINT, "motor_speed"), fixed(9));
    // B follows the changed value and keeps its own overrides.
    assert_eq!(field(&doc, &pin_b, JOINT, "motor_speed"), fixed(9));
    assert_eq!(field(&doc, &pin_b, JOINT, "max_motor_torque"), fixed(77));
    assert_eq!(field(&doc, &prop_b, COLLIDER, "friction"), fixed(1));
    assert_eq!(doc.prefab_overrides(&b).unwrap().len(), 2);
    assert_synced(&doc);

    doc.undo().unwrap();
    assert_eq!(field(&doc, &pin_b, JOINT, "motor_speed"), field(&doc, &guid_named(&doc, "motor_pin"), JOINT, "motor_speed"));
    assert_eq!(doc.prefab_overrides(&a).unwrap().len(), 1);
    doc.redo().unwrap();
    assert!(doc.prefab_overrides(&a).unwrap().is_empty());
}

#[test]
fn a_changed_prefab_updates_values_components_and_entities_and_reports_conflicts() {
    let mut doc = doc();
    let text = motor_prefab(&doc);
    let (a, map_a) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    let (b, map_b) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    let hub_a = map_a[&Guid::from_u32(1)].clone();
    let prop_a = map_a[&Guid::from_u32(2)].clone();
    let prop_b = map_b[&Guid::from_u32(2)].clone();
    set(&mut doc, &prop_a, COLLIDER, "friction", fixed(2)); // overridden in A, also changed in the prefab: conflict
    set(&mut doc, &prop_a, BODY, "angle", fixed(1)); // overridden in A only, unchanged in the prefab

    // Edit the prefab file: friction and the propeller position change, the hub
    // loses its Collider, and a new body appears.
    let mut prefab = Scene::parse(&text, &types()).unwrap();
    let prop_src = Guid::from_u32(2);
    let collider = prefab.entities.get_mut(&prop_src).unwrap().components.iter_mut().find(|(n, _)| n == COLLIDER).unwrap();
    let Value::Struct(fields) = &mut collider.1 else { panic!() };
    fields.iter_mut().find(|(n, _)| n == "friction").unwrap().1 = Value::Fixed(FP::from_int(5));
    let body = prefab.entities.get_mut(&prop_src).unwrap().components.iter_mut().find(|(n, _)| n == BODY).unwrap();
    let Value::Struct(fields) = &mut body.1 else { panic!() };
    fields.iter_mut().find(|(n, _)| n == "pos").unwrap().1 = vec2(0, 20);
    prefab.entities.get_mut(&Guid::from_u32(1)).unwrap().components.retain(|(n, _)| n != COLLIDER);
    let mut extra = prefab.entities[&Guid::from_u32(2)].clone();
    extra.name = Some("extra".into());
    prefab.entities.insert(Guid::from_u32(4), extra);
    let changed = prefab.to_yaml();

    let entities = doc.scene().entities.len();
    let report = doc.update_prefab_instances("motor.prefab.yaml", &changed, Origin::User).unwrap();
    assert_eq!(report.updated.len(), 2);
    assert_eq!(report.spawned, 2);
    assert_eq!(doc.scene().entities.len(), entities + 2);
    // Non-overridden values follow; the instance's own override stays.
    assert_eq!(field(&doc, &prop_b, COLLIDER, "friction"), fixed(5));
    assert_eq!(field(&doc, &prop_b, BODY, "pos"), vec2(0, 20));
    assert_eq!(field(&doc, &prop_a, COLLIDER, "friction"), fixed(2));
    assert_eq!(field(&doc, &prop_a, BODY, "angle"), fixed(1));
    assert_eq!(field(&doc, &prop_a, BODY, "pos"), vec2(0, 20), "not overridden: follows the prefab");
    // The conflict (both changed friction) is reported, not raised.
    assert_eq!(report.conflicts.len(), 1, "{:?}", report.conflicts);
    assert!(report.conflicts[0].contains("friction") && report.conflicts[0].contains("kept"), "{:?}", report.conflicts);
    // The removed component is gone, the added entity exists in both instances.
    assert!(doc.scene().entities[&hub_a].components.iter().all(|(n, _)| n != COLLIDER));
    let extras = doc.scene().entities.values().filter(|e| e.name.as_deref() == Some("extra")).count();
    assert_eq!(extras, 2);
    assert_eq!(members_of(&doc, &a).len(), 4);
    assert_eq!(members_of(&doc, &b).len(), 4);
    assert_synced(&doc);
    // Updating again with the same text changes nothing.
    let history = doc.history().len();
    let report = doc.update_prefab_instances("motor.prefab.yaml", &changed, Origin::User).unwrap();
    assert!(report.updated.is_empty() && report.unchanged == 2);
    assert_eq!(doc.history().len(), history);

    // One undo step restores both instances.
    doc.undo().unwrap();
    assert_eq!(doc.scene().entities.len(), entities);
    assert_eq!(field(&doc, &prop_b, COLLIDER, "friction"), field(&doc, &guid_named(&doc, "motor_propeller"), COLLIDER, "friction"));
    assert_synced(&doc);
    doc.redo().unwrap();
    assert_eq!(doc.scene().entities.len(), entities + 2);
}

#[test]
fn nested_prefabs_are_refused_with_a_clear_message() {
    let mut doc = doc();
    let text = motor_prefab(&doc);
    let (_, map) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    // A selection that includes an instance member cannot become a prefab.
    let error = doc.create_prefab_source(&map.values().cloned().collect::<Vec<_>>()).unwrap_err().to_string();
    assert!(error.contains("nested prefabs are not supported"), "{error}");
    // A prefab text that carries prefab links is refused as well.
    let nested = doc.to_yaml();
    assert!(nested.contains("
schema: orr.scene/2
"));
    let error = doc.instantiate_prefab("outer.prefab.yaml", &nested, Origin::User).unwrap_err().to_string();
    assert!(error.contains("nested prefabs are not supported"), "{error}");
}

#[test]
fn links_and_overrides_survive_save_and_reopen_and_detach_is_one_undo_step() {
    let mut doc = doc();
    let text = motor_prefab(&doc);
    let (root, map) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    let prop = map[&Guid::from_u32(2)].clone();
    set(&mut doc, &prop, BODY, "pos", vec2(3, 4));
    let saved = doc.save_yaml();
    assert!(saved.contains("
schema: orr.scene/2
"));
    let mut reopened = reopen(&mut doc);
    assert_eq!(reopened.prefab_instances().unwrap(), doc.prefab_instances().unwrap());
    assert_eq!(reopened.prefab_overrides(&root).unwrap().len(), 1);
    assert_eq!(reopened.save_yaml(), saved);
    assert_eq!(reopened.checksum(), doc.checksum());

    let history = doc.history().len();
    doc.detach_prefab_instance(&prop, Origin::User).unwrap();
    assert_eq!(doc.history().len(), history + 1);
    assert!(doc.prefab_instances().unwrap().is_empty());
    assert_eq!(field(&doc, &prop, BODY, "pos"), vec2(3, 4));
    assert!(doc.save_yaml().contains("
schema: orr.scene/1
"));
    doc.undo().unwrap();
    assert_eq!(doc.prefab_overrides(&root).unwrap().len(), 1);
}

#[test]
fn link_selection_makes_the_source_entities_an_instance() {
    let mut doc = doc();
    let source = doc.create_prefab_source(&motor(&doc)).unwrap();
    let before = doc.checksum();
    let root = doc.link_prefab_instance("motor.prefab.yaml", &source.text, source.guids.clone(), Origin::User).unwrap();
    assert_eq!(doc.checksum(), before, "linking changes no entity");
    assert!(doc.prefab_overrides(&root).unwrap().is_empty());
    // An entity cannot belong to two instances.
    assert!(doc.link_prefab_instance("again.prefab.yaml", &source.text, source.guids, Origin::User).is_err());
    // Apply on the original: its overrides go into the prefab.
    let hub = guid_named(&doc, "motor_hub");
    set(&mut doc, &hub, BODY, "pos", vec2(1, 9));
    let (_, new_text) = doc.prefab_text_with_overrides(&hub).unwrap();
    assert_ne!(new_text, source.text);
}

#[test]
fn a_reference_override_to_an_entity_outside_the_instance_cannot_be_applied() {
    let mut doc = doc();
    let text = motor_prefab(&doc);
    let (_, map) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    let pin = map[&Guid::from_u32(3)].clone();
    let outside = guid_named(&doc, "pendulum_bar");
    set(&mut doc, &pin, JOINT, "b", Value::EntityGuid(Some(outside.to_string())));
    let error = doc.prefab_text_with_overrides(&pin).unwrap_err().to_string();
    assert!(error.contains("outside the prefab instance"), "{error}");
}

#[test]
fn detached_and_unknown_instances_are_refused_without_changing_the_document() {
    let mut doc = doc();
    let before = state(&doc);
    assert!(doc.detach_prefab_instance(&guid_named(&doc, "motor_hub"), Origin::User).is_err());
    assert!(doc.prefab_overrides(&guid_named(&doc, "motor_hub")).is_err());
    assert!(doc.revert_prefab_instance(&guid_named(&doc, "motor_hub"), Origin::User).is_err());
    assert_eq!(state(&doc), before);
}

/// The motor prefab text without the given source members.
fn motor_prefab_without(text: &str, gone: &[u32]) -> String {
    let mut prefab = Scene::parse(text, &types()).unwrap();
    for id in gone {
        prefab.entities.remove(&Guid::from_u32(*id)).expect("source member");
    }
    prefab.to_yaml()
}

#[test]
fn a_member_removed_from_the_prefab_leaves_every_instance_in_one_undo_step() {
    let mut doc = doc();
    let text = motor_prefab(&doc);
    let (a, map_a) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    let (b, map_b) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    let prop_a = map_a[&Guid::from_u32(2)].clone();
    let pin_a = map_a[&Guid::from_u32(3)].clone();
    let pin_b = map_b[&Guid::from_u32(3)].clone();
    set(&mut doc, &prop_a, BODY, "angle", fixed(1)); // an override on a surviving member
    let changed = motor_prefab_without(&text, &[3]);

    let entities = doc.scene().entities.len();
    let history = doc.history().len();
    let report = doc.update_prefab_instances("motor.prefab.yaml", &changed, Origin::User).unwrap();
    assert_eq!(report.updated.len(), 2);
    assert_eq!(report.removed, 2);
    assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    assert_eq!(doc.history().len(), history + 1, "one undo step");
    assert_eq!(doc.scene().entities.len(), entities - 2);
    assert!(!doc.scene().entities.contains_key(&pin_a) && !doc.scene().entities.contains_key(&pin_b));
    assert_eq!(members_of(&doc, &a).len(), 2);
    assert_eq!(members_of(&doc, &b).len(), 2);
    assert_eq!(field(&doc, &prop_a, BODY, "angle"), fixed(1), "the override of a surviving member stays");
    assert_eq!(doc.prefab_overrides(&a).unwrap().len(), 1);
    assert_synced(&doc);

    let reopened = reopen(&mut doc);
    assert_eq!(reopened.prefab_instances().unwrap(), doc.prefab_instances().unwrap());
    assert_eq!(reopened.checksum(), doc.checksum());

    doc.undo().unwrap();
    assert_eq!(doc.scene().entities.len(), entities);
    assert_eq!(members_of(&doc, &a).len(), 3);
    assert_synced(&doc);
    doc.redo().unwrap();
    assert_eq!(doc.scene().entities.len(), entities - 2);
    assert_eq!(members_of(&doc, &b).len(), 2);
}

#[test]
fn removing_the_root_member_from_the_prefab_re_keys_the_instances() {
    let mut doc = doc();
    let text = motor_prefab(&doc);
    let (a, map_a) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    let (b, map_b) = doc.instantiate_prefab("motor.prefab.yaml", &text, Origin::User).unwrap();
    assert_eq!(a, map_a[&Guid::from_u32(1)], "the hub is the root");
    let prop_a = map_a[&Guid::from_u32(2)].clone();
    let prop_b = map_b[&Guid::from_u32(2)].clone();
    set(&mut doc, &prop_a, BODY, "angle", fixed(1));
    // The pin refers to the hub, so both leave the prefab together.
    let changed = motor_prefab_without(&text, &[1, 3]);

    let entities = doc.scene().entities.len();
    let report = doc.update_prefab_instances("motor.prefab.yaml", &changed, Origin::User).unwrap();
    assert_eq!(report.updated.len(), 2);
    assert_eq!(report.removed, 4);
    assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    assert_eq!(doc.scene().entities.len(), entities - 4);
    let instances = doc.prefab_instances().unwrap();
    assert_eq!(instances.len(), 2);
    assert!(instances.iter().all(|i| i.root != a && i.root != b), "the old root keys are gone");
    assert_eq!(members_of(&doc, &prop_a), vec![(Guid::from_u32(2).to_string(), prop_a.clone())]);
    assert_eq!(members_of(&doc, &prop_b), vec![(Guid::from_u32(2).to_string(), prop_b.clone())]);
    assert_eq!(field(&doc, &prop_a, BODY, "angle"), fixed(1));
    assert_eq!(doc.prefab_overrides(&prop_a).unwrap().len(), 1);
    assert_synced(&doc);

    let reopened = reopen(&mut doc);
    assert_eq!(reopened.prefab_instances().unwrap(), doc.prefab_instances().unwrap());
    assert_eq!(reopened.checksum(), doc.checksum());

    doc.undo().unwrap();
    assert_eq!(doc.scene().entities.len(), entities);
    assert_eq!(members_of(&doc, &a).len(), 3);
    assert_synced(&doc);
    doc.redo().unwrap();
    assert_eq!(members_of(&doc, &prop_b).len(), 1);
}
