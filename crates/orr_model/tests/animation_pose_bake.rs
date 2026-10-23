#![cfg(all(feature = "animation", feature = "import"))]
#![allow(clippy::float_arithmetic)]
#[path = "../examples/animation_pose_bake/authoring.rs"]
mod authoring;
use authoring::bake_pose;
use orr_model::{animation::*, Dependency, Image, Material, StaticModel, Vertex, Wrap, IDENTITY};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
const SOURCE_PATH: &str = "inputs/actor.animated.json";
const RECIPE_ID: &str = "variants/actor.pose.json";
const MAX_RECIPE_BYTES: usize = 16 * 1024;
fn recipe() -> Value {
    json!({"format":"orr_animation_pose_bake", "version":1,"source":SOURCE_PATH,"clip":0,"time":0.5})
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
// Independent cooked inputs. Expected geometry below is literal affine algebra,
// never AnimatedModel::sample_clip/deform or a production matrix helper.
fn fixture() -> AnimatedModel {
    let node = |name: &str, children, translation, scale| AnimatedNode {
        name: name.into(),
        children,
        rest: Trs {
            translation,
            scale,
            ..Trs::default()
        },
    };
    let vertices: Vec<_> = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
        .into_iter()
        .enumerate()
        .map(|(i, position)| SkinnedVertex {
            vertex: Vertex {
                position,
                normal: [0.6, 0.8, 0.],
                uv: if i == 0 { [-0., 0.] } else { [i as f32, -0.] },
            },
            joints: [0, 1, 0, 0],
            weights: [0.25, 0.75, 0., 0.],
        })
        .collect();
    let mut rigid = vertices.clone();
    for v in &mut rigid {
        v.joints = [0; 4];
        v.weights = [1., 0., 0., 0.];
    }
    let mut ib0 = IDENTITY;
    ib0[3][0] = -0.5;
    let mut ib1 = IDENTITY;
    ib1[3] = [0., -1., 1., 1.];
    AnimatedModel::new(AnimatedSource {
        format: "orr_animated_model".into(),
        version: 1,
        asset_id: "actors/original.glb".into(),
        dependencies: vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(b"original"),
            },
            Dependency {
                uri: "historical/missing.png".into(),
                sha256: digest(b"missing"),
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
        materials: vec![
            Material {
                base_color: [-0., 0.5, 0.75, 0.25],
                image: 0,
                linear_filter: true,
                wrap_s: Wrap::Mirror,
                wrap_t: Wrap::Repeat,
            },
            Material {
                base_color: [1., 0.25, 0., 1.],
                image: 1,
                linear_filter: false,
                wrap_s: Wrap::Clamp,
                wrap_t: Wrap::Mirror,
            },
        ],
        nodes: vec![
            node("ancestor", vec![1, 2, 3, 4], [10., 20., 30.], [2., 3., 4.]),
            node("joint-a", vec![], [1., 2., 3.], [1., 2., 1.]),
            node("joint-b", vec![], [-1., 1., 0.], [2., 1., 0.5]),
            node("skinned-mesh", vec![], [100., 200., 300.], [3., 5., 7.]),
            node("rigid-mesh", vec![], [7., 8., 9.], [5., 2., 1.]),
        ],
        skins: vec![Skin {
            name: "skin".into(),
            joints: vec![1, 2],
            inverse_bind_matrices: vec![ib0, ib1],
        }],
        primitives: vec![
            AnimatedPrimitive {
                id: "actors/original.glb#node=3/mesh=0/primitive=0".into(),
                node: 3,
                skin: Some(0),
                vertices,
                indices: vec![0, 1, 2],
                material: 0,
            },
            AnimatedPrimitive {
                id: "actors/original.glb#node=4/mesh=1/primitive=0".into(),
                node: 4,
                skin: None,
                vertices: rigid,
                indices: vec![2, 1, 0],
                material: 1,
            },
        ],
        clips: vec![
            AnimationClip {
                name: "move".into(),
                channels: vec![AnimationChannel {
                    node: 0,
                    interpolation: Interpolation::Linear,
                    times: vec![0., 1.],
                    values: ChannelValues::Translation(vec![[10., 20., 30.], [12., 20., 30.]]),
                }],
            },
            AnimationClip {
                name: "other".into(),
                channels: vec![AnimationChannel {
                    node: 0,
                    interpolation: Interpolation::Step,
                    times: vec![0., 1.],
                    values: ChannelValues::Translation(vec![[50., 20., 30.], [60., 20., 30.]]),
                }],
            },
        ],
    })
    .unwrap()
}
fn rich_fixture() -> AnimatedModel {
    fixture()
}
fn apply(input: &[u8], id: &str, source: &[u8]) -> Result<StaticModel, String> {
    bake_pose(input, id, |path| {
        assert_eq!(path, SOURCE_PATH);
        Ok(source.to_vec())
    })
}
fn reject_before_resolution(bytes: &[u8], id: &str) {
    assert!(bake_pose(bytes, id, |p| panic!("invalid recipe resolved {p}")).is_err());
}
fn assert_bits(a: &[f32], b: &[f32]) {
    assert_eq!(
        a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        b.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
}
fn near(a: [f32; 3], b: [f32; 3]) {
    for (a, b) in a.into_iter().zip(b) {
        assert!((a - b).abs() < 0.0001, "{a} != {b}");
    }
}
#[test]
fn weighted_skin_hierarchy_inverse_bind_and_rigid_model_space_oracle() {
    let original = fixture();
    let baked = apply(&recipe_bytes(), RECIPE_ID, &original.to_bytes().unwrap()).unwrap();
    let out = baked.source();
    assert_eq!(out.format, "orr_static_model");
    assert_eq!(out.version, 1);
    assert_eq!(out.asset_id, RECIPE_ID);
    assert_eq!(out.images, original.source().images);
    assert_eq!(out.materials, original.source().materials);
    for (a, b) in out.materials.iter().zip(&original.source().materials) {
        assert_bits(&a.base_color, &b.base_color);
    }
    let normalize = |v: [f32; 3]| {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.map(|x| x / n)
    };
    for (i, p) in out.primitives.iter().enumerate() {
        let old = &original.source().primitives[i];
        assert_eq!(p.transform, IDENTITY);
        assert_eq!(p.indices, old.indices);
        assert_eq!(p.material, old.material);
        assert_eq!(p.id, old.id.replacen("actors/original.glb", RECIPE_ID, 1));
        for (v, old) in p.vertices.iter().zip(&old.vertices) {
            let q = old.vertex.position;
            let expected = if i == 0 {
                [3.5 * q[0] + 9.75, 3.75 * q[1] + 21.5, 2.5 * q[2] + 34.5]
            } else {
                [10. * q[0] + 25., 6. * q[1] + 44., 4. * q[2] + 66.]
            };
            near(v.position, expected);
            near(
                v.normal,
                normalize(if i == 0 {
                    [0.6 / 3.5, 0.8 / 3.75, 0.]
                } else {
                    [0.6 / 10., 0.8 / 6., 0.]
                }),
            );
            assert_bits(&v.uv, &old.vertex.uv);
        }
    }
}
#[test]
fn f64_time_bounds_precede_f32_rounding_and_clip_index_is_exact() {
    let source = fixture().to_bytes().unwrap();
    for time in [
        json!(-1e-50),
        json!(-0.00001),
        json!(1.0000000000000002),
        json!(1e100),
        json!("0.5"),
        json!(null),
        json!(true),
    ] {
        let mut r = recipe();
        r["time"] = time;
        assert!(apply(&serde_json::to_vec(&r).unwrap(), RECIPE_ID, &source).is_err());
    }
    for clip in [
        json!(-1),
        json!(0.0),
        json!("0"),
        json!(2),
        json!(4294967296u64),
    ] {
        let mut r = recipe();
        r["clip"] = clip;
        assert!(apply(&serde_json::to_vec(&r).unwrap(), RECIPE_ID, &source).is_err());
    }
    for (clip, time, x) in [
        (0, 0., 8.75),
        (0, 1., 10.75),
        (1, 0., 48.75),
        (1, 1., 58.75),
    ] {
        let mut r = recipe();
        r["clip"] = json!(clip);
        r["time"] = json!(time);
        near(
            apply(&serde_json::to_vec(&r).unwrap(), RECIPE_ID, &source)
                .unwrap()
                .source()
                .primitives[0]
                .vertices[0]
                .position,
            [x, 21.5, 34.5],
        );
    }
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
    for field in ["format", "version", "source", "clip", "time"] {
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
        json!(["orr_animation_pose_bake", 1, SOURCE_PATH, [0, 0, 0]]),
        json!([["format", "orr_animation_pose_bake"]]),
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
fn logical_paths_reject_nonportable_unnormalized_suffixes_and_aliases() {
    for (field, suffix) in [("source", ".animated.json"), ("asset_id", ".pose.json")] {
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
    let source_path = format!("{}.animated.json", "a".repeat(240 - ".animated.json".len()));
    let id = format!("{}.pose.json", "b".repeat(240 - ".pose.json".len()));
    let mut input = recipe();
    input["source"] = json!(source_path);
    let source = fixture().to_bytes().unwrap();
    let mut calls = 0;
    let model = bake_pose(&serde_json::to_vec(&input).unwrap(), &id, |path| {
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
    bake_pose(&recipe_bytes(), RECIPE_ID, |path| {
        calls.push(path.to_owned());
        Ok(source.clone())
    })
    .unwrap();
    assert_eq!(calls, [SOURCE_PATH]);
    let error = bake_pose(&recipe_bytes(), RECIPE_ID, |path| {
        Err(format!("resolver-denied-{path}"))
    })
    .unwrap_err();
    assert_eq!(error, format!("resolver-denied-{SOURCE_PATH}"));
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
    let other_id = "variants/other.pose.json";
    let other = apply(&input, other_id, &source).unwrap();
    let mut expected = first.source().clone();
    expected.asset_id = other_id.into();
    for p in &mut expected.primitives {
        p.id = format!("{other_id}{}", p.id.strip_prefix(RECIPE_ID).unwrap());
    }
    assert_eq!(other.source(), &expected);
    assert_ne!(other.to_bytes().unwrap(), cooked);
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
            "orr-animation-pose-test-{}-{}",
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
            "orr-animation-pose-socket-{}-{}",
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
        let input = temp.join("chair.pose.json");
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
    authoring::run([OsString::from("-h")]).unwrap();
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
        (3, "../bad.pose.json"),
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
    let nested_input = nested.join("chair.pose.json");
    let mut nested_source = rich_fixture().source().clone();
    nested_source.materials[0].base_color = [0.25, 0.5, 1.0, 0.75];
    let nested_source = AnimatedModel::new(nested_source)
        .unwrap()
        .to_bytes()
        .unwrap();
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
    let raw_input = raw_parent_path(&child, "chair.pose.json");
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
    let recipe_alias = fixture.temp.join("linked.pose.json");
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
        &parent_alias.join("chair.pose.json"),
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
        bad[index] = OsString::from_vec(b"invalid-\xff.pose.json".to_vec());
        assert_eq!(bad[index].as_bytes(), b"invalid-\xff.pose.json");
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
    let input = root.join(OsString::from_vec(b"recipe-\xfe.pose.json".to_vec()));
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
        r"\\server\share\prop.pose.json",
        r"\\?\UNC\server\share\prop.pose.json",
        r"\\.\pipe\prop.pose.json",
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
    use std::os::unix::{ffi::OsStrExt, net::UnixListener};
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
        let listener = UnixListener::bind(&path).unwrap();
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
fn step_boundaries_late_early_single_tracks_and_zero_duration() {
    let mut source = fixture().source().clone();
    source.clips[0].channels[0] = AnimationChannel {
        node: 0,
        interpolation: Interpolation::Step,
        times: vec![0.25, 0.75],
        values: ChannelValues::Translation(vec![[10., 20., 30.], [14., 20., 30.]]),
    };
    source.clips[0].channels.push(AnimationChannel {
        node: 4,
        interpolation: Interpolation::Linear,
        times: vec![0.5],
        values: ChannelValues::Scale(vec![[2., 2., 2.]]),
    });
    source.clips[0].channels.push(AnimationChannel {
        node: 3,
        interpolation: Interpolation::Linear,
        times: vec![0., 1.],
        values: ChannelValues::Translation(vec![[100., 200., 300.]; 2]),
    });
    let bytes = AnimatedModel::new(source.clone())
        .unwrap()
        .to_bytes()
        .unwrap();
    for (time, x) in [
        (0., 8.75),
        (0.25, 8.75),
        (0.749, 8.75),
        (0.75, 12.75),
        (1., 12.75),
    ] {
        let mut r = recipe();
        r["time"] = json!(time);
        let out = apply(&serde_json::to_vec(&r).unwrap(), RECIPE_ID, &bytes).unwrap();
        near(
            out.source().primitives[0].vertices[0].position,
            [x, 21.5, 34.5],
        );
        let v = &out.source().primitives[1].vertices;
        near(
            [
                v[1].position[0] - v[0].position[0],
                v[2].position[1] - v[0].position[1],
                0.,
            ],
            [4., 6., 0.],
        );
    }
    // LINEAR channels clamp independently, before their first and after last key.
    source.clips[0].channels[0].interpolation = Interpolation::Linear;
    let bytes = AnimatedModel::new(source.clone())
        .unwrap()
        .to_bytes()
        .unwrap();
    let mut r = recipe();
    r["time"] = json!(0.5);
    near(
        apply(&serde_json::to_vec(&r).unwrap(), RECIPE_ID, &bytes)
            .unwrap()
            .source()
            .primitives[0]
            .vertices[0]
            .position,
        [10.75, 21.5, 34.5],
    );
    source.clips[0].channels.truncate(1);
    source.clips[0].channels[0].times = vec![0.];
    source.clips[0].channels[0].values = ChannelValues::Translation(vec![[10., 20., 30.]]);
    let bytes = AnimatedModel::new(source).unwrap().to_bytes().unwrap();
    r["time"] = json!(0.);
    assert!(apply(&serde_json::to_vec(&r).unwrap(), RECIPE_ID, &bytes).is_ok());
    r["time"] = json!(1e-50);
    assert!(apply(&serde_json::to_vec(&r).unwrap(), RECIPE_ID, &bytes).is_err());
}

#[test]
fn full_source_admission_rejects_unused_invalid_assets_and_unknown_fields() {
    let good = fixture();
    for (pointer, bad) in [
        ("/primitives/1/indices/0", json!(u32::MAX)),
        ("/primitives/1/material", json!(100)),
        ("/images/1/width", json!(0)),
        ("/materials/1/image", json!(u32::MAX)),
        ("/dependencies/0/sha256", json!("bad")),
        ("/clips/1/channels/0/times/1", json!(-1)),
        ("/skins/0/joints/0", json!(99)),
        ("/nodes/4/rest/scale/0", json!(-1)),
        ("/version", json!(2)),
    ] {
        let mut doc = serde_json::to_value(good.source()).unwrap();
        *doc.pointer_mut(pointer).unwrap() = bad;
        let bytes = serde_json::to_vec(&doc).unwrap();
        assert!(AnimatedModel::from_bytes(&bytes).is_err());
        assert!(apply(&recipe_bytes(), RECIPE_ID, &bytes).is_err());
    }
    for pointer in [
        "",
        "/clips/0",
        "/clips/0/channels/0",
        "/nodes/0",
        "/skins/0",
        "/primitives/0",
        "/materials/0",
        "/images/0",
    ] {
        let mut doc = serde_json::to_value(good.source()).unwrap();
        doc.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unsupported".into(), json!(true));
        assert!(apply(
            &recipe_bytes(),
            RECIPE_ID,
            &serde_json::to_vec(&doc).unwrap()
        )
        .is_err());
    }
    for bytes in [
        b"bad".as_slice(),
        include_bytes!("fixtures/animated_strip.glb").as_slice(),
    ] {
        assert!(apply(&recipe_bytes(), RECIPE_ID, bytes).is_err());
    }
}

#[test]
fn recipe_wrong_types_versions_and_positive_underflow_reject() {
    for (field, value) in [
        ("format", json!("other")),
        ("version", json!(0)),
        ("version", json!(2)),
        ("version", json!(1.0)),
        ("version", json!("1")),
        ("source", json!([])),
        ("clip", json!([])),
        ("time", json!({})),
    ] {
        let mut r = recipe();
        r[field] = value;
        reject_before_resolution(&serde_json::to_vec(&r).unwrap(), RECIPE_ID);
    }
    let mut too_large = recipe_bytes();
    too_large.resize(MAX_RECIPE_BYTES + 1, b' ');
    reject_before_resolution(&too_large, RECIPE_ID);
    let source = fixture().to_bytes().unwrap();
    let mut r = recipe();
    r["time"] = json!(1e-50);
    assert!(apply(&serde_json::to_vec(&r).unwrap(), RECIPE_ID, &source).is_err());
    for raw in ["NaN", "Infinity", "-Infinity", "1e999"] {
        let r=format!("{{\"format\":\"orr_animation_pose_bake\",\"version\":1,\"source\":\"{SOURCE_PATH}\",\"clip\":0,\"time\":{raw}}}");
        reject_before_resolution(r.as_bytes(), RECIPE_ID);
    }
}

#[test]
fn singular_and_mirrored_weighted_samples_fail_without_rest_pose_fallback() {
    for mirrored in [false, true] {
        let mut s = fixture().source().clone();
        for n in &mut s.nodes {
            n.children.clear();
            n.rest = Trs::default();
        }
        s.nodes[4].children = vec![0, 1, 2, 3];
        s.primitives.truncate(1);
        s.skins[0].joints = vec![0, 1, 2];
        s.skins[0].inverse_bind_matrices = vec![IDENTITY; 3];
        for v in &mut s.primitives[0].vertices {
            v.joints = [0, 1, 2, 0];
            v.weights = if mirrored {
                [1. / 3., 1. / 3., 1. / 3., 0.]
            } else {
                [0.5, 0.5, 0., 0.]
            };
        }
        let rotations = if mirrored {
            vec![[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]]
        } else {
            vec![[0., 0., 0., 1.], [0., 0., 1., 0.]]
        };
        s.clips = vec![AnimationClip {
            name: "bad-blend".into(),
            channels: rotations
                .into_iter()
                .enumerate()
                .map(|(i, q)| AnimationChannel {
                    node: i as u32,
                    interpolation: Interpolation::Step,
                    times: vec![0., 1.],
                    values: ChannelValues::Rotation(vec![[0., 0., 0., 1.], q]),
                })
                .collect(),
        }];
        let bytes = AnimatedModel::new(s).unwrap().to_bytes().unwrap();
        let mut r = recipe();
        r["time"] = json!(0.);
        assert!(apply(&serde_json::to_vec(&r).unwrap(), RECIPE_ID, &bytes).is_ok());
        r["time"] = json!(1.);
        let error = apply(&serde_json::to_vec(&r).unwrap(), RECIPE_ID, &bytes).unwrap_err();
        assert!(
            error.contains("singular") || error.contains("mirrored"),
            "{error}"
        );
    }
}

#[test]
fn baked_static_local_position_bounds_are_stricter_than_animation_bounds() {
    let mut source = fixture().source().clone();
    for vertex in &mut source.primitives[0].vertices {
        vertex.vertex.position[0] = 300_000.;
    }
    let model = AnimatedModel::new(source).unwrap();
    let pose = model.sample_clip(0, 0.5).unwrap();
    assert!(model.deform(&pose).unwrap()[0].vertices[0].position[0] > 1_000_000.);
    assert!(
        apply(&recipe_bytes(), RECIPE_ID, &model.to_bytes().unwrap())
            .unwrap_err()
            .contains("baked static model")
    );
}

#[test]
fn combined_recipe_and_source_input_budget_is_enforced() {
    let mut source = fixture().to_bytes().unwrap();
    source.resize(orr_model::MAX_FILE_BYTES - recipe_bytes().len() + 1, b' ');
    assert!(source.len() <= orr_model::MAX_FILE_BYTES);
    assert!(apply(&recipe_bytes(), RECIPE_ID, &source)
        .unwrap_err()
        .contains("64 MiB"));
}
