//! `Scene::bake_prefabs`: a null Entity field is baked as `Entity::NONE`, a real reference is rejected.

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistry, ComponentRegistryBuilder, Entity};
use orr_reflect::{BakeError, Scene, TypeRegistry};
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, orr_reflect::Reflect)]
struct Aim {
    target: Entity,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, orr_reflect::Reflect)]
struct Link {
    weight: u32,
    other: Entity,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, orr_reflect::Reflect)]
struct Nested {
    link: Link,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, orr_reflect::Reflect)]
struct Squad {
    members: [Entity; 3],
    links: [Link; 2],
}

fn types() -> TypeRegistry {
    let mut reg = TypeRegistry::new();
    reg.register_component::<Aim>("Aim");
    reg.register_component::<Nested>("Nested");
    reg.register_component::<Squad>("Squad");
    reg
}

fn ecs() -> Arc<ComponentRegistry> {
    let mut builder = ComponentRegistryBuilder::new();
    builder.register_component::<Aim>("Aim");
    builder.register_component::<Nested>("Nested");
    builder.register_component::<Squad>("Squad");
    builder.build()
}

/// The bytes of the one component of the one prefab that `entity_text` bakes to.
fn baked_bytes(entity_text: &str) -> Result<Vec<u8>, BakeError> {
    let text = format!("schema: orr.scene/1\nentities:\n  e_00000001:\n    {entity_text}\n");
    let scene = Scene::parse(&text, &types()).unwrap_or_else(|e| panic!("scene should parse:\n{e}"));
    let prefabs = scene.bake_prefabs(&types(), &ecs())?;
    assert_eq!(prefabs.len(), 1);
    assert_eq!(prefabs[0].components().len(), 1);
    Ok(prefabs[0].components()[0].1.clone())
}

#[test]
fn an_explicit_null_entity_field_is_baked_as_none() {
    let bytes = baked_bytes("Aim: { target: null }").unwrap();
    assert_eq!(bytes, bytemuck::bytes_of(&Aim { target: Entity::NONE }));
}

#[test]
fn a_null_entity_in_a_nested_struct_is_baked_as_none() {
    let bytes = baked_bytes("Nested: { link: { weight: 7, other: null } }").unwrap();
    assert_eq!(bytes, bytemuck::bytes_of(&Nested { link: Link { weight: 7, other: Entity::NONE } }));
}

#[test]
fn null_entities_in_a_list_and_in_a_list_of_structs_are_baked_as_none() {
    let bytes = baked_bytes("Squad: { members: [null, null, null], links: [{ weight: 1, other: null }, { weight: 2, other: null }] }").unwrap();
    let expected = Squad {
        members: [Entity::NONE; 3],
        links: [Link { weight: 1, other: Entity::NONE }, Link { weight: 2, other: Entity::NONE }],
    };
    assert_eq!(bytes, bytemuck::bytes_of(&expected));
}

#[test]
fn a_real_reference_is_still_rejected_also_next_to_nulls() {
    for text in [
        "Aim: { target: e_00000001 }",
        "Nested: { link: { weight: 1, other: e_00000001 } }",
        "Squad: { members: [null, e_00000001, null], links: [{ weight: 1, other: null }, { weight: 2, other: null }] }",
        "Squad: { members: [null, null, null], links: [{ weight: 1, other: null }, { weight: 2, other: e_00000001 }] }",
    ] {
        match baked_bytes(text) {
            Err(BakeError::PrefabReference(at)) => assert!(at.starts_with("e_00000001."), "{at}"),
            other => panic!("{text}: expected a prefab reference error, got {other:?}"),
        }
    }
}
