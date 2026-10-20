#![cfg(feature = "import")]
#![allow(clippy::float_arithmetic)]

// Filesystem entrypoint is exercised by the real executable, not this module copy.
#[allow(dead_code)]
#[path = "../examples/static_model_planar_uv/authoring.rs"]
mod authoring;

use authoring::project_model_uv;
use orr_model::{Dependency, Image, ModelSource, StaticModel, Wrap};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SOURCE: &str = "inputs/prop.model.json";
const ID: &str = "variants/tile.planar-uv.json";
const POSITIONS: [[f32; 3]; 5] = [
    [1., 2., 3.],
    [-2., 4., -1.],
    [0., -1., 2.],
    [3., 0., -2.],
    [-1., -2., 0.],
];

fn recipe() -> Value {
    json!({"format":"orr_static_planar_uv","version":1,"source":SOURCE,
        "material":0,"plane":"xy","origin":[1.,-2.,3.],"units_per_repeat":[2.,4.]})
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn fixture() -> StaticModel {
    let mut source = orr_model::import::import_with_resolver(
        "props/original.glb",
        include_bytes!("fixtures/model.glb"),
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
    source.primitives[0].transform = orr_model::IDENTITY;
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
fn apply(recipe: &[u8], id: &str, source: &[u8]) -> Result<StaticModel, String> {
    let mut calls = 0;
    let result = project_model_uv(recipe, id, |path| {
        calls += 1;
        assert_eq!(calls, 1);
        assert_eq!(path, SOURCE, "old provenance must never be chased");
        Ok(source.to_vec())
    });
    assert_eq!(calls, 1);
    result
}
fn reject_recipe(value: &Value) {
    reject_bytes(&serde_json::to_vec(value).unwrap(), ID);
}
fn reject_bytes(bytes: &[u8], id: &str) {
    assert!(
        project_model_uv(bytes, id, |path| panic!("invalid recipe resolved {path}")).is_err(),
        "accepted recipe {:?}, identity {id}",
        String::from_utf8_lossy(bytes)
    );
}
fn identity(source: &StaticModel, recipe: &[u8], bytes: &[u8], id: &str) -> ModelSource {
    let mut expected = source.source().clone();
    expected.asset_id = id.into();
    for p in &mut expected.primitives {
        let suffix = p.id.strip_prefix(&source.source().asset_id).unwrap();
        assert!(suffix.starts_with("#node="));
        p.id = format!("{id}{suffix}");
    }
    expected.dependencies = vec![
        Dependency {
            uri: "$source".into(),
            sha256: digest(recipe),
        },
        Dependency {
            uri: SOURCE.into(),
            sha256: digest(bytes),
        },
    ];
    expected
}
fn assert_bits(actual: &ModelSource, expected: &ModelSource) {
    assert_eq!(
        serde_json::to_vec(actual).unwrap(),
        serde_json::to_vec(expected).unwrap()
    );
    for (a, e) in actual.primitives.iter().zip(&expected.primitives) {
        assert_eq!(
            a.transform.map(|c| c.map(f32::to_bits)),
            e.transform.map(|c| c.map(f32::to_bits))
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

#[test]
fn all_planes_use_model_positions_and_preserve_unrelated_bits_and_direct_provenance() {
    // Independent arithmetic oracle. Transformed model positions are:
    // (9,-10,14), (-1,-19,8), (6,-2,9), (8,-2,2), (2,0,4).
    // Subtract origin (1,-2,3), then divide selected axes by (2,4).
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
    let mut source = original.to_bytes().unwrap();
    source.extend(b"\n  ");
    for (plane, local, transformed) in cases {
        let mut r = recipe();
        r["plane"] = json!(plane);
        let mut input = serde_json::to_vec_pretty(&r).unwrap();
        input.extend(b"\n");
        let result = apply(&input, ID, &source).unwrap();
        let mut expected = identity(&original, &input, &source, ID);
        for (index, uvs) in [(0, local), (2, transformed)] {
            for (v, uv) in expected.primitives[index].vertices.iter_mut().zip(uvs) {
                v.uv = uv;
            }
        }
        assert_bits(result.source(), &expected);
        // Identical all-zero input UVs become different coordinates: not an affine UV transform.
        assert_ne!(
            result.source().primitives[0].vertices[0].uv,
            result.source().primitives[0].vertices[1].uv
        );
        let cooked = result.to_bytes().unwrap();
        assert_eq!(
            cooked,
            apply(&input, ID, &source).unwrap().to_bytes().unwrap()
        );
        assert_bits(
            StaticModel::from_bytes(&cooked).unwrap().source(),
            &expected,
        );
        let other = "variants/other.planar-uv.json";
        let alternate = apply(&input, other, &source).unwrap();
        let mut alternate_expected = expected.clone();
        alternate_expected.asset_id = other.into();
        for p in &mut alternate_expected.primitives {
            p.id = p.id.replacen(ID, other, 1);
        }
        assert_bits(alternate.source(), &alternate_expected);
    }
    assert_eq!(original.to_bytes().unwrap(), source[..source.len() - 3]);
}

#[test]
fn strict_schema_paths_and_numeric_recipe_admission_are_bounded() {
    let original = recipe();
    for field in [
        "format",
        "version",
        "source",
        "material",
        "plane",
        "origin",
        "units_per_repeat",
    ] {
        let mut r = original.clone();
        r.as_object_mut().unwrap().remove(field);
        reject_recipe(&r);
        let mut r = original.clone();
        r[field] = Value::Null;
        reject_recipe(&r);
        let mut duplicate = serde_json::to_string(&original).unwrap();
        duplicate.pop();
        duplicate.push_str(&format!(",{field:?}:{} }}", original[field]));
        reject_bytes(duplicate.as_bytes(), ID);
    }
    for (field, value) in [
        ("unknown", json!(false)),
        ("format", json!("orr_static_material_uv")),
        ("version", json!(2)),
        ("version", json!(1.0)),
        ("material", json!(-1)),
        ("material", json!(0.0)),
        ("material", json!(4294967296u64)),
        ("plane", json!("XY")),
        ("plane", json!("zx")),
        ("plane", json!(["xy"])),
        ("origin", json!([0, 0])),
        ("origin", json!([0, 0, 0, 0])),
        ("origin", json!([0, "0", 0])),
        ("units_per_repeat", json!([1])),
        ("units_per_repeat", json!([1, 1, 1])),
        ("units_per_repeat", json!([0, 1])),
        ("units_per_repeat", json!([1, -0.0])),
        ("units_per_repeat", json!([-1, 1])),
        ("units_per_repeat", json!([1e-100, 1])),
    ] {
        let mut r = original.clone();
        r[field] = value;
        reject_recipe(&r);
    }
    for raw in [
        b"".as_slice(),
        b"{}{}",
        b"[]",
        b"null",
        b"true",
        b"\xff",
        b"{} trailing",
    ] {
        reject_bytes(raw, ID);
    }
    let sequence = json!([
        "orr_static_planar_uv",
        1,
        SOURCE,
        0,
        "xy",
        [0, 0, 0],
        [1, 1]
    ]);
    reject_recipe(&sequence);
    for (field, values) in [
        ("origin", "1e39,0,0"),
        ("origin", "0,-1e400,0"),
        ("origin", "NaN,0,0"),
        ("units_per_repeat", "1,Infinity"),
        ("units_per_repeat", "1e39,1"),
    ] {
        let mut r = original.clone();
        r.as_object_mut().unwrap().remove(field);
        let mut raw = serde_json::to_string(&r).unwrap();
        raw.pop();
        raw.push_str(&format!(",{field:?}:[{values}]}}"));
        reject_bytes(raw.as_bytes(), ID);
    }
    for suffix in [".model.json", ".planar-uv.json"] {
        for path in [
            format!("../p{suffix}"),
            format!("/p{suffix}"),
            format!("a//p{suffix}"),
            format!("a/./p{suffix}"),
            format!("a\\p{suffix}"),
            format!("C:p{suffix}"),
            format!("a%20/p{suffix}"),
            format!("a#b/p{suffix}"),
            format!("a b/p{suffix}"),
            format!(".hidden/p{suffix}"),
            format!("CON{suffix}"),
            format!("NUL/p{suffix}"),
            format!("{}{}", "a".repeat(241 - suffix.len()), suffix),
            "p.wrong".into(),
            String::new(),
        ] {
            if suffix == ".model.json" {
                let mut r = original.clone();
                r["source"] = json!(path);
                reject_recipe(&r);
            } else {
                reject_bytes(&serde_json::to_vec(&original).unwrap(), &path);
            }
        }
    }
    let mut oversized = serde_json::to_vec(&original).unwrap();
    oversized.resize(16 * 1024 + 1, b' ');
    reject_bytes(&oversized, ID);
    oversized.pop();
    assert!(apply(&oversized, ID, &fixture().to_bytes().unwrap()).is_ok());
    // Extreme finite values are recipe-valid; resolver errors must be preserved.
    for density in [[f32::from_bits(1), f32::MAX], [1., 1.]] {
        let mut r = original.clone();
        r["units_per_repeat"] = json!(density);
        r["origin"] = json!([f32::MAX, -f32::MAX, 0.]);
        assert_eq!(
            project_model_uv(&serde_json::to_vec(&r).unwrap(), ID, |_| Err(
                "resolver failure".into()
            ))
            .unwrap_err(),
            "resolver failure"
        );
    }
}

#[test]
fn full_source_admission_precedes_selection_and_binds_only_direct_inputs() {
    let original = fixture();
    let bytes = original.to_bytes().unwrap();
    for material in [2, u32::MAX] {
        let mut r = recipe();
        r["material"] = json!(material);
        assert!(apply(&serde_json::to_vec(&r).unwrap(), ID, &bytes)
            .unwrap_err()
            .contains("selected material must exist and be used"));
    }
    let mut invalid = vec![
        b"not a cooked model".to_vec(),
        include_bytes!("fixtures/model.glb").to_vec(),
    ];
    for (pointer, value) in [
        ("/primitives/1/indices/0", json!(u32::MAX)),
        ("/primitives/1/material", json!(99)),
        ("/primitives/1/vertices/0/uv/0", json!(65537)),
        ("/primitives/1/vertices/0/normal", json!([0, 0, 0])),
        ("/primitives/1/transform/0", json!([0, 0, 0, 0])),
        ("/materials/2/image", json!(u32::MAX)),
        ("/images/2/width", json!(0)),
        ("/dependencies/0/sha256", json!("bad")),
        (
            "/primitives/1/id",
            json!("foreign.glb#node=1/mesh=0/primitive=1"),
        ),
        ("/version", json!(2)),
    ] {
        let mut document = serde_json::to_value(original.source()).unwrap();
        *document.pointer_mut(pointer).unwrap() = value;
        invalid.push(serde_json::to_vec(&document).unwrap());
    }
    for bad in invalid {
        assert!(StaticModel::from_bytes(&bad).is_err());
        let mut errors = Vec::new();
        for material in [0, u32::MAX] {
            let mut r = recipe();
            r["material"] = json!(material);
            let error = apply(&serde_json::to_vec(&r).unwrap(), ID, &bad).unwrap_err();
            assert!(error.starts_with("source model:"), "{error}");
            errors.push(error);
        }
        assert_eq!(
            errors[0], errors[1],
            "selection must not hide invalid unselected content"
        );
    }
    let mut r = recipe();
    r["material"] = json!(1);
    let input = serde_json::to_vec(&r).unwrap();
    let result = apply(&input, ID, &bytes).unwrap();
    let mut expected = identity(&original, &input, &bytes, ID);
    for (expected_vertex, actual_vertex) in expected.primitives[1]
        .vertices
        .iter_mut()
        .zip(&result.source().primitives[1].vertices)
    {
        expected_vertex.uv = actual_vertex.uv;
    }
    assert_bits(result.source(), &expected);
}

#[test]
fn projection_limits_reject_before_rounding_and_total_input_budget_is_bounded() {
    let mut source = fixture().source().clone();
    for p in &mut source.primitives {
        if p.material == 0 {
            p.transform = orr_model::IDENTITY;
            for v in &mut p.vertices {
                v.position = [65536., -65536., 0.];
            }
        }
    }
    let bytes = StaticModel::new(source).unwrap().to_bytes().unwrap();
    let mut r = recipe();
    r["origin"] = json!([0, 0, 0]);
    r["units_per_repeat"] = json!([1, 1]);
    let boundary = apply(&serde_json::to_vec(&r).unwrap(), ID, &bytes).unwrap();
    for p in &boundary.source().primitives {
        if p.material == 0 {
            for v in &p.vertices {
                assert_eq!(v.uv, [65536., -65536.]);
            }
        }
    }
    // 2^-10 is below half a UV ULP here. A premature f32 cast would round
    // back onto the valid boundary and incorrectly admit these projections.
    for origin in [
        [-0.0009765625, 0., 0.],
        [0., 0.0009765625, 0.],
        [f32::MAX, 0., 0.],
    ] {
        r["origin"] = json!(origin);
        assert!(apply(&serde_json::to_vec(&r).unwrap(), ID, &bytes).is_err());
    }
    r["origin"] = json!([0, 0, 0]);
    r["units_per_repeat"] = json!([f32::from_bits(1), 1.]);
    assert!(apply(&serde_json::to_vec(&r).unwrap(), ID, &bytes).is_err());
    // f64 evaluation retains the unit between cancelling 10^12 terms;
    // f32 evaluation would silently produce zero. Source admission permits
    // this affine transform and its (f32-evaluated) bounded world coordinates.
    let mut cancellation = fixture().source().clone();
    for p in &mut cancellation.primitives {
        if p.material == 0 {
            p.transform = [
                [1e6, 0., 0., 0.],
                [1., 1., 0., 0.],
                [-1e6, 0., 1., 0.],
                [0., 0., 0., 1.],
            ];
            for v in &mut p.vertices {
                v.position = [1e6, 1., 1e6];
            }
        }
    }
    let cancellation = StaticModel::new(cancellation).unwrap().to_bytes().unwrap();
    r["units_per_repeat"] = json!([1, 1]);
    let result = apply(&serde_json::to_vec(&r).unwrap(), ID, &cancellation).unwrap();
    for p in &result.source().primitives {
        if p.material == 0 {
            for v in &p.vertices {
                assert_eq!(v.uv, [1., 1.]);
            }
        }
    }
    let input = serde_json::to_vec(&recipe()).unwrap();
    let mut padded = fixture().to_bytes().unwrap();
    padded.resize(orr_model::MAX_FILE_BYTES - input.len() + 1, b' ');
    let error = apply(&input, ID, &padded).unwrap_err();
    assert!(
        error.contains("byte limit") || error.contains("budget"),
        "{error}"
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod cli {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
    };
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let root = std::env::temp_dir().canonicalize().unwrap();
            let path = root.join(format!("orr-planar-uv-cli-{}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tree(root: &Path) -> Vec<PathBuf> {
        fn visit(root: &Path, current: &Path, out: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(current).unwrap() {
                let path = entry.unwrap().path();
                out.push(path.strip_prefix(root).unwrap().to_owned());
                let metadata = fs::symlink_metadata(&path).unwrap();
                if metadata.is_dir() && !metadata.file_type().is_symlink() {
                    visit(root, &path, out);
                }
            }
        }
        let mut out = Vec::new();
        visit(root, root, &mut out);
        out.sort();
        out
    }
    #[test]
    #[ignore = "requires explicit freshly built ORR_STATIC_PLANAR_UV_CLI"]
    fn actual_cli_new_only_preserves_inputs_and_refuses_protected_files_and_symlinks() {
        use std::os::unix::fs::symlink;
        let binary = PathBuf::from(
            std::env::var_os("ORR_STATIC_PLANAR_UV_CLI")
                .expect("set verified fresh feature-on binary from sole build owner"),
        );
        assert!(binary.is_absolute() && binary.is_file());
        let dir = Scratch::new();
        fs::create_dir(dir.0.join("inputs")).unwrap();
        let source_path = dir.0.join(SOURCE);
        let recipe_path = dir.0.join("tile.planar-uv.json");
        let source = fixture().to_bytes().unwrap();
        let input = serde_json::to_vec(&recipe()).unwrap();
        fs::write(&source_path, &source).unwrap();
        fs::write(&recipe_path, &input).unwrap();
        let run = |recipe: &Path, output: &Path| {
            Command::new(&binary)
                .arg("--recipe")
                .arg(recipe)
                .args(["--asset-id", ID])
                .arg("--output")
                .arg(output)
                .output()
                .unwrap()
        };
        let output = dir.0.join("new.model.json");
        let before = tree(&dir.0);
        let first = run(&recipe_path, &output);
        assert!(
            first.status.success(),
            "{}",
            String::from_utf8_lossy(&first.stderr)
        );
        let cooked = fs::read(&output).unwrap();
        assert_eq!(
            cooked,
            apply(&input, ID, &source).unwrap().to_bytes().unwrap()
        );
        let mut expected = before;
        expected.push(PathBuf::from("new.model.json"));
        expected.sort();
        assert_eq!(tree(&dir.0), expected);
        let again = dir.0.join("again.model.json");
        assert!(run(&recipe_path, &again).status.success());
        assert_eq!(cooked, fs::read(&again).unwrap());
        for target in [&output, &source_path, &recipe_path] {
            let contents = fs::read(target).unwrap();
            let snapshot = tree(&dir.0);
            assert!(!run(&recipe_path, target).status.success());
            assert_eq!(contents, fs::read(target).unwrap());
            assert_eq!(tree(&dir.0), snapshot);
        }
        let store = dir.0.join(".orr");
        fs::create_dir(&store).unwrap();
        let protected = store.join("protected.model.json");
        let snapshot = tree(&dir.0);
        assert!(!run(&recipe_path, &protected).status.success());
        assert_eq!(tree(&dir.0), snapshot);
        let hardlink = dir.0.join("hardlink.model.json");
        fs::hard_link(&source_path, &hardlink).unwrap();
        assert!(!run(&recipe_path, &hardlink).status.success());
        assert_eq!(fs::read(&hardlink).unwrap(), source);
        let absent = dir.0.join("absent.model.json");
        let linked_recipe = dir.0.join("linked.planar-uv.json");
        symlink(&recipe_path, &linked_recipe).unwrap();
        let snapshot = tree(&dir.0);
        assert!(!run(&linked_recipe, &absent).status.success());
        assert_eq!(tree(&dir.0), snapshot);
        for target in [&output, &absent] {
            let linked = dir.0.join("linked.model.json");
            symlink(target, &linked).unwrap();
            let snapshot = tree(&dir.0);
            assert!(!run(&recipe_path, &linked).status.success());
            assert_eq!(tree(&dir.0), snapshot);
            assert!(fs::symlink_metadata(&linked)
                .unwrap()
                .file_type()
                .is_symlink());
            fs::remove_file(linked).unwrap();
        }
        let parent_alias = dir.0.join("linked-parent");
        symlink(&dir.0, &parent_alias).unwrap();
        let snapshot = tree(&dir.0);
        assert!(!run(&recipe_path, &parent_alias.join("absent.model.json"))
            .status
            .success());
        assert!(!run(&parent_alias.join("tile.planar-uv.json"), &absent)
            .status
            .success());
        assert_eq!(tree(&dir.0), snapshot);
        let saved = dir.0.join("saved.model.json");
        fs::rename(&source_path, &saved).unwrap();
        symlink(&saved, &source_path).unwrap();
        let snapshot = tree(&dir.0);
        assert!(!run(&recipe_path, &absent).status.success());
        assert_eq!(tree(&dir.0), snapshot);
        fs::remove_file(&source_path).unwrap();
        fs::rename(&saved, &source_path).unwrap();
        let source_parent = dir.0.join("inputs");
        let saved_parent = dir.0.join("saved-inputs");
        fs::rename(&source_parent, &saved_parent).unwrap();
        symlink(&saved_parent, &source_parent).unwrap();
        let snapshot = tree(&dir.0);
        assert!(!run(&recipe_path, &absent).status.success());
        assert_eq!(tree(&dir.0), snapshot);
        fs::remove_file(&source_parent).unwrap();
        fs::rename(&saved_parent, &source_parent).unwrap();
        assert!(!absent.exists());
        assert_eq!(fs::read(&recipe_path).unwrap(), input);
        assert_eq!(fs::read(&source_path).unwrap(), source);
        assert_eq!(fs::read(&output).unwrap(), cooked);
    }
}
