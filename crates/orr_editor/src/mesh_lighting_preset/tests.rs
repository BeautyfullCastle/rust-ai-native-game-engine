use super::{Document, Preset, MAX_BYTES};
use orr_render::Lighting;
use serde_json::json;

fn document() -> Document {
    Document::from_lighting(Lighting {
        direction: [-0.7, -0.2, 0.4],
        color: [0.9, 0.7, 0.5],
        intensity: 2.25,
        sky: [0.3, 0.5, 0.8],
        ground: [0.2, 0.1, 0.05],
        ambient: 0.45,
        shadows: true,
        shadow_center: [120.0, 45.0, -70.0],
        shadow_radius: 220.0,
        tonemap: false,
        exposure: 1.4,
    })
    .unwrap()
}

fn bytes() -> Vec<u8> {
    Preset::from_document(&document())
        .unwrap()
        .encode()
        .unwrap()
}

#[test]
fn portable_roundtrip_carries_only_reusable_lighting_fields() {
    let source = document();
    let preset = Preset::from_document(&source).unwrap();
    let encoded = preset.encode().unwrap();
    assert_eq!(Preset::decode(&encoded).unwrap(), preset);
    let json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    let top = json.as_object().unwrap();
    assert_eq!(top.len(), 3);
    assert_eq!(top["schema"], 1);
    assert_eq!(top["identity"], "ORR_MESH_LIGHTING_PRESET_V1");
    let lighting = top["lighting"].as_object().unwrap();
    assert_eq!(lighting.len(), 9);
    for absent in [
        "scene_filename",
        "graph_revision",
        "shadow_center",
        "shadow_radius",
        "path",
        "name",
    ] {
        assert!(!top.contains_key(absent));
        assert!(!lighting.contains_key(absent));
    }
    let mut different_bounds = source.clone();
    different_bounds.shadow_center = [-5.0; 3];
    different_bounds.shadow_radius = 10.0;
    assert_eq!(
        Preset::from_document(&different_bounds)
            .unwrap()
            .encode()
            .unwrap(),
        encoded
    );
}

#[test]
fn merge_preserves_target_shadow_bounds_and_both_inputs() {
    let source = document();
    let preset = Preset::from_document(&source).unwrap();
    let target = Document::from_lighting(Lighting {
        shadow_center: [-31.0, 17.0, 500.0],
        shadow_radius: 49.5,
        ..Lighting::default()
    })
    .unwrap();
    let original_target = target.clone();
    let original_preset = preset.clone();
    let merged = preset.merge_into(&target).unwrap();
    let mut expected = source;
    expected.shadow_center = target.shadow_center;
    expected.shadow_radius = target.shadow_radius;
    assert_eq!(merged, expected);
    assert_eq!(target, original_target);
    assert_eq!(preset, original_preset);
}

#[test]
fn merge_validates_preserved_bounds_but_can_repair_replaced_draft_fields() {
    let preset = Preset::from_document(&document()).unwrap();
    let mut target = document();
    target.exposure = f32::NAN;
    assert!(preset.merge_into(&target).is_ok());
    target.shadow_radius = 0.0;
    assert!(preset.merge_into(&target).is_err());
    target.shadow_radius = 1.0;
    target.shadow_center[0] = f32::NAN;
    assert!(preset.merge_into(&target).is_err());
}

#[test]
fn all_entry_points_use_the_authoritative_document_validator() {
    let mut invalid = document();
    invalid.shadow_radius = 0.0;
    assert!(Preset::from_document(&invalid).is_err());
    let mut preset = Preset::from_document(&document()).unwrap();
    preset.direction = [0.0; 3];
    assert!(preset.encode().is_err());
    assert!(preset.merge_into(&document()).is_err());
    preset.direction = [0.0, -1.0, 0.0];
    preset.exposure = f32::INFINITY;
    assert!(preset.encode().is_err());
}

#[test]
fn strict_schema_rejects_version_identity_unknown_missing_and_wrong_types() {
    let valid: serde_json::Value = serde_json::from_slice(&bytes()).unwrap();
    for (field, value) in [
        ("schema", json!(0)),
        ("schema", json!(2)),
        ("schema", json!("1")),
        ("identity", json!("ORR_MESH_LIGHTING_V1")),
        ("extra", json!(true)),
        ("scene_filename", json!("source.scene.yaml")),
        ("graph_revision", json!([0, 0, 0])),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        assert!(
            Preset::decode(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "{field}"
        );
    }
    for field in ["schema", "identity", "lighting"] {
        let mut invalid = valid.clone();
        invalid.as_object_mut().unwrap().remove(field);
        assert!(Preset::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    for field in ["shadow_center", "shadow_radius", "extra"] {
        let mut invalid = valid.clone();
        invalid["lighting"][field] = json!(0);
        assert!(Preset::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    let mut invalid = valid.clone();
    invalid["lighting"]
        .as_object_mut()
        .unwrap()
        .remove("shadows");
    assert!(Preset::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    let mut invalid = valid;
    invalid["lighting"]["shadows"] = json!("true");
    assert!(Preset::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
}

#[test]
fn strict_json_rejects_sequences_duplicates_trailing_and_invalid_utf8() {
    let text = String::from_utf8(bytes()).unwrap();
    let valid: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut positional_inner = valid.clone();
    positional_inner["lighting"] = json!([
        [0.0, -1.0, 0.0],
        [1.0, 1.0, 1.0],
        1.0,
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        0.1,
        true,
        true,
        1.0
    ]);
    for invalid in [
        format!("[{text}]"),
        serde_json::to_string(&json!([
            1,
            "ORR_MESH_LIGHTING_PRESET_V1",
            valid["lighting"]
        ]))
        .unwrap(),
        serde_json::to_string(&positional_inner).unwrap(),
        text.replace("\"schema\": 1", "\"schema\": 1, \"schema\": 1"),
        text.replace("\"shadows\": true", "\"shadows\": true, \"shadows\": false"),
        format!("{text}{{}}"),
        text.replace("\"exposure\": 1.4", "\"exposure\": 1e100"),
        text.replace("\"exposure\": 1.4", "\"exposure\": NaN"),
        "null".to_string(),
    ] {
        assert!(
            Preset::decode(invalid.as_bytes()).is_err(),
            "accepted {invalid}"
        );
    }
    assert!(Preset::decode(&[0xff, 0xfe]).is_err());
}

#[test]
fn numeric_ranges_and_nonzero_direction_are_rejected_on_decode() {
    let valid: serde_json::Value = serde_json::from_slice(&bytes()).unwrap();
    for (field, value) in [
        ("direction", json!([0.0, 0.0, 0.0])),
        ("direction", json!([1e-13, 0.0, 0.0])),
        ("direction", json!([1000001.0, 0.0, 0.0])),
        ("color", json!([-0.1, 0.0, 0.0])),
        ("sky", json!([10001.0, 0.0, 0.0])),
        ("ground", json!([0.0, 0.0, -1.0])),
        ("intensity", json!(-0.1)),
        ("intensity", json!(10001.0)),
        ("ambient", json!(-0.1)),
        ("ambient", json!(10001.0)),
        ("exposure", json!(0.0)),
        ("exposure", json!(10001.0)),
    ] {
        let mut invalid = valid.clone();
        invalid["lighting"][field] = value;
        assert!(
            Preset::decode(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "{field}"
        );
    }
}

#[test]
fn exactly_sixteen_kib_is_accepted_but_one_more_byte_is_rejected() {
    let mut padded = bytes();
    padded.resize(MAX_BYTES, b' ');
    assert!(Preset::decode(&padded).is_ok());
    padded.push(b' ');
    assert!(Preset::decode(&padded).is_err());
    assert!(Preset::decode(&[]).is_err());
}

#[test]
fn scene_bound_sidecar_and_portable_preset_are_not_interchangeable() {
    let sidecar = document().encode("source.scene.yaml", [7; 32]).unwrap();
    assert!(Preset::decode(&sidecar).is_err());
    assert!(Document::decode(&bytes(), "source.scene.yaml", [7; 32]).is_err());
}
