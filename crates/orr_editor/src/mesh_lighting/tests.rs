use super::{Document, MAX_BYTES};
use orr_render::Lighting;

fn revision() -> [u8; 32] {
    [0x5a; 32]
}

#[test]
fn renderer_default_roundtrips_without_changing_any_fields() {
    let default = Lighting::default();
    let document = Document::from_lighting(default).unwrap();
    assert_eq!(document.lighting().unwrap(), default);
    let bytes = document.encode("level.scene.yaml", revision()).unwrap();
    assert_eq!(
        Document::decode(&bytes, "level.scene.yaml", revision())
            .unwrap()
            .lighting()
            .unwrap(),
        default
    );
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["identity"], "ORR_MESH_LIGHTING_V1");
    assert_eq!(json["lighting"]["tonemap"], true);
}

#[test]
fn scene_and_graph_identity_are_bound() {
    let document = Document::from_lighting(Lighting::default()).unwrap();
    let bytes = document.encode("level.scene.yaml", revision()).unwrap();
    assert!(Document::decode(&bytes, "other.scene.yaml", revision()).is_err());
    assert!(Document::decode(&bytes, "level.scene.yaml", [0x5b; 32]).is_err());
    assert!(Document::decode(&bytes, "../level.scene.yaml", revision()).is_err());
}

#[test]
fn strict_json_rejects_nonobjects_unknown_duplicate_and_trailing_data() {
    let document = Document::from_lighting(Lighting::default()).unwrap();
    let bytes = document.encode("level.scene.yaml", revision()).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    for invalid in [
        format!("[{text}]"),
        text.replace("\"schema\": 1", "\"schema\": 1, \"extra\": 0"),
        text.replace("\"schema\": 1", "\"schema\": 1, \"schema\": 1"),
        text.replace("\"tonemap\": true", "\"tonemap\": true, \"tonemap\": false"),
        format!("{text}{{}}"),
    ] {
        assert!(Document::decode(invalid.as_bytes(), "level.scene.yaml", revision()).is_err());
    }
    assert!(Document::decode(&vec![b' '; MAX_BYTES + 1], "level.scene.yaml", revision()).is_err());
}

#[test]
fn renderer_ranges_and_nonzero_direction_are_enforced() {
    for invalid in [
        Lighting {
            direction: [0.0; 3],
            ..Lighting::default()
        },
        Lighting {
            direction: [1e-13, 0.0, 0.0],
            ..Lighting::default()
        },
        Lighting {
            direction: [f32::NAN, 1.0, 0.0],
            ..Lighting::default()
        },
        Lighting {
            color: [0.0, 10_001.0, 0.0],
            ..Lighting::default()
        },
        Lighting {
            intensity: f32::INFINITY,
            ..Lighting::default()
        },
        Lighting {
            ambient: -0.1,
            ..Lighting::default()
        },
        Lighting {
            shadow_center: [0.0, 1_000_001.0, 0.0],
            ..Lighting::default()
        },
        Lighting {
            shadow_radius: 0.49,
            ..Lighting::default()
        },
        Lighting {
            exposure: 10_001.0,
            ..Lighting::default()
        },
        Lighting {
            exposure: 0.0,
            ..Lighting::default()
        },
    ] {
        assert!(Document::from_lighting(invalid).is_err());
    }
}
