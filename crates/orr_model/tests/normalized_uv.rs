#![cfg(feature = "import")]
#![allow(clippy::float_arithmetic)]
#[path = "common/normalized_uv_fixture.rs"]
mod fixture;
use fixture::Encoding;
use orr_model::{import::import_with_resolver, Error, StaticModel};
use serde_json::{json, Value};
fn static_model(bytes: &[u8]) -> Result<StaticModel, Error> {
    import_with_resolver("normalized.glb", bytes, |uri| {
        panic!("fixture unexpectedly resolves {uri}")
    })
}
#[cfg(feature = "animation")]
fn animated(bytes: &[u8]) -> Result<orr_model::animation::AnimatedModel, Error> {
    orr_model::animation_import::import_with_resolver("normalized.glb", bytes, |uri| {
        panic!("fixture unexpectedly resolves {uri}")
    })
}
#[test]
fn static_normalized_endpoints_interiors_stride_and_final_unpadded_element_match_float_twins() {
    for (integer, float) in [
        (Encoding::U8, Encoding::FloatU8),
        (Encoding::U16, Encoding::FloatU16),
    ] {
        let model = static_model(&fixture::glb(false, integer)).unwrap();
        let twin = static_model(&fixture::glb(false, float)).unwrap();
        assert_eq!(model.source().primitives, twin.source().primitives);
        assert_eq!(model.source().materials, twin.source().materials);
        assert_eq!(model.source().images, twin.source().images);
        let samples = fixture::samples(integer);
        for primitive in &model.source().primitives {
            for (i, vertex) in primitive.vertices.iter().enumerate() {
                assert_eq!(vertex.uv, [samples[(i * 2) % 6], samples[(i * 2 + 1) % 6]]);
            }
        }
        let cooked = model.to_bytes().unwrap();
        assert_eq!(
            StaticModel::from_bytes(&cooked)
                .unwrap()
                .to_bytes()
                .unwrap(),
            cooked
        );
    }
}
#[cfg(feature = "animation")]
#[test]
fn animated_normalized_twins_preserve_skin_clips_pose_deformation_and_cooked_reload() {
    use orr_model::animation::AnimatedModel;
    for (integer, float) in [
        (Encoding::U8, Encoding::FloatU8),
        (Encoding::U16, Encoding::FloatU16),
    ] {
        let model = animated(&fixture::glb(true, integer)).unwrap();
        let twin = animated(&fixture::glb(true, float)).unwrap();
        assert_eq!(model.source().primitives, twin.source().primitives);
        assert_eq!(model.source().nodes, twin.source().nodes);
        assert_eq!(model.source().skins, twin.source().skins);
        assert_eq!(model.source().clips, twin.source().clips);
        assert_eq!(model.source().materials, twin.source().materials);
        assert_eq!(model.source().images, twin.source().images);
        let samples = fixture::samples(integer);
        for primitive in &model.source().primitives {
            for (i, v) in primitive.vertices.iter().enumerate() {
                assert_eq!(
                    v.vertex.uv,
                    [samples[(i * 2) % 6], samples[(i * 2 + 1) % 6]]
                );
            }
        }
        for clip in 0..model.source().clips.len() as u32 {
            for time in [0.0, 0.25, 0.75, 1.5] {
                let a = model.sample_clip(clip, time).unwrap();
                let b = twin.sample_clip(clip, time).unwrap();
                assert_eq!(a.local(), b.local());
                assert_eq!(a.global(), b.global());
                assert_eq!(a.skin_matrices(), b.skin_matrices());
                assert_eq!(model.deform(&a).unwrap(), twin.deform(&b).unwrap());
            }
        }
        let cooked = model.to_bytes().unwrap();
        assert_eq!(
            AnimatedModel::from_bytes(&cooked)
                .unwrap()
                .to_bytes()
                .unwrap(),
            cooked
        );
    }
}
fn reject_both(edit: impl Fn(&mut Value, usize, usize)) {
    for is_animated in [false, true] {
        if is_animated && !cfg!(feature = "animation") {
            continue;
        }
        let (mut doc, bin) = fixture::parts(is_animated, Encoding::U16);
        let uv = fixture::uv_accessor(&doc);
        let view = doc["accessors"][uv]["bufferView"].as_u64().unwrap() as usize;
        edit(&mut doc, uv, view);
        let bytes = fixture::pack(&doc, &bin);
        if !is_animated {
            assert!(
                static_model(&bytes).is_err(),
                "static accepted malformed UV {doc}"
            );
        }
        #[cfg(feature = "animation")]
        if is_animated {
            assert!(
                animated(&bytes).is_err(),
                "animated accepted malformed UV {doc}"
            );
        }
    }
}
#[test]
fn malformed_normalized_accessors_reject_without_relaxing_existing_uv_contract() {
    reject_both(|d, a, _| d["accessors"][a]["normalized"] = Value::Null);
    reject_both(|d, a, _| d["accessors"][a]["componentType"] = json!(5125));
    reject_both(|d, a, _| d["accessors"][a]["componentType"] = json!(5126));

    reject_both(|d, a, _| d["accessors"][a]["normalized"] = json!(false));
    reject_both(|d, a, _| {
        d["accessors"][a]
            .as_object_mut()
            .unwrap()
            .remove("normalized");
    });
    reject_both(|d, a, _| d["accessors"][a]["type"] = json!("VEC3"));
    reject_both(|d, a, _| d["accessors"][a]["componentType"] = json!(5122));
    reject_both(|d, a, _| d["accessors"][a]["count"] = json!(1));
    reject_both(|d, a, _| d["accessors"][a]["byteOffset"] = json!(5));
    reject_both(|d, a, _| d["accessors"][a]["byteOffset"] = json!(u64::MAX));
    reject_both(|d, _, v| d["bufferViews"][v]["byteLength"] = json!(5));
    reject_both(|d, _, v| d["bufferViews"][v]["byteStride"] = json!(5));
    reject_both(
        |d, a, v| d["accessors"][a]["sparse"] = json!({"count":1,"indices":{"bufferView":v,"componentType":5121},"values":{"bufferView":v}}),
    );
    reject_both(|d, a, _| d["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_1"] = json!(a));
}
#[test]
fn normalized_integer_uv_in_unused_mesh_is_still_admitted_fail_closed() {
    reject_both(|doc, uv, _| {
        let mut primitive = doc["meshes"][0]["primitives"][0].clone();
        let mut bad = doc["accessors"][uv].clone();
        bad["normalized"] = json!(false);
        let index = doc["accessors"].as_array().unwrap().len();
        doc["accessors"].as_array_mut().unwrap().push(bad);
        primitive["attributes"]["TEXCOORD_0"] = json!(index);
        doc["meshes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"primitives":[primitive]}));
    });
}
#[test]
fn texture_transform_is_baked_once_after_integer_normalization() {
    for is_animated in [false, true] {
        if is_animated && !cfg!(feature = "animation") {
            continue;
        }
        let (mut doc, bin) = fixture::parts(is_animated, Encoding::U8);
        doc["extensionsUsed"] = json!(["KHR_texture_transform"]);
        for material in doc["materials"].as_array_mut().unwrap() {
            material["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] =
                json!({"KHR_texture_transform":{"offset":[0.25,-0.5],"scale":[2.0,0.5]}});
        }
        let bytes = fixture::pack(&doc, &bin);
        let samples = fixture::samples(Encoding::U8);
        let expected = |i: usize| {
            [
                0.25 + samples[(i * 2) % 6] * 2.0,
                -0.5 + samples[(i * 2 + 1) % 6] * 0.5,
            ]
        };
        if !is_animated {
            let m = static_model(&bytes).unwrap();
            for p in &m.source().primitives {
                for (i, v) in p.vertices.iter().enumerate() {
                    assert_eq!(v.uv, expected(i));
                }
            }
            let reload = StaticModel::from_bytes(&m.to_bytes().unwrap()).unwrap();
            assert_eq!(reload.source().primitives, m.source().primitives);
        }
        #[cfg(feature = "animation")]
        if is_animated {
            let m = animated(&bytes).unwrap();
            for p in &m.source().primitives {
                for (i, v) in p.vertices.iter().enumerate() {
                    assert_eq!(v.vertex.uv, expected(i));
                }
            }
            let reload =
                orr_model::animation::AnimatedModel::from_bytes(&m.to_bytes().unwrap()).unwrap();
            assert_eq!(reload.source().primitives, m.source().primitives);
        }
    }
}

#[test]
fn unused_normalized_primitive_references_share_bytes_but_not_validation_budget() {
    let (mut doc, bin) = fixture::parts_with_count(false, Encoding::U8, Some(300));
    let primitive = doc["meshes"][0]["primitives"][0].clone();
    doc["meshes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"primitives":vec![primitive; 900]}));
    let error = static_model(&fixture::pack(&doc, &bin))
        .expect_err("aggregate normalized UV budget must reject");
    assert!(
        error
            .to_string()
            .contains("normalized UV source vertex budget exceeded"),
        "{error}"
    );
}

#[test]
fn independent_normalization_oracle_has_literal_endpoint_and_midpoint_bits() {
    let u8s = fixture::samples(Encoding::U8);
    assert_eq!(u8s[0].to_bits(), 0);
    assert_eq!(u8s[1].to_bits(), 0x3f800000);
    assert_eq!(u8s[2].to_bits(), 0x3b808081); // 1 / 255, independently tabulated.
    assert_eq!(u8s[5].to_bits(), 0x3f008081); // 128 / 255.
    let u16s = fixture::samples(Encoding::U16);
    assert_eq!(u16s[0].to_bits(), 0);
    assert_eq!(u16s[1].to_bits(), 0x3f800000);
    assert_eq!(u16s[2].to_bits(), 0x37800080); // 1 / 65535.
    assert_eq!(u16s[5].to_bits(), 0x3f000080); // 32768 / 65535.
}

#[test]
fn u16_packed_four_byte_elements_without_stride_are_valid_but_u8_tight_pairs_reject() {
    for is_animated in [false, true] {
        if is_animated && !cfg!(feature = "animation") {
            continue;
        }
        let (mut doc, mut bin) = fixture::parts(is_animated, Encoding::U16);
        let uv = fixture::uv_accessor(&doc);
        let view = doc["accessors"][uv]["bufferView"].as_u64().unwrap() as usize;
        let start = doc["bufferViews"][view]["byteOffset"].as_u64().unwrap() as usize;
        let count = doc["accessors"][uv]["count"].as_u64().unwrap() as usize;
        let packed: Vec<u8> = (0..count)
            .flat_map(|i| bin[start + 4 + i * 8..start + 8 + i * 8].iter().copied())
            .collect();
        bin[start + 4..start + 4 + packed.len()].copy_from_slice(&packed);
        doc["bufferViews"][view]["byteLength"] = json!(4 + packed.len());
        doc["bufferViews"][view]
            .as_object_mut()
            .unwrap()
            .remove("byteStride");
        let bytes = fixture::pack(&doc, &bin);
        if !is_animated {
            assert!(static_model(&bytes).is_ok());
        }
        #[cfg(feature = "animation")]
        if is_animated {
            assert!(animated(&bytes).is_ok());
        }
        let (mut doc, bin) = fixture::parts(is_animated, Encoding::U8);
        let uv = fixture::uv_accessor(&doc);
        let view = doc["accessors"][uv]["bufferView"].as_u64().unwrap() as usize;
        doc["bufferViews"][view]
            .as_object_mut()
            .unwrap()
            .remove("byteStride");
        let bytes = fixture::pack(&doc, &bin);
        if !is_animated {
            assert!(static_model(&bytes).is_err());
        }
        #[cfg(feature = "animation")]
        if is_animated {
            assert!(animated(&bytes).is_err());
        }
    }
}
