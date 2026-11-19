//! `generic-v1` prefab links in `orr.scene/2`: any registry, strict structure.
#![cfg(all(feature = "scene", feature = "linked-prefabs"))]

use std::collections::{BTreeMap, BTreeSet};

use bytemuck::{Pod, Zeroable};
use orr_fp::FP;
use orr_reflect::{Guid, LinkProfile, PrefabLink, Reflect, Scene, TypeRegistry};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Reflect)]
struct Thing {
    weight: FP,
}

fn reg() -> TypeRegistry {
    let mut reg = TypeRegistry::new();
    reg.register_component::<Thing>("Thing");
    reg
}

fn baseline() -> String {
    Scene::parse("schema: orr.scene/1\nentities:\n  e_00000001:\n    name: A\n    Thing: { weight: 1 }\n  e_00000002:\n    Thing: { weight: 2 }\n", &reg()).unwrap().to_yaml()
}

fn link(text: &str, source: &str, targets: [u32; 2]) -> PrefabLink {
    PrefabLink {
        profile: LinkProfile::GenericV1,
        source: source.into(),
        digest: PrefabLink::canonical_digest(text),
        baseline: text.into(),
        guids: BTreeMap::from([(Guid::from_u32(1), Guid::from_u32(targets[0])), (Guid::from_u32(2), Guid::from_u32(targets[1]))]),
        ordinals: BTreeMap::new(),
        position_overrides: BTreeSet::new(),
    }
}

fn scene() -> Scene {
    let text = baseline();
    let mut scene = Scene::default();
    for (id, weight) in [(10, "1"), (11, "2"), (20, "1"), (21, "9")] {
        let doc = format!("schema: orr.scene/1\nentities:\n  e_{id:08x}:\n    Thing: {{ weight: {weight} }}\n");
        scene.entities.extend(Scene::parse(&doc, &reg()).unwrap().entities);
    }
    scene.entities.get_mut(&Guid::from_u32(10)).unwrap().name = Some("A".into());
    scene.entities.get_mut(&Guid::from_u32(20)).unwrap().name = Some("A".into());
    scene.prefab_links.insert(Guid::from_u32(10), link(&text, "boxes.prefab.yaml", [10, 11]));
    scene.prefab_links.insert(Guid::from_u32(20), link(&text, "boxes.prefab.yaml", [20, 21]));
    scene
}

#[test]
fn a_generic_scene_round_trips_on_any_registry_and_overrides_are_not_stored() {
    let scene = scene();
    scene.validate_prefab_links(&reg()).unwrap();
    let yaml = scene.to_yaml();
    assert!(yaml.starts_with("schema: orr.scene/2\n"));
    assert_eq!(yaml.matches("profile: generic-v1").count(), 2);
    assert!(!yaml.contains("ordinals") && !yaml.contains("position_overrides"));
    let parsed = Scene::parse(&yaml, &reg()).unwrap();
    assert_eq!(parsed, scene);
    assert_eq!(parsed.to_yaml(), yaml);
    // The live value that differs from the baseline (weight 9) loads without complaint.
    assert!(yaml.contains("weight: 9"));
}

#[test]
fn a_scene_without_generic_links_still_gets_the_legacy_version_diagnostic() {
    let error = Scene::parse("schema: orr.scene/2\nentities: {}\n", &reg()).unwrap_err();
    assert!(error.first().message.contains("unsupported schema 'orr.scene/2'"));
}

fn reject(scene: &Scene) {
    assert!(scene.validate_prefab_links(&reg()).is_err());
    assert!(Scene::parse(&scene.to_yaml(), &reg()).is_err());
}

#[test]
fn structural_damage_is_rejected() {
    let mut s = scene();
    s.prefab_links.get_mut(&Guid::from_u32(10)).unwrap().digest = "0".repeat(64);
    reject(&s);

    let mut s = scene();
    s.prefab_links.get_mut(&Guid::from_u32(10)).unwrap().guids.remove(&Guid::from_u32(2));
    reject(&s);

    let mut s = scene();
    s.prefab_links.get_mut(&Guid::from_u32(20)).unwrap().guids.insert(Guid::from_u32(2), Guid::from_u32(11));
    reject(&s);

    let mut s = scene();
    let l = s.prefab_links.get_mut(&Guid::from_u32(10)).unwrap();
    l.baseline = l.baseline.replace("weight: 1", "weight: 1.0 ");
    l.digest = PrefabLink::canonical_digest(&l.baseline);
    reject(&s);

    let mut s = scene();
    s.prefab_links.get_mut(&Guid::from_u32(10)).unwrap().ordinals.insert(Guid::from_u32(1), 0);
    // The writer never emits ordinals for a generic link, so only the in-memory check can see them.
    assert!(s.validate_prefab_links(&reg()).is_err());

    let mut s = scene();
    s.prefab_links.get_mut(&Guid::from_u32(10)).unwrap().source = "../escape.prefab.yaml".into();
    reject(&s);

    // The key must be one of the instance's own entities.
    let mut s = scene();
    let l = s.prefab_links.remove(&Guid::from_u32(10)).unwrap();
    s.prefab_links.insert(Guid::from_u32(20), l);
    reject(&s);
}

#[test]
fn profiles_cannot_be_mixed() {
    let mut s = scene();
    s.prefab_links.get_mut(&Guid::from_u32(10)).unwrap().profile = LinkProfile::CollectActorsV1;
    assert!(s.validate_prefab_links(&reg()).is_err());
}

#[test]
fn a_deleted_member_is_not_a_structural_error() {
    let mut s = scene();
    s.entities.remove(&Guid::from_u32(11));
    s.validate_prefab_links(&reg()).unwrap();
    assert_eq!(Scene::parse(&s.to_yaml(), &reg()).unwrap(), s);
}

#[test]
fn generic_scenes_may_exceed_the_collect_size_limit_but_not_four_mebibytes() {
    let mut s = scene();
    for n in 100..2100u32 {
        let doc = format!("schema: orr.scene/1\nentities:\n  e_{n:08x}:\n    name: filler_with_a_long_enough_name_to_grow_the_file\n    Thing: {{ weight: 1 }}\n");
        s.entities.extend(Scene::parse(&doc, &reg()).unwrap().entities);
    }
    let yaml = s.to_yaml();
    assert!(yaml.len() > 64 * 1024 && yaml.len() < 4 * 1024 * 1024);
    assert_eq!(Scene::parse(&yaml, &reg()).unwrap(), s);
    let huge = format!("{yaml}{}", "#".repeat(4 * 1024 * 1024));
    assert!(Scene::parse(&huge, &reg()).is_err());
}
