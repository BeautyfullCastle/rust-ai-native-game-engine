//! Independent geometric and fail-closed authoring contracts. The mandatory CLI
//! test requires ORR_STATIC_MODEL_CYLINDER_BIN from the source-bound gate.
#![cfg(feature = "import")]
#![allow(clippy::float_arithmetic)]

#[path = "../examples/static_model_cylinder/authoring.rs"]
mod authoring;

use authoring::author_cylinder;
use orr_model::{Dependency, Image, StaticModel, Wrap};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

const RECIPE_ID: &str = "props/key.cylinder.json";
const MAX_RECIPE_BYTES: usize = 16 * 1024;
const SEGMENTS: [u32; 5] = [8, 16, 32, 64, 128];

fn recipe(radius: f64, height: f64, segments: u32) -> Value {
    json!({"format":"orr_static_model_cylinder", "version":1,
        "radius":radius, "height":height, "segments":segments})
}
fn recipe_bytes() -> Vec<u8> {
    serde_json::to_vec(&recipe(0.5, 1.0, 32)).unwrap()
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn reject(value: &Value) {
    assert!(author_cylinder(&serde_json::to_vec(value).unwrap(), RECIPE_ID).is_err());
}
fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual {actual:?}, expected {expected:?}, tolerance {tolerance:?}"
    );
}
fn point_key(position: [f32; 3]) -> [u32; 3] {
    position.map(|value| if value == 0.0 { 0 } else { value.to_bits() })
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

// This oracle does not reuse a producer table, triangulation helper or cooked
// fixture. f64 libm is used only to check approximate geometric expectations;
// exact cross-platform producer bits are checked separately below.
fn assert_geometry(model: &StaticModel, radius: f32, height: f32, segments: u32) {
    let source = model.source();
    let n = segments as usize;
    assert_eq!(source.format, "orr_static_model");
    assert_eq!(source.version, 1);
    assert_eq!(source.primitives.len(), 3);
    assert_eq!(source.materials.len(), 3);
    assert_eq!(
        source.images,
        [Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4]
        }]
    );
    for material in &source.materials {
        assert_eq!(material.base_color, [1.0; 4]);
        assert_eq!(material.image, 0);
        assert!(!material.linear_filter);
        assert_eq!(
            (material.wrap_s, material.wrap_t),
            (Wrap::Clamp, Wrap::Clamp)
        );
    }
    let r = f64::from(radius);
    let h = f64::from(height);
    let mut positions = BTreeSet::new();
    let mut edges: BTreeMap<([u32; 3], [u32; 3]), (usize, i32)> = BTreeMap::new();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut volume_six = 0.0;
    let mut triangles = 0;
    for (slot, primitive) in source.primitives.iter().enumerate() {
        assert_eq!(
            primitive.id,
            format!("{}#node=0/mesh=0/primitive={slot}", source.asset_id)
        );
        assert_eq!(primitive.transform, orr_model::IDENTITY);
        assert_eq!(primitive.material, slot as u32);
        assert_eq!(
            primitive.vertices.len(),
            if slot == 0 { 2 * (n + 1) } else { n + 1 }
        );
        assert_eq!(
            primitive.indices.len(),
            if slot == 0 { 6 * n } else { 3 * n }
        );
        let mut used = BTreeSet::new();
        for vertex in &primitive.vertices {
            assert!(vertex
                .position
                .iter()
                .chain(&vertex.normal)
                .chain(&vertex.uv)
                .all(|v| v.is_finite()));
            assert!(vertex.uv.iter().all(|v| (0.0..=1.0).contains(v)));
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.position[axis]);
                max[axis] = max[axis].max(vertex.position[axis]);
            }
            positions.insert(point_key(vertex.position));
            close(
                dot(vertex.normal.map(f64::from), vertex.normal.map(f64::from)),
                1.0,
                2e-7,
            );
        }
        for face in primitive.indices.chunks_exact(3) {
            assert!(face
                .iter()
                .all(|&index| (index as usize) < primitive.vertices.len()));
            used.extend(face.iter().copied());
        }
        // Index-manifold checks would falsely reject intentional UV seam and
        // hard-normal duplicates. Weld positions, then require oriented closure.
        for face in primitive.indices.chunks_exact(3) {
            let p = [face[0], face[1], face[2]]
                .map(|index| primitive.vertices[index as usize].position.map(f64::from));
            let normal = cross(sub(p[1], p[0]), sub(p[2], p[0]));
            assert!(
                dot(normal, normal) > 0.0,
                "degenerate triangle in slot {slot}"
            );
            let center = [
                (p[0][0] + p[1][0] + p[2][0]) / 3.0,
                (p[0][1] + p[1][1] + p[2][1]) / 3.0,
                (p[0][2] + p[1][2] + p[2][2]) / 3.0,
            ];
            let outward = match slot {
                0 => [center[0], 0.0, center[2]],
                1 => [0.0, 1.0, 0.0],
                _ => [0.0, -1.0, 0.0],
            };
            assert!(dot(normal, outward) > 0.0, "inward winding in slot {slot}");
            for &index in face {
                assert!(
                    dot(
                        normal,
                        primitive.vertices[index as usize].normal.map(f64::from)
                    ) > 0.0
                );
            }
            volume_six += dot(p[0], cross(p[1], p[2]));
            for (a, b) in [(face[0], face[1]), (face[1], face[2]), (face[2], face[0])] {
                let a = point_key(primitive.vertices[a as usize].position);
                let b = point_key(primitive.vertices[b as usize].position);
                assert_ne!(a, b);
                let (key, direction) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
                let entry = edges.entry(key).or_default();
                entry.0 += 1;
                entry.1 += direction;
            }
            triangles += 1;
        }
        assert_eq!(
            used.len(),
            primitive.vertices.len(),
            "no unused or hidden geometry"
        );
    }
    assert_eq!(min, [-radius, -height / 2.0, -radius]);
    assert_eq!(max, [radius, height / 2.0, radius]);
    assert_eq!(positions.len(), 2 * n + 2);
    assert_eq!(triangles, 4 * n);
    assert_eq!(edges.len(), 6 * n);
    assert!(
        edges
            .values()
            .all(|&(count, balance)| count == 2 && balance == 0),
        "closed oriented positional surface required"
    );
    assert_eq!(
        positions.len() + triangles,
        edges.len() + 2,
        "closed genus-zero Euler characteristic"
    );
    let expected_volume = n as f64 * 0.5 * r * r * (std::f64::consts::TAU / n as f64).sin() * h;
    close(volume_six / 6.0, expected_volume, expected_volume * 3e-7);

    let side = &source.primitives[0];
    for i in 0..=n {
        let angle = std::f64::consts::TAU * i as f64 / n as f64;
        for (offset, y, v) in [(0, -height / 2.0, 1.0), (1, height / 2.0, 0.0)] {
            let vertex = &side.vertices[2 * i + offset];
            close(f64::from(vertex.position[0]), r * angle.cos(), r * 2e-7);
            close(f64::from(vertex.position[2]), r * angle.sin(), r * 2e-7);
            assert_eq!(vertex.position[1], y);
            close(f64::from(vertex.normal[0]), angle.cos(), 1e-7);
            close(f64::from(vertex.normal[2]), angle.sin(), 1e-7);
            assert_eq!(vertex.normal[1], 0.0);
            assert_eq!(vertex.uv, [i as f32 / n as f32, v]);
        }
    }
    for offset in [0, 1] {
        assert_eq!(
            side.vertices[offset].position.map(f32::to_bits),
            side.vertices[2 * n + offset].position.map(f32::to_bits)
        );
        assert_eq!(
            side.vertices[offset].normal.map(f32::to_bits),
            side.vertices[2 * n + offset].normal.map(f32::to_bits)
        );
        assert_eq!(
            (
                side.vertices[offset].uv[0],
                side.vertices[2 * n + offset].uv[0]
            ),
            (0.0, 1.0)
        );
    }
    for (slot, y, normal) in [
        (1, height / 2.0, [0.0, 1.0, 0.0]),
        (2, -height / 2.0, [0.0, -1.0, 0.0]),
    ] {
        let cap = &source.primitives[slot];
        assert_eq!(cap.vertices[0].position, [0.0, y, 0.0]);
        assert_eq!(cap.vertices[0].uv, [0.5, 0.5]);
        for vertex in &cap.vertices {
            assert_eq!(vertex.normal, normal, "cap normals are hard, never radial");
            assert_eq!(vertex.position[1], y);
            close(
                f64::from(vertex.uv[0]),
                0.5 + 0.5 * f64::from(vertex.position[0]) / r,
                1e-7,
            );
            close(
                f64::from(vertex.uv[1]),
                0.5 + 0.5 * f64::from(vertex.position[2]) / r,
                1e-7,
            );
        }
        for i in 0..n {
            let side_vertex = &side.vertices[2 * i + usize::from(slot == 1)];
            assert_eq!(
                cap.vertices[i + 1].position.map(f32::to_bits),
                side_vertex.position.map(f32::to_bits)
            );
            assert_ne!(cap.vertices[i + 1].normal, side_vertex.normal);
        }
    }
}

#[test]
fn independent_geometry_oracle_covers_all_segments_and_extreme_proportions() {
    for segments in SEGMENTS {
        for (radius, height) in [
            (0.5, 1.0),
            (0.001, 0.001),
            (1_000_000.0, 1_000_000.0),
            (0.001, 1_000_000.0),
            (1_000_000.0, 0.001),
        ] {
            let input = serde_json::to_vec(&recipe(radius, height, segments)).unwrap();
            let model = author_cylinder(&input, RECIPE_ID).unwrap();
            assert_geometry(&model, radius as f32, height as f32, segments);
        }
    }
}

#[test]
fn fixed_table_contract_has_exact_cardinals_diagonals_and_positive_zero() {
    // IEEE-754 binary32 nearest sqrt(1/2). This is a numeric golden, not a
    // same-process serialization check masquerading as a portability proof.
    let diagonal = f32::from_bits(0x3f35_04f3);
    let octants = [
        [1.0, 0.0],
        [diagonal, diagonal],
        [0.0, 1.0],
        [-diagonal, diagonal],
        [-1.0, 0.0],
        [-diagonal, -diagonal],
        [0.0, -1.0],
        [diagonal, -diagonal],
    ];
    for segments in SEGMENTS {
        let model = author_cylinder(
            &serde_json::to_vec(&recipe(1.0, 2.0, segments)).unwrap(),
            RECIPE_ID,
        )
        .unwrap();
        for (octant, [x, z]) in octants.into_iter().enumerate() {
            let vertex = &model.source().primitives[0].vertices[2 * octant * segments as usize / 8];
            assert_eq!(
                vertex.position.map(f32::to_bits),
                [x, -1.0, z].map(f32::to_bits)
            );
            assert_eq!(
                vertex.normal.map(f32::to_bits),
                [x, 0.0, z].map(f32::to_bits)
            );
        }
    }
}

#[test]
fn exact_recipe_provenance_stable_identity_and_cooked_roundtrip() {
    let input = recipe_bytes();
    let a = author_cylinder(&input, RECIPE_ID).unwrap();
    assert_eq!(a.source().asset_id, RECIPE_ID);
    assert_eq!(
        a.source().dependencies,
        [Dependency {
            uri: "$source".into(),
            sha256: digest(&input)
        }]
    );
    let bytes = a.to_bytes().unwrap();
    assert_eq!(
        StaticModel::from_bytes(&bytes).unwrap().to_bytes().unwrap(),
        bytes
    );
    assert_eq!(
        author_cylinder(&input, RECIPE_ID)
            .unwrap()
            .to_bytes()
            .unwrap(),
        bytes
    );
    let mut padded = input.clone();
    padded.resize(MAX_RECIPE_BYTES, b' ');
    let b = author_cylinder(&padded, RECIPE_ID).unwrap();
    assert_eq!(a.source().primitives, b.source().primitives);
    assert_eq!(a.source().materials, b.source().materials);
    assert_eq!(a.source().images, b.source().images);
    assert_eq!(b.source().dependencies[0].sha256, digest(&padded));
    assert_ne!(a.source().dependencies, b.source().dependencies);
    padded.push(b' ');
    assert!(author_cylinder(&padded, RECIPE_ID).is_err());
    let changed = author_cylinder(
        &serde_json::to_vec(&recipe(0.75, 3.0, 64)).unwrap(),
        RECIPE_ID,
    )
    .unwrap();
    assert_eq!(
        a.source()
            .primitives
            .iter()
            .map(|p| &p.id)
            .collect::<Vec<_>>(),
        changed
            .source()
            .primitives
            .iter()
            .map(|p| &p.id)
            .collect::<Vec<_>>()
    );
    let renamed = author_cylinder(&input, "props/another.cylinder.json").unwrap();
    assert_ne!(
        a.source().primitives[0].id,
        renamed.source().primitives[0].id
    );
    assert_eq!(a.source().dependencies, renamed.source().dependencies);
}

#[test]
fn strict_recipe_rejects_missing_duplicate_unknown_shapes_and_numeric_types() {
    let good = recipe(0.5, 1.0, 32);
    for key in ["format", "version", "radius", "height", "segments"] {
        let mut missing = good.clone();
        missing.as_object_mut().unwrap().remove(key);
        reject(&missing);
        let mut null = good.clone();
        null[key] = Value::Null;
        reject(&null);
        let text = serde_json::to_string(&good).unwrap();
        let duplicate = format!("{{\"{key}\":{},{}", good[key], &text[1..]);
        assert!(author_cylinder(duplicate.as_bytes(), RECIPE_ID).is_err());
    }
    for (key, values) in [
        (
            "format",
            vec![json!("orr_static_model"), json!(1), json!([])],
        ),
        (
            "version",
            vec![
                json!(0),
                json!(2),
                json!(1.0),
                json!(1.5),
                json!("1"),
                json!(true),
            ],
        ),
        (
            "segments",
            vec![
                json!(0),
                json!(3),
                json!(7),
                json!(9),
                json!(24),
                json!(127),
                json!(129),
                json!(256),
                json!(-8),
                json!(32.0),
                json!(32.5),
                json!("32"),
                json!(true),
                json!(4294967296_u64),
            ],
        ),
        (
            "radius",
            vec![json!("0.5"), json!(false), json!([]), json!({})],
        ),
        (
            "height",
            vec![json!("1"), json!(true), json!([]), json!({})],
        ),
    ] {
        for value in values {
            let mut bad = good.clone();
            bad[key] = value;
            reject(&bad);
        }
    }
    for key in [
        "source",
        "texture",
        "transform",
        "caps",
        "material",
        "unexpected",
    ] {
        let mut bad = good.clone();
        bad[key] = json!(true);
        reject(&bad);
    }
    for bad in ["", "{", "{}", "[]", "null", "true", "1", "[\"orr_static_model_cylinder\",1,0.5,1,32]", "{\"format\":\"orr_static_model_cylinder\",\"version\":1,\"radius\":NaN,\"height\":1,\"segments\":32}"] {
        assert!(author_cylinder(bad.as_bytes(), RECIPE_ID).is_err());
    }
    let mut trailing = recipe_bytes();
    trailing.extend_from_slice(b"{}");
    assert!(author_cylinder(&trailing, RECIPE_ID).is_err());
    assert!(author_cylinder(&[0xff], RECIPE_ID).is_err());
}

#[test]
fn dimensions_are_bounded_in_f64_before_rounding_to_f32() {
    for key in ["radius", "height"] {
        // The just-outside values round to a permitted f32. Admission must still
        // reject them based on the original recipe number.
        for value in [
            0.0,
            -0.0,
            -1.0,
            1e-50,
            0.000_999_999_999,
            1_000_000.000_01,
            1e100,
        ] {
            let mut bad = recipe(0.5, 1.0, 32);
            bad[key] = json!(value);
            reject(&bad);
        }
        let good = recipe(0.5, 1.0, 32);
        let text = serde_json::to_string(&good).unwrap();
        let token = format!("\"{key}\":{}", good[key]);
        assert!(text.contains(&token));
        let text = text.replace(&token, &format!("\"{key}\":1e999"));
        assert!(author_cylinder(text.as_bytes(), RECIPE_ID).is_err());
    }
    for value in [0.001, 1_000_000.0] {
        let model = author_cylinder(
            &serde_json::to_vec(&recipe(value, value, 8)).unwrap(),
            RECIPE_ID,
        )
        .unwrap();
        assert_geometry(&model, value as f32, value as f32, 8);
    }
}

#[test]
fn logical_identity_rejects_nonportable_reserved_and_traversal_paths() {
    for bad in [
        "",
        "/key.cylinder.json",
        "../key.cylinder.json",
        "a/../key.cylinder.json",
        "a//key.cylinder.json",
        "./key.cylinder.json",
        "a\\key.cylinder.json",
        "a b.cylinder.json",
        "a%20b.cylinder.json",
        "https://host/key.cylinder.json",
        "CON.cylinder.json",
        "aux.cylinder.json",
        "COM0.cylinder.json",
        "lpt9.cylinder.json",
        "a./key.cylinder.json",
        ".hidden/key.cylinder.json",
        "key.CYLINDER.JSON",
        "key.model.json",
        "한글.cylinder.json",
    ] {
        assert!(
            author_cylinder(&recipe_bytes(), bad).is_err(),
            "accepted {bad}"
        );
    }
    let longest = format!("{}.cylinder.json", "a".repeat(226));
    assert_eq!(longest.len(), 240);
    assert!(author_cylinder(&recipe_bytes(), &longest).is_ok());
    assert!(author_cylinder(&recipe_bytes(), &format!("a{longest}")).is_err());
}

static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap();
        Self::under(root)
    }
    fn under(root: PathBuf) -> Self {
        assert!(root.is_absolute());
        for ancestor in root.ancestors() {
            let meta = fs::symlink_metadata(ancestor).unwrap();
            assert!(meta.is_dir() && !meta.file_type().is_symlink());
        }
        let path = root.join(format!(
            "orr-cylinder-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
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
struct Fixture {
    temp: Temp,
    input: PathBuf,
    output: PathBuf,
    recipe: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        Self::with_temp(Temp::new())
    }
    fn with_temp(temp: Temp) -> Self {
        let input = temp.join("key.cylinder.json");
        let output = temp.join("key.model.json");
        let recipe = recipe_bytes();
        fs::write(&input, &recipe).unwrap();
        Self {
            temp,
            input,
            output,
            recipe,
        }
    }
    fn args(&self) -> Vec<OsString> {
        args(&self.input, &self.output)
    }
    fn unchanged(&self) {
        assert_eq!(fs::read(&self.input).unwrap(), self.recipe);
    }
}
fn args(input: &Path, output: &Path) -> Vec<OsString> {
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
            let path = entry.unwrap().path();
            paths.push(path.strip_prefix(root).unwrap().to_owned());
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() && !meta.file_type().is_symlink() {
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
fn cli_publishes_once_without_overwrite_and_preserves_recipe() {
    let f = Fixture::new();
    let before = tree(&f.temp.0);
    authoring::run(f.args()).unwrap();
    let cooked = fs::read(&f.output).unwrap();
    assert_geometry(&StaticModel::from_bytes(&cooked).unwrap(), 0.5, 1.0, 32);
    assert_eq!(
        cooked,
        author_cylinder(&f.recipe, RECIPE_ID)
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    let after = tree(&f.temp.0);
    assert_eq!(after.len(), before.len() + 1);
    assert!(authoring::run(f.args()).is_err());
    assert_eq!(fs::read(&f.output).unwrap(), cooked);
    assert_eq!(tree(&f.temp.0), after);
    let alias = f.temp.join("recipe-alias.model.json");
    fs::hard_link(&f.input, &alias).unwrap();
    let before = tree(&f.temp.0);
    assert!(authoring::run(args(&f.input, &alias)).is_err());
    assert_eq!(fs::read(&alias).unwrap(), f.recipe);
    assert_eq!(tree(&f.temp.0), before);
    f.unchanged();
}

#[test]
fn cli_three_unique_pairs_help_and_bounded_argument_stream() {
    let f = Fixture::new();
    let before = tree(&f.temp.0);
    authoring::run([OsString::from("--help")]).unwrap();
    authoring::run([OsString::from("-h")]).unwrap();
    let mut consumed = 0;
    let infinite = std::iter::from_fn(|| {
        consumed += 1;
        assert!(consumed <= 8);
        Some(OsString::from("--unknown"))
    });
    assert!(authoring::run(infinite).is_err());
    assert!(consumed > 6);
    assert_eq!(tree(&f.temp.0), before);
    let good = f.args();
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
        (0, "--force"),
        (0, "--install"),
        (0, "--network"),
        (0, "--input"),
        (0, "--help"),
        (1, "-"),
        (2, "--recipe"),
        (4, "--asset-id"),
        (3, "../bad.cylinder.json"),
    ] {
        let mut bad = good.clone();
        bad[index] = value.into();
        cases.push(bad);
    }
    for bad in cases {
        assert!(authoring::run(bad).is_err());
        assert!(!f.output.exists());
        assert_eq!(tree(&f.temp.0), before);
        f.unchanged();
    }
    authoring::run(
        good.chunks(2)
            .rev()
            .flat_map(|pair| pair.iter().cloned())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(StaticModel::from_bytes(&fs::read(&f.output).unwrap()).is_ok());
    f.unchanged();
}

#[test]
fn cli_portable_output_names_and_package_store_rejection() {
    let f = Fixture::new();
    let before = tree(&f.temp.0);
    for name in [
        "key.png",
        "key.json",
        "key.MODEL.JSON",
        "key name.model.json",
        "한글.model.json",
        "CON.model.json",
        "prn.model.json",
        "AUX.model.json",
        "nul.model.json",
        "COM1.model.json",
        "lpt9.model.json",
    ] {
        let output = f.temp.join(name);
        assert!(authoring::run(args(&f.input, &output)).is_err());
        assert!(!output.exists());
        assert_eq!(tree(&f.temp.0), before);
    }
    assert!(authoring::run(args(
        &f.input,
        &f.temp.join(&("x".repeat(100) + ".model.json"))
    ))
    .is_err());
    for name in [".orr.", ".orr ", "ordinary.", "ordinary "] {
        let error =
            authoring::run(args(&f.input, &f.temp.join(name).join("key.model.json"))).unwrap_err();
        assert!(error.contains("ending in a dot or space"), "{error}");
        assert_eq!(tree(&f.temp.0), before);
    }
    for (index, name) in [".orr", ".ORR", ".OrR"].into_iter().enumerate() {
        let parent = f.temp.join(&format!("store-{index}"));
        fs::create_dir(&parent).unwrap();
        let store = parent.join(name);
        fs::create_dir(&store).unwrap();
        let before = tree(&f.temp.0);
        assert!(authoring::run(args(&f.input, &store.join("key.model.json"))).is_err());
        assert_eq!(tree(&f.temp.0), before);
    }
    assert!(authoring::run(args(&f.input, &f.temp.join("missing/key.model.json"))).is_err());
    let file_parent = f.temp.join("file-parent");
    fs::write(&file_parent, b"not a directory").unwrap();
    assert!(authoring::run(args(&f.input, &file_parent.join("key.model.json"))).is_err());
    assert_eq!(fs::read(file_parent).unwrap(), b"not a directory");
    fs::create_dir(&f.output).unwrap();
    let before = tree(&f.temp.0);
    assert!(authoring::run(f.args()).is_err());
    assert_eq!(tree(&f.temp.0), before);
    f.unchanged();
}

#[test]
fn cli_missing_nonregular_malformed_and_oversized_inputs_never_publish() {
    let f = Fixture::new();
    fs::remove_file(&f.input).unwrap();
    let before = tree(&f.temp.0);
    assert!(authoring::run(f.args()).is_err());
    assert_eq!(tree(&f.temp.0), before);
    fs::create_dir(&f.input).unwrap();
    let before = tree(&f.temp.0);
    assert!(authoring::run(f.args()).is_err());
    assert_eq!(tree(&f.temp.0), before);
    fs::remove_dir(&f.input).unwrap();
    for input in [
        b"malformed".to_vec(),
        serde_json::to_vec(&recipe(0.0, 1.0, 32)).unwrap(),
        vec![b' '; MAX_RECIPE_BYTES + 1],
    ] {
        fs::write(&f.input, &input).unwrap();
        let before = tree(&f.temp.0);
        assert!(authoring::run(f.args()).is_err());
        assert_eq!(fs::read(&f.input).unwrap(), input);
        assert_eq!(tree(&f.temp.0), before);
        assert!(!f.output.exists());
    }
}

fn raw_parent_path(child: &Path, leaf: &str) -> OsString {
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
    let f = Fixture::new();
    let child = f.temp.join("child");
    fs::create_dir(&child).unwrap();
    let raw_input = raw_parent_path(&child, "key.cylinder.json");
    let raw_output = raw_parent_path(&child, "key.model.json");
    for args in [
        args(Path::new(&raw_input), &f.output),
        args(&f.input, Path::new(&raw_output)),
    ] {
        let before = tree(&f.temp.0);
        assert_eq!(
            authoring::run(args).unwrap_err(),
            "empty paths and parent traversal are unsupported"
        );
        assert_eq!(tree(&f.temp.0), before);
        assert!(!f.output.exists());
    }
    f.unchanged();
}

#[cfg(unix)]
#[test]
fn cli_symlink_inputs_ancestors_and_destinations_reject() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let alias = f.temp.join("linked.cylinder.json");
    symlink(&f.input, &alias).unwrap();
    let before = tree(&f.temp.0);
    assert!(authoring::run(args(&alias, &f.output)).is_err());
    assert_eq!(tree(&f.temp.0), before);
    for target in [&f.input, &f.temp.join("absent")] {
        symlink(target, &f.output).unwrap();
        let before = tree(&f.temp.0);
        assert!(authoring::run(f.args()).is_err());
        assert_eq!(tree(&f.temp.0), before);
        assert!(fs::symlink_metadata(&f.output)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_file(&f.output).unwrap();
    }
    let parent = f.temp.join("linked-parent");
    symlink(&f.temp.0, &parent).unwrap();
    let before = tree(&f.temp.0);
    assert!(authoring::run(args(&parent.join("key.cylinder.json"), &f.output)).is_err());
    assert!(authoring::run(args(&f.input, &parent.join("key.model.json"))).is_err());
    assert_eq!(tree(&f.temp.0), before);
    assert!(!f.output.exists());
    f.unchanged();
}

#[cfg(unix)]
#[test]
fn cli_non_utf8_option_names_and_logical_ids_reject() {
    use std::os::unix::ffi::OsStringExt;
    let f = Fixture::new();
    let before = tree(&f.temp.0);
    for (index, expected) in [
        (0, "option names must be UTF-8"),
        (3, "--asset-id must be UTF-8"),
    ] {
        let mut bad = f.args();
        bad[index] = OsString::from_vec(b"invalid-\xff".to_vec());
        assert_eq!(authoring::run(bad).unwrap_err(), expected);
        assert_eq!(tree(&f.temp.0), before);
        assert!(!f.output.exists());
    }
    f.unchanged();
}

#[cfg(target_os = "linux")]
#[test]
fn cli_non_utf8_filesystem_inputs_work() {
    use std::os::unix::ffi::OsStringExt;
    let temp = Temp::new();
    let parent = temp.0.join(OsString::from_vec(b"parent-\xff".to_vec()));
    fs::create_dir(&parent).unwrap();
    let input = parent.join(OsString::from_vec(b"recipe-\xfe.cylinder.json".to_vec()));
    let output = parent.join("key.model.json");
    fs::write(&input, recipe_bytes()).unwrap();
    authoring::run(args(&input, &output)).unwrap();
    assert_geometry(
        &StaticModel::from_bytes(&fs::read(&output).unwrap()).unwrap(),
        0.5,
        1.0,
        32,
    );
    assert_eq!(fs::read(&input).unwrap(), recipe_bytes());
}

#[cfg(windows)]
#[test]
fn cli_windows_nonlocal_prefixes_reject() {
    let f = Fixture::new();
    let before = tree(&f.temp.0);
    for remote in [
        r"\\server\share\key.model.json",
        r"\\?\UNC\server\share\key.model.json",
        r"\\.\pipe\key.model.json",
    ] {
        assert!(authoring::run(args(&f.input, Path::new(remote)))
            .unwrap_err()
            .contains("nonlocal Windows path prefixes"));
    }
    for remote in [
        r"\\server\share\key.cylinder.json",
        r"\\?\UNC\server\share\key.cylinder.json",
        r"\\.\pipe\key.cylinder.json",
    ] {
        assert!(authoring::run(args(Path::new(remote), &f.output))
            .unwrap_err()
            .contains("nonlocal Windows path prefixes"));
    }
    assert_eq!(tree(&f.temp.0), before);
    assert!(!f.output.exists());
    f.unchanged();
}

#[cfg(unix)]
#[test]
fn cli_unix_fifo_inputs_reject_without_opening_or_publishing() {
    use std::os::unix::fs::FileTypeExt;
    let f = Fixture::new();
    fs::remove_file(&f.input).unwrap();
    // A real FIFO covers special-file admission without relying on the host's
    // socket permission. No writer exists, so incorrectly opening it would
    // block and fail the acceptance runner's process timeout.
    let created = Command::new("mkfifo").arg(&f.input).output().unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    assert!(fs::symlink_metadata(&f.input)
        .unwrap()
        .file_type()
        .is_fifo());
    let before = tree(&f.temp.0);
    assert!(authoring::run(f.args())
        .unwrap_err()
        .contains("bounded regular file"));
    assert_eq!(tree(&f.temp.0), before);
    assert!(fs::symlink_metadata(&f.input)
        .unwrap()
        .file_type()
        .is_fifo());
    assert!(!f.output.exists());
}

#[test]
#[ignore = "mandatory hosted producer contract requires the pinned current-source CLI"]
fn actual_cli_publishes_validated_output_and_rejects_second_write() {
    let binary = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_CYLINDER_BIN").expect("fresh actual CLI required"),
    );
    assert!(binary.is_absolute());
    assert!(fs::metadata(&binary).unwrap().is_file());
    let f = Fixture::new();
    let result = Command::new(&binary).args(f.args()).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bytes = fs::read(&f.output).unwrap();
    let model = StaticModel::from_bytes(&bytes).unwrap();
    assert_geometry(&model, 0.5, 1.0, 32);
    assert_eq!(
        model.source().dependencies,
        [Dependency {
            uri: "$source".into(),
            sha256: digest(&f.recipe)
        }]
    );
    assert_eq!(model.to_bytes().unwrap(), bytes);
    let before = tree(&f.temp.0);
    let result = Command::new(&binary).args(f.args()).output().unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(fs::read(&f.output).unwrap(), bytes);
    assert_eq!(tree(&f.temp.0), before);
    f.unchanged();
    let help = Command::new(&binary).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("static_model_cylinder"));
    let invalid = Command::new(&binary).arg("--force").output().unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert_eq!(tree(&f.temp.0), before);
}
