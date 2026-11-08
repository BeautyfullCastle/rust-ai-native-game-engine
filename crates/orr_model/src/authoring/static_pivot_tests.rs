use super::*;
use crate::{Image, Material, ModelSource, Primitive, Vertex, Wrap};
use serde_json::{json, Value};

const SOURCE: &str = "input.model.json";
const ID: &str = "variants/public.pivot.json";

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
    let transforms = [
        [
            [-2.0, -0.0, 0.0, 0.0],
            [0.0, 3.0, 0.0, -0.0],
            [-0.0, 0.0, 0.5, 0.0],
            [4.0, -7.0, 11.0, 1.0],
        ],
        [
            [0.0, 2.0, 0.0, 0.0],
            [-3.0, 0.5, 0.0, 0.0],
            [0.0, 0.0, 4.0, -0.0],
            [-0.0, 0.0, -0.0, 1.0],
        ],
    ];
    let primitives = transforms
        .into_iter()
        .enumerate()
        .map(|(index, transform)| Primitive {
            id: format!("props/source.glb#node={index}/mesh=8/primitive=0"),
            vertices: vertices.clone(),
            indices: vec![0, 1, 2],
            material: index as u32,
            transform,
        })
        .collect();
    let material = Material {
        base_color: [-0.0, 0.5, 0.75, 0.25],
        image: 0,
        linear_filter: true,
        wrap_s: Wrap::Mirror,
        wrap_t: Wrap::Repeat,
    };
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
        materials: vec![material.clone(), material.clone(), material],
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
fn recipe(pivot: [f64; 3]) -> Vec<u8> {
    serde_json::to_vec(
        &json!({"format":"orr_static_model_pivot","version":1,"source":SOURCE,"pivot":pivot}),
    )
    .unwrap()
}
fn apply(input: &[u8], source: &[u8]) -> Result<StaticModel, String> {
    let mut calls = 0;
    let model = rebase_model_pivot(input, ID, |path| {
        calls += 1;
        assert_eq!(path, SOURCE);
        assert_eq!(calls, 1, "only the captured direct source may resolve");
        Ok(source.to_vec())
    })?;
    assert_eq!(calls, 1);
    Ok(model)
}

#[test]
fn public_pivot_api_matches_literal_origins_preserves_all_other_fields_and_binds_exact_inputs() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    let input = recipe([2.0, -3.0, 1.5]);
    let out = apply(&input, &source).unwrap();
    let origins = [[2.0, -4.0, 9.5], [-2.0, 3.0, -1.5]];
    let mut expected = original.source().clone();
    expected.asset_id = ID.into();
    for (primitive, origin) in expected.primitives.iter_mut().zip(origins) {
        primitive.id = format!(
            "{ID}{}",
            primitive
                .id
                .strip_prefix(&original.source().asset_id)
                .unwrap()
        );
        primitive.transform[3] = [origin[0], origin[1], origin[2], 1.0];
    }
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
    // Canonical serialization checks signed zeros, primitive order, geometry/UV,
    // linear transforms, materials and original/unused image tables without remapping.
    assert_eq!(bytes, serde_json::to_vec(&expected).unwrap());
    for (before, after) in original
        .source()
        .primitives
        .iter()
        .zip(&out.source().primitives)
    {
        assert_eq!(
            crate::normal_matrix(before.transform)
                .unwrap()
                .map(|column| column.map(f32::to_bits)),
            crate::normal_matrix(after.transform)
                .unwrap()
                .map(|column| column.map(f32::to_bits))
        );
        assert_eq!(
            crate::determinant(before.transform).to_bits(),
            crate::determinant(after.transform).to_bits()
        );
    }
    assert_eq!(apply(&input, &source).unwrap().to_bytes().unwrap(), bytes);
    assert_eq!(
        StaticModel::from_bytes(&bytes).unwrap().to_bytes().unwrap(),
        bytes
    );
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn public_pivot_api_preserves_signed_zero_identities_and_existing_f32_decimal_semantics() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    for zeros in ["[0,0,0]", "[-0.0,0.0,-0.0]", "[1e-50,-1e-50,0]"] {
        let input = format!(
            r#"{{"format":"orr_static_model_pivot","version":1,"source":"{SOURCE}","pivot":{zeros}}}"#
        );
        let out = apply(input.as_bytes(), &source).unwrap();
        for (before, after) in original
            .source()
            .primitives
            .iter()
            .zip(&out.source().primitives)
        {
            assert_eq!(
                before.transform.map(|column| column.map(f32::to_bits)),
                after.transform.map(|column| column.map(f32::to_bits))
            );
        }
    }
    let mut source = original.source().clone();
    for primitive in &mut source.primitives {
        primitive.transform[3] = [0.3, 0.0, -0.0, 1.0];
    }
    let source = StaticModel::new(source).unwrap().to_bytes().unwrap();
    let out = apply(&recipe([0.1, -0.2, 1e-45]), &source).unwrap();
    for primitive in &out.source().primitives {
        assert_eq!(
            primitive.transform[3].map(f32::to_bits),
            [
                f32::from_bits(0x3e4cccce),
                0.2,
                f32::from_bits(0x80000001),
                1.0
            ]
            .map(f32::to_bits)
        );
    }
}

#[test]
fn public_pivot_api_admits_strict_recipe_before_resolution_and_rejects_bounds_without_clamping() {
    let valid = recipe([2.0, -3.0, 1.5]);
    let mut unknown: Value = serde_json::from_slice(&valid).unwrap();
    unknown["extra"] = json!(0);
    let mut bad_pivot: Value = serde_json::from_slice(&valid).unwrap();
    bad_pivot["pivot"] = json!([1_000_001, 0, 0]);
    let mut too_large = valid.clone();
    too_large.resize(MAX_RECIPE_BYTES + 1, b' ');
    let duplicate = format!(r#"{{"format":"orr_static_model_pivot","version":1,"source":"{SOURCE}","pivot":[0,0,0],"pivot":[0,0,0]}}"#).into_bytes();
    let positional = br#"["orr_static_model_pivot",1,"input.model.json",[0,0,0]]"#.to_vec();
    for input in [
        serde_json::to_vec(&unknown).unwrap(),
        serde_json::to_vec(&bad_pivot).unwrap(),
        too_large,
        duplicate,
        positional,
    ] {
        assert!(
            rebase_model_pivot(&input, ID, |path| panic!("invalid recipe resolved {path}"))
                .is_err()
        );
    }
    for id in ["../bad.pivot.json", "bad.model.json", "CON.pivot.json"] {
        assert!(rebase_model_pivot(&valid, id, |path| panic!(
            "invalid identity resolved {path}"
        ))
        .is_err());
    }
    let original = fixture();
    let source = original.to_bytes().unwrap();
    assert!(apply(&valid, b"bad model").is_err());
    assert!(apply(&recipe([-1_000_000., 0., 0.]), &source)
        .unwrap_err()
        .contains("transform bounds"));
    assert_eq!(original.to_bytes().unwrap(), source);
    let mut bounds = original.source().clone();
    bounds.primitives.truncate(1);
    bounds.primitives[0].transform = crate::IDENTITY;
    bounds.primitives[0].transform[0][0] = 1_000_000.;
    for vertex in &mut bounds.primitives[0].vertices {
        vertex.position[0] = 1000.;
    }
    let bounds = StaticModel::new(bounds).unwrap().to_bytes().unwrap();
    assert!(apply(&recipe([-128., 0., 0.]), &bounds)
        .unwrap_err()
        .contains("world vertex"));
}
