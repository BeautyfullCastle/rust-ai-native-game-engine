#![cfg(feature = "import")]
//! Independent triangle-corner fixture expansion: omitted and explicit sequential
//! index twins share expanded attributes, not the original shared-vertex stream.
#![allow(clippy::float_arithmetic)]
use base64::Engine;
#[cfg(feature = "import")]
use orr_model::{import::import_with_resolver, Error, StaticModel};
use serde_json::{json, Value};
fn fixture(animated: bool) -> Value {
    serde_json::from_slice(if animated {
        include_bytes!("fixtures/animated_strip.gltf")
    } else {
        include_bytes!("fixtures/model.gltf")
    })
    .unwrap()
}
fn put(doc: &mut Value, bytes: &[u8]) {
    doc["buffers"][0]["uri"] = json!(format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ));
    doc["buffers"][0]["byteLength"] = json!(bytes.len());
}
#[allow(dead_code)]
#[path = "common/nonindexed_fixture.rs"]
mod nonindexed_fixture;
fn expanded(animated: bool, count: Option<usize>) -> Value {
    let (mut doc, bytes) = nonindexed_fixture::expanded_parts(animated, count);
    put(&mut doc, &bytes);
    doc
}
fn omit(mut doc: Value) -> Value {
    for p in doc["meshes"][0]["primitives"].as_array_mut().unwrap() {
        p.as_object_mut().unwrap().remove("indices");
    }
    doc
}
#[cfg(feature = "import")]
fn resource(uri: &str) -> Result<Vec<u8>, Error> {
    Ok(match uri {
        "model.bin" => include_bytes!("fixtures/model.bin").to_vec(),
        "quadrants.png" => include_bytes!("fixtures/quadrants.png").to_vec(),
        "marker.png" => include_bytes!("fixtures/marker.png").to_vec(),
        _ => panic!("unexpected resolver {uri}"),
    })
}
#[cfg(feature = "import")]
fn static_import(doc: &Value) -> Result<StaticModel, Error> {
    import_with_resolver(
        "triangles.gltf",
        &serde_json::to_vec(doc).unwrap(),
        resource,
    )
}
#[cfg(all(feature = "animation", feature = "import"))]
fn animated_import(doc: &Value) -> Result<orr_model::animation::AnimatedModel, Error> {
    orr_model::animation_import::import_with_resolver(
        "triangles.gltf",
        &serde_json::to_vec(doc).unwrap(),
        |_| panic!("embedded animation"),
    )
}
#[cfg(feature = "import")]
#[test]
fn static_triangle_corner_twins_preserve_render_geometry_and_cooked_reload() {
    let original = static_import(&fixture(false)).unwrap();
    let indexed = static_import(&expanded(false, None)).unwrap();
    let omitted = static_import(&omit(expanded(false, None))).unwrap();
    assert_eq!(indexed.source().primitives, omitted.source().primitives);
    for (original, expanded) in original
        .source()
        .primitives
        .iter()
        .zip(&omitted.source().primitives)
    {
        assert_eq!(original.indices.len(), expanded.vertices.len());
        for (corner, vertex) in original.indices.iter().zip(&expanded.vertices) {
            assert_eq!(&original.vertices[*corner as usize], vertex);
        }
    }

    assert_eq!(indexed.source().materials, omitted.source().materials);
    assert_eq!(indexed.source().images, omitted.source().images);
    assert_ne!(indexed.source().dependencies, omitted.source().dependencies);
    for p in &omitted.source().primitives {
        assert_eq!(p.indices, (0..p.vertices.len() as u32).collect::<Vec<_>>());
    }
    let bytes = omitted.to_bytes().unwrap();
    assert_eq!(
        StaticModel::from_bytes(&bytes).unwrap().to_bytes().unwrap(),
        bytes
    );
}
#[cfg(feature = "animation")]
#[cfg(feature = "import")]
#[test]
fn animated_corner_twins_preserve_influences_clips_pose_and_deformed_vertices() {
    let indexed = animated_import(&expanded(true, None)).unwrap();
    let omitted = animated_import(&omit(expanded(true, None))).unwrap();
    assert_eq!(indexed.source().primitives, omitted.source().primitives);
    assert_eq!(indexed.source().nodes, omitted.source().nodes);
    assert_eq!(indexed.source().skins, omitted.source().skins);
    assert_eq!(indexed.source().clips, omitted.source().clips);
    assert_eq!(indexed.source().materials, omitted.source().materials);
    assert_eq!(indexed.source().images, omitted.source().images);
    assert_ne!(indexed.source().dependencies, omitted.source().dependencies);
    for clip in 0..indexed.source().clips.len() as u32 {
        for time in [0.0, 0.25, 0.75, 1.5, 2.0] {
            let a = indexed.sample_clip(clip, time).unwrap();
            let b = omitted.sample_clip(clip, time).unwrap();
            assert_eq!(a.global(), b.global());
            assert_eq!(indexed.deform(&a).unwrap(), omitted.deform(&b).unwrap());
        }
    }
    let bytes = omitted.to_bytes().unwrap();
    assert_eq!(
        orr_model::animation::AnimatedModel::from_bytes(&bytes)
            .unwrap()
            .to_bytes()
            .unwrap(),
        bytes
    );
}
#[cfg(feature = "import")]
#[test]
fn transformed_uv_is_baked_once_for_nonindexed_triangles() {
    for animated in [false, true] {
        let mut doc = expanded(animated, None);
        doc["extensionsUsed"] = json!(["KHR_texture_transform"]);
        doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] =
            json!({"KHR_texture_transform":{"offset":[0.25,-0.5],"scale":[2,3]}});
        if !animated {
            let a = static_import(&doc).unwrap();
            let b = static_import(&omit(doc.clone())).unwrap();
            assert_eq!(a.source().primitives, b.source().primitives);
        }
        #[cfg(feature = "animation")]
        if animated {
            let a = animated_import(&doc).unwrap();
            let b = animated_import(&omit(doc)).unwrap();
            assert_eq!(a.source().primitives, b.source().primitives);
        }
    }
}
#[cfg(feature = "import")]
#[test]
fn malformed_omitted_triangle_counts_modes_and_present_index_shapes_fail() {
    for animated in [false, true] {
        for count in [0, 1, 2, 4, orr_model::MAX_VERTICES + 2] {
            let doc = omit(expanded(animated, Some(count)));
            if !animated {
                assert!(static_import(&doc).is_err());
            }
            #[cfg(feature = "animation")]
            if animated {
                assert!(animated_import(&doc).is_err());
            }
        }
        for bad in [Value::Null, json!("0"), json!(-1), json!([]), json!({})] {
            let mut doc = expanded(animated, None);
            doc["meshes"][0]["primitives"][0]["indices"] = bad;
            if !animated {
                assert!(static_import(&doc).is_err());
            }
            #[cfg(feature = "animation")]
            if animated {
                assert!(animated_import(&doc).is_err());
            }
        }
        let mut doc = omit(expanded(animated, None));
        doc["meshes"][0]["primitives"][0]["mode"] = json!(5);
        if !animated {
            assert!(static_import(&doc).is_err());
        }
        #[cfg(feature = "animation")]
        if animated {
            assert!(animated_import(&doc).is_err());
        }
    }
}
#[cfg(feature = "import")]
#[test]
fn duplicate_indices_are_rejected_before_resource_resolution() {
    let doc = expanded(false, None);
    let index = doc["meshes"][0]["primitives"][0]["indices"]
        .as_u64()
        .unwrap();
    let needle = format!("\"indices\":{index}");
    let raw = serde_json::to_string(&doc).unwrap();
    assert!(raw.contains(&needle));
    let raw = raw.replacen(&needle, &format!("{needle},{needle}"), 1);
    assert!(import_with_resolver("bad.gltf", raw.as_bytes(), |_| panic!(
        "duplicate indices resolved resources"
    ))
    .is_err());
    #[cfg(feature = "animation")]
    assert!(orr_model::animation_import::import_with_resolver(
        "bad.gltf",
        raw.as_bytes(),
        |_| panic!("duplicate indices resolved resources")
    )
    .is_err());
}
#[cfg(feature = "import")]
#[test]
fn unused_malformed_mesh_and_repeated_instance_expansion_do_not_escape_budgets() {
    for animated in [false, true] {
        let mut doc = omit(expanded(animated, None));
        let mut bad = doc["meshes"][0].clone();
        bad["primitives"][0]["mode"] = json!(5);
        doc["meshes"].as_array_mut().unwrap().push(bad);
        if !animated {
            assert!(static_import(&doc).is_err());
        }
        #[cfg(feature = "animation")]
        if animated {
            assert!(animated_import(&doc).is_err());
        }
        let mut doc = omit(expanded(animated, Some(300)));
        let original = doc["nodes"].as_array().unwrap().len();
        let node = if animated {
            doc["nodes"][3].clone()
        } else {
            json!({"mesh":0})
        };
        for _ in original..1000 {
            let i = doc["nodes"].as_array().unwrap().len();
            doc["nodes"].as_array_mut().unwrap().push(node.clone());
            doc["scenes"][0]["nodes"]
                .as_array_mut()
                .unwrap()
                .push(json!(i));
        }
        if !animated {
            assert!(static_import(&doc).is_err());
        }
        #[cfg(feature = "animation")]
        if animated {
            assert!(animated_import(&doc).is_err());
        }
    }
}

#[test]
fn shared_glb_fixture_matches_embedded_document_twins() {
    for omitted in [false, true] {
        let bytes = nonindexed_fixture::static_glb(omitted);
        let model =
            import_with_resolver("triangles.gltf", &bytes, |_| panic!("embedded GLB")).unwrap();
        assert_eq!(
            model.source().primitives,
            static_import(&expanded(false, None))
                .unwrap()
                .source()
                .primitives
        );
        #[cfg(feature = "animation")]
        {
            let bytes = nonindexed_fixture::animated_glb(omitted);
            assert!(orr_model::animation_import::import_with_resolver(
                "fixture.glb",
                &bytes,
                |_| panic!("embedded GLB")
            )
            .is_ok());
        }
    }
}
