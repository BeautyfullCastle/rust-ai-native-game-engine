//! Import-time UV baking only: texture transforms are not animation channels.
#![cfg(all(feature = "animation", feature = "import"))]
#![allow(clippy::float_arithmetic)]
use base64::Engine;
use orr_model::{animation::AnimatedModel, animation_import::import_with_resolver, Error};
use serde_json::{json, Value};
const EXT: &str = "KHR_texture_transform";
fn source() -> Value {
    serde_json::from_slice(include_bytes!("fixtures/animated_strip.gltf")).unwrap()
}
fn transformed(transform: Value) -> Value {
    let mut doc = source();
    doc["extensionsUsed"] = json!([EXT]);
    doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] =
        json!({EXT:transform});
    doc
}
fn import(doc: &Value) -> Result<AnimatedModel, Error> {
    raw(&serde_json::to_vec(doc).unwrap())
}
fn raw(bytes: &[u8]) -> Result<AnimatedModel, Error> {
    import_with_resolver("animated_strip.gltf", bytes, |_| {
        panic!("embedded fixture must not resolve")
    })
}
fn write_uv(doc: &mut Value, value: f32) {
    let uri = doc["buffers"][0]["uri"].as_str().unwrap();
    let mut bytes = base64::engine::general_purpose::STANDARD
        .decode(uri.split_once(',').unwrap().1)
        .unwrap();
    let accessor = &doc["accessors"][2];
    let view = accessor["bufferView"].as_u64().unwrap() as usize;
    let offset = doc["bufferViews"][view]["byteOffset"].as_u64().unwrap() as usize
        + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    doc["buffers"][0]["uri"] = json!(format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ));
}
fn oracle(uv: [f32; 2], offset: [f32; 2], scale: [f32; 2], rotation: f32) -> [f64; 2] {
    let x = f64::from(uv[0]) * f64::from(scale[0]);
    let y = f64::from(uv[1]) * f64::from(scale[1]);
    let r = f64::from(rotation);
    [
        f64::from(offset[0]) + r.cos() * x - r.sin() * y,
        f64::from(offset[1]) + r.sin() * x + r.cos() * y,
    ]
}
fn same_animation(a: &AnimatedModel, b: &AnimatedModel) {
    assert_eq!(a.source().nodes, b.source().nodes);
    assert_eq!(a.source().skins, b.source().skins);
    assert_eq!(a.source().clips, b.source().clips);
    assert_eq!(a.source().materials, b.source().materials);
    assert_eq!(a.source().images, b.source().images);
    assert_eq!(a.source().primitives.len(), b.source().primitives.len());
    for (a, b) in a.source().primitives.iter().zip(&b.source().primitives) {
        let mut restored = b.clone();
        for (a, b) in a.vertices.iter().zip(&mut restored.vertices) {
            b.vertex.uv = a.vertex.uv;
        }
        assert_eq!(a, &restored); // all non-UV primitive/vertex fields, including influences
    }
    for clip in 0..a.source().clips.len() as u32 {
        for time in [0.0, 0.25, 0.75, 1.0, 1.75, 2.0] {
            let pa = a.sample_clip(clip, time).unwrap();
            let pb = b.sample_clip(clip, time).unwrap();
            assert_eq!(pa.local(), pb.local());
            assert_eq!(pa.global(), pb.global());
            assert_eq!(pa.skin_matrices(), pb.skin_matrices());
            let da = a.deform(&pa).unwrap();
            let db = b.deform(&pb).unwrap();
            for (a, b) in da.iter().zip(&db) {
                for (a, b) in a.vertices.iter().zip(&b.vertices) {
                    assert_eq!(a.position, b.position);
                    assert_eq!(a.normal, b.normal);
                }
            }
        }
    }
}
#[test]
fn animated_uv_bake_matches_independent_trs_and_preserves_animation_after_cooking() {
    let baseline = import(&source()).unwrap();
    let offset = [0.25f32, -0.5];
    let scale = [-2.0f32, 0.75];
    let rotation = 0.63f32;
    let doc = transformed(json!({"offset":offset,"scale":scale,"rotation":rotation}));
    let result = import(&doc).unwrap();
    same_animation(&baseline, &result);
    for (a, b) in baseline.source().primitives[0]
        .vertices
        .iter()
        .zip(&result.source().primitives[0].vertices)
    {
        let expected = oracle(a.vertex.uv, offset, scale, rotation);
        for (actual, expected) in b.vertex.uv.into_iter().zip(expected) {
            assert!(
                (f64::from(actual) - expected).abs() < 2e-6,
                "{actual} != {expected}"
            );
        }
    }
    let cooked = result.to_bytes().unwrap();
    let reload = AnimatedModel::from_bytes(&cooked).unwrap();
    assert_eq!(reload.to_bytes().unwrap(), cooked);
    assert_eq!(reload.source().primitives, result.source().primitives);
    same_animation(&baseline, &reload);
}
#[test]
fn identity_preserves_signed_zero_and_effective_uv_override_is_respected() {
    let mut original = source();
    write_uv(&mut original, -0.0);
    let baseline = import(&original).unwrap();
    for transform in [
        json!({}),
        json!({"offset":[0,0],"scale":[1,1],"rotation":0,"texCoord":0}),
    ] {
        let mut doc = transformed(transform);
        write_uv(&mut doc, -0.0);
        let result = import(&doc).unwrap();
        assert_eq!(baseline.source().primitives, result.source().primitives);
        assert_eq!(
            result.source().primitives[0].vertices[0].vertex.uv[0].to_bits(),
            (-0.0f32).to_bits()
        );
    }
    let mut doc = transformed(json!({"texCoord":0}));
    doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"] = json!(1);
    assert!(import(&doc).is_ok());
    doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"][EXT]
        ["texCoord"] = json!(1);
    assert!(import(&doc).is_err());
}
#[test]
fn shared_accessor_receives_each_material_transform_exactly_once() {
    let mut doc = transformed(json!({"offset":[0.25,0]}));
    let mut material = doc["materials"][0].clone();
    material["pbrMetallicRoughness"]["baseColorTexture"]["extensions"][EXT] =
        json!({"scale":[2,3]});
    doc["materials"].as_array_mut().unwrap().push(material);
    let mut primitive = doc["meshes"][0]["primitives"][0].clone();
    primitive["material"] = json!(1);
    doc["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(primitive);
    let original = import(&source()).unwrap();
    let result = import(&doc).unwrap();
    assert_eq!(result.source().primitives.len(), 2);
    for (i, v) in original.source().primitives[0].vertices.iter().enumerate() {
        assert_eq!(
            result.source().primitives[0].vertices[i].vertex.uv,
            [v.vertex.uv[0] + 0.25, v.vertex.uv[1]]
        );
        assert_eq!(
            result.source().primitives[1].vertices[i].vertex.uv,
            [v.vertex.uv[0] * 2.0, v.vertex.uv[1] * 3.0]
        );
    }
}
#[test]
fn unused_mesh_and_nonselected_node_cannot_hide_transformed_overflow() {
    for with_node in [false, true] {
        let mut doc = source();
        doc["extensionsUsed"] = json!([EXT]);
        let mut bad_material = doc["materials"][0].clone();
        bad_material["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] =
            json!({EXT:{"scale":[100000,1]}});
        doc["materials"].as_array_mut().unwrap().push(bad_material);
        let mut mesh = doc["meshes"][0].clone();
        mesh["primitives"][0]["material"] = json!(1);
        doc["meshes"].as_array_mut().unwrap().push(mesh);
        if with_node {
            doc["nodes"]
                .as_array_mut()
                .unwrap()
                .push(json!({"mesh":1,"skin":0}));
        }
        let error = import(&doc).unwrap_err().to_string();
        assert!(error.contains("UV"), "{error}");
    }
    for value in [70000.0, f32::NAN, f32::INFINITY] {
        let mut doc = transformed(json!({"scale":[0,0]}));
        write_uv(&mut doc, value);
        assert!(import(&doc).is_err());
    }
}
#[test]
fn strict_extension_contract_rejects_malformed_and_inconsistent_declarations() {
    for value in [
        Value::Null,
        json!([]),
        json!({"unknown":1}),
        json!({"offset":null}),
        json!({"scale":[1]}),
        json!({"texCoord":null}),
    ] {
        assert!(import(&transformed(value)).is_err());
    }
    for key in ["extensionsUsed", "extensionsRequired"] {
        let mut doc = transformed(json!({}));
        doc[key] = json!([EXT, EXT]);
        assert!(import(&doc).is_err());
    }
    let mut doc = transformed(json!({}));
    doc.as_object_mut().unwrap().remove("extensionsUsed");
    assert!(import(&doc).is_err());
    doc["extensionsRequired"] = json!([EXT]);
    assert!(import(&doc).is_err());
    let mut doc = transformed(json!({}));
    doc["extensionsUsed"] = json!([EXT, "UNKNOWN"]);
    assert!(import(&doc).is_err());
    // Preserve duplicate keys in raw input instead of losing them in Value.
    let encoded = serde_json::to_string(&transformed(json!({"rotation":0.5}))).unwrap();
    assert!(encoded.contains("\"rotation\":0.5"));
    assert!(raw(encoded
        .replace("\"rotation\":0.5", "\"rotation\":0.5,\"rotation\":0.7")
        .as_bytes())
    .is_err());
}

#[test]
fn invalid_extension_metadata_refuses_before_external_resources() {
    let mut malformed = Vec::new();
    for extensions in [
        Value::Null,
        json!([]),
        json!({"UNKNOWN":{}}),
        json!({EXT:null}),
        json!({EXT:[]}),
    ] {
        let mut doc = transformed(json!({}));
        doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] = extensions;
        malformed.push(doc);
    }
    let mut undeclared = transformed(json!({}));
    undeclared.as_object_mut().unwrap().remove("extensionsUsed");
    malformed.push(undeclared);
    let mut missing_used = source();
    missing_used["extensionsRequired"] = json!([EXT]);
    malformed.push(missing_used);
    for mut doc in malformed {
        doc["buffers"][0]["uri"] = json!("external.bin");
        assert!(import_with_resolver(
            "animated_strip.gltf",
            &serde_json::to_vec(&doc).unwrap(),
            |_| { panic!("invalid extension metadata reached external resolver") }
        )
        .is_err());
    }
}
