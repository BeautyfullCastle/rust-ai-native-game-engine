#![cfg(feature = "import")]
#![allow(clippy::float_arithmetic)]

#[cfg(unix)]
#[path = "../../../tools/test_support/unix_socket.rs"]
mod unix_socket;

#[path = "../examples/static_model_pivot/authoring.rs"]
mod authoring;

use authoring::rebase_model_pivot;
use orr_model::{Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

const SOURCE_PATH: &str = "inputs/prop.model.json";
const RECIPE_ID: &str = "variants/chair.pivot.json";
const MAX_RECIPE_BYTES: usize = 16 * 1024;

fn recipe() -> Value {
    json!({"format":"orr_static_model_pivot", "version":1,
        "source": SOURCE_PATH, "pivot":[2.0, -3.0, 1.5]})
}
fn recipe_bytes() -> Vec<u8> {
    serde_json::to_vec(&recipe()).unwrap()
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// Independent cooked construction: no importer or authoring transform helper.
// The literal composed matrices deliberately include reflection, nonuniform
// scale, rotation/shear, different origins and signed-zero entries.
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
            [-5.0, 8.0, -9.0, 1.0],
        ],
        [
            [1.0, 0.0, -0.0, 0.0],
            [-0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-0.0, 0.0, -0.0, 1.0],
        ],
    ];
    let primitives = transforms
        .into_iter()
        .enumerate()
        .map(|(i, transform)| Primitive {
            id: format!("props/original.glb#node={}/mesh=8/primitive={i}", i + 17),
            vertices: vertices.clone(),
            indices: vec![0, 1, 2],
            material: (i % 2) as u32,
            transform,
        })
        .collect();
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: "props/original.glb".into(),
        dependencies: vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(b"original model"),
            },
            Dependency {
                uri: "historical/missing.png".into(),
                sha256: digest(b"historical image"),
            },
        ],
        primitives,
        materials: vec![
            Material {
                base_color: [-0.0, 0.5, 0.75, 0.25],
                image: 0,
                linear_filter: true,
                wrap_s: Wrap::Mirror,
                wrap_t: Wrap::Repeat,
            },
            Material {
                base_color: [1.0, 0.0, 0.25, 1.0],
                image: 0,
                linear_filter: false,
                wrap_s: Wrap::Clamp,
                wrap_t: Wrap::Mirror,
            },
            Material {
                base_color: [0.25, 0.5, 0.75, 1.0],
                image: 1,
                linear_filter: true,
                wrap_s: Wrap::Repeat,
                wrap_t: Wrap::Clamp,
            },
        ],
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
fn rich_fixture() -> StaticModel {
    fixture()
}
fn apply(input: &[u8], id: &str, source: &[u8]) -> Result<StaticModel, String> {
    rebase_model_pivot(input, id, |path| {
        assert_eq!(
            path, SOURCE_PATH,
            "only the direct cooked source may resolve"
        );
        Ok(source.to_vec())
    })
}
fn reject_before_resolution(bytes: &[u8], id: &str) {
    assert!(
        rebase_model_pivot(bytes, id, |path| panic!("invalid recipe resolved {path}")).is_err(),
        "accepted {:?} with ID {id:?}",
        String::from_utf8_lossy(bytes)
    );
}
fn assert_bits(a: &[f32], b: &[f32]) {
    assert_eq!(
        a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        b.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
}
fn assert_preserved(before: &StaticModel, after: &StaticModel, origins: &[[f32; 3]]) {
    let old = before.source();
    let new = after.source();
    assert_eq!(new.format, old.format);
    assert_eq!(new.version, old.version);
    assert_eq!(new.asset_id, RECIPE_ID);
    assert_eq!(new.primitives.len(), old.primitives.len());
    assert_eq!(origins.len(), old.primitives.len());
    assert_eq!(new.materials, old.materials);
    assert_eq!(new.images, old.images);
    for (a, b) in old.materials.iter().zip(&new.materials) {
        assert_bits(&a.base_color, &b.base_color);
    }
    for ((a, b), origin) in old.primitives.iter().zip(&new.primitives).zip(origins) {
        assert_eq!(
            b.id,
            format!("{RECIPE_ID}{}", a.id.strip_prefix(&old.asset_id).unwrap())
        );
        assert_eq!(a.vertices, b.vertices);
        assert_eq!(a.indices, b.indices);
        assert_eq!(a.material, b.material);
        for (va, vb) in a.vertices.iter().zip(&b.vertices) {
            assert_bits(&va.position, &vb.position);
            assert_bits(&va.normal, &vb.normal);
            assert_bits(&va.uv, &vb.uv);
        }
        for (old, new) in a.transform[..3].iter().zip(&b.transform[..3]) {
            assert_bits(old, new);
        }
        assert_eq!(a.transform[3][3].to_bits(), b.transform[3][3].to_bits());
        assert_bits(&b.transform[3][..3], origin);
        let old_normal = orr_model::normal_matrix(a.transform).unwrap();
        let new_normal = orr_model::normal_matrix(b.transform).unwrap();
        for (old, new) in old_normal.iter().zip(&new_normal) {
            assert_bits(old, new);
        }
        assert_eq!(
            orr_model::determinant(a.transform).to_bits(),
            orr_model::determinant(b.transform).to_bits()
        );
    }
}

#[test]
fn rebase_is_model_space_translation_for_all_composed_origins_and_preserves_bits() {
    let original = fixture();
    let bytes = original.to_bytes().unwrap();
    let result = apply(&recipe_bytes(), RECIPE_ID, &bytes).unwrap();
    // Independent literal oracle: no production helper or matrix operation.
    assert_preserved(
        &original,
        &result,
        &[[2.0, -4.0, 9.5], [-7.0, 11.0, -10.5], [-2.0, 3.0, -1.5]],
    );
    assert!(orr_model::determinant(result.source().primitives[0].transform) < 0.0);
    assert_eq!(original.to_bytes().unwrap(), bytes);
}

#[test]
fn zero_pivots_preserve_positive_and_negative_zero_translation_bits() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    for zeros in ["[0,0,0]", "[-0.0,0.0,-0.0]", "[0.0,-0.0,0.0]"] {
        let input = format!(
            r#"{{"format":"orr_static_model_pivot","version":1,"source":"{SOURCE_PATH}","pivot":{zeros}}}"#
        );
        let result = apply(input.as_bytes(), RECIPE_ID, &source).unwrap();
        assert_preserved(
            &original,
            &result,
            &[[4.0, -7.0, 11.0], [-5.0, 8.0, -9.0], [-0.0, 0.0, -0.0]],
        );
    }
    // An individually zero axis remains an identity even while other axes move.
    let mut input = recipe();
    input["pivot"] = json!([0.0, 2.0, -0.0]);
    let result = apply(&serde_json::to_vec(&input).unwrap(), RECIPE_ID, &source).unwrap();
    assert_preserved(
        &original,
        &result,
        &[[4.0, -9.0, 11.0], [-5.0, 6.0, -9.0], [-0.0, -2.0, -0.0]],
    );
}

#[test]
fn pivot_numbers_quantize_to_f32_and_subtraction_uses_f32() {
    let mut source = fixture().source().clone();
    for p in &mut source.primitives {
        p.transform[3] = [0.3, 0.0, -0.0, 1.0];
    }
    let original = StaticModel::new(source).unwrap();
    let input = format!(
        r#"{{"format":"orr_static_model_pivot","version":1,"source":"{SOURCE_PATH}","pivot":[0.1,-0.2,1e-45]}}"#
    );
    let result = apply(input.as_bytes(), RECIPE_ID, &original.to_bytes().unwrap()).unwrap();
    let origin = [f32::from_bits(0x3e4cccce), 0.2, f32::from_bits(0x80000001)];
    assert_preserved(&original, &result, &[origin; 3]);
    // Bounds apply to the declared f32 representation, including decimal rounding.
    let mut input = recipe();
    input["pivot"] = json!([1000000.01, -1000000.01, 0]);
    let mut zero = fixture().source().clone();
    for p in &mut zero.primitives {
        p.transform[3] = [0.0, 0.0, 0.0, 1.0];
    }
    let original = StaticModel::new(zero).unwrap();
    let result = apply(
        &serde_json::to_vec(&input).unwrap(),
        RECIPE_ID,
        &original.to_bytes().unwrap(),
    )
    .unwrap();
    assert_preserved(&original, &result, &[[-1000000.0, 1000000.0, 0.0]; 3]);
}

#[test]
fn translation_and_pivot_bounds_reject_overflow_without_saturation() {
    for axis in 0..3 {
        for sign in [-1.0, 1.0] {
            let mut source = fixture().source().clone();
            source.primitives[1].transform[3][axis] = sign * 1000000.0;
            let original = StaticModel::new(source).unwrap();
            let mut input = recipe();
            input["pivot"] = json!([0, 0, 0]);
            input["pivot"][axis] = json!(-sign);
            let error = apply(
                &serde_json::to_vec(&input).unwrap(),
                RECIPE_ID,
                &original.to_bytes().unwrap(),
            )
            .unwrap_err();
            assert!(error.contains("transform bounds"), "{error}");
            let mut source = original.source().clone();
            source.primitives[1].transform[3][axis] = sign * 999999.0;
            assert!(apply(
                &serde_json::to_vec(&input).unwrap(),
                RECIPE_ID,
                &StaticModel::new(source).unwrap().to_bytes().unwrap()
            )
            .is_ok());
        }
    }
    for pivot in [
        "[1000000.1,0,0]",
        "[-1000000.1,0,0]",
        "[0,1e39,0]",
        "[0,0,-1e39]",
        "[NaN,0,0]",
        "[Infinity,0,0]",
        "[1e400,0,0]",
    ] {
        let input = format!(
            r#"{{"format":"orr_static_model_pivot","version":1,"source":"{SOURCE_PATH}","pivot":{pivot}}}"#
        );
        reject_before_resolution(input.as_bytes(), RECIPE_ID);
    }
}

#[test]
fn rebase_revalidates_world_vertex_bounds_after_translation() {
    let mut source = fixture().source().clone();
    let p = &mut source.primitives[0];
    p.transform = [
        [1000000.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    for vertex in &mut p.vertices {
        vertex.position[0] = 1000.0;
    }
    let source = StaticModel::new(source).unwrap().to_bytes().unwrap();
    let mut input = recipe();
    input["pivot"] = json!([-128.0, 0.0, 0.0]);
    let error = apply(&serde_json::to_vec(&input).unwrap(), RECIPE_ID, &source).unwrap_err();
    assert!(
        error.contains("rebased model validation") && error.contains("world vertex"),
        "{error}"
    );
}

#[test]
fn recipe_accepts_exact_object_whitespace_and_16kib_boundary() {
    let source = fixture().to_bytes().unwrap();
    let mut spaced = b" \n\t".to_vec();
    spaced.extend(serde_json::to_vec_pretty(&recipe()).unwrap());
    spaced.extend(b"\n ");
    let mut boundary = recipe_bytes();
    boundary.resize(MAX_RECIPE_BYTES, b' ');
    for bytes in [recipe_bytes(), spaced, boundary] {
        let result = apply(&bytes, RECIPE_ID, &source).unwrap();
        assert_eq!(result.source().dependencies[0].sha256, digest(&bytes));
    }
}

#[test]
fn recipe_rejects_missing_duplicate_unknown_null_and_positional_aliases() {
    let original = recipe();
    for field in ["format", "version", "source", "pivot"] {
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
    for field in ["center", "translation", "overwrite", "Pivot", "asset_id"] {
        let mut unknown = original.clone();
        unknown[field] = json!(false);
        reject_before_resolution(&serde_json::to_vec(&unknown).unwrap(), RECIPE_ID);
    }
    for alias in [
        json!(["orr_static_model_pivot", 1, SOURCE_PATH, [0, 0, 0]]),
        json!([["format", "orr_static_model_pivot"]]),
        json!([]),
        json!(null),
        json!(true),
        json!(1),
        json!("recipe"),
    ] {
        reject_before_resolution(&serde_json::to_vec(&alias).unwrap(), RECIPE_ID);
    }
    for bytes in [b"".as_slice(), b"{}{}", b"{", b"\xff", b"{} trailing"] {
        reject_before_resolution(bytes, RECIPE_ID);
    }
}

#[test]
fn recipe_rejects_wrong_types_versions_pivot_lengths_and_byte_budget() {
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
        ("source", json!({"path":SOURCE_PATH})),
        ("pivot", json!([])),
        ("pivot", json!([0, 0])),
        ("pivot", json!([0, 0, 0, 0])),
        ("pivot", json!(["0", 0, 0])),
        ("pivot", json!([false, 0, 0])),
        ("pivot", json!([null, 0, 0])),
        ("pivot", json!([[0], 0, 0])),
        ("pivot", json!({"x":0,"y":0,"z":0})),
        ("pivot", json!(0)),
    ] {
        let mut bad = recipe();
        bad[field] = value;
        reject_before_resolution(&serde_json::to_vec(&bad).unwrap(), RECIPE_ID);
    }
    let mut large = recipe_bytes();
    large.resize(MAX_RECIPE_BYTES + 1, b' ');
    reject_before_resolution(&large, RECIPE_ID);
}

#[test]
fn logical_paths_reject_nonportable_unnormalized_suffixes_and_aliases() {
    for (field, suffix) in [("source", ".model.json"), ("asset_id", ".pivot.json")] {
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
            format!("COM1{suffix}"),
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
    let mut bad = recipe();
    bad["source"] = json!(RECIPE_ID);
    reject_before_resolution(&serde_json::to_vec(&bad).unwrap(), RECIPE_ID);
}

#[test]
fn logical_paths_accept_240_byte_boundary() {
    let source_path = format!("{}.model.json", "a".repeat(240 - ".model.json".len()));
    let id = format!("{}.pivot.json", "b".repeat(240 - ".pivot.json".len()));
    let mut input = recipe();
    input["source"] = json!(source_path);
    let source = fixture().to_bytes().unwrap();
    let mut calls = 0;
    let model = rebase_model_pivot(&serde_json::to_vec(&input).unwrap(), &id, |path| {
        calls += 1;
        assert_eq!(path, source_path);
        Ok(source.clone())
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(model.source().asset_id, id);
}

#[test]
fn source_resolves_once_no_historical_dependencies_and_errors_propagate() {
    let source = fixture().to_bytes().unwrap();
    let mut calls = Vec::new();
    rebase_model_pivot(&recipe_bytes(), RECIPE_ID, |path| {
        calls.push(path.to_owned());
        Ok(source.clone())
    })
    .unwrap();
    assert_eq!(calls, [SOURCE_PATH]);
    let error = rebase_model_pivot(&recipe_bytes(), RECIPE_ID, |path| {
        Err(format!("resolver-denied-{path}"))
    })
    .unwrap_err();
    assert_eq!(error, format!("resolver-denied-{SOURCE_PATH}"));
}

#[test]
fn entire_cooked_source_is_admitted_before_transforming() {
    let good = fixture();
    let mut invalid = vec![
        b"not cooked".to_vec(),
        include_bytes!("fixtures/model.glb").to_vec(),
    ];
    for (pointer, bad) in [
        ("/primitives/1/indices/0", json!(u32::MAX)),
        ("/primitives/1/material", json!(100)),
        ("/primitives/1/transform/0", json!([0, 0, 0, 0])),
        ("/primitives/1/transform/3/0", json!(1000001)),
        ("/images/1/width", json!(0)),
        ("/materials/2/image", json!(u32::MAX)),
        ("/materials/2/base_color/0", json!(2)),
        ("/dependencies/0/sha256", json!("not-a-digest")),
        ("/version", json!(2)),
    ] {
        let mut doc = serde_json::to_value(good.source()).unwrap();
        *doc.pointer_mut(pointer).unwrap() = bad;
        invalid.push(serde_json::to_vec(&doc).unwrap());
    }
    for source in invalid {
        assert!(StaticModel::from_bytes(&source).is_err());
        let mut calls = 0;
        let error = rebase_model_pivot(&recipe_bytes(), RECIPE_ID, |path| {
            calls += 1;
            assert_eq!(path, SOURCE_PATH);
            Ok(source.clone())
        })
        .unwrap_err();
        assert_eq!(calls, 1);
        assert!(error.starts_with("source model:"), "{error}");
    }
}

#[test]
fn provenance_binds_exact_bytes_and_roundtrip_is_deterministic() {
    let original = fixture();
    let mut source = original.to_bytes().unwrap();
    source.extend(b"\n  ");
    let mut input = recipe_bytes();
    input.extend(b"\n");
    let first = apply(&input, RECIPE_ID, &source).unwrap();
    assert_eq!(
        first.source().dependencies,
        [
            Dependency {
                uri: "$source".into(),
                sha256: digest(&input)
            },
            Dependency {
                uri: SOURCE_PATH.into(),
                sha256: digest(&source)
            }
        ]
    );
    let cooked = first.to_bytes().unwrap();
    assert_eq!(
        cooked,
        apply(&input, RECIPE_ID, &source)
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    let reloaded = StaticModel::from_bytes(&cooked).unwrap();
    assert_eq!(reloaded.source(), first.source());
    assert_eq!(reloaded.to_bytes().unwrap(), cooked);
    assert_ne!(digest(&source), digest(&original.to_bytes().unwrap()));
    assert_ne!(digest(&input), digest(&recipe_bytes()));
    let other_id = "variants/other.pivot.json";
    let other = apply(&input, other_id, &source).unwrap();
    let mut expected = first.source().clone();
    expected.asset_id = other_id.into();
    for p in &mut expected.primitives {
        p.id = format!("{other_id}{}", p.id.strip_prefix(RECIPE_ID).unwrap());
    }
    assert_eq!(other.source(), &expected);
    assert_ne!(other.to_bytes().unwrap(), cooked);
}

// Keep large allocations in one test to bound parallel hosted-runner memory.
#[test]
fn combined_input_and_independent_cooked_output_budgets_are_enforced() {
    let input = recipe_bytes();
    {
        let oversized = vec![b' '; orr_model::MAX_FILE_BYTES + 1];
        assert!(apply(&input, RECIPE_ID, &oversized)
            .unwrap_err()
            .contains("64 MiB"));
    }
    {
        let mut source = fixture().to_bytes().unwrap();
        source.resize(orr_model::MAX_FILE_BYTES - input.len(), b' ');
        assert!(StaticModel::from_bytes(&source).is_ok());
        assert!(apply(&input, RECIPE_ID, &source).is_ok());
        source.push(b' ');
        assert!(apply(&input, RECIPE_ID, &source)
            .unwrap_err()
            .contains("64 MiB"));
    }
    {
        // Size an independently constructed image fixture so the admitted source
        // barely fits the combined input limit. Each 255 -> 0 replaces three
        // JSON digits with one; no production authoring helper builds this data.
        let mut document = fixture().source().clone();
        document.images = vec![Image {
            width: 2048,
            height: 2048,
            rgba8: vec![255; 2048 * 2048 * 4],
        }];
        for m in &mut document.materials {
            m.image = 0;
        }
        let oversized = serde_json::to_vec(&document).unwrap().len();
        assert!(oversized > orr_model::MAX_FILE_BYTES);
        let remove_digits = oversized + input.len() - orr_model::MAX_FILE_BYTES;
        let zero_count = remove_digits.div_ceil(2) + 1;
        document.images[0].rgba8[..zero_count].fill(0);
        let source = StaticModel::new(document).unwrap().to_bytes().unwrap();
        assert!(source.len() + input.len() <= orr_model::MAX_FILE_BYTES);
        assert!(orr_model::MAX_FILE_BYTES - source.len() - input.len() <= 3);
        let id = format!("{}.pivot.json", "z".repeat(240 - ".pivot.json".len()));
        let error = apply(&input, &id, &source).unwrap_err();
        assert!(error.contains("cook") && error.contains("byte"), "{error}");
    }
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
            "orr-static-pivot-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    #[cfg(unix)]
    fn socket() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        // Canonical per-user temporary roots can exceed macOS's socket path
        // limit. Only socket fixtures use this short root, with physical
        // ancestry even where /tmp itself is a symlink.
        let root = Path::new("/tmp").canonicalize().unwrap();
        assert!(root.is_absolute());
        for ancestor in root.ancestors() {
            let metadata = fs::symlink_metadata(ancestor).unwrap();
            assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
        }
        let path = root.join(format!(
            "orr-static-pivot-socket-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed),
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
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
        Self::with_temp(Temp::new())
    }

    fn with_temp(temp: Temp) -> Self {
        let input = temp.join("chair.pivot.json");
        let output = temp.join("chair.model.json");
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
    assert_eq!(
        StaticModel::from_bytes(&cooked).unwrap().source(),
        expected.source()
    );
    fixture.assert_unchanged();
    let after = tree(&fixture.temp.0);
    assert_eq!(after.len(), before.len() + 1);
    assert!(authoring::run(fixture.args()).is_err());
    assert_eq!(fs::read(&fixture.output).unwrap(), cooked);
    assert_eq!(tree(&fixture.temp.0), after);
    // An existing hardlink alias of either source is also never replaced.
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
    authoring::run([OsString::from("--help")]).unwrap();
    assert!(authoring::run([OsString::from("-h")]).is_err());
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
        (3, "../bad.pivot.json"),
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
    // Different parents preserve both spellings on case-insensitive filesystems.
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
        // Directory fixtures exercise nonregular rejection without socket or
        // FIFO setup and without opening a potentially blocking input.
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
    fixture.assert_unchanged();
}

#[test]
fn cli_resolver_is_anchored_to_recipe_parent() {
    let fixture = CliFixture::new();
    let nested = fixture.temp.join("recipe-parent");
    fs::create_dir(&nested).unwrap();
    let nested_input = nested.join("chair.pivot.json");
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
    // PathBuf::join normalizes .. for Windows verbatim roots. Construct the CLI
    // spelling directly, then assert the precondition that traversal is present.
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
    // Both normalized counterfactuals stay inside this owned fixture, and the
    // normalized source exists, so incidental NotFound cannot make this pass.
    assert!(fixture.input.exists());
    let raw_input = raw_parent_path(&child, "chair.pivot.json");
    let raw_output = raw_parent_path(&child, "chair.model.json");
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
    let recipe_alias = fixture.temp.join("linked.pivot.json");
    symlink(&fixture.input, &recipe_alias).unwrap();
    let before = tree(&fixture.temp.0);
    assert!(authoring::run(cli_args(&recipe_alias, &fixture.output)).is_err());
    assert_eq!(tree(&fixture.temp.0), before);
    {
        let relative = SOURCE_PATH;
        let path = fixture.temp.join(relative);
        let saved = path.with_extension("saved");
        fs::rename(&path, &saved).unwrap();
        symlink(&saved, &path).unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(tree(&fixture.temp.0), before);
        fs::remove_file(&path).unwrap();
        fs::rename(&saved, &path).unwrap();
    }
    // Symlinked dependency parents cannot be followed even if the leaf is a file.
    {
        let directory = "inputs";
        let path = fixture.temp.join(directory);
        let saved = fixture.temp.join(&format!("{directory}-saved"));
        fs::rename(&path, &saved).unwrap();
        symlink(&saved, &path).unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(tree(&fixture.temp.0), before);
        fs::remove_file(&path).unwrap();
        fs::rename(&saved, &path).unwrap();
    }
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
        &parent_alias.join("chair.pivot.json"),
        &fixture.output
    ))
    .is_err());
    assert!(authoring::run(cli_args(
        &fixture.input,
        &parent_alias.join("chair.model.json")
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
        bad[index] = OsString::from_vec(b"invalid-\xff.pivot.json".to_vec());
        assert_eq!(bad[index].as_bytes(), b"invalid-\xff.pivot.json");
        assert!(bad[index].to_str().is_none());
        assert_eq!(authoring::run(bad).unwrap_err(), expected);
        assert!(!fixture.output.exists());
        assert_eq!(tree(&fixture.temp.0), before);
    }
    fixture.assert_unchanged();
}

// Arbitrary invalid-UTF8 filenames are supported by the Linux CI filesystem.
// macOS filesystems can reject them before the CLI is invoked; that capability
// is not assumed or reported as passing by the portable logical-ID test above.
#[cfg(target_os = "linux")]
#[test]
fn cli_non_utf8_filesystem_inputs_work() {
    use std::os::unix::ffi::OsStringExt;
    let temp = Temp::new();
    let root = temp.0.join(OsString::from_vec(b"parent-\xff".to_vec()));
    fs::create_dir(&root).unwrap();
    let input = root.join(OsString::from_vec(b"recipe-\xfe.pivot.json".to_vec()));
    let output = root.join("chair.model.json");
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
        r"\\server\share\prop.pivot.json",
        r"\\?\UNC\server\share\prop.pivot.json",
        r"\\.\pipe\prop.pivot.json",
    ] {
        let error = authoring::run(cli_args(Path::new(remote), &fixture.output)).unwrap_err();
        assert!(error.contains("nonlocal Windows path prefixes"), "{error}");
        assert_eq!(tree(&fixture.temp.0), before);
    }
    assert!(!fixture.output.exists());
    fixture.assert_unchanged();
}

#[cfg(unix)]
#[test]
fn cli_unix_socket_inputs_reject_without_opening_or_publishing() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = CliFixture::with_temp(Temp::socket());
    let inputs = [
        (fixture.input.clone(), fixture.recipe.as_slice()),
        (fixture.temp.join(SOURCE_PATH), fixture.source.as_slice()),
    ];
    for (path, _) in &inputs {
        assert!(
            path.as_os_str().as_bytes().len() <= 100,
            "socket fixture path exceeds the conservative Unix limit: {path:?}"
        );
    }
    for (path, original) in inputs {
        fs::remove_file(&path).unwrap();
        // Capability errors are real test failures; there is no runtime skip or
        // new ignored test. A restricted executor must record an explicit local
        // exception rather than count socket coverage as passed.
        let listener = unix_socket::SocketFixture::create(&path);
        let before = tree(&fixture.temp.0);
        let error = authoring::run(fixture.args()).unwrap_err();
        assert!(error.contains("bounded regular file"), "{error}");
        assert!(!fixture.output.exists());
        assert_eq!(tree(&fixture.temp.0), before);
        drop(listener);
        fs::remove_file(&path).unwrap();
        fs::write(&path, original).unwrap();
    }
    fixture.assert_unchanged();
}
