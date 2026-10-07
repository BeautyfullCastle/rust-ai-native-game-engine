mod common;

use bytemuck::{Pod, Zeroable};
use common::*;
use orr_ecs::{ComponentRegistryBuilder, Entity};
use orr_edit::{
    EditorDoc, FragmentTranslation2D, Op, Origin, SceneFragment, FRAGMENT_MAX_BYTES,
    FRAGMENT_MAX_ENTITIES,
};
use orr_fp::{FPVec2, FP};
use orr_reflect::{
    bytemuck, Guid, Reflect, Scene, SceneEntity, TaggedDesc, TypeDesc, TypeRegistry, Value,
    VariantDesc, ViewField,
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Reflect)]
#[bytemuck(crate = "orr_reflect::bytemuck")]
struct Links {
    targets: [Entity; 3],
    asset: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
#[bytemuck(crate = "orr_reflect::bytemuck")]
struct TaggedLinks {
    links: Links,
}

impl Reflect for TaggedLinks {
    fn describe() -> TypeDesc {
        TypeDesc::tagged(TaggedDesc {
            size: core::mem::size_of::<Self>(),
            variants: vec![VariantDesc {
                name: "linked".into(),
                doc: String::new(),
                fields: vec![ViewField::new("nested", Links::describe(), "Nested links")],
                default: vec![(
                    "nested".into(),
                    Links::describe().read(bytemuck::bytes_of(&Links::default_value())),
                )],
            }],
            read: |bytes| {
                Value::Variant(
                    "linked".into(),
                    vec![("nested".into(), Links::describe().read(bytes))],
                )
            },
            write: |bytes, value| {
                let nested = value.field("nested").ok_or("missing nested links")?;
                Links::describe()
                    .write(bytes, nested)
                    .map_err(|e| e.to_string())
            },
        })
    }
}

fn links_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    types.register_component::<Links>("Links");
    types.register_component::<TaggedLinks>("TaggedLinks");
    types
}

fn links_doc(scene: Scene) -> EditorDoc {
    let mut registry = ComponentRegistryBuilder::new();
    registry.register_component::<Links>("Links");
    registry.register_component::<TaggedLinks>("TaggedLinks");
    EditorDoc::from_scene(scene, links_types(), registry.build(), 7).unwrap()
}

fn graph() -> Scene {
    let mut scene = Scene::default();
    for (id, targets) in [(1, [1, 2, 2]), (2, [1, 0, 1])] {
        scene.entities.insert(
            Guid::from_u32(id),
            SceneEntity {
                name: Some(format!("node {id}")),
                components: vec![(
                    "Links".into(),
                    Value::Struct(vec![
                        (
                            "targets".into(),
                            Value::Array(
                                targets
                                    .into_iter()
                                    .map(|n| {
                                        Value::EntityGuid(
                                            (n != 0).then(|| Guid::from_u32(n).to_string()),
                                        )
                                    })
                                    .collect(),
                            ),
                        ),
                        ("asset".into(), Value::Int(987654321)),
                    ]),
                )],
            },
        );
    }
    for entity in scene.entities.values_mut() {
        let nested = entity.components[0].1.clone();
        entity.components.push((
            "TaggedLinks".into(),
            Value::Variant("linked".into(), vec![("nested".into(), nested)]),
        ));
    }
    scene
}

fn translate(x: i32, y: i32) -> FragmentTranslation2D {
    FragmentTranslation2D {
        component: "orr_physics::Body".into(),
        path: "pos".into(),
        delta: FPVec2::new(FP::from_int(x), FP::from_int(y)),
    }
}

#[test]
fn cyclic_graph_copies_are_disjoint_and_survive_undo_redo_reopen() {
    let source = graph();
    let fragment =
        SceneFragment::capture(&source, &[Guid::from_u32(2), Guid::from_u32(1)]).unwrap();
    let fragment = SceneFragment::from_yaml(&fragment.to_yaml(), &links_types()).unwrap();
    let mut doc = links_doc(source);
    let initial = (doc.to_yaml(), doc.frame().to_bytes());
    let first = doc
        .instantiate_fragment(&fragment, None, Origin::User)
        .unwrap();
    let after_first = (doc.to_yaml(), doc.frame().to_bytes());
    let second = doc
        .instantiate_fragment(&fragment, None, Origin::Agent("author".into()))
        .unwrap();
    for (source, copy) in &first.guids {
        assert_ne!(source, copy);
        assert!(!second.guids.values().any(|other| other == copy));
    }
    for instance in [&first, &second] {
        for (source, copy) in &instance.guids {
            let links = doc
                .frame()
                .get::<Links>(doc.index().entity(copy).unwrap())
                .unwrap();
            let expected = if source == &Guid::from_u32(1) {
                [1, 2, 2]
            } else {
                [1, 0, 1]
            };
            assert_eq!(links.asset, 987654321);
            let tagged = doc
                .frame()
                .get::<TaggedLinks>(doc.index().entity(copy).unwrap())
                .unwrap();
            assert_eq!(tagged.links.targets, links.targets);
            assert_eq!(tagged.links.asset, links.asset);
            for (actual, id) in links.targets.iter().zip(expected) {
                assert_eq!(
                    *actual,
                    if id == 0 {
                        Entity::NONE
                    } else {
                        doc.index()
                            .entity(&instance.guids[&Guid::from_u32(id)])
                            .unwrap()
                    }
                );
            }
        }
    }
    assert_eq!(doc.history().len(), 2);
    let after_second = (doc.to_yaml(), doc.frame().to_bytes());
    doc.undo().unwrap();
    assert_eq!((doc.to_yaml(), doc.frame().to_bytes()), after_first);
    doc.undo().unwrap();
    assert_eq!((doc.to_yaml(), doc.frame().to_bytes()), initial);
    doc.redo().unwrap();
    doc.redo().unwrap();
    assert_eq!((doc.to_yaml(), doc.frame().to_bytes()), after_second);
    let yaml = doc.save_yaml();
    let reopened = links_doc(Scene::parse(&yaml, &links_types()).unwrap());
    assert_eq!(reopened.scene(), doc.scene());
    assert_eq!(reopened.frame().to_bytes(), doc.frame().to_bytes());

    let mut empty = links_doc(Scene::default());
    let inserted = empty
        .instantiate_fragment(&fragment, None, Origin::User)
        .unwrap();
    assert!(inserted
        .guids
        .values()
        .all(|g| !fragment.entities().contains_key(g)));
    let mut twin = links_doc(Scene::default());
    assert_eq!(
        inserted,
        twin.instantiate_fragment(&fragment, None, Origin::User)
            .unwrap()
    );
}

#[test]
fn physics_copies_translate_only_selected_field_and_edit_independently() {
    let mut doc = demo_doc();
    let selected = [guid_named(&doc, "body_01"), guid_named(&doc, "body_02")];
    let fragment = SceneFragment::capture(doc.scene(), &selected).unwrap();
    let fragment = SceneFragment::from_yaml(&fragment.to_yaml(), &types()).unwrap();
    let first = doc
        .instantiate_fragment(&fragment, Some(&translate(4, 2)), Origin::User)
        .unwrap();
    let second = doc
        .instantiate_fragment(&fragment, Some(&translate(-4, 3)), Origin::User)
        .unwrap();
    for source in &selected {
        let get = |g: &Guid| {
            *doc.frame()
                .get::<orr_physics::Body>(doc.index().entity(g).unwrap())
                .unwrap()
        };
        let original = get(source);
        let copy = get(&first.guids[source]);
        assert_eq!(copy.pos, original.pos + translate(4, 2).delta);
        assert_eq!(copy.vel, original.vel);
        assert_eq!(copy.angle, original.angle);
    }
    let before = state(&doc);
    let untouched = doc.scene().entities[&second.guids[&selected[0]]].clone();
    doc.apply(
        Op::SetField {
            guid: first.guids[&selected[0]].clone(),
            component: "orr_physics::Body".into(),
            path: "pos".into(),
            value: vec2(2, 3),
        },
        Origin::User,
    )
    .unwrap();
    assert_eq!(doc.scene().entities[&second.guids[&selected[0]]], untouched);
    let edited = state(&doc);
    doc.undo().unwrap();
    assert_eq!(state(&doc), before);
    doc.redo().unwrap();
    assert_eq!(state(&doc), edited);
    let yaml = doc.save_yaml();
    let reopened =
        EditorDoc::from_yaml(&yaml, types(), doc.frame_registry().clone(), SEED).unwrap();
    assert_eq!(reopened.scene(), doc.scene());
    assert_eq!(reopened.checksum(), doc.checksum());
}

#[test]
fn failed_instantiation_preserves_every_observable_state_and_allocator() {
    let mut doc = demo_doc();
    let mut control = demo_doc();
    let body = guid_named(&doc, "body_01");
    for d in [&mut doc, &mut control] {
        d.apply(
            Op::Rename {
                guid: body.clone(),
                name: Some("redo survives".into()),
            },
            Origin::User,
        )
        .unwrap();
        d.undo().unwrap();
    }
    let fragment = SceneFragment::capture(doc.scene(), std::slice::from_ref(&body)).unwrap();
    let before = (
        state(&doc),
        doc.frame().to_bytes(),
        doc.history(),
        doc.revision(),
        doc.is_dirty(),
        format!("{:?}", doc.index()),
    );
    let placements = [
        translate(40000, 0),
        FragmentTranslation2D {
            delta: FPVec2::new(FP::from_raw(i64::MAX), FP::from_raw(i64::MAX)),
            ..translate(0, 0)
        },
        FragmentTranslation2D {
            path: "angle".into(),
            ..translate(0, 0)
        },
        FragmentTranslation2D {
            path: "missing".into(),
            ..translate(0, 0)
        },
        FragmentTranslation2D {
            component: "Missing".into(),
            ..translate(0, 0)
        },
    ];
    for placement in placements {
        assert!(doc
            .instantiate_fragment(&fragment, Some(&placement), Origin::User)
            .is_err());
        assert_eq!(
            (
                state(&doc),
                doc.frame().to_bytes(),
                doc.history(),
                doc.revision(),
                doc.is_dirty(),
                format!("{:?}", doc.index())
            ),
            before
        );
        assert!(doc.can_redo());
    }
    let mut bad = Scene::default();
    bad.entities.insert(
        Guid::from_u32(1),
        SceneEntity {
            components: vec![("Unknown".into(), Value::Bool(true))],
            ..Default::default()
        },
    );
    let bad = SceneFragment::capture(&bad, &[Guid::from_u32(1)]).unwrap();
    assert!(doc.instantiate_fragment(&bad, None, Origin::User).is_err());
    assert_eq!(
        (
            state(&doc),
            doc.frame().to_bytes(),
            doc.history(),
            doc.revision(),
            doc.is_dirty(),
            format!("{:?}", doc.index())
        ),
        before
    );
    doc.redo().unwrap();
    control.redo().unwrap();
    let spawn = || Op::SpawnEntity {
        guid: None,
        name: None,
        components: vec![],
    };
    assert_eq!(
        doc.apply(spawn(), Origin::User).unwrap(),
        control.apply(spawn(), Origin::User).unwrap()
    );
    assert_eq!(state(&doc), state(&control));
    assert_eq!(doc.history(), control.history());
}

#[test]
fn rejects_open_selections_raw_handles_singletons_and_bounds() {
    let mut scene = graph();
    assert!(SceneFragment::capture(&scene, &[]).is_err());
    assert!(SceneFragment::capture(&scene, &[Guid::from_u32(1)])
        .unwrap_err()
        .to_string()
        .contains("e_00000002"));
    assert!(SceneFragment::capture(&scene, &[Guid::from_u32(1), Guid::from_u32(1)]).is_err());
    assert!(SceneFragment::capture(&scene, &[Guid::from_u32(99)]).is_err());
    scene
        .entities
        .get_mut(&Guid::from_u32(1))
        .unwrap()
        .components[0]
        .1 = Value::Entity(Entity::NONE);
    assert!(SceneFragment::capture(&scene, &[Guid::from_u32(1), Guid::from_u32(2)]).is_err());
    assert!(SceneFragment::from_yaml(&demo_text(), &types()).is_err());
    assert!(SceneFragment::from_yaml(&" ".repeat(FRAGMENT_MAX_BYTES + 1), &types()).is_err());
    let ids: Vec<_> = (0..=FRAGMENT_MAX_ENTITIES as u32)
        .map(Guid::from_u32)
        .collect();
    assert!(SceneFragment::capture(&Scene::default(), &ids).is_err());
    let duplicate = "schema: orr.scene/1\nentities:\n  e_00000001: {}\n  e_00000001: {}\n";
    assert!(SceneFragment::from_yaml(duplicate, &types()).is_err());
}

#[test]
fn absent_components_are_untouched_and_open_transactions_are_rejected() {
    let mut doc = demo_doc();
    let body = guid_named(&doc, "body_01");
    let marker = Guid::from_u32(9999);
    let mut source = doc.scene().clone();
    source
        .entities
        .insert(marker.clone(), SceneEntity::default());
    let fragment = SceneFragment::capture(&source, &[body, marker.clone()]).unwrap();
    let copy = doc
        .instantiate_fragment(&fragment, Some(&translate(1, 1)), Origin::User)
        .unwrap();
    assert!(doc.scene().entities[&copy.guids[&marker]]
        .components
        .is_empty());
    doc.begin_tx("existing transaction", Origin::User).unwrap();
    let before = (
        state(&doc),
        doc.frame().to_bytes(),
        doc.history(),
        doc.revision(),
    );
    assert!(doc
        .instantiate_fragment(&fragment, None, Origin::User)
        .is_err());
    assert_eq!(
        (
            state(&doc),
            doc.frame().to_bytes(),
            doc.history(),
            doc.revision()
        ),
        before
    );
    assert!(doc.in_tx());
    doc.rollback_tx().unwrap();
}

#[test]
fn inbound_references_do_not_expand_the_selection() {
    let mut scene = graph();
    let first = Guid::from_u32(1);
    // The unselected node still points at this empty selected node.
    scene.entities.get_mut(&first).unwrap().components.clear();
    let fragment = SceneFragment::capture(&scene, std::slice::from_ref(&first)).unwrap();
    assert_eq!(fragment.entities().len(), 1);
    assert!(fragment.entities().contains_key(&first));
}
