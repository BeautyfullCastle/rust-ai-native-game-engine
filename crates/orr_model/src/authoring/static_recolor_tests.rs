use super::*;
use crate::{Image, Wrap};
use serde_json::{json, Value};

const SOURCE: &str = "input.model.json";
const ID: &str = "variants/public.recolor.json";

fn fixture() -> StaticModel {
    let model = crate::import::import_with_resolver(
        "props/original.glb",
        include_bytes!("../../tests/fixtures/model.glb"),
        |_| panic!("fixture must not resolve external resources"),
    )
    .unwrap();
    let mut source = model.source().clone();
    source.materials[0].base_color = [-0.0, 0.5, 0.75, 0.25];
    source.materials[1].base_color = [0.125, 0.25, 0.5, -0.0];
    source.materials[1].image = source.materials[0].image;
    source.materials[1].linear_filter = true;
    source.materials[1].wrap_s = Wrap::Mirror;
    source.materials[1].wrap_t = Wrap::Repeat;
    source.materials.push(source.materials[1].clone());
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![19, 37, 71, 103],
    });
    source.primitives[0].vertices[0].position[2] = -0.0;
    source.primitives[0].vertices[0].normal[0] = -0.0;
    source.primitives[0].vertices[0].uv = [-0.0, 0.0];
    source.primitives[0].transform[3] = [-0.0, 0.0, -0.0, 1.0];
    let mut shared = source.primitives[1].clone();
    shared.id = format!("{}#node=17/mesh=8/primitive=3", source.asset_id);
    shared.transform = [
        [-2.0, -0.0, 0.0, 0.0],
        [0.0, 3.0, 0.0, -0.0],
        [-0.0, 0.0, 0.5, 0.0],
        [4.0, -7.0, 11.0, 1.0],
    ];
    source.primitives.push(shared);
    source.dependencies.push(Dependency {
        uri: "history/missing.png".into(),
        sha256: digest(b"history never resolved"),
    });
    StaticModel::new(source).unwrap()
}
fn recipe(rgb: [f64; 3]) -> Value {
    json!({"format":"orr_static_material_color","version":1,"source":SOURCE,"linear_rgb":rgb,"material":1})
}
fn apply(input: &[u8], source: &[u8]) -> Result<StaticModel, String> {
    let mut calls = 0;
    let out = recolor_model(input, ID, |path| {
        calls += 1;
        assert_eq!(path, SOURCE);
        assert_eq!(calls, 1, "resolve only the captured direct source once");
        Ok(source.to_vec())
    })?;
    assert_eq!(calls, 1);
    Ok(out)
}

#[test]
fn public_recolor_api_preserves_alpha_and_all_other_payload_bits_and_exact_provenance() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    for rgb in [
        [0.1, 0.2, 0.3],
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        [-0.0, 1.0, f64::MIN_POSITIVE],
        [1.0 - f64::EPSILON, 0.25, 0.875],
    ] {
        let input = serde_json::to_vec(&recipe(rgb)).unwrap();
        let out = apply(&input, &source).unwrap();
        let mut expected = original.source().clone();
        expected.asset_id = ID.into();
        for primitive in &mut expected.primitives {
            primitive.id = format!(
                "{ID}{}",
                primitive
                    .id
                    .strip_prefix(&original.source().asset_id)
                    .unwrap()
            );
        }
        let narrowed = rgb.map(|v| v as f32);
        expected.materials[1].base_color = [narrowed[0], narrowed[1], narrowed[2], -0.0];
        expected.dependencies = vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(&input),
            },
            Dependency {
                uri: SOURCE.into(),
                sha256: digest(&source),
            },
        ];
        let bytes = out.to_bytes().unwrap();
        // Full serialization checks shared images, unselected/unused tables, all geometry,
        // sampler state, color alpha and every unowned positive/negative-zero bit.
        assert_eq!(bytes, serde_json::to_vec(&expected).unwrap());
        assert_eq!(
            out.source().materials[1].base_color[3].to_bits(),
            (-0.0f32).to_bits()
        );
        assert_eq!(apply(&input, &source).unwrap().to_bytes().unwrap(), bytes);
        assert_eq!(
            StaticModel::from_bytes(&bytes).unwrap().to_bytes().unwrap(),
            bytes
        );
    }
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn public_recolor_api_validates_f64_range_and_strict_recipe_before_resolution() {
    // Both would appear in-range if narrowed before checking their original value.
    for bad in [1.0 + f64::EPSILON, -f64::MIN_POSITIVE] {
        assert!((0.0f32..=1.0).contains(&(bad as f32)));
        let input = serde_json::to_vec(&recipe([bad, 0.0, 0.0])).unwrap();
        assert!(recolor_model(&input, ID, |path| panic!("invalid f64 resolved {path}")).is_err());
    }
    let valid = recipe([0.875, 0.25, 0.0625]);
    for rgb in [
        json!([-1e-50, 0, 0]),
        json!([0, 0, 1.1]),
        json!(["0", 0, 0]),
        json!([null, 0, 0]),
        json!([true, 0, 0]),
        json!([0, 1]),
        json!([0, 1, 0, 1]),
    ] {
        let mut invalid = valid.clone();
        invalid["linear_rgb"] = rgb;
        assert!(
            recolor_model(&serde_json::to_vec(&invalid).unwrap(), ID, |path| panic!(
                "invalid RGB resolved {path}"
            ))
            .is_err()
        );
    }
    for key in ["format", "version", "source", "linear_rgb", "material"] {
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(
            recolor_model(&serde_json::to_vec(&missing).unwrap(), ID, |path| panic!(
                "missing field resolved {path}"
            ))
            .is_err()
        );
        let text = serde_json::to_string(&valid).unwrap();
        let duplicate = format!("{{\"{key}\":{},{}", valid[key], &text[1..]);
        assert!(recolor_model(duplicate.as_bytes(), ID, |path| panic!(
            "duplicate field resolved {path}"
        ))
        .is_err());
    }
    let mut extra = valid.clone();
    extra["extra"] = json!(true);
    let mut oversized = serde_json::to_vec(&valid).unwrap();
    oversized.resize(MAX_RECIPE_BYTES + 1, b' ');
    let positional = br#"["orr_static_material_color",1,"input.model.json",[0,1,0],1]"#.to_vec();
    for input in [serde_json::to_vec(&extra).unwrap(), oversized, positional] {
        assert!(
            recolor_model(&input, ID, |path| panic!("invalid recipe resolved {path}")).is_err()
        );
    }
    let input = serde_json::to_vec(&valid).unwrap();
    for id in ["../bad.recolor.json", "bad.model.json", "CON.recolor.json"] {
        assert!(recolor_model(&input, id, |path| panic!(
            "invalid identity resolved {path}"
        ))
        .is_err());
    }
}

#[test]
fn public_recolor_api_requires_used_material_and_admits_entire_source_without_mutation() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    let valid = recipe([0.875, 0.25, 0.0625]);
    for material in [2, 3, u32::MAX] {
        let mut input = valid.clone();
        input["material"] = json!(material);
        assert!(apply(&serde_json::to_vec(&input).unwrap(), &source)
            .unwrap_err()
            .contains("exist and be used"));
    }
    let input = serde_json::to_vec(&valid).unwrap();
    assert!(apply(&input, b"bad model").is_err());
    let mut bad = original.source().clone();
    bad.materials[2].image = u32::MAX;
    assert!(apply(&input, &serde_json::to_vec(&bad).unwrap())
        .unwrap_err()
        .contains("source model"));
    let error = recolor_model(&input, ID, |_| Err("original resolver failure".into())).unwrap_err();
    assert_eq!(error, "original resolver failure");
    assert_eq!(original.to_bytes().unwrap(), source);
}
