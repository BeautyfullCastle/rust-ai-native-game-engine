#![cfg(all(feature = "animation", feature = "import"))]
#![allow(clippy::float_arithmetic)]
use base64::Engine;
use orr_model::{
    animation::{AnimatedModel, ChannelValues, Interpolation, MAX_ANIMATION_KEYS},
    animation_import::{import_path, import_with_resolver},
    Error, IDENTITY,
};
use serde_json::{json, Value};
use std::path::Path;

fn source() -> Value {
    serde_json::from_slice(include_bytes!("fixtures/animated_strip.gltf")).unwrap()
}
fn import(value: &Value) -> Result<AnimatedModel, Error> {
    import_with_resolver(
        "animated_strip.gltf",
        &serde_json::to_vec(value).unwrap(),
        |_| panic!("self-contained fixture"),
    )
}
fn set(value: &mut Value, pointer: &str, replacement: Value) {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    value.pointer_mut(parent).unwrap()[key] = replacement;
}
fn buffer(value: &Value) -> Vec<u8> {
    let uri = value["buffers"][0]["uri"].as_str().unwrap();
    base64::engine::general_purpose::STANDARD
        .decode(uri.split_once(',').unwrap().1)
        .unwrap()
}
fn put_buffer(value: &mut Value, bytes: &[u8]) {
    value["buffers"][0]["uri"] = json!(format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ));
    value["buffers"][0]["byteLength"] = json!(bytes.len());
}
fn write_accessor(value: &mut Value, index: usize, byte_offset: usize, replacement: &[u8]) {
    let view = value["accessors"][index]["bufferView"].as_u64().unwrap() as usize;
    let start = value["bufferViews"][view]["byteOffset"].as_u64().unwrap() as usize
        + value["accessors"][index]["byteOffset"]
            .as_u64()
            .unwrap_or(0) as usize
        + byte_offset;
    let mut bytes = buffer(value);
    bytes[start..start + replacement.len()].copy_from_slice(replacement);
    put_buffer(value, &bytes);
}
fn replace_accessor(
    value: &mut Value,
    index: usize,
    component: u32,
    normalized: bool,
    data: &[u8],
) {
    let mut bytes = buffer(value);
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    let view = value["bufferViews"].as_array().unwrap().len();
    value["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer":0,"byteOffset":bytes.len(),"byteLength":data.len(),"target":34962}));
    bytes.extend(data);
    value["accessors"][index]["bufferView"] = json!(view);
    value["accessors"][index]["componentType"] = json!(component);
    value["accessors"][index]["normalized"] = json!(normalized);
    put_buffer(value, &bytes);
}

#[test]
fn genuine_gltf_glb_cook_reload_and_original_hierarchy_match() {
    let a = import_path(
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")),
        "animated_strip.gltf",
    )
    .unwrap();
    let b = import_with_resolver(
        "animated_strip.gltf",
        include_bytes!("fixtures/animated_strip.glb"),
        |_| panic!(),
    )
    .unwrap();
    assert_eq!(a.source().nodes, b.source().nodes);
    assert_eq!(a.source().primitives, b.source().primitives);
    assert_eq!(a.source().materials, b.source().materials);
    assert_eq!(a.source().images, b.source().images);
    assert_eq!(a.source().skins, b.source().skins);
    assert_eq!(a.source().clips, b.source().clips);
    assert_ne!(a.source().dependencies, b.source().dependencies);
    assert_eq!(a.source().dependencies.len(), 1);
    let source = a.source();
    assert_eq!(
        (source.nodes.len(), source.skins.len(), source.clips.len()),
        (4, 1, 2)
    );
    assert_eq!(source.nodes[0].children, [1]);
    assert_eq!(source.nodes[0].rest.translation, [0.3, 0.2, 0.0]);
    assert_eq!(source.nodes[3].rest.translation, [7.0, 0.0, 0.0]);
    assert_eq!(source.skins[0].joints, [1, 2]);
    assert_ne!(source.skins[0].inverse_bind_matrices[0], IDENTITY);
    assert_ne!(source.skins[0].inverse_bind_matrices[1], IDENTITY);
    assert_eq!(
        source.primitives[0].vertices[0].vertex.position,
        [-0.28, 0.0, 0.0]
    );
    assert_eq!(
        source.primitives[0].vertices[2].weights,
        [0.8, 0.2, 0.0, 0.0]
    );
    assert_eq!(source.clips[0].name, "bend");
    assert_eq!(source.clips[1].name, "pulse");
    assert_eq!(
        source.clips[1].channels[0].interpolation,
        Interpolation::Step
    );
    let cooked = a.to_bytes().unwrap();
    let reloaded = AnimatedModel::from_bytes(&cooked).unwrap();
    assert_eq!(reloaded.to_bytes().unwrap(), cooked);
    assert_eq!(reloaded.source(), source);
    let rest = a.deform(&a.rest_pose().unwrap()).unwrap();
    for (raw, posed) in source.primitives[0].vertices.iter().zip(&rest[0].vertices) {
        for axis in 0..3 {
            assert!((raw.vertex.position[axis] - posed.position[axis]).abs() < 1e-6);
        }
    }
    let mid = a.deform(&a.sample_clip(0, 0.5).unwrap()).unwrap();
    assert!(mid[0].vertices[7].position[0] < rest[0].vertices[7].position[0] - 0.5);
    assert_eq!(
        a.to_bytes().unwrap(),
        import_path(
            Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")),
            "animated_strip.gltf"
        )
        .unwrap()
        .to_bytes()
        .unwrap()
    );
}

#[test]
fn static_import_entrypoint_still_rejects_animated_sources_before_resolver() {
    for bytes in [
        include_bytes!("fixtures/animated_strip.gltf").as_slice(),
        include_bytes!("fixtures/animated_strip.glb").as_slice(),
    ] {
        assert!(
            orr_model::import::import_with_resolver("asset.gltf", bytes, |_| panic!()).is_err()
        );
    }
    let original: Value = serde_json::from_slice(include_bytes!("fixtures/model.gltf")).unwrap();
    for key in ["skins", "animations"] {
        let mut value = original.clone();
        value[key] = json!([]);
        assert!(orr_model::import::import_with_resolver(
            "asset.gltf",
            &serde_json::to_vec(&value).unwrap(),
            |_| panic!()
        )
        .is_err());
    }
}

#[test]
fn explicit_null_new_properties_reject_before_resource_resolution() {
    let original: Value = serde_json::from_slice(include_bytes!("fixtures/model.gltf")).unwrap();
    let mut value = original;
    value["nodes"][1]["skin"] = Value::Null;
    assert!(orr_model::import::import_with_resolver(
        "asset.gltf",
        &serde_json::to_vec(&value).unwrap(),
        |_| panic!("null skin reached resolver")
    )
    .is_err());
    for path in [
        "/skins",
        "/animations",
        "/nodes/3/skin",
        "/skins/0/inverseBindMatrices",
        "/skins/0/skeleton",
        "/skins/0/name",
        "/animations/0/name",
        "/animations/0/samplers/0/interpolation",
    ] {
        let mut value = source();
        set(&mut value, path, Value::Null);
        // Make a valid external URI so an erroneous null acceptance cannot hide
        // behind a resolver that is skipped by the embedded fixture.
        value["buffers"][0]["uri"] = json!("fixture.bin");
        assert!(
            import_with_resolver(
                "asset.gltf",
                &serde_json::to_vec(&value).unwrap(),
                |_| panic!("null {path} reached resolver")
            )
            .is_err(),
            "accepted null {path}"
        );
    }
}

#[test]
fn absent_skin_animation_collections_allowed_but_present_empty_reject() {
    for key in ["skins", "animations"] {
        let mut value = source();
        value[key] = json!([]);
        value["buffers"][0]["uri"] = json!("fixture.bin");
        assert!(import_with_resolver(
            "asset.gltf",
            &serde_json::to_vec(&value).unwrap(),
            |_| panic!("empty {key} reached resolver")
        )
        .is_err());
    }
    let mut value = source();
    value.as_object_mut().unwrap().remove("animations");
    assert!(import(&value).unwrap().source().clips.is_empty());
    value.as_object_mut().unwrap().remove("skins");
    value["nodes"][3].as_object_mut().unwrap().remove("skin");
    value["meshes"][0]["primitives"][0]["attributes"]
        .as_object_mut()
        .unwrap()
        .remove("JOINTS_0");
    value["meshes"][0]["primitives"][0]["attributes"]
        .as_object_mut()
        .unwrap()
        .remove("WEIGHTS_0");
    let model = import(&value).unwrap();
    assert!(model.source().skins.is_empty());
    assert!(model.source().clips.is_empty());
}

#[test]
fn u8_u16_joints_and_float_normalized_integer_weights_are_supported() {
    let original = import(&source()).unwrap();
    for joint_component in [5121, 5123] {
        for weight_component in [5121, 5123, 5126] {
            let mut value = source();
            let mut joints = Vec::new();
            let mut weights = Vec::new();
            for vertex in &original.source().primitives[0].vertices {
                for joint in vertex.joints {
                    if joint_component == 5121 {
                        joints.push(joint as u8);
                    } else {
                        joints.extend((joint as u16).to_le_bytes());
                    }
                }
                match weight_component {
                    5121 => {
                        let first = (vertex.weights[0] * 255.0).round() as u8;
                        weights.extend([first, 255 - first, 0, 0]);
                    }
                    5123 => {
                        let first = (vertex.weights[0] * 65535.0).round() as u16;
                        for value in [first, 65535 - first, 0, 0] {
                            weights.extend(value.to_le_bytes());
                        }
                    }
                    _ => {
                        for weight in vertex.weights {
                            weights.extend(weight.to_le_bytes());
                        }
                    }
                }
            }
            replace_accessor(&mut value, 3, joint_component, false, &joints);
            replace_accessor(
                &mut value,
                4,
                weight_component,
                weight_component != 5126,
                &weights,
            );
            let imported = import(&value).unwrap();
            for (a, b) in imported.source().primitives[0]
                .vertices
                .iter()
                .zip(&original.source().primitives[0].vertices)
            {
                assert_eq!(a.joints, b.joints);
                assert!((a.weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
                for axis in 0..4 {
                    assert!((a.weights[axis] - b.weights[axis]).abs() <= 0.004);
                }
            }
        }
    }
}

#[test]
fn normalized_integer_weights_require_exact_raw_unit_sum() {
    for component in [5121, 5123] {
        for below in [true, false] {
            let mut value = source();
            let mut bytes = Vec::new();
            for vertex in 0..8 {
                let weights = if vertex == 0 {
                    if component == 5121 {
                        if below {
                            [254u16, 0, 0, 0]
                        } else {
                            [255, 1, 0, 0]
                        }
                    } else if below {
                        [65534, 0, 0, 0]
                    } else {
                        [65535, 1, 0, 0]
                    }
                } else {
                    [if component == 5121 { 255 } else { 65535 }, 0, 0, 0]
                };
                for weight in weights {
                    if component == 5121 {
                        bytes.push(weight as u8);
                    } else {
                        bytes.extend(weight.to_le_bytes());
                    }
                }
            }
            replace_accessor(&mut value, 4, component, true, &bytes);
            let error = import(&value).unwrap_err().to_string();
            assert!(
                error.contains("exactly"),
                "{component} below={below}: {error}"
            );
        }
    }
}

#[test]
fn orphan_f32_accessors_are_finite_and_scan_work_is_bounded() {
    for number in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut value = source();
        let mut bytes = buffer(&value);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        let offset = bytes.len();
        bytes.extend(number.to_le_bytes());
        let view = value["bufferViews"].as_array().unwrap().len();
        value["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(json!({"buffer":0,"byteOffset":offset,"byteLength":4}));
        value["accessors"]
            .as_array_mut()
            .unwrap()
            .push(json!({"bufferView":view,"componentType":5126,"count":1,"type":"SCALAR"}));
        put_buffer(&mut value, &bytes);
        assert!(import(&value)
            .unwrap_err()
            .to_string()
            .contains("nonfinite"));
    }
    let mut value = source();
    let mut bytes = buffer(&value);
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    let offset = bytes.len();
    bytes.resize(offset + 1024 * 1024, 0);
    let view = value["bufferViews"].as_array().unwrap().len();
    value["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer":0,"byteOffset":offset,"byteLength":1024*1024}));
    for _ in 0..65 {
        value["accessors"]
            .as_array_mut()
            .unwrap()
            .push(json!({"bufferView":view,"componentType":5126,"count":262144,"type":"SCALAR"}));
    }
    put_buffer(&mut value, &bytes);
    assert!(import(&value)
        .unwrap_err()
        .to_string()
        .contains("validation byte budget"));
}

#[test]
fn malformed_skin_hierarchy_references_and_scene_membership_reject() {
    for (path, replacement) in [
        ("/skins/0/joints", json!([])),
        ("/skins/0/joints", json!([1, 1])),
        ("/skins/0/joints", json!([1, 999])),
        ("/skins/0/joints", json!([1, 3])),
        ("/skins/0/skeleton", json!(3)),
        ("/skins/0/skeleton", json!(999)),
        ("/skins/0/inverseBindMatrices", json!(999)),
        ("/nodes/3/skin", json!(999)),
        ("/nodes/0/children", json!([1, 1])),
        ("/nodes/0/children", json!([999])),
        ("/nodes/2/children", json!([0])),
        ("/nodes/3/children", json!([1])),
        ("/scenes/0/nodes", json!([0, 0])),
        ("/scenes/0/nodes", json!([1, 3])),
        ("/scenes/0/nodes", json!([3])),
        ("/scene", json!(999)),
        ("/nodes/0/scale", json!([0, 1, 1])),
        ("/nodes/0/scale", json!([-1, 1, 1])),
        ("/nodes/0/rotation", json!([0, 0, 0, 0])),
        (
            "/nodes/0/matrix",
            json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]),
        ),
        ("/accessors/6/count", json!(1)),
        ("/accessors/6/type", json!("VEC4")),
        ("/accessors/6/componentType", json!(5123)),
    ] {
        let mut value = source();
        set(&mut value, path, replacement);
        assert!(import(&value).is_err(), "accepted {path}: {value}");
    }
    let mut value = source();
    value["nodes"][3].as_object_mut().unwrap().remove("mesh");
    assert!(import(&value).is_err());
    let mut value = source();
    value["nodes"][3].as_object_mut().unwrap().remove("skin");
    assert!(import(&value).is_err());
    let mut value = source();
    let nodes = value["nodes"].as_array_mut().unwrap();
    nodes.push(json!({}));
    for i in 5..72 {
        nodes.push(json!({"children":[i-1]}));
    }
    assert!(import(&value).unwrap_err().to_string().contains("depth"));
}

#[test]
fn missing_inverse_bind_defaults_to_identity() {
    let mut value = source();
    value["skins"][0]
        .as_object_mut()
        .unwrap()
        .remove("inverseBindMatrices");
    let model = import(&value).unwrap();
    assert_eq!(model.source().skins[0].inverse_bind_matrices, [IDENTITY; 2]);
}

#[test]
fn malformed_animation_shapes_paths_duplicates_and_unsupported_modes_reject() {
    for (path, replacement) in [
        (
            "/animations/0/samplers/0/interpolation",
            json!("CUBICSPLINE"),
        ),
        ("/animations/0/samplers/0/interpolation", json!("BEZIER")),
        ("/animations/0/samplers/0/input", json!(999)),
        ("/animations/0/samplers/0/output", json!(999)),
        ("/animations/0/channels/0/sampler", json!(999)),
        ("/animations/0/channels/0/target/node", json!(999)),
        ("/animations/0/channels/0/target/path", json!("weights")),
        ("/animations/0/channels/0/target/path", json!("matrix")),
        ("/animations/0/channels/0/target/path", json!("translation")),
        ("/accessors/7/type", json!("VEC2")),
        ("/accessors/7/componentType", json!(5123)),
        ("/accessors/7/count", json!(2)),
        ("/accessors/8/count", json!(2)),
        ("/accessors/8/normalized", json!(true)),
        ("/accessors/8/componentType", json!(5123)),
        ("/accessors/7/min", json!([1])),
        ("/accessors/7/max", json!([1])),
        ("/bufferViews/7/target", json!(34962)),
        ("/bufferViews/7/byteStride", json!(4)),
    ] {
        let mut value = source();
        set(&mut value, path, replacement);
        assert!(import(&value).is_err(), "accepted {path}");
    }
    let mut value = source();
    let duplicate = value["animations"][0]["channels"][0].clone();
    value["animations"][0]["channels"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    assert!(import(&value).is_err());
    let mut value = source();
    value["accessors"][7].as_object_mut().unwrap().remove("min");
    assert!(import(&value).is_err());
    let mut value = source();
    value["animations"][0]["samplers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"input":999,"output":8}));
    assert!(import(&value).is_err());
}

#[test]
fn binary_nonfinite_times_rotations_weights_joints_inverse_binds_reject() {
    for (accessor, offset, replacement) in [
        (7, 0, (-1f32).to_le_bytes().to_vec()),
        (7, 4, 0f32.to_le_bytes().to_vec()),
        (7, 4, f32::NAN.to_le_bytes().to_vec()),
        (7, 4, f32::INFINITY.to_le_bytes().to_vec()),
        (8, 12, 0f32.to_le_bytes().to_vec()),
        (8, 0, f32::NAN.to_le_bytes().to_vec()),
        (11, 0, 0f32.to_le_bytes().to_vec()),
        (11, 0, (-1f32).to_le_bytes().to_vec()),
        (4, 0, (-1f32).to_le_bytes().to_vec()),
        (4, 0, 0f32.to_le_bytes().to_vec()),
        (4, 0, f32::NAN.to_le_bytes().to_vec()),
        (4, 0, 0.2f32.to_le_bytes().to_vec()),
        (3, 0, vec![255]),
        (3, 3, vec![2]),
        (6, 0, 0f32.to_le_bytes().to_vec()),
        (6, 0, f32::NAN.to_le_bytes().to_vec()),
        (6, 12, 1f32.to_le_bytes().to_vec()),
        (6, 0, (-1f32).to_le_bytes().to_vec()),
        (0, 0, f32::NAN.to_le_bytes().to_vec()),
        (1, 8, 0f32.to_le_bytes().to_vec()),
        (5, 0, u16::MAX.to_le_bytes().to_vec()),
    ] {
        let mut value = source();
        write_accessor(&mut value, accessor, offset, &replacement);
        assert!(
            import(&value).is_err(),
            "accepted accessor {accessor}, offset {offset}"
        );
    }
    let mut value = source();
    write_accessor(&mut value, 3, 2 * 4 + 1, &[0]); // mixed-weight row's two joints become equal
    assert!(import(&value).is_err());
}

#[test]
fn unsupported_extra_influences_morphs_extensions_sparse_and_bad_accessor_layout_reject() {
    for (path, replacement) in [
        ("/meshes/0/primitives/0/attributes/JOINTS_1", json!(3)),
        ("/meshes/0/primitives/0/attributes/WEIGHTS_1", json!(4)),
        ("/meshes/0/primitives/0/targets", json!([])),
        ("/meshes/0/weights", json!([1])),
        ("/nodes/3/weights", json!([1])),
        ("/accessors/3/normalized", json!(true)),
        ("/accessors/3/componentType", json!(5125)),
        ("/accessors/3/count", json!(4)),
        ("/accessors/4/componentType", json!(5123)),
        ("/accessors/4/normalized", json!(true)),
        ("/accessors/3/type", json!("VEC3")),
        ("/accessors/0/sparse", json!({})),
        ("/accessors/0/byteOffset", json!(usize::MAX)),
        ("/accessors/0/bufferView", json!(999)),
        ("/bufferViews/0/byteLength", json!(4)),
        ("/bufferViews/0/byteStride", json!(8)),
        ("/bufferViews/3/byteOffset", json!(1)),
        ("/bufferViews/0/buffer", json!(999)),
        ("/skins/0/extensions", json!({})),
        ("/animations/0/extensions", json!({})),
        ("/extensionsUsed", json!(["KHR_mesh_quantization"])),
    ] {
        let mut value = source();
        set(&mut value, path, replacement);
        assert!(import(&value).is_err(), "accepted {path}");
    }
}

#[test]
fn unused_mesh_values_and_unselected_node_skin_references_are_checked() {
    let mut value = source();
    let bad_mesh = value["meshes"][0].clone();
    value["meshes"].as_array_mut().unwrap().push(bad_mesh);
    value["meshes"][1]["primitives"][0]["attributes"]["NORMAL"] = json!(0); // positions are not unit normals
    assert!(import(&value).is_err());
    let mut value = source();
    value["skins"]
        .as_array_mut()
        .unwrap()
        .push(json!({"joints":[1]}));
    value["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"mesh":0,"skin":1}));
    assert!(import(&value).is_err());
}

#[test]
fn collection_and_aggregate_key_budgets_reject() {
    for (key, item, count) in [
        ("skins", source()["skins"][0].clone(), 17),
        ("animations", source()["animations"][0].clone(), 33),
    ] {
        let mut value = source();
        value[key] = Value::Array(vec![item; count]);
        assert!(import(&value).is_err());
    }
    let mut value = source();
    value["skins"][0]["joints"] = json!((0..65).collect::<Vec<_>>());
    assert!(import(&value).is_err());
    let mut value = source();
    value["animations"][0]["channels"] =
        Value::Array(vec![value["animations"][0]["channels"][0].clone(); 257]);
    assert!(import(&value).is_err());
    let mut value = source();
    value["accessors"][7]["count"] = json!(MAX_ANIMATION_KEYS + 1);
    assert!(import(&value).is_err());
    // Many samplers reuse a valid large input accessor: aggregate budget must
    // count expanded inputs before any channel output vectors are allocated.
    let mut value = source();
    let count = 1025usize;
    let mut bytes = buffer(&value);
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    let offset = bytes.len();
    for i in 0..count {
        bytes.extend((i as f32).to_le_bytes());
    }
    let output_offset = bytes.len();
    for _ in 0..count {
        for f in [0f32, 0.0, 0.0, 1.0] {
            bytes.extend(f.to_le_bytes());
        }
    }
    let views = value["bufferViews"].as_array_mut().unwrap();
    let view = views.len();
    views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":count*4}));
    views.push(json!({"buffer":0,"byteOffset":output_offset,"byteLength":count*16}));
    value["accessors"][7] = json!({"bufferView":view,"componentType":5126,"count":count,"type":"SCALAR","min":[0],"max":[1024]});
    value["accessors"][8] =
        json!({"bufferView":view+1,"componentType":5126,"count":count,"type":"VEC4"});
    value["animations"][0]["samplers"] = Value::Array(vec![json!({"input":7,"output":8}); 256]);
    put_buffer(&mut value, &bytes);
    assert!(import(&value).unwrap_err().to_string().contains("budget"));
}

#[test]
fn malformed_glb_and_external_paths_fail_closed() {
    let original = include_bytes!("fixtures/animated_strip.glb");
    for n in [0, 4, 11, 20, original.len() - 1] {
        assert!(import_with_resolver("x.glb", &original[..n], |_| panic!()).is_err());
    }
    for (offset, word) in [(4, 1u32), (8, 99), (12, u32::MAX), (16, 0)] {
        let mut bytes = original.to_vec();
        bytes[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        assert!(import_with_resolver("x.glb", &bytes, |_| panic!()).is_err());
    }
    for uri in [
        "$source",
        "../rig.bin",
        "/tmp/rig.bin",
        "a\\rig.bin",
        "https://example.com/rig.bin",
        "%2e%2e/rig.bin",
    ] {
        let mut value = source();
        value["buffers"][0]["uri"] = json!(uri);
        assert!(import(&value).is_err());
    }
}

#[test]
fn cooked_reload_rejects_invalid_data_and_clip_switch_resets_missing_tracks() {
    let model = import(&source()).unwrap();
    let mut raw = model.source().clone();
    raw.skins[0].joints[1] = 999;
    assert!(AnimatedModel::from_bytes(&serde_json::to_vec(&raw).unwrap()).is_err());
    let mut raw = model.source().clone();
    raw.primitives[0].vertices[0].weights = [0.0; 4];
    assert!(AnimatedModel::new(raw).is_err());
    let mut raw = model.source().clone();
    raw.clips[0].channels[0].times[1] = 0.0;
    assert!(AnimatedModel::new(raw).is_err());
    let pulse = model.sample_clip(1, 0.5).unwrap();
    assert_eq!(pulse.local()[0].translation, [0.55, 0.4, 0.0]);
    let bend = model.sample_clip(0, 0.5).unwrap();
    assert_eq!(bend.local()[0].translation, [0.3, 0.2, 0.0]);
    assert_eq!(bend.local()[1].scale, [1.0; 3]);
    assert!(matches!(
        model.source().clips[0].channels[0].values,
        ChannelValues::Rotation(_)
    ));
}
