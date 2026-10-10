//! Linked metadata remains opt-in and rejects untrusted or stale snapshots.
#![cfg(feature = "scene")]

use orr_reflect::{Scene, TypeRegistry};

#[test]
fn legacy_schema_never_accepts_prefabs() {
    let error = Scene::parse(
        "schema: orr.scene/1\nentities: {}\nprefabs: {}\n",
        &TypeRegistry::new(),
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("unknown top-level key 'prefabs'"));
}

#[test]
fn unrelated_registry_retains_legacy_version_diagnostic() {
    let error =
        Scene::parse("schema: orr.scene/2\nentities: {}\n", &TypeRegistry::new()).unwrap_err();
    assert!(error
        .first()
        .message
        .contains("unsupported schema 'orr.scene/2'"));
    assert_eq!((error.first().pos.line, error.first().pos.col), (1, 9));
}

#[cfg(not(feature = "linked-prefabs"))]
#[test]
fn feature_disabled_rejects_schema_two() {
    assert!(
        Scene::parse("schema: orr.scene/2\nentities: {}\n", &TypeRegistry::new())
            .unwrap_err()
            .to_string()
            .contains("unsupported schema")
    );
}

#[cfg(feature = "linked-prefabs")]
mod enabled {
    use super::*;
    use bytemuck::{Pod, Zeroable};
    use orr_fp::FPVec2;
    use orr_reflect::{Guid, PrefabLink, Reflect, Value};
    use std::collections::{BTreeMap, BTreeSet};

    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable, Reflect)]
    struct Actor {
        position: FPVec2,
        velocity: FPVec2,
        kind: u32,
        ordinal: u32,
    }

    fn reg() -> TypeRegistry {
        let mut reg = TypeRegistry::new();
        reg.register_component::<Actor>("CollectDodgeV1::Actor");
        reg
    }

    fn scene() -> Scene {
        let source = Guid::from_u32(1);
        let target = Guid::from_u32(10);
        let baseline = Scene::parse("schema: orr.scene/1\nentities:\n  e_00000001:\n    name: Coin\n    CollectDodgeV1::Actor: { position: [1, 2], velocity: [0, 0], kind: 1, ordinal: 0 }\n", &reg()).unwrap();
        let mut live = Scene::default();
        live.entities
            .insert(target.clone(), baseline.entities[&source].clone());
        let baseline = baseline.to_yaml();
        live.prefab_links.insert(
            target.clone(),
            PrefabLink {
                source: "prefabs/coin.scene.yaml".into(),
                digest: PrefabLink::canonical_digest(&baseline),
                baseline,
                guids: BTreeMap::from([(source.clone(), target)]),
                ordinals: BTreeMap::from([(source, 0)]),
                position_overrides: BTreeSet::new(),
            },
        );
        live
    }

    fn link(scene: &mut Scene) -> &mut PrefabLink {
        scene.prefab_links.values_mut().next().unwrap()
    }
    fn reject(scene: &Scene) {
        assert!(scene.validate_prefab_links(&reg()).is_err());
        assert!(Scene::parse(&scene.to_yaml(), &reg()).is_err());
    }

    #[test]
    fn linked_json_schema_exports_exact_closed_metadata_and_version_gates() {
        use serde_json::json;
        let text = reg().json_schema();
        assert_eq!(text, reg().json_schema());
        let schema: serde_json::Value = serde_json::from_str(&text).unwrap();
        let guid = json!({"type": "string", "pattern": "^e_[0-9a-f]{8,32}$"});
        let map = |value| {
            json!({"type": "object", "minProperties": 1, "maxProperties": 8,
            "propertyNames": guid.clone(), "additionalProperties": value})
        };
        let expected = json!({
            "type": "object",
            "properties": {
                "profile": {"const": "collect-actors-v1"},
                "source": {"type": "string", "minLength": 1, "maxLength": 240,
                    "pattern": "^[A-Za-z0-9_.-]+(/[A-Za-z0-9_.-]+)*$"},
                "digest": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                "baseline": {"type": "string", "maxLength": 32768},
                "guids": map(guid.clone()),
                "ordinals": map(json!({"type": "integer", "minimum": 0, "maximum": 4294967295u64})),
                "position_overrides": {"type": "array", "items": guid.clone(), "maxItems": 8, "uniqueItems": true}
            },
            "required": ["profile", "source", "digest", "baseline", "guids", "ordinals", "position_overrides"],
            "additionalProperties": false
        });
        assert_eq!(schema["properties"]["prefabs"], map(expected));
        assert_eq!(
            schema["properties"]["schema"],
            json!({"enum": ["orr.scene/1", "orr.scene/2"]})
        );
        assert_eq!(
            schema["if"],
            json!({"properties": {"schema": {"const": "orr.scene/1"}}})
        );
        assert_eq!(schema["then"], json!({"properties": {"prefabs": false}}));
        assert_eq!(schema["else"], json!({"required": ["prefabs"]}));
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["schema", "entities"]));
        assert!(schema["description"]
            .as_str()
            .unwrap()
            .contains("no filesystem access"));
        assert!(schema["$defs"]["CollectDodgeV1::Actor"].is_object());

        // Neither an unrelated registry nor a lookalike singleton opts in.
        let mut unrelated = TypeRegistry::new();
        unrelated.register_component::<Actor>("Other::Actor");
        let mut singleton = TypeRegistry::new();
        singleton.register_singleton::<Actor>("CollectDodgeV1::Actor");
        for registry in [TypeRegistry::new(), unrelated, singleton] {
            let legacy: serde_json::Value = serde_json::from_str(&registry.json_schema()).unwrap();
            assert_eq!(
                legacy["properties"]["schema"],
                json!({"const": "orr.scene/1"})
            );
            assert!(legacy["properties"].get("prefabs").is_none());
            assert!(legacy.get("if").is_none());
        }
    }

    #[test]
    fn round_trip_and_legacy_are_stable() {
        let scene = scene();
        let yaml = scene.to_yaml();
        assert!(yaml.starts_with("schema: orr.scene/2\n"));
        let parsed = Scene::parse(&yaml, &reg()).unwrap();
        assert_eq!(scene, parsed);
        assert_eq!(yaml, parsed.to_yaml());
        let legacy = "schema: orr.scene/1\nentities: {}\n";
        assert_eq!(Scene::parse(legacy, &reg()).unwrap().to_yaml(), legacy);
        let large_legacy = format!(
            "# {}\nschema: orr.scene/1\nentities: {{}}\n",
            "x".repeat(70000)
        );
        assert_eq!(
            Scene::parse(&large_legacy, &reg()).unwrap().to_yaml(),
            large_legacy
        );
    }

    #[test]
    fn rejects_unknown_duplicate_and_missing_fields() {
        let yaml = scene().to_yaml();
        for bad in [
            yaml.replace("    profile:", "    unexpected: true\n    profile:"),
            yaml.replace(
                "    profile:",
                "    profile: collect-actors-v1\n    profile:",
            ),
            yaml.replace("    profile: collect-actors-v1\n", ""),
            yaml.replace("collect-actors-v1", "future-profile"),
            yaml.replace("orr.scene/2", "orr.scene/3"),
            yaml.replace(
                "position_overrides: []",
                "position_overrides: [e_00000001, e_00000001]",
            ),
        ] {
            assert!(Scene::parse(&bad, &reg()).is_err(), "{bad}");
        }
        assert!(Scene::parse("schema: orr.scene/2\nentities: {}\nprefabs: {}\n", &reg()).is_err());
    }

    #[test]
    fn source_labels_are_portable_and_inert() {
        for path in [
            "",
            "/abs",
            "../coin",
            "a/../coin",
            "a/./coin",
            "a//coin",
            "C:/coin",
            "a\\coin",
            "a\0coin",
            "café",
            "CON",
            "aux.yaml",
            "a/COM1.scene",
            "Lpt9",
            "a.",
        ] {
            let mut scene = scene();
            link(&mut scene).source = path.into();
            reject(&scene);
        }
        let mut scene = scene();
        link(&mut scene).source = "does-not-exist/coin.scene".into();
        scene.validate_prefab_links(&reg()).unwrap();
    }

    #[test]
    fn digest_and_baseline_are_exact() {
        assert_eq!(
            PrefabLink::canonical_digest(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let mut bad = scene();
        link(&mut bad).digest = "0".repeat(64);
        reject(&bad);
        let mut bad = scene();
        link(&mut bad).digest = link(&mut bad).digest.to_uppercase();
        reject(&bad);
        for change in [
            "comment",
            "trailing",
            "nested",
            "empty",
            "player",
            "singletons",
        ] {
            let mut bad = scene();
            let link = link(&mut bad);
            link.baseline = match change {
                "comment" => format!("# source comment\n{}", link.baseline),
                "trailing" => format!("{}\n", link.baseline),
                "nested" => link.baseline.replace("orr.scene/1", "orr.scene/2"),
                "empty" => "schema: orr.scene/1\nentities: {}\n".into(),
                "player" => link.baseline.replace("kind: 1", "kind: 0"),
                _ => format!("{}prefabs: {{}}\n", link.baseline),
            };
            link.digest = PrefabLink::canonical_digest(&link.baseline);
            reject(&bad);
        }
    }

    #[test]
    fn mapping_keys_targets_roots_and_ownership_are_strict() {
        let mut bad = scene();
        link(&mut bad).guids.clear();
        reject(&bad);
        let mut bad = scene();
        link(&mut bad).ordinals.clear();
        reject(&bad);
        let mut bad = scene();
        link(&mut bad).position_overrides.insert(Guid::from_u32(99));
        reject(&bad);
        let mut bad = scene();
        bad.entities.clear();
        reject(&bad);
        let mut bad = scene();
        let value = bad.prefab_links.pop_first().unwrap().1;
        bad.prefab_links.insert(Guid::from_u32(99), value);
        reject(&bad);
        let mut bad = scene();
        let original = link(&mut bad).clone();
        bad.prefab_links.insert(Guid::from_u32(99), original);
        reject(&bad);
    }

    #[test]
    fn entity_limit_and_disjoint_ownership_are_enforced() {
        let mut bad = scene();
        for i in 2..=9 {
            link(&mut bad)
                .guids
                .insert(Guid::from_u32(i), Guid::from_u32(i + 10));
        }
        reject(&bad);
        let mut bad = scene();
        let baseline_text = link(&mut bad).baseline.clone();
        let mut baseline = Scene::parse(&baseline_text, &reg()).unwrap();
        baseline.entities.insert(
            Guid::from_u32(2),
            baseline.entities[&Guid::from_u32(1)].clone(),
        );
        let text = baseline.to_yaml();
        let mut first = link(&mut bad).clone();
        first.baseline = text.clone();
        first.digest = PrefabLink::canonical_digest(&text);
        first.guids.insert(Guid::from_u32(2), Guid::from_u32(11));
        first.ordinals.insert(Guid::from_u32(2), 0);
        bad.entities.insert(
            Guid::from_u32(11),
            baseline.entities[&Guid::from_u32(2)].clone(),
        );
        bad.prefab_links.insert(Guid::from_u32(10), first.clone());
        bad.validate_prefab_links(&reg()).unwrap();
        // Both roots exist in their mappings, but the two links overlap.
        bad.prefab_links.insert(Guid::from_u32(11), first);
        assert!(bad
            .validate_prefab_links(&reg())
            .unwrap_err()
            .contains("disjoint"));
        reject(&bad);
        let mut bad = scene();
        let template = link(&mut bad).clone();
        for i in 11..19 {
            bad.prefab_links.insert(Guid::from_u32(i), template.clone());
        }
        assert!(bad
            .validate_prefab_links(&reg())
            .unwrap_err()
            .contains("8 links"));
        reject(&bad);
    }

    #[test]
    fn flattened_values_must_match_except_explicit_position_and_ordinal() {
        for name in ["name", "position", "velocity", "kind", "ordinal"] {
            let mut bad = scene();
            let entity = bad.entities.values_mut().next().unwrap();
            if name == "name" {
                entity.name = Some("changed".into());
            } else {
                let Value::Struct(fields) = &mut entity.components[0].1 else {
                    unreachable!()
                };
                let value = &mut fields.iter_mut().find(|(n, _)| n == name).unwrap().1;
                *value = match value {
                    Value::Vec2(_) => Value::Vec2(FPVec2::ZERO),
                    _ => Value::Int(2),
                };
                // Initial velocity is zero; make its mutation distinguishable.
                if name == "velocity" {
                    *value = Value::Vec2(FPVec2::new(orr_fp::FP::ONE, orr_fp::FP::ZERO));
                }
            }
            reject(&bad);
        }
        let mut good = scene();
        let Value::Struct(fields) = &mut good.entities.values_mut().next().unwrap().components[0].1
        else {
            unreachable!()
        };
        fields[0].1 = Value::Vec2(FPVec2::ZERO);
        fields[3].1 = Value::Int(7);
        link(&mut good).position_overrides.insert(Guid::from_u32(1));
        link(&mut good).ordinals.insert(Guid::from_u32(1), 7);
        good.validate_prefab_links(&reg()).unwrap();
        Scene::parse(&good.to_yaml(), &reg()).unwrap();
    }

    #[test]
    fn bounds_apply_before_baseline_recursion() {
        let mut bad = scene();
        link(&mut bad).baseline = "x".repeat(32769);
        reject(&bad);
        let mut bad = scene();
        bad.header_comments.push("x".repeat(65536));
        reject(&bad);
        let yaml = format!(
            "schema: orr.scene/2\n# {}\nentities: {{}}\nprefabs: {{}}\n",
            "x".repeat(65536)
        );
        assert!(Scene::parse(&yaml, &reg())
            .unwrap_err()
            .to_string()
            .contains("64 KiB"));
    }

    #[test]
    fn oversized_preflight_rejects_late_duplicate_schema_and_syntax_errors() {
        let prefix = format!(
            "schema: orr.scene/1\nentities: {{}}\n# {}\n",
            "x".repeat(70000)
        );
        let duplicate = format!("{prefix}schema: orr.scene/2\n");
        // The size diagnostic proves preflight, rather than full duplicate-key
        // tree construction, rejected the late version switch.
        assert!(Scene::parse(&duplicate, &reg())
            .unwrap_err()
            .to_string()
            .contains("64 KiB"));
        let syntax = format!("{prefix}broken: [\n");
        assert!(Scene::parse(&syntax, &reg())
            .unwrap_err()
            .to_string()
            .contains("YAML syntax error"));
        let deep = format!("{prefix}broken: {}0{}\n", "[".repeat(49), "]".repeat(49));
        assert!(Scene::parse(&deep, &reg())
            .unwrap_err()
            .to_string()
            .contains("nesting is deeper"));
    }

    #[test]
    fn bake_rejects_hand_built_invalid_links_before_mutating_frame() {
        use orr_ecs::{ComponentRegistryBuilder, Frame};
        let mut builder = ComponentRegistryBuilder::new();
        builder.register_component::<Actor>("CollectDodgeV1::Actor");
        let mut frame = Frame::new(builder.build());
        let mut bad = scene();
        link(&mut bad).digest.clear();
        assert!(bad
            .bake(&reg(), &mut frame)
            .unwrap_err()
            .to_string()
            .contains("prefab"));
        assert_eq!(frame.entities().count(), 0);
    }
}
