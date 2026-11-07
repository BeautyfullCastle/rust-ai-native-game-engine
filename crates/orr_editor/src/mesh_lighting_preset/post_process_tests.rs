use super::*;
use orr_render::{Lighting, PostProcessSettings};
use serde_json::{json, Value};
fn document() -> Document {
    Document::from_lighting(Lighting {
        direction: [0.0, -1.0, 0.0],
        color: [1.0; 3],
        intensity: 1.0,
        sky: [0.0; 3],
        ground: [0.0; 3],
        ambient: 0.0,
        shadows: false,
        shadow_center: [17.0, -3.0, 9.0],
        shadow_radius: 25.0,
        tonemap: false,
        exposure: 1.0,
    })
    .unwrap()
}
fn effects() -> PostProcessSettings {
    PostProcessSettings {
        enabled: true,
        bloom: true,
        threshold: 2.0,
        strength: 1.25,
        radius: 6,
        iterations: 3,
    }
}
fn encoded() -> Vec<u8> {
    Preset::from_document_with_post_process(&document().with_post_process(effects()).unwrap())
        .unwrap()
        .encode()
        .unwrap()
}
fn rejects(v: &Value) {
    assert!(Preset::decode(&serde_json::to_vec(v).unwrap()).is_err());
}
#[test]
fn legacy_constructor_and_wire_bytes_remain_exact_even_with_source_effects() {
    let old = br#"{
  "schema": 1,
  "identity": "ORR_MESH_LIGHTING_PRESET_V1",
  "lighting": {
    "direction": [
      0.0,
      -1.0,
      0.0
    ],
    "color": [
      1.0,
      1.0,
      1.0
    ],
    "intensity": 1.0,
    "sky": [
      0.0,
      0.0,
      0.0
    ],
    "ground": [
      0.0,
      0.0,
      0.0
    ],
    "ambient": 0.0,
    "shadows": false,
    "tonemap": false,
    "exposure": 1.0
  }
}
"#;
    let source = document();
    let with_effects = source.clone().with_post_process(effects()).unwrap();
    for source in [&source, &with_effects] {
        let preset = Preset::from_document(source).unwrap();
        assert!(!preset.includes_post_process());
        assert_eq!(preset.encode().unwrap(), old);
    }
    assert_eq!(Preset::decode(old).unwrap().encode().unwrap(), old);
}
#[test]
fn v2_roundtrip_transfers_effects_and_preserves_target_bounds_and_both_inputs() {
    let source = document().with_post_process(effects()).unwrap();
    let preset = Preset::from_document_with_post_process(&source).unwrap();
    assert!(preset.includes_post_process());
    let bytes = preset.encode().unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["schema"], 2);
    assert_eq!(value["identity"], "ORR_MESH_LIGHTING_PRESET_V2");
    assert_eq!(
        value["lighting"]["post_process"].as_object().unwrap().len(),
        6
    );
    assert_eq!(Preset::decode(&bytes).unwrap(), preset);
    let target = Document::from_lighting(Lighting {
        shadow_center: [-31.0, 50.0, 102.0],
        shadow_radius: 175.0,
        ..Lighting::default()
    })
    .unwrap()
    .with_post_process(PostProcessSettings {
        threshold: 4.0,
        ..Default::default()
    })
    .unwrap();
    let original = target.clone();
    let original_source = source.clone();
    let original_preset = preset.clone();
    let merged = preset.merge_into(&target).unwrap();
    assert_eq!(merged.post_process_settings(), effects());
    assert!(merged.has_post_process());
    assert_eq!(merged.shadow_center, target.shadow_center);
    assert_eq!(merged.shadow_radius, target.shadow_radius);
    assert_eq!(merged.direction, source.direction);
    assert_eq!(target, original);
    assert_eq!(source, original_source);
    assert_eq!(preset, original_preset);
    let plain = document();
    let merged = preset.merge_into(&plain).unwrap();
    assert!(merged.has_post_process());
    assert_eq!(merged.post_process_settings(), effects());
    assert!(!plain.has_post_process());
}
#[test]
fn explicit_default_capture_disables_destination_while_v1_preserves_its_authored_presence() {
    let source = document();
    assert!(!source.has_post_process());
    let capture = Preset::from_document_with_post_process(&source).unwrap();
    assert!(capture.includes_post_process());
    let target = document().with_post_process(effects()).unwrap();
    let merged = capture.merge_into(&target).unwrap();
    assert!(merged.has_post_process());
    assert_eq!(
        merged.post_process_settings(),
        PostProcessSettings::default()
    );
    assert!(!merged.post_process_settings().enabled);
    let legacy = Preset::from_document(&source).unwrap();
    let retained = legacy.merge_into(&target).unwrap();
    assert!(retained.has_post_process());
    assert_eq!(retained.post_process_settings(), effects());
    let absent = legacy.merge_into(&source).unwrap();
    assert!(!absent.has_post_process());
    assert_eq!(source, document());
    assert_eq!(target.post_process_settings(), effects());
    let explicit_disabled = source
        .clone()
        .with_post_process(PostProcessSettings {
            bloom: false,
            threshold: 7.0,
            strength: 0.5,
            radius: 2,
            iterations: 1,
            ..Default::default()
        })
        .unwrap();
    let copy = Preset::from_document_with_post_process(&explicit_disabled)
        .unwrap()
        .merge_into(&target)
        .unwrap();
    assert_eq!(
        copy.post_process_settings(),
        explicit_disabled.post_process_settings()
    );
}
#[test]
fn v2_strict_objects_required_fields_schema_identity_and_duplicate_fields_reject() {
    let good: Value = serde_json::from_slice(&encoded()).unwrap();
    let effect = good["lighting"]["post_process"].clone();
    for invalid in [
        Value::Null,
        json!([]),
        json!([true, true, 2.0, 1.25, 6, 3]),
        json!({"enabled":true}),
    ] {
        let mut bad = good.clone();
        bad["lighting"]["post_process"] = invalid;
        rejects(&bad);
    }
    for field in [
        "enabled",
        "bloom",
        "threshold",
        "strength",
        "radius",
        "iterations",
    ] {
        let mut bad = good.clone();
        bad["lighting"]["post_process"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        rejects(&bad);
    }
    let mut extra = good.clone();
    extra["lighting"]["post_process"]["unknown"] = json!(0);
    rejects(&extra);
    let mut missing = good.clone();
    missing["lighting"]
        .as_object_mut()
        .unwrap()
        .remove("post_process");
    rejects(&missing);
    for (schema, identity, present) in [
        (1, "ORR_MESH_LIGHTING_PRESET_V1", true),
        (1, "ORR_MESH_LIGHTING_PRESET_V2", false),
        (2, "ORR_MESH_LIGHTING_PRESET_V1", true),
        (2, "ORR_MESH_LIGHTING_PRESET_V2", false),
        (3, "ORR_MESH_LIGHTING_PRESET_V2", true),
    ] {
        let mut bad = good.clone();
        bad["schema"] = json!(schema);
        bad["identity"] = json!(identity);
        if !present {
            bad["lighting"]
                .as_object_mut()
                .unwrap()
                .remove("post_process");
        }
        rejects(&bad);
    }
    let mut bad = good.clone();
    bad["lighting"] = json!([]);
    rejects(&bad);
    rejects(&json!([2, "ORR_MESH_LIGHTING_PRESET_V2", good["lighting"]]));
    let text = String::from_utf8(encoded()).unwrap();
    for bad in [
        text.replace("\"enabled\": true", "\"enabled\": true, \"enabled\": false"),
        text.replace(
            "\"post_process\":",
            &format!("\"post_process\": {}, \"post_process\":", effect),
        ),
        text.replace("\"schema\": 2", "\"schema\": 2, \"schema\": 2"),
    ] {
        assert!(Preset::decode(bad.as_bytes()).is_err());
    }
}
#[test]
fn effects_bounds_apply_even_when_disabled_and_errors_never_mutate_target() {
    let good: Value = serde_json::from_slice(&encoded()).unwrap();
    for (field, value) in [
        ("threshold", json!(-0.1)),
        ("threshold", json!(65505.0)),
        ("strength", json!(-0.1)),
        ("strength", json!(8.01)),
        ("radius", json!(0)),
        ("radius", json!(9)),
        ("radius", json!(1.5)),
        ("iterations", json!(0)),
        ("iterations", json!(5)),
        ("enabled", json!(1)),
    ] {
        for enabled in [false, true] {
            let mut bad = good.clone();
            bad["lighting"]["post_process"]["enabled"] = json!(enabled);
            bad["lighting"]["post_process"][field] = value.clone();
            rejects(&bad);
        }
    }
    let mut preset = Preset::from_document_with_post_process(&document()).unwrap();
    let original_target = document().with_post_process(effects()).unwrap();
    let target = original_target.clone();
    preset.post_process = Some(
        PostProcessSettings {
            threshold: f32::NAN,
            ..Default::default()
        }
        .into(),
    );
    assert!(preset.encode().is_err());
    assert!(preset.merge_into(&target).is_err());
    assert_eq!(target, original_target);
    let mut invalid_source = document();
    invalid_source.post_process = Some(
        PostProcessSettings {
            radius: 0,
            ..Default::default()
        }
        .into(),
    );
    let original_source = invalid_source.clone();
    assert!(Preset::from_document_with_post_process(&invalid_source).is_err());
    assert_eq!(invalid_source, original_source);
    assert!(Preset::decode(&[]).is_err());
    assert!(Preset::decode(&vec![b' '; MAX_BYTES + 1]).is_err());
    let mut trailing = encoded();
    trailing.extend_from_slice(b"{}");
    assert!(Preset::decode(&trailing).is_err());
}
