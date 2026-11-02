use super::{Template, MAX_BYTES};
use crate::mesh_art::Snapshot;
use orr_model::Wrap;
use orr_package::Manifest;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const STATIC_GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");
const LICENSE: &[u8] = b"Example prop license text\n";
const PREFAB_SCENE_FILENAME: &str = "mesh-prop-prefab.scene.yaml";

fn package(root: &Path) -> PathBuf {
    let package_root = root.join("verified-static-package");
    fs::create_dir(&package_root).unwrap();
    fs::write(package_root.join("model.glb"), STATIC_GLB).unwrap();
    fs::write(package_root.join("LICENSE.txt"), LICENSE).unwrap();
    let runtime = orr_package::Runtime::content_only();
    let manifest = Manifest {
        schema: 1,
        name: "verified-static".into(),
        version: "1.0.0".into(),
        engine: format!("={}", runtime.engine_version),
        capabilities: BTreeSet::from(["models".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["LICENSE.txt".into(), "model.glb".into()]),
    };
    fs::write(
        package_root.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    package_root
}

fn snapshot(root: &Path, revision: [u8; 32]) -> Snapshot {
    let package_root = package(root);
    let snapshot =
        Snapshot::from_package(&package_root, "model.glb", "LICENSE.txt", revision).unwrap();
    let material = snapshot.material_choices()[0];
    let source_material = &snapshot.source_model_arc().source().materials[material as usize];
    let mut slot = crate::mesh_material_slots::Settings::identity(material, source_material);
    slot.tint = [0.5, 0.75, 1.0, 1.0];
    slot.linear_filter = !source_material.linear_filter;
    slot.wrap_s = Wrap::Mirror;
    slot.wrap_t = Wrap::Repeat;
    snapshot
        .with_tint([0.8, 0.9, 1.0, 1.0])
        .unwrap()
        .with_material_slots(vec![slot])
        .unwrap()
        .with_surface(Some(crate::mesh_surface::Settings {
            material,
            uv_offset: [0.25, -0.125],
            uv_scale: [1.5, 0.75],
            uv_quarter_turns: 1,
            pivot: [0.25, -0.5, 0.125],
        }))
        .unwrap()
        .with_transform([1.25, -0.5, 2.0], 0.75)
        .unwrap()
        .with_rotation(crate::mesh_rotation::from_degrees([17.0, 31.0, 53.0]).unwrap())
        .unwrap()
        .with_facing(crate::mesh_facing::Facing::Route)
        .unwrap()
}

fn wrapper_value(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap()
}

#[test]
fn prefab_round_trip_rebinds_graph_and_preserves_art_provenance_and_appearance() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let original_revision = [0x35; 32];
    let target_revision = [0xa7; 32];
    let original = snapshot(&root, original_revision);
    let original_model = original.source_model_arc().to_bytes().unwrap();
    let original_embedded = original.encode_embedded(PREFAB_SCENE_FILENAME).unwrap();
    let original_json: serde_json::Value = serde_json::from_slice(&original_embedded).unwrap();

    let template = Template::capture("Blue Gate", &original).unwrap();
    let bytes = template.encode().unwrap();
    let wire = wrapper_value(&bytes);
    assert_eq!(wire["schema"], 1);
    assert_eq!(wire["name"], "Blue Gate");
    assert_eq!(wire["graph_revision"], serde_json::json!(original_revision));
    assert_eq!(
        wire["art_json"].as_str().unwrap(),
        String::from_utf8(original_embedded.clone()).unwrap()
    );

    let decoded = Template::decode(&bytes, target_revision).unwrap();
    assert_eq!(decoded.name(), "Blue Gate");
    assert_eq!(decoded.art().graph_revision(), target_revision);
    assert_ne!(decoded.art().graph_revision(), original_revision);
    assert_eq!(decoded.art().offset(), original.offset());
    assert_eq!(decoded.art().scale(), original.scale());
    assert_eq!(decoded.art().rotation(), original.rotation());
    assert_eq!(decoded.art().tint(), original.tint());
    assert_eq!(decoded.art().facing(), original.facing());
    assert_eq!(decoded.art().surface(), original.surface());
    assert_eq!(decoded.art().material_slots(), original.material_slots());
    assert_eq!(
        decoded.art().source_model_arc().to_bytes().unwrap(),
        original_model
    );

    let rebound_embedded = decoded
        .art()
        .encode_embedded(PREFAB_SCENE_FILENAME)
        .unwrap();
    let rebound_json: serde_json::Value = serde_json::from_slice(&rebound_embedded).unwrap();
    let mut expected_json = original_json;
    expected_json["graph_revision"] = serde_json::json!(target_revision);
    assert_eq!(rebound_json, expected_json);

    let reencoded = decoded.encode().unwrap();
    assert_eq!(
        Template::decode(&reencoded, target_revision)
            .unwrap()
            .name(),
        "Blue Gate"
    );
}

#[test]
fn prefab_wrapper_rejects_invalid_names_unknown_duplicate_null_and_versions() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let art = snapshot(&root, [0x42; 32]);
    for invalid_name in ["", "  ", " leading", "trailing ", "bad\nname"] {
        assert!(Template::capture(invalid_name, &art).is_err());
    }
    assert!(Template::capture(&"x".repeat(65), &art).is_err());

    let bytes = Template::capture("Valid", &art).unwrap().encode().unwrap();
    let mut unknown = wrapper_value(&bytes);
    unknown["extra"] = serde_json::json!(true);
    assert!(Template::decode(&serde_json::to_vec(&unknown).unwrap(), [0x99; 32]).is_err());

    let mut null_name = wrapper_value(&bytes);
    null_name["name"] = serde_json::Value::Null;
    assert!(Template::decode(&serde_json::to_vec(&null_name).unwrap(), [0x99; 32]).is_err());

    let mut wrong_schema = wrapper_value(&bytes);
    wrong_schema["schema"] = serde_json::json!(2);
    assert!(Template::decode(&serde_json::to_vec(&wrong_schema).unwrap(), [0x99; 32]).is_err());

    let duplicate = format!(
        "{{\"schema\":1,\"schema\":1,\"name\":\"Valid\",\"graph_revision\":{},\"art_json\":\"{{}}\"}}",
        serde_json::to_string(&[0x42; 32]).unwrap()
    );
    assert!(Template::decode(duplicate.as_bytes(), [0x99; 32]).is_err());
}

#[test]
fn prefab_decode_rejects_original_graph_mismatch_and_nested_malformed_art() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let art = snapshot(&root, [0x24; 32]);
    let bytes = Template::capture("Valid", &art).unwrap().encode().unwrap();
    let mut mismatched_revision = wrapper_value(&bytes);
    mismatched_revision["graph_revision"] = serde_json::to_value([0x52u8; 32]).unwrap();
    assert!(Template::decode(
        &serde_json::to_vec(&mismatched_revision).unwrap(),
        [0x99; 32]
    )
    .is_err());

    let mut malformed_art = wrapper_value(&bytes);
    malformed_art["art_json"] = serde_json::json!(
        r#"{"schema":1,"scene_filename":"mesh-prop-prefab.scene.yaml","scene_filename":"mesh-prop-prefab.scene.yaml"}"#
    );
    assert!(Template::decode(&serde_json::to_vec(&malformed_art).unwrap(), [0x99; 32]).is_err());

    let mut null_art = wrapper_value(&bytes);
    null_art["art_json"] = serde_json::Value::Null;
    assert!(Template::decode(&serde_json::to_vec(&null_art).unwrap(), [0x99; 32]).is_err());
}

#[test]
fn prefab_input_is_bounded_before_json_parsing() {
    assert!(Template::decode(&vec![b' '; MAX_BYTES + 1], [0x71; 32]).is_err());
}

#[test]
fn absent_shading_retains_exact_legacy_wire_bytes_and_present_field_is_strict() {
    #[derive(serde::Serialize)]
    struct LegacyWire {
        schema: u32,
        name: String,
        graph_revision: [u8; 32],
        art_json: String,
    }
    let root = tempfile::tempdir().unwrap();
    let revision = [0x35; 32];
    let art = snapshot(root.path(), revision);
    let expected = serde_json::to_vec(&LegacyWire {
        schema: 1,
        name: "Legacy".into(),
        graph_revision: revision,
        art_json: String::from_utf8(art.encode_embedded(PREFAB_SCENE_FILENAME).unwrap()).unwrap(),
    })
    .unwrap();
    let captured = Template::capture("Legacy", &art).unwrap();
    assert_eq!(captured.encode().unwrap(), expected);
    assert_eq!(
        Template::decode(&expected, revision)
            .unwrap()
            .encode()
            .unwrap(),
        expected
    );
    let mut value = wrapper_value(&expected);
    value["shading"] = serde_json::Value::Null;
    assert!(Template::decode(&serde_json::to_vec(&value).unwrap(), revision).is_err());
    #[cfg(not(feature = "mesh-prop-shading"))]
    {
        value["shading"] = serde_json::json!({"version":1,"slots":[{"material":0,"metallic":0.5,"roughness":0.5}]});
        assert!(Template::decode(&serde_json::to_vec(&value).unwrap(), revision).is_err());
    }
}

#[cfg(feature = "mesh-prop-shading")]
#[test]
fn shaded_prefab_roundtrip_and_graph_rebind_preserve_art_and_explicit_clear_is_local() {
    use crate::mesh_prop_shading::{Settings, Slot};
    let root = tempfile::tempdir().unwrap();
    let revision = [0x35; 32];
    let target = [0xa7; 32];
    let art = snapshot(root.path(), revision);
    let model = art.source_model_arc();
    let material = model.source().primitives[0].material;
    let shading = Settings::new(
        vec![Slot {
            material,
            metallic: 0.7,
            roughness: 0.2,
        }],
        model,
    )
    .unwrap();
    let legacy = Template::capture("Shaded", &art).unwrap();
    let legacy_bytes = legacy.encode().unwrap();
    let shaded = legacy.clone().with_shading(Some(shading.clone())).unwrap();
    let bytes = shaded.encode().unwrap();
    let reopened = Template::decode(&bytes, target).unwrap();
    assert_eq!(reopened.shading(), Some(&shading));
    assert_eq!(reopened.art().graph_revision(), target);
    assert_eq!(
        reopened.art().source_model_arc().to_bytes().unwrap(),
        model.to_bytes().unwrap()
    );
    let rebound = art
        .clone()
        .with_graph_revision(target)
        .encode_embedded(PREFAB_SCENE_FILENAME)
        .unwrap();
    assert_eq!(
        reopened
            .art()
            .encode_embedded(PREFAB_SCENE_FILENAME)
            .unwrap(),
        rebound
    );
    assert_eq!(
        Template::decode(&bytes, revision)
            .unwrap()
            .encode()
            .unwrap(),
        bytes
    );
    assert_eq!(
        shaded.clone().with_shading(None).unwrap().encode().unwrap(),
        legacy_bytes
    );
    assert_eq!(shaded.shading(), Some(&shading));
    assert!(legacy.shading().is_none());
}

#[cfg(feature = "mesh-prop-shading")]
#[test]
fn shaded_prefab_rejects_invalid_records_and_foreign_material_slots() {
    use serde_json::json;
    let root = tempfile::tempdir().unwrap();
    let revision = [0x35; 32];
    let art = snapshot(root.path(), revision);
    let baseline = Template::capture("Strict", &art).unwrap().encode().unwrap();
    for shading in [
        json!(null),
        json!([]),
        json!({"version":1,"slots":[]}),
        json!({"version":2,"slots":[{"material":0,"metallic":0,"roughness":1}]}),
        json!({"version":1,"slots":[{"material":999,"metallic":0,"roughness":1}]}),
        json!({"version":1,"slots":[{"material":0,"metallic":-1,"roughness":1}]}),
        json!({"version":1,"slots":[{"material":0,"metallic":0,"roughness":2}]}),
        json!({"version":1,"slots":[{"material":0,"metallic":null,"roughness":1}]}),
        json!({"version":1,"slots":[{"material":0,"metallic":0,"roughness":1}],"extra":true}),
    ] {
        let mut value = wrapper_value(&baseline);
        value["shading"] = shading;
        assert!(Template::decode(&serde_json::to_vec(&value).unwrap(), revision).is_err());
    }
    for slots in [
        vec![json!({"material":0,"metallic":0,"roughness":1}); 2],
        (0..257)
            .map(|material| json!({"material":material,"metallic":0,"roughness":1}))
            .collect(),
    ] {
        let mut value = wrapper_value(&baseline);
        value["shading"] = json!({"version":1,"slots":slots});
        assert!(Template::decode(&serde_json::to_vec(&value).unwrap(), revision).is_err());
    }
    let mut value = wrapper_value(&baseline);
    value["shading"] = json!({"version":1,"slots":[{"material":0,"metallic":0.5,"roughness":0.5}]});
    let raw = serde_json::to_string(&value).unwrap();
    assert!(Template::decode(
        raw.replace("\"version\":1", "\"version\":1,\"version\":1")
            .as_bytes(),
        revision
    )
    .is_err());
    assert!(Template::decode(
        raw.replace("\"shading\":", "\"shading\":null,\"shading\":")
            .as_bytes(),
        revision
    )
    .is_err());
    let foreign: crate::mesh_prop_shading::Settings = serde_json::from_value(
        json!({"version":1,"slots":[{"material":999,"metallic":0,"roughness":1}]}),
    )
    .unwrap();
    assert!(Template::capture("Strict", &art)
        .unwrap()
        .with_shading(Some(foreign))
        .is_err());
    let oversized = vec![b' '; MAX_BYTES + 1];
    assert!(Template::decode(&oversized, revision).is_err());
}
