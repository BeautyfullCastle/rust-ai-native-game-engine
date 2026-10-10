//! Static authoring contract: transform UV0 once, before immutable cooking.
#![cfg(feature = "import")]
#![allow(clippy::float_arithmetic)]
use orr_model::{StaticModel, import::import_with_resolver};
use serde_json::{Value, json};

const NAME: &str = "KHR_texture_transform";
fn source() -> Value {
    serde_json::from_slice(include_bytes!("fixtures/model.gltf")).unwrap()
}
fn authored(transform: Value) -> Value {
    let mut doc = source();
    doc["extensionsUsed"] = json!([NAME]);
    doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] =
        json!({NAME: transform});
    doc
}
fn resource(uri: &str, buffer: &[u8]) -> Result<Vec<u8>, orr_model::Error> {
    Ok(match uri {
        "model.bin" => buffer.to_vec(),
        "quadrants.png" => include_bytes!("fixtures/quadrants.png").to_vec(),
        "marker.png" => include_bytes!("fixtures/marker.png").to_vec(),
        _ => panic!("unexpected resource {uri}"),
    })
}
fn import_buffer(doc: &Value, buffer: &[u8]) -> Result<StaticModel, orr_model::Error> {
    import_with_resolver("model.gltf", &serde_json::to_vec(doc).unwrap(), |uri| {
        resource(uri, buffer)
    })
}
fn import(doc: &Value) -> Result<StaticModel, orr_model::Error> {
    import_buffer(doc, include_bytes!("fixtures/model.bin"))
}
fn rejects_before_resolution(doc: &Value) {
    let error = import_with_resolver("model.gltf", &serde_json::to_vec(doc).unwrap(), |_| {
        panic!("invalid declaration resolved resources")
    });
    assert!(error.is_err(), "accepted {doc}");
}

#[test]
fn omitted_and_explicit_identity_preserve_exact_vertex_bits_and_cooked_reload() {
    let mut buffer = include_bytes!("fixtures/model.bin").to_vec();
    buffer[24..28].copy_from_slice(&(-0.0f32).to_le_bytes());
    let baseline = import_buffer(&source(), &buffer).unwrap();
    for transform in [
        json!({}),
        json!({"offset":[0,0],"rotation":0,"scale":[1,1],"texCoord":0}),
    ] {
        let transformed = import_buffer(&authored(transform), &buffer).unwrap();
        assert_eq!(
            baseline.source().primitives,
            transformed.source().primitives
        );
        assert_eq!(baseline.source().materials, transformed.source().materials);
        assert_eq!(baseline.source().images, transformed.source().images);
        assert_eq!(
            transformed.source().primitives[0].vertices[0].uv[0].to_bits(),
            (-0.0f32).to_bits()
        );
        let cooked = transformed.to_bytes().unwrap();
        assert_eq!(
            cooked,
            StaticModel::from_bytes(&cooked)
                .unwrap()
                .to_bytes()
                .unwrap()
        );
    }
}

#[test]
fn affine_order_is_scale_then_origin_rotation_then_translation() {
    let base = import(&source()).unwrap();
    let result = import(&authored(
        json!({"offset":[0.25,0.75],"rotation":std::f32::consts::FRAC_PI_2,"scale":[0.5,-0.25]}),
    ))
    .unwrap();
    for (before, after) in base.source().primitives[0]
        .vertices
        .iter()
        .zip(&result.source().primitives[0].vertices)
    {
        // Independent exact quarter-turn matrix, not the production trig helper.
        let expected = [0.25 + 0.25 * before.uv[1], 0.75 + 0.5 * before.uv[0]];
        assert!((0..2).all(|axis| (after.uv[axis] - expected[axis]).abs() < 1e-6));
        assert_eq!(before.position, after.position);
        assert_eq!(before.normal, after.normal);
    }
    assert_eq!(base.source().primitives[1], result.source().primitives[1]);
    assert_eq!(base.source().materials, result.source().materials);
    assert_eq!(base.source().images, result.source().images);
    assert_eq!(
        base.source().primitives[0].id,
        result.source().primitives[0].id
    );
    assert_ne!(base.source().dependencies, result.source().dependencies);
}

#[test]
fn material_local_transform_does_not_mutate_shared_accessor() {
    let mut doc = authored(json!({"offset":[0.25,0.5],"scale":[-1,2]}));
    let mut shared = doc["meshes"][0]["primitives"][0].clone();
    shared["material"] = json!(1);
    doc["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(shared);
    let model = import(&doc).unwrap();
    let base = import(&source()).unwrap();
    assert_eq!(
        model.source().primitives[2].vertices,
        base.source().primitives[0].vertices
    );
    assert_ne!(
        model.source().primitives[0].vertices,
        model.source().primitives[2].vertices
    );
}

#[test]
fn effective_texcoord_override_zero_is_supported_other_sets_fail_closed() {
    let mut doc = authored(json!({"texCoord":0}));
    doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"] = json!(1);
    assert!(import(&doc).is_ok());
    for transform in [
        json!({}),
        json!({"texCoord":1}),
        json!({"texCoord":4294967295u32}),
    ] {
        doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"][NAME] =
            transform;
        rejects_before_resolution(&doc);
    }
}

#[test]
fn unknown_duplicate_required_and_undeclared_extensions_reject_before_io() {
    let good = authored(json!({}));
    let mut required = good.clone();
    required["extensionsRequired"] = json!([NAME]);
    assert!(import(&required).is_ok());
    for (key, value) in [
        ("extensionsUsed", json!([])),
        ("extensionsUsed", json!([NAME, NAME])),
        ("extensionsUsed", json!([NAME, "OTHER"])),
        ("extensionsRequired", json!(["OTHER"])),
        ("extensionsRequired", json!([NAME, NAME])),
    ] {
        let mut doc = good.clone();
        doc[key] = value;
        rejects_before_resolution(&doc);
    }
    let mut required_only = source();
    required_only["extensionsRequired"] = json!([NAME]);
    rejects_before_resolution(&required_only);
    let mut orphan = good.clone();
    orphan["materials"][1]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] =
        json!({"OTHER":{}});
    rejects_before_resolution(&orphan);
}

#[test]
fn malformed_extension_objects_and_fields_reject_before_io() {
    for value in [
        Value::Null,
        json!([]),
        json!([{}, 0]),
        json!(1),
        json!("bad"),
        json!({"unknown":0}),
        json!({"offset":null}),
        json!({"offset":[0]}),
        json!({"offset":[0,0,0]}),
        json!({"scale":"1"}),
        json!({"rotation":null}),
        json!({"texCoord":null}),
        json!({"texCoord":-1}),
        json!({"texCoord":0.5}),
    ] {
        rejects_before_resolution(&authored(value));
    }
    for value in [
        Value::Null,
        json!([]),
        json!([{}]),
        json!({}),
        json!({NAME:{},"OTHER":{}}),
    ] {
        let mut doc = authored(json!({}));
        doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] = value;
        rejects_before_resolution(&doc);
    }
    let raw = serde_json::to_string(&authored(json!({"rotation":0}))).unwrap();
    for malformed in [
        raw.replace("\"rotation\":0", "\"rotation\":0,\"rotation\":1"),
        raw.replace("\"rotation\":0", "\"rotation\":1e100"),
        raw.replace(
            "\"KHR_texture_transform\":{\"rotation\":0}",
            "\"KHR_texture_transform\":{},\"KHR_texture_transform\":{}",
        ),
    ] {
        assert_ne!(malformed, raw);
        assert!(
            import_with_resolver("model.gltf", malformed.as_bytes(), |_| panic!(
                "malformed object resolved resources"
            ))
            .is_err()
        );
    }
}

#[test]
fn original_uv_bounds_cannot_be_hidden_by_zero_scale() {
    for invalid in [
        65537.0f32,
        -65537.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
    ] {
        let mut buffer = include_bytes!("fixtures/model.bin").to_vec();
        buffer[24..28].copy_from_slice(&invalid.to_le_bytes());
        assert!(import_buffer(&authored(json!({"scale":[0,0]})), &buffer).is_err());
    }
}

#[test]
fn transformed_uv_bounds_and_overflow_are_rejected_without_clamping() {
    assert!(import(&authored(json!({"offset":[65535,0]}))).is_ok());
    for transform in [
        json!({"offset":[65536,0]}),
        json!({"offset":[-65537,0]}),
        json!({"scale":[1e38,1e38]}),
        json!({"offset":[1e38,1e38],"scale":[1e38,1e38]}),
    ] {
        assert!(import(&authored(transform)).is_err());
    }
    let mut buffer = include_bytes!("fixtures/model.bin").to_vec();
    buffer[24..28].copy_from_slice(&65536f32.to_le_bytes());
    assert!(import_buffer(&authored(json!({"scale":[1e38,1]})), &buffer).is_err());
}

fn transformed_glb() -> Vec<u8> {
    let original = include_bytes!("fixtures/model.glb");
    let json_len = u32::from_le_bytes(original[12..16].try_into().unwrap()) as usize;
    let mut doc: Value = serde_json::from_slice(&original[20..20 + json_len]).unwrap();
    doc["extensionsUsed"] = json!([NAME]);
    doc["extensionsRequired"] = json!([NAME]);
    doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] =
        json!({NAME:{"offset":[0.25,0.5],"scale":[-1,2]}});
    let mut json = serde_json::to_vec(&doc).unwrap();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let tail = &original[20 + json_len..];
    let mut glb = b"glTF".to_vec();
    glb.extend(2u32.to_le_bytes());
    glb.extend(
        u32::try_from(20 + json.len() + tail.len())
            .unwrap()
            .to_le_bytes(),
    );
    glb.extend(u32::try_from(json.len()).unwrap().to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(json);
    glb.extend(tail);
    glb
}

#[test]
fn gltf_glb_and_immutable_cook_have_equivalent_geometry_and_pixels_inputs() {
    let a = import(&authored(json!({"offset":[0.25,0.5],"scale":[-1,2]}))).unwrap();
    let b = import_with_resolver("model.glb", &transformed_glb(), |_| {
        panic!("embedded GLB resolved externally")
    })
    .unwrap();
    assert_eq!(a.source().materials, b.source().materials);
    assert_eq!(a.source().images, b.source().images);
    for (p, q) in a.source().primitives.iter().zip(&b.source().primitives) {
        assert_eq!(p.vertices, q.vertices);
        assert_eq!(p.indices, q.indices);
        assert_eq!(p.transform, q.transform);
    }
    for model in [a, b] {
        let cooked = model.to_bytes().unwrap();
        assert_eq!(
            cooked,
            StaticModel::from_bytes(&cooked)
                .unwrap()
                .to_bytes()
                .unwrap()
        );
    }
}

#[cfg(feature = "animation")]
#[test]
fn animated_entrypoint_rejects_transform_before_external_resolution() {
    for doc in [authored(json!({})), {
        let mut d = source();
        d["extensionsUsed"] = json!([NAME]);
        d
    }] {
        let error = orr_model::animation_import::import_with_resolver(
            "model.gltf",
            &serde_json::to_vec(&doc).unwrap(),
            |_| panic!("animated transform reached resolver"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("texture transform"), "{error}");
    }
}
