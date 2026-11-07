//! Strict optional light object. Missing preserves legacy bytes; explicit null rejects.
use orr_render::PointLight;
use serde::Deserializer;
pub(super) fn deserialize_present<'de, D: Deserializer<'de>>(
    decoder: D,
) -> Result<Option<PointLight>, D::Error> {
    let light: PointLight = super::deserialize_object(decoder)?;
    orr_render::PointLightSettings {
        point_light: Some(light),
    }
    .validate()
    .map_err(serde::de::Error::custom)?;
    Ok(Some(light))
}
#[cfg(test)]
mod tests {
    use super::super::Document;
    fn base() -> Document {
        Document::from_lighting(orr_render::Lighting::default()).unwrap()
    }
    fn light() -> orr_render::PointLight {
        orr_render::PointLight {
            position: [1.0, 4.0, -2.0],
            color: [1.0, 0.8, 0.5],
            intensity: 3.0,
            range: 12.0,
        }
    }
    #[test]
    fn optional_point_light_roundtrips_schema_three_alone_and_with_effects() {
        for effects in [false, true] {
            let mut d = base();
            d.point_light = Some(light());
            if effects {
                d = d
                    .with_post_process(orr_render::PostProcessSettings::default())
                    .unwrap();
            }
            let bytes = d.encode("level.scene.yaml", [7; 32]).unwrap();
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(v["schema"], 3);
            assert_eq!(
                Document::decode(&bytes, "level.scene.yaml", [7; 32]).unwrap(),
                d
            );
            assert_eq!(d.point_light_settings().unwrap().point_light, Some(light()));
        }
    }
    #[test]
    fn absent_point_light_keeps_legacy_one_two_and_serialized_document_bytes() {
        for effects in [false, true] {
            let mut d = base();
            if effects {
                d = d
                    .with_post_process(orr_render::PostProcessSettings::default())
                    .unwrap();
            }
            let bytes = d.encode("level.scene.yaml", [7; 32]).unwrap();
            let text = String::from_utf8(bytes.clone()).unwrap();
            assert!(!text.contains("point_light"));
            assert_eq!(
                Document::decode(&bytes, "level.scene.yaml", [7; 32])
                    .unwrap()
                    .encode("level.scene.yaml", [7; 32])
                    .unwrap(),
                bytes
            );
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(v["schema"], if effects { 2 } else { 1 });
            assert_eq!(
                d.point_light_settings().unwrap(),
                orr_render::PointLightSettings::default()
            );
        }
    }
    #[test]
    fn point_light_strict_shapes_duplicates_unknown_and_schema_mismatch_reject() {
        let mut d = base();
        d.point_light = Some(light());
        let bytes = d.encode("level.scene.yaml", [7; 32]).unwrap();
        let original: String = String::from_utf8(bytes).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&original).unwrap();
        for bad in [
            serde_json::Value::Null,
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!({"position":[1,4,-2],"color":[1,0.8,0.5],"intensity":3,"range":12,"extra":true}),
        ] {
            value["lighting"]["point_light"] = bad;
            assert!(Document::decode(
                &serde_json::to_vec(&value).unwrap(),
                "level.scene.yaml",
                [7; 32]
            )
            .is_err());
        }
        for bad in [
            original.replace("\"range\": 12.0", "\"range\": 12.0, \"range\": 4.0"),
            original.replace(
                "\"point_light\": {",
                "\"point_light\": {}, \"point_light\": {",
            ),
            original.replace("\"schema\": 3", "\"schema\": 1"),
            original.replace("\"schema\": 3", "\"schema\": 2"),
        ] {
            assert!(Document::decode(bad.as_bytes(), "level.scene.yaml", [7; 32]).is_err());
        }
        let mut v: serde_json::Value = serde_json::from_str(&original).unwrap();
        v["lighting"].as_object_mut().unwrap().remove("point_light");
        assert!(Document::decode(
            &serde_json::to_vec(&v).unwrap(),
            "level.scene.yaml",
            [7; 32]
        )
        .is_err());
    }
    #[test]
    fn existing_renderer_bounds_are_reused_and_other_lighting_is_preserved() {
        let mut d = base();
        let before = d.lighting().unwrap();
        d.point_light = Some(light());
        assert_eq!(d.lighting().unwrap(), before);
        for bad in [f32::NAN, f32::INFINITY, -1.0, 10001.0] {
            let mut l = light();
            l.intensity = bad;
            d.point_light = Some(l);
            assert!(d.encode("level.scene.yaml", [7; 32]).is_err());
        }
        for bad in [0.0, 1e-7, 1e10, f32::NAN] {
            let mut l = light();
            l.range = bad;
            d.point_light = Some(l);
            assert!(d.point_light_settings().is_err());
        }
        let l = orr_render::PointLight {
            position: [1e9, -1e9, 0.0],
            color: [1e4, 0.0, 0.0],
            intensity: 1e4,
            range: 1e-6,
        };
        d.point_light = Some(l);
        d.validate().unwrap();
    }
    #[test]
    fn presets_preserve_target_point_light_without_gaining_a_point_light_schema() {
        let mut target = base();
        target.point_light = Some(light());
        for effects in [false, true] {
            let preset = if effects {
                crate::mesh_lighting_preset::Preset::from_document_with_post_process(&base())
                    .unwrap()
            } else {
                crate::mesh_lighting_preset::Preset::from_document(&base()).unwrap()
            };
            let bytes = preset.encode().unwrap();
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(v["schema"], if effects { 2 } else { 1 });
            assert!(!String::from_utf8(bytes).unwrap().contains("point_light"));
            let merged = preset.merge_into(&target).unwrap();
            assert_eq!(merged.point_light, target.point_light);
            let encoded = merged.encode("level.scene.yaml", [7; 32]).unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&encoded).unwrap()["schema"],
                3
            );
        }
    }
    #[test]
    fn pre_point_light_schema_one_and_two_match_independent_pinned_bytes() {
        // Fixed pre-change wire order/pretty formatting, not encoded by the code under test.
        const LEGACY_ONE: &str = r#"{
  "schema": 1,
  "identity": "ORR_MESH_LIGHTING_V1",
  "scene_filename": "legacy.scene.yaml",
  "graph_revision": [
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0
  ],
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
    "intensity": 2.0,
    "sky": [
      0.5,
      0.5,
      0.5
    ],
    "ground": [
      0.25,
      0.25,
      0.25
    ],
    "ambient": 1.0,
    "shadows": false,
    "shadow_center": [
      0.0,
      0.0,
      0.0
    ],
    "shadow_radius": 8.0,
    "tonemap": false,
    "exposure": 1.0
  }
}
"#;
        const LEGACY_TWO: &str = r#"{
  "schema": 2,
  "identity": "ORR_MESH_LIGHTING_V1",
  "scene_filename": "legacy.scene.yaml",
  "graph_revision": [
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0
  ],
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
    "intensity": 2.0,
    "sky": [
      0.5,
      0.5,
      0.5
    ],
    "ground": [
      0.25,
      0.25,
      0.25
    ],
    "ambient": 1.0,
    "shadows": false,
    "shadow_center": [
      0.0,
      0.0,
      0.0
    ],
    "shadow_radius": 8.0,
    "tonemap": false,
    "exposure": 1.0,
    "post_process": {
      "enabled": true,
      "bloom": true,
      "threshold": 1.0,
      "strength": 0.5,
      "radius": 2,
      "iterations": 1
    }
  }
}
"#;
        let lighting = orr_render::Lighting {
            direction: [0.0, -1.0, 0.0],
            color: [1.0; 3],
            intensity: 2.0,
            sky: [0.5; 3],
            ground: [0.25; 3],
            ambient: 1.0,
            shadows: false,
            shadow_center: [0.0; 3],
            shadow_radius: 8.0,
            tonemap: false,
            exposure: 1.0,
        };
        let document = Document::from_lighting(lighting).unwrap();
        assert_eq!(
            document.encode("legacy.scene.yaml", [0; 32]).unwrap(),
            LEGACY_ONE.as_bytes()
        );
        let effects = orr_render::PostProcessSettings {
            enabled: true,
            bloom: true,
            threshold: 1.0,
            strength: 0.5,
            radius: 2,
            iterations: 1,
        };
        let document = document.with_post_process(effects).unwrap();
        assert_eq!(
            document.encode("legacy.scene.yaml", [0; 32]).unwrap(),
            LEGACY_TWO.as_bytes()
        );
    }
}
