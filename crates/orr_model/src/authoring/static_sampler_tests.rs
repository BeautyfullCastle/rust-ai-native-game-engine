use super::*;
use crate::{Image, Material, Primitive, Vertex};
use serde_json::{json, Value};

const SOURCE: &str = "input.model.json";
const ID: &str = "variants/public.sampler.json";

fn fixture() -> StaticModel {
    let vertices = vec![
        Vertex {
            position: [-0.0, 0.0, 0.0],
            normal: [0.0, -0.0, 1.0],
            uv: [-0.0, 0.0],
        },
        Vertex {
            position: [1.0, 0.0, -0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, -0.0],
        },
        Vertex {
            position: [0.0, 1.0, 0.0],
            normal: [-0.0, 0.0, 1.0],
            uv: [0.0, 1.0],
        },
    ];
    let primitives = [0, 1, 1]
        .into_iter()
        .enumerate()
        .map(|(index, material)| Primitive {
            id: format!("props/source.glb#node={index}/mesh=8/primitive=0"),
            vertices: vertices.clone(),
            indices: vec![0, 1, 2],
            material,
            transform: [
                [-2.0, -0.0, 0.0, 0.0],
                [0.0, 3.0, 0.0, -0.0],
                [-0.0, 0.0, 0.5, 0.0],
                [4.0, -7.0, 11.0, 1.0],
            ],
        })
        .collect();
    let material = Material {
        base_color: [-0.0, 0.5, 0.75, 0.25],
        image: 0,
        linear_filter: true,
        wrap_s: Wrap::Mirror,
        wrap_t: Wrap::Repeat,
    };
    let mut unused = material.clone();
    unused.image = 1;
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: "props/source.glb".into(),
        dependencies: vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(b"original"),
            },
            Dependency {
                uri: "history/missing.png".into(),
                sha256: digest(b"history never resolved"),
            },
        ],
        primitives,
        materials: vec![material.clone(), material, unused],
        images: vec![
            Image {
                width: 1,
                height: 1,
                rgba8: vec![19, 37, 71, 103],
            },
            Image {
                width: 1,
                height: 1,
                rgba8: vec![2, 3, 5, 7],
            },
        ],
    })
    .unwrap()
}
fn recipe(linear_filter: bool, wrap_s: Wrap, wrap_t: Wrap) -> Value {
    json!({"format":"orr_static_material_sampler","version":1,"source":SOURCE,
        "linear_filter":linear_filter,"wrap_s":wrap_s,"wrap_t":wrap_t,"material":1})
}
fn apply(input: &[u8], source: &[u8]) -> Result<StaticModel, String> {
    let mut calls = 0;
    let model = author_sampler(input, ID, |path| {
        calls += 1;
        assert_eq!(path, SOURCE);
        assert_eq!(calls, 1, "only the direct captured source may resolve");
        Ok(source.to_vec())
    })?;
    assert_eq!(calls, 1);
    Ok(model)
}

#[test]
fn public_sampler_api_changes_only_selected_defaults_for_every_wrap_filter_combination() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    for filter in [false, true] {
        for s in [Wrap::Clamp, Wrap::Repeat, Wrap::Mirror] {
            for t in [Wrap::Clamp, Wrap::Repeat, Wrap::Mirror] {
                let input = serde_json::to_vec(&recipe(filter, s, t)).unwrap();
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
                expected.materials[1].linear_filter = filter;
                expected.materials[1].wrap_s = s;
                expected.materials[1].wrap_t = t;
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
                // Full serialization checks signed-zero color/geometry/UV/transform bits,
                // unused tables and the unselected material that shares the same image.
                assert_eq!(bytes, serde_json::to_vec(&expected).unwrap());
                assert_eq!(apply(&input, &source).unwrap().to_bytes().unwrap(), bytes);
                assert_eq!(
                    StaticModel::from_bytes(&bytes).unwrap().to_bytes().unwrap(),
                    bytes
                );
            }
        }
    }
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn public_sampler_api_rejects_strict_recipe_types_aliases_and_paths_before_resolution() {
    let valid = recipe(false, Wrap::Repeat, Wrap::Clamp);
    for (key, value) in [
        ("linear_filter", json!(0)),
        ("linear_filter", json!("false")),
        ("wrap_s", json!({"clamp":null})),
        ("wrap_s", json!("Repeat")),
        ("wrap_t", json!(null)),
        ("wrap_t", json!(["mirror"])),
        ("material", json!(-1)),
        ("source", json!("../input.model.json")),
    ] {
        let mut bad = valid.clone();
        bad[key] = value;
        let input = serde_json::to_vec(&bad).unwrap();
        assert!(
            author_sampler(&input, ID, |path| panic!("invalid recipe resolved {path}")).is_err()
        );
    }
    for key in [
        "format",
        "version",
        "source",
        "linear_filter",
        "wrap_s",
        "wrap_t",
        "material",
    ] {
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(
            author_sampler(&serde_json::to_vec(&missing).unwrap(), ID, |path| panic!(
                "missing field resolved {path}"
            ))
            .is_err()
        );
        let text = serde_json::to_string(&valid).unwrap();
        let duplicate = format!("{{\"{key}\":{},{}", valid[key], &text[1..]);
        assert!(author_sampler(duplicate.as_bytes(), ID, |path| panic!(
            "duplicate field resolved {path}"
        ))
        .is_err());
    }
    let mut extra = valid.clone();
    extra["extra"] = json!(true);
    let mut large = serde_json::to_vec(&valid).unwrap();
    large.resize(MAX_RECIPE_BYTES + 1, b' ');
    let positional =
        br#"["orr_static_material_sampler",1,"input.model.json",false,"repeat","clamp",1]"#
            .to_vec();
    for input in [serde_json::to_vec(&extra).unwrap(), large, positional] {
        assert!(
            author_sampler(&input, ID, |path| panic!("invalid shape resolved {path}")).is_err()
        );
    }
    let input = serde_json::to_vec(&valid).unwrap();
    for id in ["../bad.sampler.json", "bad.model.json", "CON.sampler.json"] {
        assert!(author_sampler(&input, id, |path| panic!(
            "invalid identity resolved {path}"
        ))
        .is_err());
    }
}

#[test]
fn public_sampler_api_requires_used_material_and_full_source_admission_without_mutation() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    let valid = recipe(false, Wrap::Repeat, Wrap::Clamp);
    for material in [2, 3, u32::MAX] {
        let mut input = valid.clone();
        input["material"] = json!(material);
        assert!(apply(&serde_json::to_vec(&input).unwrap(), &source)
            .unwrap_err()
            .contains("exist and be used"));
    }
    let input = serde_json::to_vec(&valid).unwrap();
    assert!(apply(&input, b"bad source").is_err());
    let mut invalid = original.source().clone();
    invalid.materials[2].image = u32::MAX;
    assert!(apply(&input, &serde_json::to_vec(&invalid).unwrap())
        .unwrap_err()
        .contains("source model"));
    let error =
        author_sampler(&input, ID, |_| Err("original resolver failure".into())).unwrap_err();
    assert_eq!(error, "original resolver failure");
    assert_eq!(original.to_bytes().unwrap(), source);
}
