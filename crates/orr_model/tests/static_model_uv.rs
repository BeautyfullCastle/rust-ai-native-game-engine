#![cfg(feature = "import")]
#![allow(clippy::float_arithmetic)]

#[path = "../examples/static_model_uv/authoring.rs"]
mod authoring;

use authoring::transform_model_uv;
use orr_model::{Dependency, Image, ModelSource, StaticModel, Wrap};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

const SOURCE_PATH: &str = "inputs/prop.model.json";
const RECIPE_ID: &str = "variants/tile.uv.json";
const MAX_RECIPE_BYTES: usize = 16 * 1024;
const UVS: [[f32; 2]; 5] = [
    [-1.5, 2.25],
    [0.25, -0.5],
    [1.75, 0.125],
    [-0.0, 0.0],
    [0.5, 1.5],
];

fn recipe() -> Value {
    json!({
        "format": "orr_static_material_uv",
        "version": 1,
        "source": SOURCE_PATH,
        "material": 0,
        "offset": [0.0, 0.0],
        "scale": [1.0, 1.0],
        "quarter_turns": 0,
    })
}

fn recipe_bytes() -> Vec<u8> {
    serde_json::to_vec(&recipe()).unwrap()
}

fn fixture() -> StaticModel {
    orr_model::import::import_with_resolver(
        "props/original.glb",
        include_bytes!("fixtures/model.glb"),
        |_| panic!("embedded fixture must not resolve external resources"),
    )
    .unwrap()
}

fn rich_fixture() -> StaticModel {
    let mut source = fixture().source().clone();
    source.materials[0].base_color = [0.125, 0.5, 0.75, 0.25];
    source.materials[0].linear_filter = true;
    source.materials[0].wrap_s = Wrap::Mirror;
    source.materials[0].wrap_t = Wrap::Repeat;
    source.materials[1].base_color = [-0.0, 0.25, 0.75, 1.0];
    source.materials[1].linear_filter = false;
    source.materials[1].wrap_s = Wrap::Clamp;
    source.materials[1].wrap_t = Wrap::Mirror;
    // The other used material shares the selected material's original image.
    source.materials[1].image = source.materials[0].image;
    source.materials.push(source.materials[1].clone());
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![19, 37, 71, 103],
    });
    assert_eq!(source.primitives[0].vertices.len(), UVS.len());
    for (vertex, uv) in source.primitives[0].vertices.iter_mut().zip(UVS) {
        vertex.uv = uv;
        vertex.position[2] = -0.0;
        vertex.normal[0] = -0.0;
    }
    for vertex in &mut source.primitives[1].vertices {
        vertex.uv = [-0.0, 2.5];
    }
    let mut shared = source.primitives[0].clone();
    shared.id = format!("{}#node=17/mesh=8/primitive=3", source.asset_id);
    shared.transform = [
        [-2.0, -0.0, 0.0, 0.0],
        [0.0, 3.0, -0.0, 0.0],
        [0.0, 0.0, 0.5, -0.0],
        [4.0, -7.0, 11.0, 1.0],
    ];
    source.primitives.push(shared);
    source.dependencies.push(Dependency {
        uri: "original-materials/paint.png".into(),
        sha256: digest(b"old source provenance is bound but never resolved"),
    });
    StaticModel::new(source).unwrap()
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn apply(recipe: &[u8], id: &str, source: &[u8]) -> Result<StaticModel, String> {
    transform_model_uv(recipe, id, |path| {
        assert_eq!(path, SOURCE_PATH, "old dependency must never be resolved");
        Ok(source.to_vec())
    })
}

fn reject_before_resolution(bytes: &[u8], id: &str) {
    assert!(
        transform_model_uv(bytes, id, |path| panic!("invalid recipe resolved {path}")).is_err(),
        "accepted recipe {:?} with identity {id:?}",
        String::from_utf8_lossy(bytes),
    );
}

fn with_uvs(uv: [f32; 2]) -> Vec<u8> {
    let mut source = rich_fixture().source().clone();
    for primitive in &mut source.primitives {
        if primitive.material == 0 {
            for vertex in &mut primitive.vertices {
                vertex.uv = uv;
            }
        }
    }
    StaticModel::new(source).unwrap().to_bytes().unwrap()
}

fn assert_source_bits(actual: &ModelSource, expected: &ModelSource) {
    // Full serialization checks every field, including unused tables and -0.0.
    assert_eq!(
        serde_json::to_vec(actual).unwrap(),
        serde_json::to_vec(expected).unwrap()
    );
    assert_eq!(actual, expected);
    for (a, e) in actual.primitives.iter().zip(&expected.primitives) {
        assert_eq!(
            a.transform.map(|row| row.map(f32::to_bits)),
            e.transform.map(|row| row.map(f32::to_bits))
        );
        for (av, ev) in a.vertices.iter().zip(&e.vertices) {
            assert_eq!(av.position.map(f32::to_bits), ev.position.map(f32::to_bits));
            assert_eq!(av.normal.map(f32::to_bits), ev.normal.map(f32::to_bits));
            assert_eq!(av.uv.map(f32::to_bits), ev.uv.map(f32::to_bits));
        }
    }
    for (a, e) in actual.materials.iter().zip(&expected.materials) {
        assert_eq!(
            a.base_color.map(f32::to_bits),
            e.base_color.map(f32::to_bits)
        );
    }
}

fn expected_identity(source: &StaticModel, input: &[u8], bytes: &[u8], id: &str) -> ModelSource {
    let mut expected = source.source().clone();
    expected.asset_id = id.into();
    for primitive in &mut expected.primitives {
        let suffix = primitive
            .id
            .strip_prefix(&source.source().asset_id)
            .unwrap();
        assert!(suffix.starts_with("#node="));
        primitive.id = format!("{id}{suffix}");
    }
    expected.dependencies = vec![
        Dependency {
            uri: "$source".into(),
            sha256: digest(input),
        },
        Dependency {
            uri: SOURCE_PATH.into(),
            sha256: digest(bytes),
        },
    ];
    expected
}

#[test]
fn recipe_accepts_exact_required_object_and_whitespace() {
    let source = fixture().to_bytes().unwrap();
    let mut spaced = b" \n\t".to_vec();
    spaced.extend(serde_json::to_vec_pretty(&recipe()).unwrap());
    spaced.extend(b"\n\t ");
    for bytes in [recipe_bytes(), spaced] {
        let result = apply(&bytes, RECIPE_ID, &source).unwrap();
        assert_eq!(result.source().asset_id, RECIPE_ID);
        assert_eq!(result.source().dependencies[0].sha256, digest(&bytes));
    }
    let mut exact_limit = recipe_bytes();
    exact_limit.resize(MAX_RECIPE_BYTES, b' ');
    assert!(apply(&exact_limit, RECIPE_ID, &source).is_ok());
}

#[test]
fn recipe_rejects_unknown_duplicate_missing_null_and_sequence_fields_before_resolution() {
    let original = recipe();
    for field in [
        "format",
        "version",
        "source",
        "material",
        "offset",
        "scale",
        "quarter_turns",
    ] {
        let mut missing = original.clone();
        missing.as_object_mut().unwrap().remove(field);
        reject_before_resolution(&serde_json::to_vec(&missing).unwrap(), RECIPE_ID);
        let mut null = original.clone();
        null[field] = Value::Null;
        reject_before_resolution(&serde_json::to_vec(&null).unwrap(), RECIPE_ID);
        let mut duplicate = serde_json::to_string(&original).unwrap();
        duplicate.pop();
        duplicate.push_str(&format!(",{field:?}:{} }}", original[field]));
        reject_before_resolution(duplicate.as_bytes(), RECIPE_ID);
    }
    let mut unknown = original.clone();
    unknown["overwrite"] = json!(false);
    reject_before_resolution(&serde_json::to_vec(&unknown).unwrap(), RECIPE_ID);
    for alias in [
        json!([
            "orr_static_material_uv",
            1,
            SOURCE_PATH,
            0,
            [0, 0],
            [1, 1],
            0
        ]),
        json!([["format", "orr_static_material_uv"], ["version", 1]]),
        json!([]),
        json!(null),
        json!(true),
        json!(1),
        json!("recipe"),
    ] {
        reject_before_resolution(&serde_json::to_vec(&alias).unwrap(), RECIPE_ID);
    }
    for bytes in [
        b"".as_slice(),
        b"{}{}",
        b"{",
        b"[]",
        b"\xff",
        b"{} trailing",
    ] {
        reject_before_resolution(bytes, RECIPE_ID);
    }
}

#[test]
fn recipe_rejects_format_version_types_and_byte_budget_before_resolution() {
    for (field, value) in [
        ("format", json!("orr_static_model")),
        ("format", json!(1)),
        ("version", json!(0)),
        ("version", json!(2)),
        ("version", json!("1")),
        ("version", json!(1.0)),
        ("version", json!(true)),
        ("version", json!(-1)),
        ("version", json!(4294967296u64)),
        ("source", json!([])),
        ("source", json!(3)),
        ("material", json!("0")),
        ("material", json!(0.0)),
        ("material", json!(-1)),
        ("material", json!(true)),
        ("material", json!([])),
        ("material", json!(4294967296u64)),
        ("offset", json!([])),
        ("offset", json!([0])),
        ("offset", json!([0, 0, 0])),
        ("offset", json!(["0", 0])),
        ("offset", json!([null, 0])),
        ("offset", json!([false, 0])),
        ("offset", json!({"x": 0, "y": 0})),
        ("scale", json!([])),
        ("scale", json!([1])),
        ("scale", json!([1, 1, 1])),
        ("scale", json!([1, "1"])),
        ("scale", json!([1, null])),
        ("scale", json!([1, true])),
        ("scale", json!({"x": 1, "y": 1})),
        ("quarter_turns", json!("0")),
        ("quarter_turns", json!(0.0)),
        ("quarter_turns", json!(-1)),
        ("quarter_turns", json!(true)),
        ("quarter_turns", json!([])),
        ("quarter_turns", json!(4294967296u64)),
    ] {
        let mut bad = recipe();
        bad[field] = value;
        reject_before_resolution(&serde_json::to_vec(&bad).unwrap(), RECIPE_ID);
    }
    let mut too_large = recipe_bytes();
    too_large.resize(MAX_RECIPE_BYTES + 1, b' ');
    reject_before_resolution(&too_large, RECIPE_ID);
}

#[test]
fn recipe_rejects_nonfinite_nonpositive_scale_and_quarter_turn_boundaries_before_resolution() {
    for field in ["offset", "scale"] {
        for (left, right) in [
            ("1e39", "1"),
            ("1", "-1e39"),
            ("1e400", "1"),
            ("NaN", "1"),
            ("1", "Infinity"),
        ] {
            let mut object = recipe();
            object.as_object_mut().unwrap().remove(field);
            let mut text = serde_json::to_string(&object).unwrap();
            text.pop();
            text.push_str(&format!(",{field:?}:[{left},{right}]}}"));
            reject_before_resolution(text.as_bytes(), RECIPE_ID);
        }
    }
    for scale in [
        [0.0, 1.0],
        [-0.0, 1.0],
        [-1.0, 1.0],
        [1.0, 0.0],
        [1.0, -0.0],
        [1.0, -1.0],
        [1e-100, 1.0],
    ] {
        let mut bad = recipe();
        bad["scale"] = json!(scale);
        reject_before_resolution(&serde_json::to_vec(&bad).unwrap(), RECIPE_ID);
    }
    for turns in [4, u32::MAX] {
        let mut bad = recipe();
        bad["quarter_turns"] = json!(turns);
        reject_before_resolution(&serde_json::to_vec(&bad).unwrap(), RECIPE_ID);
    }
}

#[test]
fn recipe_accepts_positive_f32_extremes_and_finite_offsets() {
    for scale in [[f32::from_bits(1), f32::MAX], [f32::MIN_POSITIVE, 1.0]] {
        let mut input = recipe();
        input["scale"] = json!(scale);
        let bytes = serde_json::to_vec(&input).unwrap();
        let result = apply(&bytes, RECIPE_ID, &with_uvs([0.0, 0.0])).unwrap();
        assert_eq!(result.source().primitives[0].vertices[0].uv, [0.0, 0.0]);
    }
    // Extreme finite offsets pass recipe admission, even when a given model's
    // final UVs would not. Resolver failure distinguishes these two boundaries.
    for offset in [[f32::MAX, -f32::MAX], [-0.0, 0.0]] {
        let mut input = recipe();
        input["offset"] = json!(offset);
        let error = transform_model_uv(&serde_json::to_vec(&input).unwrap(), RECIPE_ID, |path| {
            assert_eq!(path, SOURCE_PATH);
            Err("admitted-finite-recipe".into())
        })
        .unwrap_err();
        assert_eq!(error, "admitted-finite-recipe");
    }
}

#[test]
fn paths_and_identity_are_normalized_portable_and_rejected_before_resolution() {
    for (field, suffix) in [("source", ".model.json"), ("asset_id", ".uv.json")] {
        let leaf = format!("prop{suffix}");
        for path in [
            String::new(),
            format!("/{leaf}"),
            format!("../{leaf}"),
            format!("a/../{leaf}"),
            format!("./{leaf}"),
            format!("a/./{leaf}"),
            format!("a//{leaf}"),
            format!("{leaf}/"),
            format!("a\\{leaf}"),
            format!("C:{leaf}"),
            format!("https://example.test/{leaf}"),
            format!("a%20/{leaf}"),
            format!("a?b/{leaf}"),
            format!("a#b/{leaf}"),
            format!(".hidden/{leaf}"),
            format!("a/.hidden/{leaf}"),
            format!(".{leaf}"),
            format!("a /{leaf}"),
            format!("a./{leaf}"),
            format!("a b/{leaf}"),
            format!("한글/{leaf}"),
            format!("a\0/{leaf}"),
            format!("a\n/{leaf}"),
            format!("CON{suffix}"),
            format!("prn{suffix}"),
            format!("AUX{suffix}"),
            format!("nul{suffix}"),
            format!("COM0{suffix}"),
            format!("lpt9{suffix}"),
            format!("NUL/{leaf}"),
            format!("a/CON.extra/{leaf}"),
            format!("{}{}", "a".repeat(241 - suffix.len()), suffix),
            "prop.wrong".into(),
            format!("prop{}", suffix.to_ascii_uppercase()),
        ] {
            let mut bad = recipe();
            let id = if field == "asset_id" {
                path.as_str()
            } else {
                bad[field] = json!(path);
                RECIPE_ID
            };
            reject_before_resolution(&serde_json::to_vec(&bad).unwrap(), id);
        }
    }
    reject_before_resolution(&recipe_bytes(), SOURCE_PATH);
    reject_before_resolution(&recipe_bytes(), &SOURCE_PATH.to_ascii_uppercase());
    let mut alias = recipe();
    alias["source"] = json!(RECIPE_ID);
    reject_before_resolution(&serde_json::to_vec(&alias).unwrap(), RECIPE_ID);
}

#[test]
fn portable_logical_paths_accept_the_240_byte_boundary() {
    let source_path = format!("{}.model.json", "a".repeat(240 - ".model.json".len()));
    let id = format!("{}.uv.json", "b".repeat(240 - ".uv.json".len()));
    let mut input = recipe();
    input["source"] = json!(source_path);
    let source = fixture().to_bytes().unwrap();
    let mut resolved = Vec::new();
    let result = transform_model_uv(&serde_json::to_vec(&input).unwrap(), &id, |path| {
        resolved.push(path.to_owned());
        assert_eq!(path, source_path);
        Ok(source.clone())
    })
    .unwrap();
    assert_eq!(result.source().asset_id, id);
    assert_eq!(resolved, [source_path]);
}

#[test]
fn resolution_is_exactly_once_and_failures_are_preserved() {
    let source = rich_fixture().to_bytes().unwrap();
    let mut resolved = Vec::new();
    transform_model_uv(&recipe_bytes(), RECIPE_ID, |path| {
        resolved.push(path.to_owned());
        assert_eq!(resolved.len(), 1);
        assert_eq!(path, SOURCE_PATH);
        Ok(source.clone())
    })
    .unwrap();
    assert_eq!(resolved, [SOURCE_PATH]);
    let mut calls = 0;
    let error = transform_model_uv(&recipe_bytes(), RECIPE_ID, |path| {
        calls += 1;
        assert_eq!(path, SOURCE_PATH);
        Err("resolver-denied-source".into())
    })
    .unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(error, "resolver-denied-source");
}

#[test]
fn source_is_fully_admitted_before_material_selection() {
    let good = rich_fixture();
    let mut invalid = vec![
        b"not a cooked static model".to_vec(),
        include_bytes!("fixtures/model.glb").to_vec(),
    ];
    for (pointer, value) in [
        ("/primitives/1/indices/0", json!(u32::MAX)),
        ("/primitives/1/material", json!(100)),
        ("/primitives/1/transform/0", json!([0, 0, 0, 0])),
        ("/primitives/1/vertices/0/uv/0", json!(65537)),
        ("/primitives/1/vertices/0/normal", json!([0, 0, 0])),
        ("/images/2/width", json!(0)),
        ("/materials/2/image", json!(u32::MAX)),
        ("/materials/1/base_color/0", json!(2)),
        ("/dependencies/0/sha256", json!("not-a-digest")),
        (
            "/primitives/1/id",
            json!("foreign.glb#node=1/mesh=0/primitive=1"),
        ),
        ("/version", json!(2)),
    ] {
        let mut document = serde_json::to_value(good.source()).unwrap();
        *document.pointer_mut(pointer).unwrap() = value;
        invalid.push(serde_json::to_vec(&document).unwrap());
    }
    for source in invalid {
        assert!(StaticModel::from_bytes(&source).is_err());
        let mut errors = Vec::new();
        for material in [0, u32::MAX] {
            let mut input = recipe();
            input["material"] = json!(material);
            let mut calls = 0;
            let error =
                transform_model_uv(&serde_json::to_vec(&input).unwrap(), RECIPE_ID, |path| {
                    calls += 1;
                    assert_eq!(path, SOURCE_PATH);
                    Ok(source.clone())
                })
                .unwrap_err();
            assert_eq!(calls, 1);
            assert!(error.starts_with("source model:"), "{error}");
            errors.push(error);
        }
        assert_eq!(
            errors[0], errors[1],
            "selector ran before full source admission"
        );
    }
}

#[test]
fn selection_requires_existing_used_material() {
    let original = rich_fixture();
    let source = original.to_bytes().unwrap();
    assert!(!original.source().primitives.iter().any(|p| p.material == 2));
    for material in [2, 3, u32::MAX] {
        let mut input = recipe();
        input["material"] = json!(material);
        let mut calls = 0;
        let error = transform_model_uv(&serde_json::to_vec(&input).unwrap(), RECIPE_ID, |path| {
            calls += 1;
            assert_eq!(path, SOURCE_PATH);
            Ok(source.clone())
        })
        .unwrap_err();
        assert_eq!(calls, 1);
        assert!(
            error.contains("selected material must exist and be used"),
            "{error}"
        );
    }
    let mut input = recipe();
    input["material"] = json!(1);
    input["offset"] = json!([0.5, -1.0]);
    let bytes = serde_json::to_vec(&input).unwrap();
    let result = apply(&bytes, RECIPE_ID, &source).unwrap();
    let mut expected = expected_identity(&original, &bytes, &source, RECIPE_ID);
    for vertex in &mut expected.primitives[1].vertices {
        vertex.uv = [0.5, 1.5];
    }
    assert_source_bits(result.source(), &expected);
}

#[test]
fn scale_then_origin_quarter_turn_then_offset_matches_hand_computed_coordinates() {
    // Hand-calculated from UVS with scale [2, 1/2], then origin rotation,
    // then offset [1/4, -3/4]. No production transform is used as the oracle.
    let expected = [
        [
            [-2.75, 0.375],
            [0.75, -1.0],
            [3.75, -0.6875],
            [0.25, -0.75],
            [1.25, 0.0],
        ],
        [
            [-0.875, -3.75],
            [0.5, -0.25],
            [0.1875, 2.75],
            [0.25, -0.75],
            [-0.5, 0.25],
        ],
        [
            [3.25, -1.875],
            [-0.25, -0.5],
            [-3.25, -0.8125],
            [0.25, -0.75],
            [-0.75, -1.5],
        ],
        [
            [1.375, 2.25],
            [0.0, -1.25],
            [0.3125, -4.25],
            [0.25, -0.75],
            [1.0, -1.75],
        ],
    ];
    let original = rich_fixture();
    let source = original.to_bytes().unwrap();
    for (turn, uvs) in expected.into_iter().enumerate() {
        let mut input = recipe();
        input["scale"] = json!([2.0, 0.5]);
        input["offset"] = json!([0.25, -0.75]);
        input["quarter_turns"] = json!(turn);
        let bytes = serde_json::to_vec(&input).unwrap();
        let result = apply(&bytes, RECIPE_ID, &source).unwrap();
        let mut full_expected = expected_identity(&original, &bytes, &source, RECIPE_ID);
        let mut selected = 0;
        for primitive in &mut full_expected.primitives {
            if primitive.material == 0 {
                selected += 1;
                for (vertex, uv) in primitive.vertices.iter_mut().zip(uvs) {
                    vertex.uv = uv;
                }
            }
        }
        assert_eq!(selected, 2);
        assert_source_bits(result.source(), &full_expected);
        assert_eq!(original.to_bytes().unwrap(), source);
    }
}

#[test]
fn transform_preserves_all_unrelated_bits_shared_images_and_multiple_primitive_users() {
    let original = rich_fixture();
    let source = original.to_bytes().unwrap();
    let mut input = recipe();
    input["offset"] = json!([0.25, -0.75]);
    let bytes = serde_json::to_vec(&input).unwrap();
    let result = apply(&bytes, RECIPE_ID, &source).unwrap();
    let mut expected = expected_identity(&original, &bytes, &source, RECIPE_ID);
    let uvs = [
        [-1.25, 1.5],
        [0.5, -1.25],
        [2.0, -0.625],
        [0.25, -0.75],
        [0.75, 0.75],
    ];
    for primitive in &mut expected.primitives {
        if primitive.material == 0 {
            for (vertex, uv) in primitive.vertices.iter_mut().zip(uvs) {
                vertex.uv = uv;
            }
        }
    }
    assert_source_bits(result.source(), &expected);
    assert_eq!(result.source().materials, original.source().materials);
    assert_eq!(result.source().images, original.source().images);
    assert_eq!(result.source().materials.len(), 3);
    assert_eq!(result.source().images.len(), 3);
    assert_eq!(
        result.source().materials[0].image,
        result.source().materials[1].image
    );
    assert_eq!(
        result
            .source()
            .primitives
            .iter()
            .filter(|p| p.material == 0)
            .count(),
        2
    );
    assert!(orr_model::determinant(result.source().primitives[2].transform) < 0.0);
}

#[test]
fn identity_preserves_all_uv_signed_zero_bits() {
    let mut source = rich_fixture().source().clone();
    let zeros = [
        [-0.0, 0.0],
        [0.0, -0.0],
        [-0.0, -0.0],
        [0.0, 0.0],
        [-2.5, 1.25],
    ];
    for primitive in &mut source.primitives {
        if primitive.material == 0 {
            for (vertex, uv) in primitive.vertices.iter_mut().zip(zeros) {
                vertex.uv = uv;
            }
        }
    }
    let original = StaticModel::new(source).unwrap();
    let source = original.to_bytes().unwrap();
    assert_source_bits(
        StaticModel::from_bytes(&source).unwrap().source(),
        original.source(),
    );
    for offset in [[0.0, 0.0], [-0.0, 0.0], [0.0, -0.0], [-0.0, -0.0]] {
        let mut input = recipe();
        input["offset"] = json!(offset);
        let bytes = serde_json::to_vec(&input).unwrap();
        let result = apply(&bytes, RECIPE_ID, &source).unwrap();
        assert_source_bits(
            result.source(),
            &expected_identity(&original, &bytes, &source, RECIPE_ID),
        );
        for primitive in &result.source().primitives {
            if primitive.material == 0 {
                for (vertex, uv) in primitive.vertices.iter().zip(zeros) {
                    assert_eq!(vertex.uv.map(f32::to_bits), uv.map(f32::to_bits));
                }
            }
        }
    }
    // Identity is decided after f32 admission. Preserve the original decimal
    // spellings: these scales round to one and these offsets to signed zero.
    let quantized_identity = br#"{
        "format":"orr_static_material_uv",
        "version":1,
        "source":"inputs/prop.model.json",
        "material":0,
        "offset":[1e-100,-1e-100],
        "scale":[1.0000000000000002,0.9999999999999999],
        "quarter_turns":0
    }"#;
    let result = apply(quantized_identity, RECIPE_ID, &source).unwrap();
    assert_source_bits(
        result.source(),
        &expected_identity(&original, quantized_identity, &source, RECIPE_ID),
    );
    for primitive in &result.source().primitives {
        if primitive.material == 0 {
            for (vertex, uv) in primitive.vertices.iter().zip(zeros) {
                assert_eq!(vertex.uv.map(f32::to_bits), uv.map(f32::to_bits));
            }
        }
    }
}

#[test]
fn all_turns_at_exact_uv_limit_are_accepted_without_clamping() {
    let source = with_uvs([65536.0, -65536.0]);
    let expected = [
        [65536.0, -65536.0],
        [65536.0, 65536.0],
        [-65536.0, 65536.0],
        [-65536.0, -65536.0],
    ];
    for (turn, uv) in expected.into_iter().enumerate() {
        let mut input = recipe();
        input["quarter_turns"] = json!(turn);
        let result = apply(&serde_json::to_vec(&input).unwrap(), RECIPE_ID, &source).unwrap();
        for primitive in &result.source().primitives {
            if primitive.material == 0 {
                for vertex in &primitive.vertices {
                    assert_eq!(vertex.uv, uv);
                }
            }
        }
    }
}

#[test]
fn scale_intermediate_infinity_rejects_before_offset_cancellation() {
    for (uv, scale, offset) in [
        ([2.0, 0.0], [f32::MAX, 1.0], [-f32::MAX, 0.0]),
        ([0.0, -2.0], [1.0, f32::MAX], [0.0, f32::MAX]),
    ] {
        let mut input = recipe();
        input["scale"] = json!(scale);
        input["offset"] = json!(offset);
        let error = apply(
            &serde_json::to_vec(&input).unwrap(),
            RECIPE_ID,
            &with_uvs(uv),
        )
        .unwrap_err();
        assert!(error.contains("nonfinite intermediate"), "{error}");
    }
}

#[test]
fn final_overflow_and_one_ulp_beyond_both_limits_reject_without_clamping() {
    for (uv, scale, offset) in [
        ([65536.0, 0.0], [1.0, 1.0], [0.0078125, 0.0]),
        ([-65536.0, 0.0], [1.0, 1.0], [-0.0078125, 0.0]),
        ([0.0, 65536.0], [1.0, 1.0], [0.0, 0.0078125]),
        ([0.0, -65536.0], [1.0, 1.0], [0.0, -0.0078125]),
        ([1.0, 0.0], [f32::MAX, 1.0], [f32::MAX, 0.0]),
        ([0.0, -1.0], [1.0, f32::MAX], [0.0, -f32::MAX]),
    ] {
        let mut input = recipe();
        input["scale"] = json!(scale);
        input["offset"] = json!(offset);
        let error = apply(
            &serde_json::to_vec(&input).unwrap(),
            RECIPE_ID,
            &with_uvs(uv),
        )
        .unwrap_err();
        assert!(error.contains("within +/-65536"), "{error}");
    }
}

#[test]
fn finite_intermediates_above_uv_limit_may_return_inside_final_bounds() {
    let mut input = recipe();
    input["scale"] = json!([2.0, 2.0]);
    input["offset"] = json!([-65536.0, 65536.0]);
    let result = apply(
        &serde_json::to_vec(&input).unwrap(),
        RECIPE_ID,
        &with_uvs([65536.0, -65536.0]),
    )
    .unwrap();
    for primitive in &result.source().primitives {
        if primitive.material == 0 {
            for vertex in &primitive.vertices {
                assert_eq!(vertex.uv, [65536.0, -65536.0]);
            }
        }
    }
}

#[test]
fn dependencies_bind_exact_recipe_and_source_without_chasing_old_uris() {
    let original = rich_fixture();
    let mut source = original.to_bytes().unwrap();
    source.extend(b"\n  ");
    let mut input = recipe_bytes();
    input.extend(b"\n");
    let mut resolved = Vec::new();
    let result = transform_model_uv(&input, RECIPE_ID, |path| {
        resolved.push(path.to_owned());
        assert_eq!(path, SOURCE_PATH);
        Ok(source.clone())
    })
    .unwrap();
    assert_eq!(resolved, [SOURCE_PATH]);
    let expected = expected_identity(&original, &input, &source, RECIPE_ID);
    assert_eq!(result.source().dependencies.len(), 2);
    assert_source_bits(result.source(), &expected);
    assert!(!result
        .source()
        .dependencies
        .iter()
        .any(|d| d.uri == "original-materials/paint.png"));
    assert_ne!(expected.dependencies[0].sha256, digest(&recipe_bytes()));
    assert_ne!(
        expected.dependencies[1].sha256,
        digest(&original.to_bytes().unwrap())
    );
}

#[test]
fn cooked_roundtrip_is_byte_deterministic_and_recipe_ids_remain_distinct() {
    let original = rich_fixture();
    let source = original.to_bytes().unwrap();
    let input = recipe_bytes();
    let first = apply(&input, RECIPE_ID, &source).unwrap();
    let second = apply(&input, RECIPE_ID, &source).unwrap();
    let cooked = first.to_bytes().unwrap();
    assert_eq!(cooked, second.to_bytes().unwrap());
    let reload = StaticModel::from_bytes(&cooked).unwrap();
    assert_source_bits(reload.source(), first.source());
    assert_eq!(reload.to_bytes().unwrap(), cooked);
    let other_id = "variants/other.uv.json";
    let other = apply(&input, other_id, &source).unwrap();
    assert_ne!(first.source().asset_id, other.source().asset_id);
    assert_ne!(cooked, other.to_bytes().unwrap());
    assert_source_bits(
        other.source(),
        &expected_identity(&original, &input, &source, other_id),
    );
}

#[test]
fn recipe_and_source_combined_budget_boundary_is_enforced() {
    let input = recipe_bytes();
    let mut source = fixture().to_bytes().unwrap();
    source.resize(orr_model::MAX_FILE_BYTES - input.len(), b' ');
    assert_eq!(input.len() + source.len(), orr_model::MAX_FILE_BYTES);
    assert!(apply(&input, RECIPE_ID, &source).is_ok());
    source.push(b' ');
    // Each individual input remains legal; only the cumulative budget rejects.
    assert!(source.len() <= orr_model::MAX_FILE_BYTES);
    assert!(StaticModel::from_bytes(&source).is_ok());
    let error = apply(&input, RECIPE_ID, &source).unwrap_err();
    assert!(
        error.contains("recipe plus cooked source exceeds 64 MiB"),
        "{error}"
    );
}

#[test]
fn transformed_cooked_output_budget_is_enforced() {
    // Near-limit valid data with many short primitive namespaces. Expanding
    // only their logical identity must still obey the cooked output budget.
    let mut source = fixture().source().clone();
    source.asset_id = "a.glb".into();
    let mut primitive = source.primitives[0].clone();
    primitive.vertices.truncate(3);
    primitive.indices = vec![0, 1, 2];
    source.primitives = (0..orr_model::MAX_PRIMITIVES)
        .map(|node| {
            let mut p = primitive.clone();
            p.id = format!("a.glb#node={node}/mesh=0/primitive=0");
            p
        })
        .collect();
    source.images = vec![
        Image {
            width: 2048,
            height: 2048,
            rgba8: vec![0; 2048 * 2048 * 4],
        },
        Image {
            width: 2048,
            height: 1,
            rgba8: vec![0; 2048 * 4],
        },
    ];
    let input = recipe_bytes();
    let baseline_len = serde_json::to_vec(&source).unwrap().len();
    let extra_rows =
        (orr_model::MAX_FILE_BYTES - input.len() - 8192 - baseline_len) / (2048 * 4 * 2);
    source.images[1].height += extra_rows as u32;
    source.images[1]
        .rgba8
        .resize(2048 * (extra_rows + 1) * 4, 0);
    let original = StaticModel::new(source).unwrap();
    let bytes = original.to_bytes().unwrap();
    assert!(bytes.len() + input.len() < orr_model::MAX_FILE_BYTES);
    let id = format!("{}.uv.json", "z".repeat(240 - ".uv.json".len()));
    let growth = (id.len() - "a.glb".len()) * (orr_model::MAX_PRIMITIVES + 1);
    assert!(bytes.len() + growth > orr_model::MAX_FILE_BYTES);
    let error = apply(&input, &id, &bytes).unwrap_err();
    assert!(error.starts_with("UV model cook:"), "{error}");
    assert!(error.contains("cooked model exceeds byte limit"), "{error}");
}

static SERIAL: AtomicUsize = AtomicUsize::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        // macOS may spell its temporary directory through /var. Positive tests
        // use physical ancestry; explicit symlink tests retain their aliases.
        let root = std::env::temp_dir().canonicalize().unwrap();
        assert!(root.is_absolute());
        for ancestor in root.ancestors() {
            let metadata = fs::symlink_metadata(ancestor).unwrap();
            assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
        }
        let path = root.join(format!(
            "orr-static-uv-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct CliFixture {
    temp: Temp,
    input: PathBuf,
    output: PathBuf,
    source: Vec<u8>,
    recipe: Vec<u8>,
}

impl CliFixture {
    fn new() -> Self {
        let temp = Temp::new();
        let input = temp.join("tile.uv.json");
        let output = temp.join("tile.model.json");
        let source = rich_fixture().to_bytes().unwrap();
        let recipe = recipe_bytes();
        populate(&temp.0, &input, &source, &recipe);
        Self {
            temp,
            input,
            output,
            source,
            recipe,
        }
    }

    fn args(&self) -> Vec<OsString> {
        cli_args(&self.input, &self.output)
    }

    fn assert_unchanged(&self) {
        assert_eq!(fs::read(&self.input).unwrap(), self.recipe);
        assert_eq!(fs::read(self.temp.join(SOURCE_PATH)).unwrap(), self.source);
    }
}

fn populate(root: &Path, input: &Path, source: &[u8], recipe: &[u8]) {
    fs::create_dir(root.join("inputs")).unwrap();
    fs::write(root.join(SOURCE_PATH), source).unwrap();
    fs::write(input, recipe).unwrap();
}

fn cli_args(input: &Path, output: &Path) -> Vec<OsString> {
    vec![
        "--recipe".into(),
        input.into(),
        "--asset-id".into(),
        RECIPE_ID.into(),
        "--output".into(),
        output.into(),
    ]
}

fn tree(root: &Path) -> Vec<PathBuf> {
    fn visit(root: &Path, current: &Path, paths: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            paths.push(path.strip_prefix(root).unwrap().to_owned());
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                visit(root, &path, paths);
            }
        }
    }
    let mut paths = Vec::new();
    visit(root, root, &mut paths);
    paths.sort();
    paths
}

#[test]
fn cli_publishes_once_without_overwrite_and_preserves_all_inputs() {
    let fixture = CliFixture::new();
    let before = tree(&fixture.temp.0);
    authoring::run(fixture.args()).unwrap();
    let cooked = fs::read(&fixture.output).unwrap();
    let expected = apply(&fixture.recipe, RECIPE_ID, &fixture.source).unwrap();
    assert_eq!(cooked, expected.to_bytes().unwrap());
    assert_source_bits(
        StaticModel::from_bytes(&cooked).unwrap().source(),
        expected.source(),
    );
    fixture.assert_unchanged();
    let after = tree(&fixture.temp.0);
    assert_eq!(after.len(), before.len() + 1);
    assert!(authoring::run(fixture.args()).is_err());
    assert_eq!(fs::read(&fixture.output).unwrap(), cooked);
    assert_eq!(tree(&fixture.temp.0), after);
    // Existing hardlink aliases of either input are also never replaced.
    for (name, target) in [
        ("source-alias.model.json", fixture.temp.join(SOURCE_PATH)),
        ("recipe-alias.model.json", fixture.input.clone()),
    ] {
        let alias = fixture.temp.join(name);
        fs::hard_link(target, &alias).unwrap();
        let snapshot = fs::read(&alias).unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(cli_args(&fixture.input, &alias)).is_err());
        assert_eq!(fs::read(&alias).unwrap(), snapshot);
        assert_eq!(tree(&fixture.temp.0), before);
    }
    fixture.assert_unchanged();
}

#[test]
fn cli_help_and_three_unique_pairs_are_bounded() {
    let fixture = CliFixture::new();
    let before = tree(&fixture.temp.0);
    for option in ["--help", "-h"] {
        authoring::run([OsString::from(option)]).unwrap();
    }
    assert_eq!(tree(&fixture.temp.0), before);
    fixture.assert_unchanged();
    let mut consumed = 0;
    let infinite = std::iter::from_fn(|| {
        consumed += 1;
        assert!(consumed <= 8, "CLI read an unbounded argument stream");
        Some(OsString::from("--unknown"))
    });
    assert!(authoring::run(infinite).is_err());
    assert!(consumed > 6);
    assert_eq!(tree(&fixture.temp.0), before);
    let shuffled = fixture
        .args()
        .chunks(2)
        .rev()
        .flat_map(|pair| pair.iter().cloned())
        .collect::<Vec<_>>();
    authoring::run(shuffled).unwrap();
    assert!(StaticModel::from_bytes(&fs::read(&fixture.output).unwrap()).is_ok());
    fixture.assert_unchanged();
}

#[test]
fn cli_bad_arguments_fail_without_output_or_staging() {
    let fixture = CliFixture::new();
    let good = fixture.args();
    let mut cases = vec![
        vec![],
        good[..5].to_vec(),
        vec!["--help".into(), "extra".into()],
    ];
    for start in [0, 2, 4] {
        let mut missing = good.clone();
        missing.drain(start..start + 2);
        cases.push(missing);
        let mut duplicate = good.clone();
        duplicate.extend_from_slice(&good[start..start + 2]);
        cases.push(duplicate);
        let mut empty = good.clone();
        empty[start + 1] = "".into();
        cases.push(empty);
        let mut long = good.clone();
        long[start + 1] = "x".repeat(4097).into();
        cases.push(long);
    }
    for (index, value) in [
        (0, "--unknown"),
        (0, "--input"),
        (0, "--force"),
        (0, "--stdin"),
        (0, "--install"),
        (0, "--network"),
        (0, "--help"),
        (2, "--recipe"),
        (4, "--asset-id"),
        (3, "../bad.uv.json"),
        (1, "-"),
    ] {
        let mut bad = good.clone();
        bad[index] = value.into();
        cases.push(bad);
    }
    let before = tree(&fixture.temp.0);
    for bad in cases {
        assert!(authoring::run(bad).is_err());
        assert!(!fixture.output.exists());
        assert_eq!(tree(&fixture.temp.0), before);
        fixture.assert_unchanged();
    }
}

#[test]
fn cli_portable_output_names_and_package_store_rejection() {
    let fixture = CliFixture::new();
    let before = tree(&fixture.temp.0);
    for name in [
        "prop.png",
        "prop.json",
        "prop.MODEL.JSON",
        "prop name.model.json",
        "한글.model.json",
        "CON.model.json",
        "prn.model.json",
        "AUX.model.json",
        "nul.model.json",
        "COM1.model.json",
        "lpt9.model.json",
    ] {
        let output = fixture.temp.join(name);
        assert!(authoring::run(cli_args(&fixture.input, &output)).is_err());
        assert!(!output.exists());
        assert_eq!(tree(&fixture.temp.0), before);
    }
    let long = fixture.temp.join(&("x".repeat(100) + ".model.json"));
    assert!(authoring::run(cli_args(&fixture.input, &long)).is_err());
    assert!(!long.exists());
    for name in [".orr.", ".orr ", "ordinary.", "ordinary "] {
        let output = fixture.temp.join(name).join("prop.model.json");
        let error = authoring::run(cli_args(&fixture.input, &output)).unwrap_err();
        assert!(error.contains("ending in a dot or space"), "{error}");
        assert_eq!(tree(&fixture.temp.0), before);
    }
    // Different parents preserve all spellings on case-insensitive filesystems.
    for (index, name) in [".orr", ".ORR", ".OrR"].into_iter().enumerate() {
        let parent = fixture.temp.join(&format!("store-case-{index}"));
        fs::create_dir(&parent).unwrap();
        let store = parent.join(name);
        fs::create_dir(&store).unwrap();
        let output = store.join("prop.model.json");
        let snapshot = tree(&fixture.temp.0);
        assert!(authoring::run(cli_args(&fixture.input, &output)).is_err());
        assert!(!output.exists());
        assert_eq!(tree(&fixture.temp.0), snapshot);
    }
    let missing_parent = fixture.temp.join("missing").join("prop.model.json");
    assert!(authoring::run(cli_args(&fixture.input, &missing_parent)).is_err());
    let file_parent = fixture.temp.join("file-parent");
    fs::write(&file_parent, b"a file cannot be an ancestor").unwrap();
    assert!(authoring::run(cli_args(
        &fixture.input,
        &file_parent.join("prop.model.json")
    ))
    .is_err());
    assert_eq!(
        fs::read(file_parent).unwrap(),
        b"a file cannot be an ancestor"
    );
    fs::create_dir(&fixture.output).unwrap();
    let snapshot = tree(&fixture.temp.0);
    assert!(authoring::run(fixture.args()).is_err());
    assert_eq!(tree(&fixture.temp.0), snapshot);
    fixture.assert_unchanged();
}

#[test]
fn cli_missing_nonregular_and_oversized_inputs_leave_no_output() {
    let fixture = CliFixture::new();
    for (path, original, limit) in [
        (
            fixture.input.clone(),
            fixture.recipe.as_slice(),
            MAX_RECIPE_BYTES,
        ),
        (
            fixture.temp.join(SOURCE_PATH),
            fixture.source.as_slice(),
            orr_model::MAX_FILE_BYTES,
        ),
    ] {
        fs::remove_file(&path).unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(tree(&fixture.temp.0), before);
        assert!(!fixture.output.exists());
        // Directory fixtures reject before opening, without sockets or FIFOs.
        fs::create_dir(&path).unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(tree(&fixture.temp.0), before);
        assert!(!fixture.output.exists());
        fs::remove_dir(&path).unwrap();
        fs::write(&path, b"malformed").unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"malformed");
        assert_eq!(tree(&fixture.temp.0), before);
        let file = fs::File::create(&path).unwrap();
        file.set_len(limit as u64 + 1).unwrap();
        drop(file);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), limit as u64 + 1);
        assert_eq!(tree(&fixture.temp.0), before);
        assert!(!fixture.output.exists());
        fs::write(&path, original).unwrap();
    }
    // Individually bounded source still cannot exceed the remaining budget.
    let path = fixture.temp.join(SOURCE_PATH);
    let file = fs::File::create(&path).unwrap();
    file.set_len((orr_model::MAX_FILE_BYTES - fixture.recipe.len() + 1) as u64)
        .unwrap();
    drop(file);
    let before = tree(&fixture.temp.0);
    let error = authoring::run(fixture.args()).unwrap_err();
    assert!(error.contains("bounded regular file"), "{error}");
    assert!(!fixture.output.exists());
    assert_eq!(tree(&fixture.temp.0), before);
    fs::write(path, &fixture.source).unwrap();
    fixture.assert_unchanged();
}

#[test]
fn cli_resolver_is_anchored_to_recipe_parent() {
    let fixture = CliFixture::new();
    let nested = fixture.temp.join("recipe-parent");
    fs::create_dir(&nested).unwrap();
    let nested_input = nested.join("tile.uv.json");
    let mut nested_source = rich_fixture().source().clone();
    nested_source.materials[0].base_color = [0.25, 0.5, 1.0, 0.75];
    let nested_source = StaticModel::new(nested_source).unwrap().to_bytes().unwrap();
    populate(&nested, &nested_input, &nested_source, &fixture.recipe);
    authoring::run(cli_args(&nested_input, &fixture.output)).unwrap();
    let cooked = fs::read(&fixture.output).unwrap();
    assert_eq!(
        cooked,
        apply(&fixture.recipe, RECIPE_ID, &nested_source)
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    assert_ne!(
        cooked,
        apply(&fixture.recipe, RECIPE_ID, &fixture.source)
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    fixture.assert_unchanged();
    fs::remove_file(&fixture.output).unwrap();
    fs::remove_file(nested.join(SOURCE_PATH)).unwrap();
    let before = tree(&fixture.temp.0);
    assert!(authoring::run(cli_args(&nested_input, &fixture.output)).is_err());
    assert_eq!(tree(&fixture.temp.0), before);
    assert!(
        !fixture.output.exists(),
        "must not fall back to a different input parent"
    );
}

fn raw_parent_path(child: &Path, leaf: &str) -> OsString {
    // PathBuf::join normalizes .. for Windows verbatim roots. Retain the exact
    // CLI spelling and assert that the intended rejection precondition holds.
    let mut raw = child.as_os_str().to_os_string();
    raw.push(std::path::MAIN_SEPARATOR_STR);
    raw.push("..");
    raw.push(std::path::MAIN_SEPARATOR_STR);
    raw.push(leaf);
    assert!(Path::new(&raw)
        .components()
        .any(|part| matches!(part, Component::ParentDir)));
    raw
}

#[test]
fn cli_parent_traversal_preserves_raw_spelling_and_never_publishes() {
    let fixture = CliFixture::new();
    let child = fixture.temp.join("traversal-child");
    fs::create_dir(&child).unwrap();
    // Both normalized counterfactuals stay inside the owned fixture. The input
    // exists, so an incidental NotFound cannot make the rejection test pass.
    assert_eq!(child.parent(), Some(fixture.temp.0.as_path()));
    assert!(fixture.input.exists());
    let raw_input = raw_parent_path(&child, "tile.uv.json");
    let raw_output = raw_parent_path(&child, "tile.model.json");
    for args in [
        cli_args(Path::new(&raw_input), &fixture.output),
        cli_args(&fixture.input, Path::new(&raw_output)),
    ] {
        assert!(Path::new(&args[1])
            .components()
            .chain(Path::new(&args[5]).components())
            .any(|part| matches!(part, Component::ParentDir)));
        let before = tree(&fixture.temp.0);
        let error = authoring::run(args).unwrap_err();
        assert_eq!(error, "empty paths and parent traversal are unsupported");
        assert_eq!(tree(&fixture.temp.0), before);
        assert!(!fixture.output.exists());
    }
    fixture.assert_unchanged();
    assert_eq!(fs::read_dir(child).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn cli_symlink_inputs_ancestors_and_destinations_reject() {
    use std::os::unix::fs::symlink;
    let fixture = CliFixture::new();
    let recipe_alias = fixture.temp.join("linked.uv.json");
    symlink(&fixture.input, &recipe_alias).unwrap();
    let before = tree(&fixture.temp.0);
    assert!(authoring::run(cli_args(&recipe_alias, &fixture.output)).is_err());
    assert_eq!(tree(&fixture.temp.0), before);
    let path = fixture.temp.join(SOURCE_PATH);
    let saved = path.with_extension("saved");
    fs::rename(&path, &saved).unwrap();
    symlink(&saved, &path).unwrap();
    let before = tree(&fixture.temp.0);
    assert!(authoring::run(fixture.args()).is_err());
    assert_eq!(tree(&fixture.temp.0), before);
    fs::remove_file(&path).unwrap();
    fs::rename(&saved, &path).unwrap();
    // A symlinked dependency parent cannot be followed even for a file leaf.
    let path = fixture.temp.join("inputs");
    let saved = fixture.temp.join("inputs-saved");
    fs::rename(&path, &saved).unwrap();
    symlink(&saved, &path).unwrap();
    let before = tree(&fixture.temp.0);
    assert!(authoring::run(fixture.args()).is_err());
    assert_eq!(tree(&fixture.temp.0), before);
    fs::remove_file(&path).unwrap();
    fs::rename(&saved, &path).unwrap();
    for target in [fixture.input.clone(), fixture.temp.join("absent")] {
        symlink(target, &fixture.output).unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(tree(&fixture.temp.0), before);
        assert!(fs::symlink_metadata(&fixture.output)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_file(&fixture.output).unwrap();
    }
    let parent_alias = fixture.temp.join("linked-parent");
    symlink(&fixture.temp.0, &parent_alias).unwrap();
    let before = tree(&fixture.temp.0);
    assert!(authoring::run(cli_args(
        &parent_alias.join("tile.uv.json"),
        &fixture.output
    ))
    .is_err());
    assert!(authoring::run(cli_args(
        &fixture.input,
        &parent_alias.join("tile.model.json")
    ))
    .is_err());
    assert_eq!(tree(&fixture.temp.0), before);
    assert!(!fixture.output.exists());
    fixture.assert_unchanged();
}

#[cfg(unix)]
#[test]
fn cli_non_utf8_option_names_and_logical_ids_reject() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let fixture = CliFixture::new();
    let before = tree(&fixture.temp.0);
    for (index, expected) in [
        (0, "option names must be UTF-8"),
        (3, "--asset-id must be UTF-8"),
    ] {
        let mut bad = fixture.args();
        bad[index] = OsString::from_vec(b"invalid-\xff.uv.json".to_vec());
        assert_eq!(bad[index].as_bytes(), b"invalid-\xff.uv.json");
        assert!(bad[index].to_str().is_none());
        assert_eq!(authoring::run(bad).unwrap_err(), expected);
        assert!(!fixture.output.exists());
        assert_eq!(tree(&fixture.temp.0), before);
    }
    fixture.assert_unchanged();
}

// Arbitrary invalid-UTF8 filesystem names are only a Linux positive fixture;
// macOS may reject their creation before the CLI can be exercised.
#[cfg(target_os = "linux")]
#[test]
fn cli_non_utf8_filesystem_inputs_work() {
    use std::os::unix::ffi::OsStringExt;
    let temp = Temp::new();
    let root = temp.0.join(OsString::from_vec(b"parent-\xff".to_vec()));
    fs::create_dir(&root).unwrap();
    let input = root.join(OsString::from_vec(b"recipe-\xfe.uv.json".to_vec()));
    let output = root.join("tile.model.json");
    let source = rich_fixture().to_bytes().unwrap();
    let recipe = recipe_bytes();
    populate(&root, &input, &source, &recipe);
    assert!(input.to_str().is_none());
    assert!(output.to_str().is_none());
    let before = tree(&temp.0);
    authoring::run(cli_args(&input, &output)).unwrap();
    assert_eq!(
        fs::read(&output).unwrap(),
        apply(&recipe, RECIPE_ID, &source)
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    assert_eq!(fs::read(&input).unwrap(), recipe);
    assert_eq!(fs::read(root.join(SOURCE_PATH)).unwrap(), source);
    assert_eq!(tree(&temp.0).len(), before.len() + 1);
}

#[cfg(windows)]
#[test]
fn cli_windows_nonlocal_prefixes_reject() {
    let fixture = CliFixture::new();
    let before = tree(&fixture.temp.0);
    for remote in [
        r"\\server\share\prop.model.json",
        r"\\?\UNC\server\share\prop.model.json",
        r"\\.\pipe\prop.model.json",
    ] {
        let error = authoring::run(cli_args(&fixture.input, Path::new(remote))).unwrap_err();
        assert!(error.contains("nonlocal Windows path prefixes"), "{error}");
        assert_eq!(tree(&fixture.temp.0), before);
    }
    for remote in [
        r"\\server\share\prop.uv.json",
        r"\\?\UNC\server\share\prop.uv.json",
        r"\\.\pipe\prop.uv.json",
    ] {
        let error = authoring::run(cli_args(Path::new(remote), &fixture.output)).unwrap_err();
        assert!(error.contains("nonlocal Windows path prefixes"), "{error}");
        assert_eq!(tree(&fixture.temp.0), before);
    }
    assert!(!fixture.output.exists());
    fixture.assert_unchanged();
}
