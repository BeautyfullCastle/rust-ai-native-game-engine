//! Factor-only import and immutable pre-change cook compatibility.
#![cfg(feature = "import")]
#![allow(clippy::float_arithmetic)]
#[path = "common/untextured_fixture.rs"]
mod fixture;
use orr_model::{ModelSource, StaticModel, Wrap};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
fn import(v: &Value, b: &[u8]) -> Result<StaticModel, orr_model::Error> {
    orr_model::import::import_with_resolver("factor.glb", &fixture::glb(v, b), |u| {
        fixture::resolve(u, b)
    })
}
fn check(s: &ModelSource, missing: bool) {
    assert_eq!(s.images.len(), 1);
    assert_eq!(
        (
            s.images[0].width,
            s.images[0].height,
            s.images[0].rgba8.as_slice()
        ),
        (1, 1, [255; 4].as_slice())
    );
    assert!(s.materials.iter().all(|m| m.image == 0
        && !m.linear_filter
        && m.wrap_s == Wrap::Clamp
        && m.wrap_t == Wrap::Clamp));
    assert_eq!(s.materials[0].base_color, [0.25, 0.5, 0.75, 1.0]);
    if missing {
        assert!(s
            .primitives
            .iter()
            .flat_map(|p| &p.vertices)
            .all(|v| v.uv == [0.0; 2]));
    }
    assert!(s.primitives.iter().all(|p| p.id.contains("#node=")));
}
#[test]
fn static_factor_gltf_glb_cook_and_optional_uv() {
    for with_uv in [false, true] {
        let (v, b) = fixture::factor(false, with_uv);
        assert_eq!(
            b.len(),
            v["buffers"][0]["byteLength"].as_u64().unwrap() as usize
        );
        let mut padded = b.clone();
        padded.push(0);
        let error =
            orr_model::import::import_with_resolver("factor.gltf", &fixture::gltf(&v), |u| {
                fixture::resolve(u, &padded)
            })
            .unwrap_err();
        assert!(error.to_string().contains("buffer byteLength mismatch"));
        for (id, bytes) in [
            ("factor.gltf", fixture::gltf(&v)),
            ("factor.glb", fixture::glb(&v, &b)),
        ] {
            let m =
                orr_model::import::import_with_resolver(id, &bytes, |u| fixture::resolve(u, &b))
                    .unwrap();
            check(m.source(), !with_uv);
            let cooked = m.to_bytes().unwrap();
            assert_eq!(
                StaticModel::from_bytes(&cooked)
                    .unwrap()
                    .to_bytes()
                    .unwrap(),
                cooked
            );
            assert_eq!(
                orr_model::import::import_with_resolver(id, &bytes, |u| fixture::resolve(u, &b))
                    .unwrap()
                    .to_bytes()
                    .unwrap(),
                cooked
            );
            assert_eq!(
                m.source().dependencies.len(),
                if id.ends_with("gltf") { 2 } else { 1 }
            );
            if with_uv {
                assert_eq!(m.source().primitives[0].vertices[0].uv, [0.0, 1.0]);
            }
        }
    }
    let (mut v, b) = fixture::factor(false, false);
    v["materials"][0]["pbrMetallicRoughness"]
        .as_object_mut()
        .unwrap()
        .remove("baseColorFactor");
    assert_eq!(
        import(&v, &b).unwrap().source().materials[0].base_color,
        [1.0; 4]
    );
}
fn reject_cases(animated: bool) {
    let (base, b) = fixture::factor(animated, true);
    let bad = [
        (
            "/materials/0/pbrMetallicRoughness/baseColorTexture",
            json!(null),
        ),
        ("/materials/0/pbrMetallicRoughness", json!(null)),
        ("/meshes/0/primitives/0/material", json!(null)),
        ("/meshes/0/primitives/0/material", json!(999)),
        (
            "/materials/0/pbrMetallicRoughness/baseColorFactor",
            json!([1.1, 0.0, 0.0, 1.0]),
        ),
        (
            "/materials/0/pbrMetallicRoughness/baseColorFactor",
            json!([-0.1, 0.0, 0.0, 1.0]),
        ),
        ("/materials/0/alphaMode", json!("BLEND")),
        ("/materials/0/alphaMode", json!("MASK")),
        ("/materials/0/doubleSided", json!(true)),
        ("/materials/0/extensions", json!({"KHR_materials_unlit":{}})),
        (
            "/materials/0/pbrMetallicRoughness/metallicFactor",
            json!(1.0),
        ),
        (
            "/materials/0/pbrMetallicRoughness/roughnessFactor",
            json!(0.5),
        ),
        ("/meshes/0/primitives/0/attributes/TEXCOORD_1", json!(2)),
    ];
    let mut huge = base.clone();
    huge["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] = json!([1e100, 0.0, 0.0, 1.0]);
    assert!(accepted(animated, &huge, &b).is_err());
    for (path, value) in bad {
        let mut v = base.clone();
        let (parent, key) = path.rsplit_once('/').unwrap();
        v.pointer_mut(parent).unwrap()[key] = value;
        assert!(accepted(animated, &v, &b).is_err(), "accepted {path}");
    }
    for (parent, key) in [
        ("/materials/0", "pbrMetallicRoughness"),
        ("/materials/0/pbrMetallicRoughness", "metallicFactor"),
        ("/meshes/0/primitives/0", "material"),
    ] {
        let mut v = base.clone();
        v.pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        let err = accepted(animated, &v, &b).unwrap_err();
        assert!(err.to_string().contains("metallic"), "{err}");
    }
    for value in [f32::NAN, f32::INFINITY, 65537.0] {
        let mut b = b.clone();
        fixture::set_first_uv(&base, &mut b, value);
        assert!(accepted(animated, &base, &b).is_err());
    }
    let uv = fixture::uv_accessor(&base);
    for (key, value) in [
        ("count", json!(999)),
        ("type", json!("VEC3")),
        ("componentType", json!(5123)),
    ] {
        let mut v = base.clone();
        v["accessors"][uv][key] = value;
        assert!(accepted(animated, &v, &b).is_err());
    }
    let mut v = base.clone();
    v["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] = json!(999);
    assert!(accepted(animated, &v, &b).is_err());
    let (mut v, b) = fixture::factor(animated, false);
    fixture::white_textured(&mut v);
    assert!(accepted(animated, &v, &b).is_err());
    let (mut v, b) = fixture::factor(animated, true);
    fixture::white_textured(&mut v);
    v["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"] = json!(1);
    assert!(accepted(animated, &v, &b).is_err());
}
fn accepted(animated: bool, v: &Value, b: &[u8]) -> Result<(), orr_model::Error> {
    #[cfg(feature = "animation")]
    if animated {
        return orr_model::animation_import::import_with_resolver(
            "factor.glb",
            &fixture::glb(v, b),
            |u| fixture::resolve(u, b),
        )
        .map(|_| ());
    }
    assert!(!animated);
    import(v, b).map(|_| ())
}
#[test]
fn static_factor_rejections_preserve_diffuse_subset() {
    reject_cases(false);
}
fn mixed(animated: bool) {
    let (mut v, b) = fixture::factor(animated, true);
    v["images"] = json!([{"uri":"white.png"}]);
    v["textures"] = json!([{"source":0}]);
    v["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index":0});
    if animated {
        let mut extra = v["materials"][0].clone();
        extra["pbrMetallicRoughness"]
            .as_object_mut()
            .unwrap()
            .remove("baseColorTexture");
        v["materials"].as_array_mut().unwrap().push(extra);
    }
    assert!(accepted(animated, &v, &b).is_ok());
    if !animated {
        let m = import(&v, &b).unwrap();
        assert_eq!(m.source().images.len(), 2);
        assert_eq!(m.source().materials[0].image, 0);
        assert_eq!(m.source().materials[1].image, 1);
    }
    v["textures"]
        .as_array_mut()
        .unwrap()
        .push(json!({"source":1})); // Unused but authored out-of-range must not alias fallback.
    assert!(accepted(animated, &v, &b).is_err());
    v["textures"] = json!([{"source":0}]);
    v["images"] = json!(vec![json!({"uri":"white.png"}); 32]);
    assert!(accepted(animated, &v, &b).is_err());
    v["images"].as_array_mut().unwrap().pop();
    assert!(accepted(animated, &v, &b).is_ok());
}
#[test]
fn static_mixed_material_image_bounds_and_authored_index_fence() {
    mixed(false);
}
#[cfg(feature = "animation")]
#[test]
fn animated_factor_gltf_glb_cook_optional_uv_and_poses() {
    use orr_model::animation::AnimatedModel;
    for with_uv in [false, true] {
        let (v, b) = fixture::factor(true, with_uv);
        assert_eq!(
            b.len(),
            v["buffers"][0]["byteLength"].as_u64().unwrap() as usize
        );
        let mut padded = b.clone();
        padded.push(0);
        let error = orr_model::animation_import::import_with_resolver(
            "factor.gltf",
            &fixture::gltf(&v),
            |u| fixture::resolve(u, &padded),
        )
        .unwrap_err();
        assert!(error.to_string().contains("buffer byteLength mismatch"));
        for (id, bytes) in [
            ("factor.gltf", fixture::gltf(&v)),
            ("factor.glb", fixture::glb(&v, &b)),
        ] {
            let m = orr_model::animation_import::import_with_resolver(id, &bytes, |u| {
                fixture::resolve(u, &b)
            })
            .unwrap();
            assert_eq!(m.source().images.len(), 1);
            assert!(m.source().materials.iter().all(|m| m.image == 0));
            if !with_uv {
                assert!(m
                    .source()
                    .primitives
                    .iter()
                    .flat_map(|p| &p.vertices)
                    .all(|v| v.vertex.uv == [0.0; 2]));
            }
            let cooked = m.to_bytes().unwrap();
            assert_eq!(
                AnimatedModel::from_bytes(&cooked)
                    .unwrap()
                    .to_bytes()
                    .unwrap(),
                cooked
            );
            assert_eq!(
                orr_model::animation_import::import_with_resolver(
                    id,
                    &bytes,
                    |u| fixture::resolve(u, &b)
                )
                .unwrap()
                .to_bytes()
                .unwrap(),
                cooked
            );
            for t in [0.0, 0.5, 1.0] {
                let pose = m.sample_clip(0, t).unwrap();
                assert!(!m.deform(&pose).unwrap().is_empty());
            }
        }
    }
}
#[cfg(feature = "animation")]
#[test]
fn animated_factor_rejections_and_mixed_image_bounds() {
    reject_cases(true);
    mixed(true);
}
fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[test]
fn static_textured_cooked_bytes_match_immutable_prechange_goldens() {
    {
        let m = orr_model::import::import_path(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")),
            "model.gltf",
        )
        .unwrap();
        assert_eq!(
            hex(&m.to_bytes().unwrap()),
            "b19e36c5d600f98444c0113a670287dca759448d23e85a2aab71fc67a7e41565"
        );
        let expected:Value=serde_json::from_str(r#"[{"uri":"$source","sha256":"2c9f7b7cfa8b4c44b7af88a634967a8f688df07c2ceebf2bed8ebc3acdc115f1"},{"uri":"marker.png","sha256":"2a3e30807f4a58fe3630880984937cb706835e5861447b7ca8d38f2dbc3daa3d"},{"uri":"model.bin","sha256":"7668608df4f010a8fee3962404c42d7f2ee4eefc02ac74221f099a6958322ee7"},{"uri":"quadrants.png","sha256":"d283459d062cede3ec4200115ac55490edb4f2ca2859764ddac29b7b6a63f47a"}]"#).unwrap();
        assert_eq!(
            serde_json::to_value(&m.source().dependencies).unwrap(),
            expected
        );
    }
    {
        let m = orr_model::import::import_path(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")),
            "model.glb",
        )
        .unwrap();
        assert_eq!(
            hex(&m.to_bytes().unwrap()),
            "bc0a6d916ff6c888f7bb8586db046f8affac909f600c0af3cc859af25fa84795"
        );
        let expected:Value=serde_json::from_str(r#"[{"uri":"$source","sha256":"ef1617780b40538c34b80e31a198d1a4740a590e77f7f20f6ca40f993f5b3834"}]"#).unwrap();
        assert_eq!(
            serde_json::to_value(&m.source().dependencies).unwrap(),
            expected
        );
    }
}
#[cfg(feature = "animation")]
#[test]
fn animated_textured_cooked_bytes_match_immutable_prechange_goldens() {
    {
        let m = orr_model::animation_import::import_path(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")),
            "animated_strip.gltf",
        )
        .unwrap();
        assert_eq!(
            hex(&m.to_bytes().unwrap()),
            "1a99cb09ba5de7ef6da09289b61557616c2644b60b749a623f303c82ae585008"
        );
        let expected:Value=serde_json::from_str(r#"[{"uri":"$source","sha256":"0cdbfbfd14abe04388fe542b19901063e7f35fbe561047d15e85e9e7395b2153"}]"#).unwrap();
        assert_eq!(
            serde_json::to_value(&m.source().dependencies).unwrap(),
            expected
        );
    }
    {
        let m = orr_model::animation_import::import_path(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")),
            "animated_strip.glb",
        )
        .unwrap();
        assert_eq!(
            hex(&m.to_bytes().unwrap()),
            "acb0fbd6a2b1e85f8162b7a3b4e4a15a529f96817992a7ac0112bc9a62bd6bfd"
        );
        let expected:Value=serde_json::from_str(r#"[{"uri":"$source","sha256":"c83ef7308ef16184aee59e59378a5f5624dfc2c7ac378f449477117f5381ac52"}]"#).unwrap();
        assert_eq!(
            serde_json::to_value(&m.source().dependencies).unwrap(),
            expected
        );
    }
}
#[test]
fn fallback_charges_decoded_budget_before_allocation() {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 2048, 2048);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![255; 2048 * 2048 * 4])
            .unwrap();
    }
    for animated in [false, true] {
        if animated && !cfg!(feature = "animation") {
            continue;
        }
        let (mut v, b) = fixture::factor(animated, false);
        for count in [1, 2] {
            v["images"] = json!(vec![json!({"uri":"large.png"}); count]);
            let bytes = fixture::glb(&v, &b);
            let resolver = |u: &str| -> Result<Vec<u8>, orr_model::Error> {
                assert_eq!(u, "large.png");
                Ok(png.clone())
            };
            #[cfg(feature = "animation")]
            if animated {
                let result = orr_model::animation_import::import_with_resolver(
                    "factor.glb",
                    &bytes,
                    resolver,
                );
                assert_eq!(result.is_ok(), count == 1);
                if count == 2 {
                    assert!(result
                        .unwrap_err()
                        .to_string()
                        .contains("factor-only white image exceeds image/decoded budget"));
                }
                continue;
            }
            let result = orr_model::import::import_with_resolver("factor.glb", &bytes, resolver);
            assert_eq!(result.is_ok(), count == 1);
            if count == 2 {
                assert!(result
                    .unwrap_err()
                    .to_string()
                    .contains("factor-only white image exceeds image/decoded budget"));
            }
        }
    }
}
#[cfg(feature = "animation")]
#[test]
fn animated_mixed_static_and_skinned_primitives_share_optional_uv_contract() {
    let reference = orr_model::animation_import::import_with_resolver(
        "original.glb",
        include_bytes!("fixtures/animated_strip.glb"),
        |_| panic!("embedded"),
    )
    .unwrap();
    for with_uv in [false, true] {
        let (mut v, b) = fixture::factor(true, with_uv);
        let mut mesh = v["meshes"][0].clone();
        for primitive in mesh["primitives"].as_array_mut().unwrap() {
            let attrs = primitive["attributes"].as_object_mut().unwrap();
            attrs.remove("JOINTS_0");
            attrs.remove("WEIGHTS_0");
        }
        v["meshes"].as_array_mut().unwrap().push(mesh);
        let node = v["nodes"].as_array().unwrap().len();
        v["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"mesh":1,"translation":[-2.0,0.0,0.0]}));
        v["scenes"][0]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!(node));
        let model = orr_model::animation_import::import_with_resolver(
            "mixed.glb",
            &fixture::glb(&v, &b),
            |u| fixture::resolve(u, &b),
        )
        .unwrap();
        assert_eq!(model.source().primitives.len(), 2);
        assert_eq!(model.source().images.len(), 1);
        assert!(model.source().primitives.iter().any(|p| p.skin.is_some()));
        assert!(model.source().primitives.iter().any(|p| p.skin.is_none()));
        for primitive in &model.source().primitives {
            assert_eq!(
                primitive.vertices.len(),
                reference.source().primitives[0].vertices.len()
            );
            for (actual, original) in primitive
                .vertices
                .iter()
                .zip(&reference.source().primitives[0].vertices)
            {
                assert_eq!(
                    actual.vertex.uv,
                    if with_uv {
                        original.vertex.uv
                    } else {
                        [0.0; 2]
                    }
                );
            }
        }
        let bytes = model.to_bytes().unwrap();
        assert_eq!(
            orr_model::animation::AnimatedModel::from_bytes(&bytes)
                .unwrap()
                .to_bytes()
                .unwrap(),
            bytes
        );
    }
}
