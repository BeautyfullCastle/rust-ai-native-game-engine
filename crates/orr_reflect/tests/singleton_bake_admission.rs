//! Programmatic Scene::bake admission for entity references in singletons.

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_reflect::{BakeError, Guid, Scene, SceneEntity, TypeRegistry, Value};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, orr_reflect::Reflect)]
struct RefComponent {
    target: Entity,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, orr_reflect::Reflect)]
struct RefSingleton {
    target: Entity,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, orr_reflect::Reflect)]
struct RefArraySingleton {
    targets: [Entity; 2],
}

fn types() -> TypeRegistry {
    let mut reg = TypeRegistry::new();
    reg.register_component::<RefComponent>("RefComponent");
    reg.register_singleton::<RefSingleton>("RefSingleton");
    reg.register_singleton::<RefArraySingleton>("RefArraySingleton");
    reg
}

fn frame() -> Frame {
    let mut builder = ComponentRegistryBuilder::new();
    builder.register_component::<RefComponent>("RefComponent");
    builder.register_singleton::<RefSingleton>("RefSingleton");
    builder.register_singleton::<RefArraySingleton>("RefArraySingleton");
    Frame::new(builder.build())
}

fn ref_value(target: Option<&str>) -> Value {
    Value::Struct(vec![(
        "target".into(),
        Value::EntityGuid(target.map(str::to_string)),
    )])
}

fn entity_with_component_ref(target: Option<&str>) -> SceneEntity {
    SceneEntity {
        name: None,
        components: vec![("RefComponent".into(), ref_value(target))],
    }
}

#[test]
fn missing_component_reference_remains_first_and_leaves_frame_untouched() {
    let mut scene = Scene::default();
    scene.entities.insert(
        Guid::from_u32(1),
        entity_with_component_ref(Some("e_00000002")),
    );
    scene
        .singletons
        .push(("RefSingleton".into(), ref_value(Some("e_00000003"))));
    let mut frame = frame();

    let error = scene.bake(&types(), &mut frame).unwrap_err();

    assert_eq!(error, BakeError::MissingEntity("e_00000002".into()));
    assert_eq!(frame.alive_count(), 0);
}

#[test]
fn missing_singleton_reference_is_rejected_before_frame_mutation() {
    let mut scene = Scene::default();
    scene
        .entities
        .insert(Guid::from_u32(1), SceneEntity::default());
    scene
        .singletons
        .push(("RefSingleton".into(), ref_value(Some("e_00000002"))));
    let mut frame = frame();
    let existing = frame.spawn();
    frame.add::<RefComponent>(existing, RefComponent { target: existing });
    frame.singleton_mut::<RefSingleton>().target = existing;
    let checksum_before = frame.checksum();

    let error = scene
        .bake(&types(), &mut frame)
        .expect_err("a missing singleton GUID must be rejected during bake preflight");

    assert_eq!(error, BakeError::MissingEntity("e_00000002".into()));
    assert_eq!(frame.alive_count(), 1);
    assert!(frame.exists(existing));
    assert_eq!(
        frame.get::<RefComponent>(existing).unwrap().target,
        existing
    );
    assert_eq!(frame.singleton::<RefSingleton>().target, existing);
    assert_eq!(frame.checksum(), checksum_before);
}

#[test]
fn valid_forward_references_in_components_and_singletons_resolve() {
    let source = Guid::from_u32(1);
    let target = Guid::from_u32(2);
    let mut scene = Scene::default();
    scene.entities.insert(
        source.clone(),
        entity_with_component_ref(Some(target.as_str())),
    );
    scene
        .entities
        .insert(target.clone(), SceneEntity::default());
    scene
        .singletons
        .push(("RefSingleton".into(), ref_value(Some(target.as_str()))));
    let mut frame = frame();

    let index = scene.bake(&types(), &mut frame).unwrap();

    let source_entity = index.entity(&source).unwrap();
    let target_entity = index.entity(&target).unwrap();
    assert_eq!(
        frame.get::<RefComponent>(source_entity).unwrap().target,
        target_entity
    );
    assert_eq!(frame.singleton::<RefSingleton>().target, target_entity);
}

#[test]
fn explicit_null_component_and_singleton_references_remain_none() {
    let guid = Guid::from_u32(1);
    let mut scene = Scene::default();
    scene
        .entities
        .insert(guid.clone(), entity_with_component_ref(None));
    scene
        .singletons
        .push(("RefSingleton".into(), ref_value(None)));
    let mut frame = frame();

    let index = scene.bake(&types(), &mut frame).unwrap();

    let entity = index.entity(&guid).unwrap();
    assert_eq!(
        frame.get::<RefComponent>(entity).unwrap().target,
        Entity::NONE
    );
    assert_eq!(frame.singleton::<RefSingleton>().target, Entity::NONE);
}

#[test]
fn empty_singleton_map_preserves_existing_singleton_state() {
    let scene = Scene::default();
    let mut frame = frame();
    let singleton_before = frame.singleton::<RefSingleton>().target;

    let index = scene.bake(&types(), &mut frame).unwrap();

    assert!(index.is_empty());
    assert_eq!(frame.alive_count(), 0);
    assert_eq!(frame.singleton::<RefSingleton>().target, singleton_before);
}

#[test]
fn nested_array_singleton_reference_is_checked_recursively() {
    let present = Guid::from_u32(1);
    let mut scene = Scene::default();
    scene
        .entities
        .insert(present.clone(), SceneEntity::default());
    scene.singletons.push((
        "RefArraySingleton".into(),
        Value::Struct(vec![(
            "targets".into(),
            Value::Array(vec![
                Value::EntityGuid(Some(present.as_str().to_string())),
                Value::EntityGuid(Some("e_00000002".into())),
            ]),
        )]),
    ));
    let mut frame = frame();

    let error = scene.bake(&types(), &mut frame).unwrap_err();

    assert_eq!(error, BakeError::MissingEntity("e_00000002".into()));
    assert_eq!(frame.alive_count(), 0);
}

#[test]
fn strict_yaml_admission_still_rejects_missing_singleton_references() {
    let text =
        "schema: orr.scene/1\nsingletons:\n  RefSingleton: { target: e_00000002 }\nentities: {}\n";

    let error = Scene::parse(text, &types()).unwrap_err();

    assert!(error
        .to_string()
        .contains("no entity with GUID 'e_00000002'"));
}
