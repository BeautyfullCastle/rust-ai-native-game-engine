use super::Document;
use orr_render::{Lighting, PostProcessSettings};
fn legacy() -> Document {
    Document::from_lighting(Lighting::default()).unwrap()
}
fn enabled() -> PostProcessSettings {
    PostProcessSettings {
        enabled: true,
        ..Default::default()
    }
}
#[test]
fn legacy_bytes_and_schema_two_roundtrip() {
    let document = legacy();
    let old_bytes = document.encode("level.scene.yaml", [7; 32]).unwrap();
    assert!(!String::from_utf8_lossy(&old_bytes).contains("post_process"));
    let decoded = Document::decode(&old_bytes, "level.scene.yaml", [7; 32]).unwrap();
    assert!(!decoded.has_post_process());
    assert!(!decoded.post_process_settings().enabled);
    assert_eq!(
        decoded.encode("level.scene.yaml", [7; 32]).unwrap(),
        old_bytes
    );
    let new = document.with_post_process(enabled()).unwrap();
    let bytes = new.encode("level.scene.yaml", [7; 32]).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["schema"],
        2
    );
    assert_eq!(
        Document::decode(&bytes, "level.scene.yaml", [7; 32]).unwrap(),
        new
    );
}
#[test]
fn strict_objects_schema_and_fields() {
    let doc = legacy().with_post_process(enabled()).unwrap();
    let bytes = doc.encode("level.scene.yaml", [7; 32]).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for value in [
        serde_json::Value::Null,
        serde_json::json!([]),
        serde_json::json!({"enabled":true}),
    ] {
        let mut bad = json.clone();
        bad["lighting"]["post_process"] = value;
        assert!(Document::decode(
            &serde_json::to_vec(&bad).unwrap(),
            "level.scene.yaml",
            [7; 32]
        )
        .is_err());
    }
    let text = String::from_utf8(bytes).unwrap();
    for bad in [
        text.replace("\"schema\": 2", "\"schema\": 1"),
        text.replace("\"enabled\": true", "\"enabled\": true, \"enabled\": false"),
        text.replace("\"enabled\": true", "\"enabled\": true, \"unknown\": 0"),
    ] {
        assert!(Document::decode(bad.as_bytes(), "level.scene.yaml", [7; 32]).is_err());
    }
}
#[test]
fn disabled_settings_still_enforce_finite_and_renderer_bounds() {
    let defaults = PostProcessSettings::default();
    for settings in [
        PostProcessSettings {
            threshold: f32::NAN,
            ..defaults
        },
        PostProcessSettings {
            threshold: f32::INFINITY,
            ..defaults
        },
        PostProcessSettings {
            threshold: 65505.0,
            ..defaults
        },
        PostProcessSettings {
            strength: 8.1,
            ..defaults
        },
        PostProcessSettings {
            radius: 0,
            ..defaults
        },
        PostProcessSettings {
            radius: 9,
            ..defaults
        },
        PostProcessSettings {
            iterations: 0,
            ..defaults
        },
        PostProcessSettings {
            iterations: 5,
            ..defaults
        },
    ] {
        assert!(legacy().with_post_process(settings).is_err());
    }
}
#[test]
fn hdr_store_roundtrip_and_stale_save_preserve_external_bytes() {
    use crate::mesh_lighting_store::{load, save, sidecar_path};
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("level.scene.yaml");
    std::fs::write(&scene, b"scene").unwrap();
    let doc = legacy().with_post_process(enabled()).unwrap();
    let stamp = save(&scene, Some(&doc), None, [7; 32]).unwrap().unwrap();
    assert_eq!(load(&scene, [7; 32]).unwrap().unwrap().0, doc);
    let path = sidecar_path(&scene).unwrap();
    std::fs::write(&path, b"external").unwrap();
    assert!(save(&scene, Some(&doc), Some(&stamp), [7; 32]).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"external");
}
