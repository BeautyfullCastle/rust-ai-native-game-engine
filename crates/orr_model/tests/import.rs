#![cfg(feature = "import")]
#![allow(clippy::float_arithmetic)]
use orr_model::{
    import::{import_path, import_with_resolver},
    normal_matrix, StaticModel, IDENTITY,
};
use serde_json::{json, Value};
use std::path::Path;
fn fixture() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"))
}
fn source() -> Value {
    serde_json::from_slice(include_bytes!("fixtures/model.gltf")).unwrap()
}
fn import(v: &Value) -> Result<StaticModel, orr_model::Error> {
    import_with_resolver("model.gltf", &serde_json::to_vec(v).unwrap(), |uri| {
        // Fixture resolver never sees uncontrolled filesystem paths.
        let data = match uri {
            "model.bin" => include_bytes!("fixtures/model.bin").as_slice(),
            "quadrants.png" => include_bytes!("fixtures/quadrants.png").as_slice(),
            "marker.png" => include_bytes!("fixtures/marker.png").as_slice(),
            _ => panic!("unexpected resolver URI {uri}"),
        };
        Ok(data.to_vec())
    })
}
#[test]
fn genuine_external_and_embedded_import_cook_load_match() {
    let external = import_path(fixture(), "model.gltf").unwrap();
    let embedded = import_with_resolver("model.glb", include_bytes!("fixtures/model.glb"), |_| {
        panic!("GLB must be self-contained")
    })
    .unwrap();
    let a = external.source();
    let b = embedded.source();
    assert_eq!(
        (a.primitives.len(), a.materials.len(), a.images.len()),
        (2, 2, 2)
    );
    assert_eq!(a.dependencies.len(), 4);
    assert_eq!(b.dependencies.len(), 1);
    assert_eq!(a.materials, b.materials);
    assert_eq!(a.images, b.images);
    for (p, q) in a.primitives.iter().zip(&b.primitives) {
        assert_eq!(p.vertices, q.vertices);
        assert_eq!(p.indices, q.indices);
        assert_eq!(p.transform, q.transform);
        assert!(p.id.contains("#node=1/mesh=0/primitive="));
    }
    assert_eq!(a.primitives[0].indices, [0, 1, 2, 0, 2, 3, 0, 3, 4]);
    assert_eq!(a.primitives[0].vertices[0].uv, [0.0, 1.0]);
    let m = a.primitives[0].transform;
    assert!((m[3][0] + 0.1).abs() < 1e-6 && m[3][1].abs() < 1e-6 && (m[3][2] - 0.1).abs() < 1e-6);
    assert!((m[0][0] - 0.8 * 15f32.to_radians().cos()).abs() < 1e-6);
    let cooked = external.to_bytes().unwrap();
    let loaded = StaticModel::from_bytes(&cooked).unwrap();
    assert_eq!(&cooked, &loaded.to_bytes().unwrap());
    assert_eq!(a, loaded.source());
    let again = import_path(fixture(), "model.gltf").unwrap();
    assert_eq!(external.to_bytes().unwrap(), again.to_bytes().unwrap());
}
#[test]
fn malformed_accessor_bounds_stride_references_and_shapes_reject() {
    for (pointer, value) in [
        ("/accessors/0/count", json!(999999999)),
        ("/accessors/0/byteOffset", json!(usize::MAX)),
        ("/accessors/0/bufferView", json!(999)),
        ("/accessors/0/componentType", json!(5123)),
        ("/accessors/0/normalized", json!(true)),
        ("/accessors/0/min", json!([2, 0, 0])),
        ("/accessors/0/max", json!([0, 0, 0])),
        ("/accessors/0/min", json!([0, 0])),
        ("/accessors/0/type", json!("MAT4")),
        ("/accessors/1/count", json!(4)),
        ("/bufferViews/0/byteLength", json!(20)),
        ("/bufferViews/0/byteOffset", json!(usize::MAX)),
        ("/bufferViews/0/buffer", json!(999)),
        ("/bufferViews/0/byteStride", json!(8)),
        ("/bufferViews/0/byteStride", json!(255)),
        ("/buffers/0/byteLength", json!(2)),
        ("/meshes/0/primitives/0/indices", json!(999)),
        ("/meshes/0/primitives/0/material", json!(999)),
        ("/nodes/1/mesh", json!(999)),
        ("/scene", json!(999)),
        ("/textures/0/source", json!(999)),
    ] {
        let mut v = source();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        v.pointer_mut(parent).unwrap()[key] = value;
        assert!(import(&v).is_err(), "accepted {pointer}: {v}");
    }
    let mut v = source();
    v["accessors"][6]["count"] = json!(2);
    assert!(import(&v).is_err());
    let mut v = source();
    v["bufferViews"][1]["byteStride"] = json!(4);
    assert!(import(&v).is_err());
    let mut v = source();
    v["accessors"][0]["sparse"] = json!({});
    assert!(import(&v).is_err());
}
#[test]
fn unsupported_features_and_material_defaults_are_explicit_errors() {
    for (key, value) in [
        ("skins", json!([])),
        ("animations", json!([])),
        ("extensionsRequired", json!(["KHR_draco_mesh_compression"])),
        ("extensionsUsed", json!(["KHR_materials_unlit"])),
        ("extensions", json!({"x":{}})),
    ] {
        let mut v = source();
        v[key] = value;
        assert!(import(&v).is_err(), "{key}");
    }
    for (key, value) in [
        ("skin", json!(0)),
        ("weights", json!([1])),
        ("extensions", json!({})),
    ] {
        let mut v = source();
        v["nodes"][1][key] = value;
        assert!(import(&v).is_err(), "{key}");
    }
    let mut v = source();
    v["meshes"][0]["primitives"][0]["mode"] = json!(5);
    assert!(import(&v).is_err());
    let mut v = source();
    v["meshes"][0]["primitives"][0]["targets"] = json!([]);
    assert!(import(&v).is_err());
    let mut v = source();
    v["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(0);
    assert!(import(&v).is_err());
    for (key, value) in [
        ("alphaMode", json!("BLEND")),
        ("doubleSided", json!(true)),
        ("normalTexture", json!({"index":0})),
    ] {
        let mut v = source();
        v["materials"][0][key] = value;
        assert!(import(&v).is_err(), "{key}");
    }
    let mut v = source();
    v["materials"][0]["pbrMetallicRoughness"]
        .as_object_mut()
        .unwrap()
        .remove("metallicFactor");
    assert!(import(&v).unwrap_err().to_string().contains("metallic=0"));
    for (key, value) in [
        ("minFilter", 9729986),
        ("minFilter", 9987),
        ("magFilter", 9729),
        ("wrapS", 7),
    ] {
        let mut v = source();
        v["samplers"][0][key] = json!(value);
        assert!(import(&v).is_err(), "{key}");
    }
}
#[test]
fn node_cycles_duplicate_roots_parents_and_bad_transforms_reject() {
    let mut v = source();
    v["nodes"][1]["children"] = json!([0]);
    assert!(import(&v).is_err());
    let mut v = source();
    v["nodes"][0]["children"] = json!([1, 1]);
    assert!(import(&v).is_err());
    let mut v = source();
    v["nodes"][0]["children"] = json!([99]);
    assert!(import(&v).is_err());
    let mut v = source();
    v["scenes"][0]["nodes"] = json!([0, 0]);
    assert!(import(&v).is_err());
    let mut v = source();
    v["scenes"][0]["nodes"] = json!([1]);
    assert!(import(&v).is_err());
    for (key, value) in [
        ("scale", json!([0, 1, 1])),
        ("rotation", json!([0, 0, 0, 0])),
        (
            "matrix",
            json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]),
        ),
    ] {
        let mut v = source();
        v["nodes"][1][key] = value;
        assert!(import(&v).is_err());
    }
    let mut v = source();
    let mut nodes = Vec::new();
    for i in 0..66 {
        nodes.push(json!({"children":[i+1]}));
    }
    nodes.push(json!({"mesh":0}));
    v["nodes"] = json!(nodes);
    assert!(import(&v).is_err());
}
#[test]
fn binary_glb_header_chunks_indices_and_nonfinite_geometry_reject() {
    let original = include_bytes!("fixtures/model.glb");
    for n in [0, 4, 11, 20, original.len() - 1] {
        assert!(import_with_resolver("test.glb", &original[..n], |_| panic!()).is_err());
    }
    for (offset, word) in [(4, 1u32), (8, 99), (12, u32::MAX), (16, 0)] {
        let mut bytes = original.to_vec();
        bytes[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        assert!(import_with_resolver("test.glb", &bytes, |_| panic!()).is_err());
    }
    let json = serde_json::to_vec(&source()).unwrap();
    for (offset, replacement) in [
        (256, vec![255, 255]),
        (0, f32::NAN.to_le_bytes().to_vec()),
        (20, [0; 4].to_vec()),
    ] {
        let result = import_with_resolver("test.gltf", &json, |uri| {
            let mut data = std::fs::read(fixture().join(uri)).unwrap();
            if uri == "model.bin" {
                data[offset..offset + replacement.len()].copy_from_slice(&replacement);
            }
            Ok(data)
        });
        assert!(result.is_err());
    }
}
#[test]
fn external_paths_and_symlink_escape_reject_before_read() {
    for uri in [
        "$source",
        "../model.bin",
        "/tmp/model.bin",
        "a/../../x",
        "a\\x",
        "https://example.com/x",
        "%2e%2e/x",
        "x?foo",
        "./x",
    ] {
        let mut v = source();
        v["buffers"][0]["uri"] = json!(uri);
        assert!(
            import_with_resolver("asset.gltf", &serde_json::to_vec(&v).unwrap(), |_| panic!(
                "invalid URI reached resolver"
            ))
            .is_err()
        );
    }
    assert!(import_path(fixture(), "../model.gltf").is_err());
    #[cfg(unix)]
    {
        let root = std::env::temp_dir().join(format!("orr-model-escape-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(fixture().join("model.gltf"), root.join("escape.gltf")).unwrap();
        assert!(import_path(&root, "escape.gltf")
            .unwrap_err()
            .to_string()
            .contains("escapes"));
        std::fs::remove_file(root.join("escape.gltf")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
#[test]
fn data_uri_resources_and_changed_content_preserve_identity() {
    use base64::Engine;
    let mut v = source();
    v["buffers"][0]["uri"] = json!(format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(include_bytes!("fixtures/model.bin"))
    ));
    assert!(import(&v).is_ok());
    let original = import(&source()).unwrap();
    let mut v = source();
    v["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] = json!([0.5, 1, 1, 1]);
    let changed = import(&v).unwrap();
    assert_eq!(original.source().asset_id, changed.source().asset_id);
    assert_eq!(
        original.source().primitives[0].id,
        changed.source().primitives[0].id
    );
    assert_ne!(
        original.source().dependencies,
        changed.source().dependencies
    );
}
#[test]
fn cooked_model_is_revalidated_and_inverse_transpose_is_correct() {
    let model = import(&source()).unwrap();
    let mut raw = model.source().clone();
    raw.primitives[0].indices[0] = 999;
    assert!(StaticModel::new(raw).is_err());
    let mut raw = model.source().clone();
    raw.images[0].rgba8.pop();
    assert!(StaticModel::new(raw).is_err());
    let mut raw = model.source().clone();
    raw.primitives[0].vertices[0].uv = [f32::INFINITY, 0.0];
    assert!(StaticModel::new(raw).is_err());
    let mut m = IDENTITY;
    m[0][0] = -2.0;
    m[1][1] = 3.0;
    m[2][2] = 4.0;
    let n = normal_matrix(m).unwrap();
    assert_eq!(n[0][0], -0.5);
    assert!((n[1][1] - 1.0 / 3.0).abs() < 1e-6);
    assert_eq!(n[2][2], 0.25);
    m[0][0] = 0.0;
    assert!(normal_matrix(m).is_err());
    m[0][0] = f32::NAN;
    assert!(normal_matrix(m).is_err());
    let mut m = IDENTITY;
    m[0][3] = 1.0;
    assert!(normal_matrix(m).is_err());
}
#[test]
fn png_animation_and_decoded_extent_reject() {
    use std::io::Cursor;
    fn png(animated: bool, width: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(Cursor::new(&mut out), width, 1);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            if animated {
                e.set_animated(1, 0).unwrap();
            }
            let mut w = e.write_header().unwrap();
            w.write_image_data(&vec![255; width as usize * 4]).unwrap();
        }
        out
    }
    for data in [png(true, 2), png(false, 2049), vec![0; 10]] {
        let result = import_with_resolver(
            "test.gltf",
            &serde_json::to_vec(&source()).unwrap(),
            |uri| {
                Ok(if uri == "quadrants.png" {
                    data.clone()
                } else {
                    std::fs::read(fixture().join(uri)).unwrap()
                })
            },
        );
        assert!(result.is_err());
    }
}

#[cfg(unix)]
#[test]
fn fifo_is_rejected_before_open_can_block() {
    let root = std::env::temp_dir().join(format!("orr-model-fifo-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let fifo = root.join("source.gltf");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    let error = import_path(&root, "source.gltf").unwrap_err();
    assert!(error.to_string().contains("file/size"));
    std::fs::remove_file(fifo).unwrap();
    std::fs::remove_dir(root).unwrap();
}
#[test]
fn unused_reverse_ordered_deep_tree_is_rejected() {
    let mut v = source();
    let nodes = v["nodes"].as_array_mut().unwrap();
    nodes.push(json!({}));
    for i in 3..70 {
        nodes.push(json!({"children":[i-1]}));
    }
    // The chosen scene still uses only original shallow nodes 0 -> 1.
    assert!(import(&v).unwrap_err().to_string().contains("depth"));
}
