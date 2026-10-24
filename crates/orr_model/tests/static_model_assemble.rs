#![cfg(feature = "import")]
#![allow(clippy::float_arithmetic)]

#[cfg(unix)]
#[path = "../../../tools/test_support/unix_socket.rs"]
mod unix_socket;

#[path = "../examples/static_model_assemble/authoring.rs"]
mod authoring;

use authoring::assemble_model;
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
const RECIPE_ID: &str = "variants/chair.assemble.json";
const MAX_RECIPE_BYTES: usize = 16 * 1024;

fn recipe() -> Value {
    json!({"format":"orr_static_model_assemble", "version":1,
        "parts":[{"key":"chair", "source":SOURCE_PATH, "placement":orr_model::IDENTITY}]})
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
    assemble_model(input, id, |path| {
        assert_eq!(path, SOURCE_PATH);
        Ok(source.to_vec())
    })
}
fn reject_before_resolution(value: &Value) {
    let bytes = serde_json::to_vec(value).unwrap();
    assert!(assemble_model(&bytes, RECIPE_ID, |path| panic!(
        "invalid recipe resolved {path}"
    ))
    .is_err());
}
fn assert_bits(a: &[f32], b: &[f32]) {
    assert_eq!(
        a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        b.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
}

#[test]
fn first_use_tables_repeated_sources_colliding_ids_and_bit_preservation() {
    let a = fixture();
    let mut b = fixture().source().clone();
    b.images[0].rgba8 = vec![201, 77, 33, 255];
    b.materials[0].base_color = [0.125, -0.0, 0.5, 1.0];
    let b = StaticModel::new(b).unwrap();
    let a_bytes = a.to_bytes().unwrap();
    let b_bytes = b.to_bytes().unwrap();
    let mut input = recipe();
    input["parts"] = json!([
        {"key":"b", "source":"inputs/b.model.json", "placement":orr_model::IDENTITY},
        {"key":"a", "source":SOURCE_PATH, "placement":orr_model::IDENTITY},
        {"key":"again", "source":"inputs/b.model.json", "placement":orr_model::IDENTITY}
    ]);
    let bytes = serde_json::to_vec(&input).unwrap();
    let mut calls = Vec::new();
    let out = assemble_model(&bytes, RECIPE_ID, |path| {
        calls.push(path.to_string());
        Ok(if path == SOURCE_PATH {
            a_bytes.clone()
        } else {
            b_bytes.clone()
        })
    })
    .unwrap();
    assert_eq!(calls, ["inputs/b.model.json", SOURCE_PATH]);
    let out = out.source();
    assert_eq!(out.materials.len(), 6);
    assert_eq!(
        out.images,
        [b.source().images.clone(), a.source().images.clone()].concat()
    );
    for (part, (source, key, offset)) in [
        (b.source(), "b", 0),
        (a.source(), "a", 3),
        (b.source(), "again", 0),
    ]
    .iter()
    .enumerate()
    {
        for (i, old) in source.primitives.iter().enumerate() {
            let new = &out.primitives[part * 3 + i];
            assert_eq!(
                new.id,
                format!(
                    "{RECIPE_ID}#node=part:{key}{}",
                    old.id.strip_prefix(&source.asset_id).unwrap()
                )
            );
            assert_eq!(new.material, old.material + offset);
            assert_eq!(new.indices, old.indices);
            for (x, y) in new.vertices.iter().zip(&old.vertices) {
                assert_bits(&x.position, &y.position);
                assert_bits(&x.normal, &y.normal);
                assert_bits(&x.uv, &y.uv);
            }
            for (x, y) in new.transform.iter().zip(old.transform) {
                assert_bits(x, &y);
            }
        }
    }
    for (index, source) in [b.source(), a.source()].iter().enumerate() {
        for (slot, old) in source.materials.iter().enumerate() {
            let new = &out.materials[index * 3 + slot];
            assert_bits(&new.base_color, &old.base_color);
            assert_eq!(new.image, old.image + (index * 2) as u32);
            assert_eq!(
                (new.linear_filter, new.wrap_s, new.wrap_t),
                (old.linear_filter, old.wrap_s, old.wrap_t)
            );
        }
    }
    assert_eq!(
        out.dependencies,
        vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(&bytes)
            },
            Dependency {
                uri: "inputs/b.model.json".into(),
                sha256: digest(&b_bytes)
            },
            Dependency {
                uri: SOURCE_PATH.into(),
                sha256: digest(&a_bytes)
            },
        ]
    );
    assert_eq!(a.to_bytes().unwrap(), a_bytes);
}

#[test]
fn placement_left_multiplies_noncommuting_source_and_mirrors() {
    let mut input = recipe();
    // Column-major 90-degree rotation, anisotropic scales and translation.
    input["parts"][0]["placement"] =
        json!([[0, 2, 0, 0], [-3, 0, 0, 0], [0, 0, 4, 0], [10, 20, 30, 1]]);
    let original = fixture();
    let result = apply(
        &serde_json::to_vec(&input).unwrap(),
        RECIPE_ID,
        &original.to_bytes().unwrap(),
    )
    .unwrap();
    let expected = [
        [
            [0., -4., 0., 0.],
            [-9., 0., 0., 0.],
            [0., 0., 2., 0.],
            [31., 28., 74., 1.],
        ],
        [
            [-6., 0., 0., 0.],
            [-1.5, -6., 0., 0.],
            [0., 0., 16., 0.],
            [-14., 10., -6., 1.],
        ],
        [
            [0., 2., 0., 0.],
            [-3., 0., 0., 0.],
            [0., 0., 4., 0.],
            [10., 20., 30., 1.],
        ],
    ];
    for (p, expected) in result.source().primitives.iter().zip(expected) {
        assert_eq!(p.transform, expected);
    }
    assert!(orr_model::determinant(result.source().primitives[0].transform) < 0.);
    // Mirroring placement cancels the first source's mirror, independently.
    input["parts"][0]["placement"] =
        json!([[-1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]);
    let mirrored = apply(
        &serde_json::to_vec(&input).unwrap(),
        RECIPE_ID,
        &original.to_bytes().unwrap(),
    )
    .unwrap();
    assert_eq!(mirrored.source().primitives[0].transform[0][0], 2.);
    assert_eq!(mirrored.source().primitives[0].transform[3][0], -4.);
    assert!(orr_model::determinant(mirrored.source().primitives[0].transform) > 0.);
}

#[test]
fn strict_recipe_parts_aliases_types_and_byte_limits_reject_before_resolution() {
    for field in ["format", "version", "parts"] {
        let mut value = recipe();
        value.as_object_mut().unwrap().remove(field);
        reject_before_resolution(&value);
        let mut value = recipe();
        value[field] = Value::Null;
        reject_before_resolution(&value);
    }
    for field in ["key", "source", "placement"] {
        let mut value = recipe();
        value["parts"][0].as_object_mut().unwrap().remove(field);
        reject_before_resolution(&value);
    }
    for (field, bad) in [
        ("version", json!(1.0)),
        ("version", json!(2)),
        ("parts", json!([])),
        ("parts", json!({})),
        ("unknown", json!(0)),
    ] {
        let mut value = recipe();
        value[field] = bad;
        reject_before_resolution(&value);
    }
    for key in ["", "a/b", "a.b", "_a", "a b", "한글", &"a".repeat(65)] {
        let mut value = recipe();
        value["parts"][0]["key"] = json!(key);
        reject_before_resolution(&value);
    }
    for path in [
        "../a.model.json",
        "a//b.model.json",
        "./a.model.json",
        "A.MODEL.JSON",
        "a.glb",
        "C:a.model.json",
        "a%20.model.json",
    ] {
        let mut value = recipe();
        value["parts"][0]["source"] = json!(path);
        reject_before_resolution(&value);
    }
    for count in [0, 33] {
        let mut value = recipe();
        value["parts"]=json!((0..count).map(|i|json!({"key":format!("p{i}"),"source":SOURCE_PATH,"placement":orr_model::IDENTITY})).collect::<Vec<_>>());
        reject_before_resolution(&value);
    }
    for (key, path) in [
        ("chair", SOURCE_PATH),
        ("CHAIR", SOURCE_PATH),
        ("other", "Inputs/prop.model.json"),
    ] {
        let mut value = recipe();
        let mut part = value["parts"][0].clone();
        part["key"] = json!(key);
        part["source"] = json!(path);
        value["parts"].as_array_mut().unwrap().push(part);
        reject_before_resolution(&value);
    }
    let duplicate = br#"{"format":"orr_static_model_assemble","version":1,"parts":[],"parts":[]}"#;
    assert!(assemble_model(duplicate, RECIPE_ID, |_| panic!("resolved")).is_err());
    let mut oversized = recipe_bytes();
    oversized.resize(MAX_RECIPE_BYTES + 1, b' ');
    assert!(assemble_model(&oversized, RECIPE_ID, |_| panic!("resolved")).is_err());
}

#[test]
fn singular_nonaffine_nonfinite_and_composition_overflow_reject() {
    for placement in [
        json!([[0, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]),
        json!([[1, 0, 0, 1], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]),
        json!([[1e39, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]),
        json!([1, 2, 3]),
    ] {
        let mut value = recipe();
        value["parts"][0]["placement"] = placement;
        reject_before_resolution(&value);
    }
    let mut value = recipe();
    value["parts"][0]["placement"] =
        json!([[1000000, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]);
    assert!(apply(
        &serde_json::to_vec(&value).unwrap(),
        RECIPE_ID,
        &fixture().to_bytes().unwrap()
    )
    .is_err());
}

#[test]
fn full_source_admission_includes_unused_resources_and_aggregate_limits() {
    for (pointer, bad) in [
        ("/images/1/width", json!(0)),
        ("/materials/2/image", json!(u32::MAX)),
        ("/materials/2/base_color/0", json!(2)),
        ("/primitives/1/indices/0", json!(u32::MAX)),
        ("/dependencies/0/sha256", json!("bad")),
    ] {
        let mut source = serde_json::to_value(fixture().source()).unwrap();
        *source.pointer_mut(pointer).unwrap() = bad;
        assert!(apply(
            &recipe_bytes(),
            RECIPE_ID,
            &serde_json::to_vec(&source).unwrap()
        )
        .is_err());
    }
    let mut source = fixture().source().clone();
    source.images = vec![source.images[0].clone(); orr_model::MAX_IMAGES];
    let bytes = StaticModel::new(source).unwrap().to_bytes().unwrap();
    let mut value = recipe();
    let mut part = value["parts"][0].clone();
    part["key"] = json!("second");
    part["source"] = json!("inputs/second.model.json");
    value["parts"].as_array_mut().unwrap().push(part);
    assert!(
        assemble_model(&serde_json::to_vec(&value).unwrap(), RECIPE_ID, |_| Ok(
            bytes.clone()
        ))
        .is_err()
    );
}

#[test]
fn thirty_two_repeated_parts_resolve_once_and_roundtrip_deterministically() {
    let mut value = recipe();
    value["parts"] = json!((0..32)
        .map(
            |i| json!({"key":format!("p{i}"),"source":SOURCE_PATH,"placement":orr_model::IDENTITY})
        )
        .collect::<Vec<_>>());
    let mut input = serde_json::to_vec(&value).unwrap();
    input.resize(MAX_RECIPE_BYTES, b' ');
    let source = fixture().to_bytes().unwrap();
    let mut calls = 0;
    let model = assemble_model(&input, RECIPE_ID, |_| {
        calls += 1;
        Ok(source.clone())
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(model.source().primitives.len(), 96);
    assert_eq!(model.source().materials.len(), 3);
    assert_eq!(model.source().dependencies[0].sha256, digest(&input));
    let bytes = model.to_bytes().unwrap();
    assert_eq!(
        StaticModel::from_bytes(&bytes).unwrap().to_bytes().unwrap(),
        bytes
    );
    assert_eq!(
        apply(&input, RECIPE_ID, &source)
            .unwrap()
            .to_bytes()
            .unwrap(),
        bytes
    );
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
            "orr-static-assemble-test-{}-{}",
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
            "orr-static-assemble-socket-{}-{}",
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
        let input = temp.join("chair.assemble.json");
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
        (3, "../bad.assemble.json"),
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
    let nested_input = nested.join("chair.assemble.json");
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
    let raw_input = raw_parent_path(&child, "chair.assemble.json");
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
    let recipe_alias = fixture.temp.join("linked.assemble.json");
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
        &parent_alias.join("chair.assemble.json"),
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
        bad[index] = OsString::from_vec(b"invalid-\xff.assemble.json".to_vec());
        assert_eq!(bad[index].as_bytes(), b"invalid-\xff.assemble.json");
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
    let input = root.join(OsString::from_vec(b"recipe-\xfe.assemble.json".to_vec()));
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
        r"\\server\share\prop.assemble.json",
        r"\\?\UNC\server\share\prop.assemble.json",
        r"\\.\pipe\prop.assemble.json",
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

#[test]
fn actual_cli_publishes_validated_output_and_rejects_second_write() {
    let binary =
        std::env::var_os("ORR_STATIC_MODEL_ASSEMBLE_BIN").expect("fresh actual CLI required");
    let fixture = CliFixture::new();
    let first = std::process::Command::new(&binary)
        .args(fixture.args())
        .output()
        .unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let bytes = fs::read(&fixture.output).unwrap();
    assert_eq!(
        bytes,
        apply(&fixture.recipe, RECIPE_ID, &fixture.source)
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    let second = std::process::Command::new(&binary)
        .args(fixture.args())
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(2));
    assert_eq!(fs::read(&fixture.output).unwrap(), bytes);
    fixture.assert_unchanged();
}

#[test]
fn aggregate_material_geometry_decoded_and_input_budgets_are_independent() {
    let mut repeated = recipe();
    repeated["parts"] = json!((0..32)
        .map(
            |i| json!({"key":format!("p{i}"),"source":SOURCE_PATH,"placement":orr_model::IDENTITY})
        )
        .collect::<Vec<_>>());
    let repeated = serde_json::to_vec(&repeated).unwrap();
    for kind in ["primitives", "vertices", "indices"] {
        let mut source = fixture().source().clone();
        source.primitives.truncate(1);
        match kind {
            "primitives" => {
                let p = source.primitives[0].clone();
                source.primitives = (0..33)
                    .map(|i| {
                        let mut p = p.clone();
                        p.id = format!("{}#node={i}/mesh=0/primitive=0", source.asset_id);
                        p
                    })
                    .collect();
            }
            "vertices" => {
                source.primitives[0].vertices = vec![source.primitives[0].vertices[0]; 8000];
            }
            "indices" => {
                source.primitives[0].indices = vec![0; 24000];
            }
            _ => unreachable!(),
        }
        let bytes = StaticModel::new(source).unwrap().to_bytes().unwrap();
        assert!(
            apply(&repeated, RECIPE_ID, &bytes).is_err(),
            "aggregate {kind}"
        );
    }
    let mut two = recipe();
    let mut part = two["parts"][0].clone();
    part["key"] = json!("second");
    part["source"] = json!("inputs/second.model.json");
    two["parts"].as_array_mut().unwrap().push(part);
    let two = serde_json::to_vec(&two).unwrap();
    let mut source = fixture().source().clone();
    source.materials = vec![source.materials[0].clone(); 129];
    let bytes = StaticModel::new(source).unwrap().to_bytes().unwrap();
    assert!(assemble_model(&two, RECIPE_ID, |_| Ok(bytes.clone())).is_err());
    let mut source = fixture().source().clone();
    source.images = vec![
        Image {
            width: 2048,
            height: 2048,
            rgba8: vec![0; 2048 * 2048 * 4],
        },
        Image {
            width: 1,
            height: 1,
            rgba8: vec![0; 4],
        },
    ];
    let bytes = StaticModel::new(source).unwrap().to_bytes().unwrap();
    // Each source is <=32MiB decoded; their independently admissible image
    // tables total 32MiB+8 bytes. JSON encoding necessarily reaches the
    // input-byte bound first; do not claim independent decoded-branch coverage.
    assert!(bytes.len() * 2 + two.len() < orr_model::MAX_FILE_BYTES + 8192);
    let error = assemble_model(&two, RECIPE_ID, |_| Ok(bytes.clone())).unwrap_err();
    assert!(error.contains("byte"), "{error}");
    let mut padded = fixture().to_bytes().unwrap();
    padded.resize(orr_model::MAX_FILE_BYTES / 2, b' ');
    let error = assemble_model(&two, RECIPE_ID, |_| Ok(padded.clone())).unwrap_err();
    assert!(
        error.contains("byte") || error.contains("budget"),
        "{error}"
    );
}

#[test]
fn positional_and_duplicate_part_records_are_rejected() {
    for value in [
        json!(["orr_static_model_assemble", 1, recipe()["parts"]]),
        json!({"format":"orr_static_model_assemble","version":1,"parts":[["chair",SOURCE_PATH,orr_model::IDENTITY]]}),
    ] {
        reject_before_resolution(&value);
    }
    let mut input = serde_json::to_string(&recipe()).unwrap();
    input = input.replace("\"key\":\"chair\"", "\"key\":\"chair\",\"key\":\"other\"");
    assert!(assemble_model(input.as_bytes(), RECIPE_ID, |_| panic!("resolved")).is_err());
}
