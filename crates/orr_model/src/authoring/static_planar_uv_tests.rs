use super::*;
use crate::{Image, Wrap};
use serde_json::{json, Value};

const SOURCE: &str = "input.model.json";
const ID: &str = "variants/public.planar-uv.json";
const POSITIONS: [[f32; 3]; 5] = [
    [1., 2., 3.],
    [-2., 4., -1.],
    [0., -1., 2.],
    [3., 0., -2.],
    [-1., -2., 0.],
];

// Fixture retains unused tables and an unselected shared-image material, with one
// selected primitive under reflection, shear, nonuniform scale and translation.
fn fixture() -> StaticModel {
    let mut source = crate::import::import_with_resolver(
        "props/original.glb",
        include_bytes!("../../tests/fixtures/model.glb"),
        |_| panic!("embedded fixture must not resolve external resources"),
    )
    .unwrap()
    .source()
    .clone();
    source.materials[0].base_color = [0.125, 0.5, 0.75, 0.25];
    source.materials[0].wrap_s = Wrap::Mirror;
    source.materials[0].wrap_t = Wrap::Repeat;
    source.materials[1].base_color[0] = -0.0;
    source.materials[1].image = source.materials[0].image;
    source.materials.push(source.materials[1].clone());
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![19, 37, 71, 103],
    });
    assert_eq!(source.primitives[0].vertices.len(), POSITIONS.len());
    source.primitives[0].transform = crate::IDENTITY;
    for (v, position) in source.primitives[0].vertices.iter_mut().zip(POSITIONS) {
        v.position = position;
        v.normal[0] = -0.0;
        v.uv = [0., 0.];
    }
    for v in &mut source.primitives[1].vertices {
        v.uv = [-0.0, 2.5];
    }
    let mut shared = source.primitives[0].clone();
    shared.id = format!("{}#node=17/mesh=8/primitive=3", source.asset_id);
    // Column-major matrix with shear, reflection, nonuniform scale and translation.
    shared.transform = [
        [2., 1., 0., 0.],
        [0., -3., 1., 0.],
        [1., 0., 2., 0.],
        [4., -5., 6., 1.],
    ];
    source.primitives.push(shared);
    source.dependencies.push(Dependency {
        uri: "old/paint.png".into(),
        sha256: digest(b"never resolve old provenance"),
    });
    StaticModel::new(source).unwrap()
}

fn recipe(plane: &str) -> Value {
    json!({"format":"orr_static_planar_uv","version":1,"source":SOURCE,"material":0,
        "origin":[1.0,-2.0,3.0],"units_per_repeat":[2.0,4.0],"plane":plane})
}
fn apply(input: &[u8], source: &[u8]) -> Result<StaticModel, String> {
    let mut calls = 0;
    let model = project_model_uv(input, ID, |path| {
        calls += 1;
        assert_eq!(path, SOURCE);
        assert_eq!(calls, 1, "resolve only the captured direct source once");
        Ok(source.to_vec())
    })?;
    assert_eq!(calls, 1);
    Ok(model)
}

#[test]
fn public_planar_api_projects_all_planes_with_literal_reflected_model_space_oracles_and_preserves_bits(
) {
    // The reflected/sheared model positions are (9,-10,14),(-1,-19,8),(6,-2,9),
    // (8,-2,2),(2,0,4). These literal UVs subtract origin then divide by (2,4).
    let cases = [
        (
            "xy",
            [[0., 1.], [-1.5, 1.5], [-0.5, 0.25], [1., 0.5], [-1., 0.]],
            [[4., -2.], [-1., -4.25], [2.5, 0.], [3.5, 0.], [0.5, 0.5]],
        ),
        (
            "xz",
            [
                [0., 0.],
                [-1.5, -1.],
                [-0.5, -0.25],
                [1., -1.25],
                [-1., -0.75],
            ],
            [
                [4., 2.75],
                [-1., 1.25],
                [2.5, 1.5],
                [3.5, -0.25],
                [0.5, 0.25],
            ],
        ),
        (
            "yz",
            [[2., 0.], [3., -1.], [0.5, -0.25], [1., -1.25], [0., -0.75]],
            [
                [-4., 2.75],
                [-8.5, 1.25],
                [0., 1.5],
                [0., -0.25],
                [1., 0.25],
            ],
        ),
    ];
    let original = fixture();
    let original_bytes = original.to_bytes().unwrap();
    let mut source = original_bytes.clone();
    source.extend(b"\n  ");
    let captured = source.clone();
    for (plane, local, transformed) in cases {
        let mut input = serde_json::to_vec_pretty(&recipe(plane)).unwrap();
        input.push(b'\n');
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
        for (index, uvs) in [(0, local), (2, transformed)] {
            for (vertex, uv) in expected.primitives[index].vertices.iter_mut().zip(uvs) {
                vertex.uv = uv;
            }
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
        // Full serialization checks geometry/normals/transforms, unselected signed-zero
        // UVs, shared images, unused table entries, samplers and color-factor bits.
        assert_eq!(bytes, serde_json::to_vec(&expected).unwrap());
        assert_ne!(
            out.source().primitives[0].vertices[0].uv,
            out.source().primitives[0].vertices[1].uv
        );
        assert_eq!(apply(&input, &source).unwrap().to_bytes().unwrap(), bytes);
        assert_eq!(
            StaticModel::from_bytes(&bytes).unwrap().to_bytes().unwrap(),
            bytes
        );
    }
    assert_eq!(source, captured);
    assert_eq!(original.to_bytes().unwrap(), original_bytes);
}

#[test]
fn public_planar_api_admits_strict_shape_numeric_fields_and_paths_before_resolution() {
    let valid = recipe("xy");
    for (field, value) in [
        ("plane", json!("XY")),
        ("plane", json!({"xy":null})),
        ("origin", json!([0, 0])),
        ("origin", json!([1e100, 0, 0])),
        ("units_per_repeat", json!([0, 1])),
        ("units_per_repeat", json!([-1, 1])),
        ("units_per_repeat", json!([1e-100, 1])),
        ("material", json!(0.0)),
        ("source", json!("../input.model.json")),
        ("version", json!(2)),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        assert!(
            project_model_uv(&serde_json::to_vec(&invalid).unwrap(), ID, |path| panic!(
                "invalid recipe resolved {path}"
            ))
            .is_err()
        );
    }
    for key in [
        "format",
        "version",
        "source",
        "material",
        "origin",
        "units_per_repeat",
        "plane",
    ] {
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(
            project_model_uv(&serde_json::to_vec(&missing).unwrap(), ID, |path| panic!(
                "missing field resolved {path}"
            ))
            .is_err()
        );
        let text = serde_json::to_string(&valid).unwrap();
        let duplicate = format!("{{\"{key}\":{},{}", valid[key], &text[1..]);
        assert!(project_model_uv(duplicate.as_bytes(), ID, |path| panic!(
            "duplicate field resolved {path}"
        ))
        .is_err());
    }
    let mut extra = valid.clone();
    extra["extra"] = json!(true);
    let mut oversized = serde_json::to_vec(&valid).unwrap();
    oversized.resize(MAX_RECIPE_BYTES + 1, b' ');
    let positional =
        br#"["orr_static_planar_uv",1,"input.model.json",0,[0,0,0],[1,1],"xy"]"#.to_vec();
    for input in [serde_json::to_vec(&extra).unwrap(), oversized, positional] {
        assert!(
            project_model_uv(&input, ID, |path| panic!("invalid shape resolved {path}")).is_err()
        );
    }
    let input = serde_json::to_vec(&valid).unwrap();
    for id in [
        "../bad.planar-uv.json",
        "bad.model.json",
        "CON.planar-uv.json",
    ] {
        assert!(project_model_uv(&input, id, |path| panic!(
            "invalid identity resolved {path}"
        ))
        .is_err());
    }
}

#[test]
fn public_planar_api_rejects_unused_slots_and_overflow_before_narrowing_and_keeps_wide_model_precision(
) {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    let valid = recipe("xy");
    for material in [2, 3, u32::MAX] {
        let mut input = valid.clone();
        input["material"] = json!(material);
        assert!(apply(&serde_json::to_vec(&input).unwrap(), &source)
            .unwrap_err()
            .contains("exist and be used"));
    }
    let input = serde_json::to_vec(&valid).unwrap();
    assert!(apply(&input, b"bad model").is_err());
    let mut invalid = original.source().clone();
    invalid.materials[2].image = u32::MAX;
    assert!(apply(&input, &serde_json::to_vec(&invalid).unwrap())
        .unwrap_err()
        .contains("source model"));
    let mut boundary = original.source().clone();
    for primitive in &mut boundary.primitives {
        if primitive.material == 0 {
            primitive.transform = crate::IDENTITY;
            for vertex in &mut primitive.vertices {
                vertex.position = [65536., -65536., 0.];
            }
        }
    }
    let boundary = StaticModel::new(boundary).unwrap().to_bytes().unwrap();
    let mut input = valid.clone();
    input["origin"] = json!([0, 0, 0]);
    input["units_per_repeat"] = json!([1, 1]);
    let out = apply(&serde_json::to_vec(&input).unwrap(), &boundary).unwrap();
    for primitive in &out.source().primitives {
        if primitive.material == 0 {
            for vertex in &primitive.vertices {
                assert_eq!(vertex.uv, [65536., -65536.]);
            }
        }
    }
    // This excess would round back to 65536 if checked only after f32 narrowing.
    assert_eq!((65536.0f64 + 0.0009765625) as f32, 65536.0);
    for origin in [
        [-0.0009765625, 0., 0.],
        [0., 0.0009765625, 0.],
        [f32::MAX, 0., 0.],
    ] {
        input["origin"] = json!(origin);
        assert!(apply(&serde_json::to_vec(&input).unwrap(), &boundary)
            .unwrap_err()
            .contains("projected UV"));
    }
    input["origin"] = json!([0, 0, 0]);
    input["units_per_repeat"] = json!([f32::from_bits(1), 1.]);
    assert!(apply(&serde_json::to_vec(&input).unwrap(), &boundary).is_err());
    let mut wide = original.source().clone();
    for primitive in &mut wide.primitives {
        if primitive.material == 0 {
            primitive.transform = [
                [1e6, 0., 0., 0.],
                [1., 1., 0., 0.],
                [-1e6, 0., 1., 0.],
                [0., 0., 0., 1.],
            ];
            for vertex in &mut primitive.vertices {
                vertex.position = [1e6, 1., 1e6];
            }
        }
    }
    input["units_per_repeat"] = json!([1, 1]);
    let wide = StaticModel::new(wide).unwrap().to_bytes().unwrap();
    let out = apply(&serde_json::to_vec(&input).unwrap(), &wide).unwrap();
    for primitive in &out.source().primitives {
        if primitive.material == 0 {
            for vertex in &primitive.vertices {
                assert_eq!(vertex.uv, [1., 1.]);
            }
        }
    }
    assert_eq!(original.to_bytes().unwrap(), source);
}
