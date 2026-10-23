#![cfg(feature = "import")]
#![allow(clippy::float_arithmetic)]

#[path = "../examples/static_model_recolor/authoring.rs"]
mod authoring;

use authoring::recolor_model;
use orr_model::{Dependency, Image, StaticModel, Wrap};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

const SOURCE_PATH: &str = "inputs/prop.model.json";
const RECIPE_ID: &str = "variants/blue.recolor.json";
const MAX_RECIPE_BYTES: usize = 16 * 1024;

fn recipe() -> Value {
    json!({
        "format": "orr_static_material_color",
        "version": 1,
        "source": SOURCE_PATH,
        "linear_rgb": [0.875, 0.25, 0.0625],
        "material": 0,
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
    // The unselected slot shares the original selected image. It must retain it.
    source.materials[1].image = source.materials[0].image;
    source.materials.push(source.materials[1].clone());
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![19, 37, 71, 103],
    });
    let mut shared = source.primitives[0].clone();
    shared.id = format!("{}#node=17/mesh=8/primitive=3", source.asset_id);
    shared.transform = [
        [-2.0, 0.0, 0.0, 0.0],
        [0.0, 3.0, 0.0, 0.0],
        [0.0, 0.0, 0.5, 0.0],
        [4.0, -7.0, 11.0, 1.0],
    ];
    source.primitives.push(shared);
    source.dependencies.push(Dependency {
        uri: "original-materials/paint.png".into(),
        sha256: digest(b"original source metadata remains transitively bound"),
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
    recolor_model(recipe, id, |path| {
        assert_eq!(path, SOURCE_PATH);
        Ok(source.to_vec())
    })
}
fn reject_before_resolution(bytes: &[u8], id: &str) {
    assert!(recolor_model(bytes, id, |path| panic!("invalid recipe resolved {path}")).is_err());
}

#[test]
fn strict_recipe_types_duplicates_shapes_and_f64_bounds() {
    let good = recipe();
    for key in ["format", "version", "source", "material", "linear_rgb"] {
        let mut missing = good.clone();
        missing.as_object_mut().unwrap().remove(key);
        reject_before_resolution(&serde_json::to_vec(&missing).unwrap(), RECIPE_ID);
        let mut null = good.clone();
        null[key] = Value::Null;
        reject_before_resolution(&serde_json::to_vec(&null).unwrap(), RECIPE_ID);
        let text = serde_json::to_string(&good).unwrap();
        let duplicate = format!("{{\"{key}\":{},{}", good[key], &text[1..]);
        reject_before_resolution(duplicate.as_bytes(), RECIPE_ID);
    }
    for (key, values) in [
        (
            "format",
            vec![json!("orr_static_material_texture"), json!(1), json!([])],
        ),
        (
            "version",
            vec![json!(0), json!(2), json!(1.5), json!("1"), json!(true)],
        ),
        (
            "material",
            vec![
                json!(-1),
                json!(0.5),
                json!("0"),
                json!(true),
                json!(4294967296_u64),
            ],
        ),
        (
            "linear_rgb",
            vec![
                json!([]),
                json!([0, 1]),
                json!([0, 1, 0, 1]),
                json!("0,1,0"),
                json!(["0", 0, 0]),
                json!([null, 0, 0]),
                json!([true, 0, 0]),
                json!([-1e-50, 0, 0]),
                json!([1.00000001, 0, 0]),
                json!([0, 0, 1.1]),
            ],
        ),
    ] {
        for value in values {
            let mut bad = good.clone();
            bad[key] = value;
            reject_before_resolution(&serde_json::to_vec(&bad).unwrap(), RECIPE_ID);
        }
    }
    for bad in [
        "[]",
        "null",
        "true",
        "{}",
        "{",
        "[\"orr_static_material_color\",1,\"inputs/prop.model.json\",0,[0,1,0]]",
    ] {
        reject_before_resolution(bad.as_bytes(), RECIPE_ID);
    }
    let mut unknown = good.clone();
    unknown["texture"] = json!("x.png");
    reject_before_resolution(&serde_json::to_vec(&unknown).unwrap(), RECIPE_ID);
    let mut overflow = serde_json::to_string(&good).unwrap();
    overflow = overflow.replace("0.875", "1e999");
    reject_before_resolution(overflow.as_bytes(), RECIPE_ID);
    reject_before_resolution(&vec![b' '; MAX_RECIPE_BYTES + 1], RECIPE_ID);
}

#[test]
fn portable_paths_and_identity_reject_before_resolution() {
    for bad in [
        "",
        "/prop.model.json",
        "../prop.model.json",
        "a/../prop.model.json",
        "a//prop.model.json",
        "./prop.model.json",
        "a\\prop.model.json",
        "a b.model.json",
        "a%20b.model.json",
        "https://host/a.model.json",
        "CON.model.json",
        "aux.model.json",
        "a./prop.model.json",
        ".hidden/prop.model.json",
        "prop.MODEL.JSON",
        "한글.model.json",
    ] {
        let mut input = recipe();
        input["source"] = json!(bad);
        reject_before_resolution(&serde_json::to_vec(&input).unwrap(), RECIPE_ID);
    }
    for bad in [
        "",
        "../blue.recolor.json",
        "blue.model.json",
        "a//blue.recolor.json",
        "NUL.recolor.json",
        "blue.RECOLOR.JSON",
    ] {
        reject_before_resolution(&recipe_bytes(), bad);
    }
    let mut input = recipe();
    input["source"] = json!(format!("{}.model.json", "a".repeat(241)));
    reject_before_resolution(&serde_json::to_vec(&input).unwrap(), RECIPE_ID);
}

#[test]
fn full_source_admission_used_slot_and_bounded_resolution() {
    let source = rich_fixture().to_bytes().unwrap();
    let mut calls = 0;
    recolor_model(&recipe_bytes(), RECIPE_ID, |path| {
        calls += 1;
        assert_eq!(path, SOURCE_PATH);
        Ok(source.clone())
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert!(recolor_model(&recipe_bytes(), RECIPE_ID, |_| Err(
        "fixture resolution failure".into()
    ))
    .is_err());
    for slot in [2, 3, u32::MAX] {
        let mut input = recipe();
        input["material"] = json!(slot);
        assert!(apply(&serde_json::to_vec(&input).unwrap(), RECIPE_ID, &source).is_err());
    }
    let valid: Value = serde_json::from_slice(&source).unwrap();
    for (pointer, value) in [
        ("/materials/1/image", json!(999)),
        ("/materials/1/base_color/3", json!(2)),
        ("/primitives/1/material", json!(999)),
        ("/primitives/1/indices/0", json!(999999)),
        ("/primitives/1/transform/0/0", json!(null)),
        ("/images/1/width", json!(0)),
        ("/dependencies/0/sha256", json!("bad")),
    ] {
        let mut bad = valid.clone();
        *bad.pointer_mut(pointer).expect(pointer) = value;
        assert!(
            apply(
                &recipe_bytes(),
                RECIPE_ID,
                &serde_json::to_vec(&bad).unwrap()
            )
            .is_err(),
            "{pointer}"
        );
    }
    for bytes in [b"null".to_vec(), vec![b' '; orr_model::MAX_FILE_BYTES + 1]] {
        assert!(apply(&recipe_bytes(), RECIPE_ID, &bytes).is_err());
    }
}

fn assert_bits(actual: &StaticModel, original: &StaticModel, id: &str, rgb: [f32; 3]) {
    let old = original.source();
    let new = actual.source();
    assert_eq!(new.asset_id, id);
    assert_eq!(new.images, old.images);
    assert_eq!(new.primitives.len(), old.primitives.len());
    for (a, b) in old.primitives.iter().zip(&new.primitives) {
        assert_eq!(
            b.id,
            format!("{id}#node={}", a.id.split_once("#node=").unwrap().1)
        );
        assert_eq!(a.indices, b.indices);
        assert_eq!(a.material, b.material);
        assert_eq!(a.vertices.len(), b.vertices.len());
        for (x, y) in a
            .transform
            .iter()
            .flatten()
            .zip(b.transform.iter().flatten())
        {
            assert_eq!(x.to_bits(), y.to_bits());
        }
        for (x, y) in a.vertices.iter().zip(&b.vertices) {
            for (u, v) in x
                .position
                .iter()
                .chain(&x.normal)
                .chain(&x.uv)
                .zip(y.position.iter().chain(&y.normal).chain(&y.uv))
            {
                assert_eq!(u.to_bits(), v.to_bits());
            }
        }
    }
    assert_eq!(old.materials.len(), new.materials.len());
    for (slot, (a, b)) in old.materials.iter().zip(&new.materials).enumerate() {
        assert_eq!(a.image, b.image);
        assert_eq!(a.linear_filter, b.linear_filter);
        assert_eq!(a.wrap_s, b.wrap_s);
        assert_eq!(a.wrap_t, b.wrap_t);
        for (channel, actual) in b.base_color.iter().enumerate() {
            let expected = if slot == 0 && channel < 3 {
                rgb[channel]
            } else {
                a.base_color[channel]
            };
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
    }
}

#[test]
fn replacement_preserves_every_unowned_float_bit_including_signed_zero() {
    let mut source = rich_fixture().source().clone();
    source.materials[0].base_color[3] = -0.0;
    source.materials[1].base_color = [-0.0, 0.125, 0.5, -0.0];
    for primitive in &mut source.primitives {
        primitive.transform[0][1] = -0.0;
        for vertex in &mut primitive.vertices {
            vertex.position[2] = -0.0;
            vertex.normal[0] = -0.0;
            vertex.uv[0] = -0.0;
        }
    }
    let original = StaticModel::new(source).unwrap();
    let bytes = original.to_bytes().unwrap();
    let before = bytes.clone();
    let result = apply(&recipe_bytes(), RECIPE_ID, &bytes).unwrap();
    assert_bits(&result, &original, RECIPE_ID, [0.875, 0.25, 0.0625]);
    let cooked = result.to_bytes().unwrap();
    let reloaded = StaticModel::from_bytes(&cooked).unwrap();
    assert_bits(&reloaded, &original, RECIPE_ID, [0.875, 0.25, 0.0625]);
    assert_eq!(reloaded.to_bytes().unwrap(), cooked);
    assert_eq!(bytes, before);
    assert_eq!(original.to_bytes().unwrap(), before);
}

#[test]
fn provenance_exact_bytes_distinct_ids_same_rgb_and_nested_variants() {
    let original = rich_fixture();
    let bytes = original.to_bytes().unwrap();
    let recipe = recipe_bytes();
    let a = apply(&recipe, RECIPE_ID, &bytes).unwrap();
    assert_eq!(
        a.source().dependencies,
        vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(&recipe)
            },
            Dependency {
                uri: SOURCE_PATH.into(),
                sha256: digest(&bytes)
            }
        ]
    );
    let id = "variants/other.recolor.json";
    let b = apply(&recipe, id, &bytes).unwrap();
    assert_ne!(a.to_bytes().unwrap(), b.to_bytes().unwrap());
    assert_bits(&b, &original, id, [0.875, 0.25, 0.0625]);
    let mut spaced = recipe.clone();
    spaced.extend_from_slice(b" \n");
    let c = apply(&spaced, RECIPE_ID, &bytes).unwrap();
    assert_ne!(a.source().dependencies, c.source().dependencies);
    let nested = apply(&recipe, id, &a.to_bytes().unwrap()).unwrap();
    assert_bits(&nested, &a, id, [0.875, 0.25, 0.0625]);
    assert_eq!(
        nested.source().dependencies[1].sha256,
        digest(&a.to_bytes().unwrap())
    );
    let mut same = recipe_bytes();
    let mut value = serde_json::from_slice::<Value>(&same).unwrap();
    value["linear_rgb"] = json!([0.125, 0.5, 0.75]);
    same = serde_json::to_vec(&value).unwrap();
    let unchanged = apply(&same, RECIPE_ID, &bytes).unwrap();
    assert_bits(&unchanged, &original, RECIPE_ID, [0.125, 0.5, 0.75]);
}

#[test]
fn rgb_endpoints_and_rounding_are_admitted_after_f64_validation() {
    let original = rich_fixture();
    let bytes = original.to_bytes().unwrap();
    for rgb in [[0.0, 1.0, -0.0], [0.1, 0.2, 0.3], [1e-50, 0.99999999, 0.5]] {
        let mut input = recipe();
        input["linear_rgb"] = json!(rgb);
        let model = apply(&serde_json::to_vec(&input).unwrap(), RECIPE_ID, &bytes).unwrap();
        assert_bits(&model, &original, RECIPE_ID, rgb.map(|v| v as f32));
    }
}

#[test]
fn exact_recipe_path_and_model_table_budgets() {
    let source = rich_fixture().to_bytes().unwrap();
    let mut padded = recipe_bytes();
    padded.resize(MAX_RECIPE_BYTES, b' ');
    apply(&padded, RECIPE_ID, &source).unwrap();
    padded.push(b' ');
    reject_before_resolution(&padded, RECIPE_ID);
    let source_path = format!("{}.model.json", "s".repeat(229));
    let id = format!("{}.recolor.json", "v".repeat(227));
    assert_eq!(source_path.len(), 240);
    assert_eq!(id.len(), 240);
    let mut input = recipe();
    input["source"] = json!(source_path);
    recolor_model(&serde_json::to_vec(&input).unwrap(), &id, |path| {
        assert_eq!(path, source_path);
        Ok(source.clone())
    })
    .unwrap();
    let mut model = rich_fixture().source().clone();
    model
        .materials
        .resize(orr_model::MAX_MATERIALS, model.materials[0].clone());
    model
        .images
        .resize(orr_model::MAX_IMAGES, model.images[0].clone());
    let original = StaticModel::new(model).unwrap();
    let result = apply(&recipe_bytes(), RECIPE_ID, &original.to_bytes().unwrap()).unwrap();
    assert_bits(&result, &original, RECIPE_ID, [0.875, 0.25, 0.0625]);
    for (field, limit) in [
        ("materials", orr_model::MAX_MATERIALS),
        ("images", orr_model::MAX_IMAGES),
        ("primitives", orr_model::MAX_PRIMITIVES),
    ] {
        let mut value = serde_json::to_value(original.source()).unwrap();
        let array = value[field].as_array_mut().unwrap();
        array.resize(limit + 1, array[0].clone());
        assert!(apply(
            &recipe_bytes(),
            RECIPE_ID,
            &serde_json::to_vec(&value).unwrap()
        )
        .is_err());
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
            "orr-static-recolor-test-{}-{}",
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
        let input = temp.join("blue.recolor.json");
        let output = temp.join("blue.model.json");
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
        (3, "../bad.recolor.json"),
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
    let nested_input = nested.join("blue.recolor.json");
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
    let raw_input = raw_parent_path(&child, "blue.recolor.json");
    let raw_output = raw_parent_path(&child, "blue.model.json");
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
    let recipe_alias = fixture.temp.join("linked.recolor.json");
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
        &parent_alias.join("blue.recolor.json"),
        &fixture.output
    ))
    .is_err());
    assert!(authoring::run(cli_args(
        &fixture.input,
        &parent_alias.join("blue.model.json")
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
        bad[index] = OsString::from_vec(b"invalid-\xff.recolor.json".to_vec());
        assert_eq!(bad[index].as_bytes(), b"invalid-\xff.recolor.json");
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
    let input = root.join(OsString::from_vec(b"recipe-\xfe.recolor.json".to_vec()));
    let output = root.join("blue.model.json");
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
        r"\\server\share\prop.recolor.json",
        r"\\?\UNC\server\share\prop.recolor.json",
        r"\\.\pipe\prop.recolor.json",
    ] {
        let error = authoring::run(cli_args(Path::new(remote), &fixture.output)).unwrap_err();
        assert!(error.contains("nonlocal Windows path prefixes"), "{error}");
        assert_eq!(tree(&fixture.temp.0), before);
    }
    assert!(!fixture.output.exists());
    fixture.assert_unchanged();
}
