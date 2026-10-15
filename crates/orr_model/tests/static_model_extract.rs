#![cfg(feature = "import")]
#![allow(clippy::float_arithmetic)]
#[path = "../examples/static_model_extract/authoring.rs"]
mod authoring;
use authoring::{extract_glb_node, list_glb_nodes};
use orr_model::{StaticModel, IDENTITY};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
const SOURCE_ID: &str = "props/source.glb";
const ORIGINAL: &[u8] = include_bytes!("fixtures/model.glb");

fn parts(bytes: &[u8]) -> (Value, Vec<u8>) {
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    (
        serde_json::from_slice(&bytes[20..20 + length]).unwrap(),
        bytes[28 + length..].to_vec(),
    )
}
fn pack(document: &Value, binary: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(document).unwrap();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut output = b"glTF".to_vec();
    output.extend(2u32.to_le_bytes());
    output.extend(((28 + json.len() + binary.len()) as u32).to_le_bytes());
    output.extend((json.len() as u32).to_le_bytes());
    output.extend(b"JSON");
    output.extend(json);
    output.extend((binary.len() as u32).to_le_bytes());
    output.extend(b"BIN\0");
    output.extend(binary);
    output
}
fn fixture_parts() -> (Value, Vec<u8>) {
    let (mut doc, bin) = parts(ORIGINAL);
    doc["nodes"] = json!([
        {"name":"assembly", "translation":[3,2,1], "children":[1,2,4]},
        {"name":"prop", "mesh":0,"translation":[1,0,0],"scale":[-2,1,1],"children":[3]},
        {"name":"sibling", "mesh":0,"translation":[-10,0,0]},
        {"name":"prop", "mesh":0,"translation":[0,4,0],"scale":[1,2,1]},
        {"name":"empty"},
        {"name":"other scene", "mesh":0}
    ]);
    doc["scenes"] = json!([{"nodes":[0]}, {"nodes":[5]}]);
    (doc, bin)
}
fn fixture() -> Vec<u8> {
    let (d, b) = fixture_parts();
    pack(&d, &b)
}
fn original_model(bytes: &[u8]) -> StaticModel {
    orr_model::import::import_with_resolver(SOURCE_ID, bytes, |_| panic!("fixture is embedded"))
        .unwrap()
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn reject(bytes: &[u8]) {
    assert!(extract_glb_node(bytes, SOURCE_ID, 1).is_err());
    assert!(list_glb_nodes(bytes, SOURCE_ID).is_err());
}

#[test]
fn subtree_preserves_geometry_uv_material_tables_and_source_slot_order() {
    let input = fixture();
    let original = original_model(&input);
    let before = input.clone();
    let result = extract_glb_node(&input, SOURCE_ID, 1).unwrap();
    let src = result.source();
    assert_eq!(input, before);
    assert_eq!(src.primitives.len(), 4);
    assert_eq!(src.primitives, original.source().primitives[..4]);
    assert_eq!(src.materials, original.source().materials);
    assert_eq!(src.images, original.source().images);
    assert_eq!(src.primitives[0].indices, [0, 1, 2, 0, 2, 3, 0, 3, 4]);
    assert_eq!(src.primitives[0].vertices[0].position, [-1., -1., 0.]);
    assert_eq!(src.primitives[0].vertices[0].uv, [0., 1.]);
    assert_eq!(src.primitives[0].material, 0);
    assert_eq!(src.primitives[1].material, 1);
    assert_eq!(
        src.primitives[0].id,
        "props/source.glb#node=1/mesh=0/primitive=0"
    );
    assert_eq!(
        src.primitives[2].id,
        "props/source.glb#node=3/mesh=0/primitive=0"
    );
}
#[test]
fn nested_selection_keeps_ancestor_mirror_nonuniform_transform_without_recentering() {
    let input = fixture();
    let result = extract_glb_node(&input, SOURCE_ID, 3).unwrap();
    let expected = [
        [-2., 0., 0., 0.],
        [0., 2., 0., 0.],
        [0., 0., 1., 0.],
        [4., 6., 1., 1.],
    ];
    assert_eq!(result.source().primitives.len(), 2);
    for primitive in &result.source().primitives {
        assert_eq!(primitive.transform, expected);
    }
    assert!(orr_model::determinant(expected) < 0.);
    assert_eq!(
        result.source().primitives[0].vertices[1].position,
        [1., -1., 0.]
    );
}
#[test]
fn listing_uses_default_scene_nonempty_subtrees_and_unambiguous_indices() {
    let entries = list_glb_nodes(&fixture(), SOURCE_ID).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|n| (n.index, n.primitive_count))
            .collect::<Vec<_>>(),
        [(0, 6), (1, 4), (2, 2), (3, 2)]
    );
    assert_eq!(entries[1].name, entries[3].name);
    assert_eq!(entries[1].name.as_deref(), Some("prop"));
}
#[test]
fn root_selection_roundtrips_exact_full_import_and_is_deterministic() {
    let input = fixture();
    let expected = original_model(&input).to_bytes().unwrap();
    let first = extract_glb_node(&input, SOURCE_ID, 0)
        .unwrap()
        .to_bytes()
        .unwrap();
    let second = extract_glb_node(&input, SOURCE_ID, 0)
        .unwrap()
        .to_bytes()
        .unwrap();
    assert_eq!(first, expected);
    assert_eq!(first, second);
    assert_eq!(
        StaticModel::from_bytes(&first).unwrap().to_bytes().unwrap(),
        first
    );
}
#[test]
fn explicit_and_implicit_default_scenes_control_selection() {
    let (mut d, b) = fixture_parts();
    d["scene"] = json!(1);
    let input = pack(&d, &b);
    assert!(extract_glb_node(&input, SOURCE_ID, 1).is_err());
    let model = extract_glb_node(&input, SOURCE_ID, 5).unwrap();
    assert_eq!(model.source().primitives.len(), 2);
    assert_eq!(model.source().primitives[0].transform, IDENTITY);
    assert_eq!(list_glb_nodes(&input, SOURCE_ID).unwrap()[0].index, 5);
    d.as_object_mut().unwrap().remove("scene");
    let input = pack(&d, &b);
    assert!(extract_glb_node(&input, SOURCE_ID, 1).is_ok());
    assert!(extract_glb_node(&input, SOURCE_ID, 5).is_err());
}
#[test]
fn absent_out_of_scene_and_empty_subtree_selections_reject() {
    let input = fixture();
    for node in [4, 5, 6, u32::MAX] {
        assert!(extract_glb_node(&input, SOURCE_ID, node).is_err(), "{node}");
    }
}
#[test]
fn logical_identity_and_exact_input_digest_survive_extraction() {
    let input = fixture();
    let model = extract_glb_node(&input, SOURCE_ID, 1).unwrap();
    assert_eq!(model.source().asset_id, SOURCE_ID);
    assert_eq!(model.source().dependencies.len(), 1);
    assert_eq!(model.source().dependencies[0].uri, "$source");
    assert_eq!(model.source().dependencies[0].sha256, digest(&input));
    for id in [
        "",
        "../source.glb",
        "/source.glb",
        "https://source.glb",
        "source#x.glb",
        "x\\source.glb",
    ] {
        assert!(extract_glb_node(&input, id, 1).is_err());
        assert!(list_glb_nodes(&input, id).is_err());
    }
}
#[test]
fn full_source_admission_rejects_bad_siblings_before_selection() {
    let (mut d, b) = fixture_parts();
    d["nodes"][2]["scale"] = json!([0, 1, 1]);
    reject(&pack(&d, &b));
    let (mut d, b) = fixture_parts();
    d["nodes"][2]["mesh"] = json!(900);
    reject(&pack(&d, &b));
    let (mut d, b) = fixture_parts();
    d["nodes"][2]["children"] = json!([0]);
    reject(&pack(&d, &b));
    let (mut d, b) = fixture_parts();
    d["nodes"][2]["children"] = json!([3]);
    reject(&pack(&d, &b));
}
#[test]
fn unsupported_animation_skin_extensions_and_attributes_are_never_dropped() {
    for key in ["skins", "animations"] {
        for value in [json!([]), json!(null)] {
            let (mut d, b) = fixture_parts();
            d[key] = value;
            reject(&pack(&d, &b));
        }
    }
    let (mut d, b) = fixture_parts();
    d["nodes"][2]["skin"] = json!(0);
    reject(&pack(&d, &b));
    let (mut d, b) = fixture_parts();
    d["extensionsUsed"] = json!(["KHR_texture_transform"]);
    reject(&pack(&d, &b));
    let (mut d, b) = fixture_parts();
    d["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(2);
    reject(&pack(&d, &b));
}
#[test]
fn external_data_uri_and_unknown_container_resources_reject() {
    for uri in ["external.bin", "data:application/octet-stream;base64,AAAA"] {
        let (mut d, b) = fixture_parts();
        d["buffers"][0]["uri"] = json!(uri);
        reject(&pack(&d, &b));
    }
    for uri in ["external.png", "data:image/png;base64,AAAA"] {
        let (mut d, b) = fixture_parts();
        d["images"][0]["uri"] = json!(uri);
        reject(&pack(&d, &b));
    }
    let (mut d, b) = fixture_parts();
    d["buffers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"byteLength":0}));
    reject(&pack(&d, &b));
    let (mut d, b) = fixture_parts();
    d["images"][0].as_object_mut().unwrap().remove("bufferView");
    reject(&pack(&d, &b));
}
#[test]
fn malformed_envelopes_truncation_and_duplicate_fields_reject() {
    for bytes in [
        b"not a GLB".as_slice(),
        &ORIGINAL[..12],
        &ORIGINAL[..27],
        &ORIGINAL[..ORIGINAL.len() - 1],
    ] {
        reject(bytes);
    }
    let mut bad = fixture();
    bad[4] = 1;
    reject(&bad);
    let mut bad = fixture();
    bad[16..20].copy_from_slice(b"NOPE");
    reject(&bad);
    let mut bad = fixture();
    bad.extend([0; 8]);
    let n = bad.len() as u32;
    bad[8..12].copy_from_slice(&n.to_le_bytes());
    reject(&bad);
    let (d, b) = fixture_parts();
    let json = serde_json::to_string(&d).unwrap();
    let json = json.replacen("\"scene\":0", "\"scene\":0,\"scene\":1", 1);
    assert!(json.contains("\"scene\":0,\"scene\":1"));
    let mut raw = json.into_bytes();
    while raw.len() % 4 != 0 {
        raw.push(b' ');
    }
    let mut duplicate = b"glTF".to_vec();
    duplicate.extend(2u32.to_le_bytes());
    duplicate.extend(((28 + raw.len() + b.len()) as u32).to_le_bytes());
    duplicate.extend((raw.len() as u32).to_le_bytes());
    duplicate.extend(b"JSON");
    duplicate.extend(raw);
    duplicate.extend((b.len() as u32).to_le_bytes());
    duplicate.extend(b"BIN\0");
    duplicate.extend(b);
    reject(&duplicate);
}
#[test]
fn source_json_and_node_budgets_reject_before_output() {
    let (mut d, b) = fixture_parts();
    d["nodes"] = json!((0..1025).map(|_| json!({})).collect::<Vec<_>>());
    reject(&pack(&d, &b));
    let (mut d, b) = fixture_parts();
    d["extras"] = json!("x".repeat(4 * 1024 * 1024));
    reject(&pack(&d, &b));
    let oversized = vec![0; orr_model::MAX_FILE_BYTES + 1];
    reject(&oversized);
}
#[test]
fn unused_material_and_image_slots_are_retained_without_remapping() {
    let (mut d, b) = fixture_parts();
    let mat = d["materials"][1].clone();
    let img = d["images"][0].clone();
    d["materials"].as_array_mut().unwrap().push(mat);
    d["images"].as_array_mut().unwrap().push(img);
    let input = pack(&d, &b);
    let full = original_model(&input);
    let selected = extract_glb_node(&input, SOURCE_ID, 3).unwrap();
    assert_eq!(selected.source().materials.len(), 3);
    assert_eq!(selected.source().images.len(), 3);
    assert_eq!(selected.source().materials, full.source().materials);
    assert_eq!(selected.source().images, full.source().images);
}

static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        // OS temporary paths may use aliases (for example /var on macOS).
        // Positive fixtures use trusted physical ancestry; explicit symlink
        // rejection cases below continue to pass their aliases unchanged.
        let root = std::env::temp_dir().canonicalize().unwrap();
        assert!(root.is_absolute());
        for ancestor in root.ancestors() {
            let metadata = fs::symlink_metadata(ancestor).unwrap();
            assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
        }
        let p = root.join(format!(
            "orr-static-prop-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn join(&self, s: &str) -> PathBuf {
        self.0.join(s)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn args(input: &Path, output: &Path) -> Vec<OsString> {
    vec![
        "--input".into(),
        input.into(),
        "--asset-id".into(),
        SOURCE_ID.into(),
        "--node".into(),
        "1".into(),
        "--output".into(),
        output.into(),
    ]
}
#[test]
fn cli_publishes_reloaded_model_once_and_preserves_source_and_existing_targets() {
    let temp = Temp::new();
    let input = temp.join("source.glb");
    let output = temp.join("prop.model.json");
    let bytes = fixture();
    fs::write(&input, &bytes).unwrap();
    authoring::run(args(&input, &output)).unwrap();
    let cooked = fs::read(&output).unwrap();
    assert_eq!(
        StaticModel::from_bytes(&cooked)
            .unwrap()
            .source()
            .primitives
            .len(),
        4
    );
    assert_eq!(
        cooked,
        extract_glb_node(&bytes, SOURCE_ID, 1)
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    assert!(authoring::run(args(&input, &output)).is_err());
    assert_eq!(fs::read(&output).unwrap(), cooked);
    assert_eq!(fs::read(&input).unwrap(), bytes);
    let alias = temp.join("source.model.json");
    fs::hard_link(&input, &alias).unwrap();
    assert!(authoring::run(args(&input, &alias)).is_err());
    assert_eq!(fs::read(&input).unwrap(), bytes);
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 3);
}
#[test]
fn cli_listing_and_help_do_not_publish_or_change_input() {
    let temp = Temp::new();
    let input = temp.join("source.glb");
    let bytes = fixture();
    fs::write(&input, &bytes).unwrap();
    authoring::run([OsString::from("--help")]).unwrap();
    for list in [
        vec![
            "--input".into(),
            input.clone().into_os_string(),
            "--asset-id".into(),
            SOURCE_ID.into(),
            "--list".into(),
        ],
        vec![
            "--list".into(),
            "--asset-id".into(),
            SOURCE_ID.into(),
            "--input".into(),
            input.clone().into_os_string(),
        ],
    ] {
        authoring::run(list).unwrap();
    }
    assert_eq!(fs::read(&input).unwrap(), bytes);
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
}
#[test]
fn cli_bad_arguments_fail_without_output_or_staging() {
    let temp = Temp::new();
    let input = temp.join("source.glb");
    let output = temp.join("prop.model.json");
    let bytes = fixture();
    fs::write(&input, &bytes).unwrap();
    let good = args(&input, &output);
    let mut cases = vec![vec![], good[..7].to_vec()];
    for start in [0, 2, 4, 6] {
        let mut a = good.clone();
        a.drain(start..start + 2);
        cases.push(a);
    }
    for (i, v) in [
        (0, "--unknown"),
        (4, "--input"),
        (5, "-1"),
        (5, "4294967296"),
        (5, "1.0"),
        (5, ""),
        (3, "../bad.glb"),
        (6, "--list"),
    ] {
        let mut a = good.clone();
        a[i] = v.into();
        cases.push(a);
    }
    let mut a = good.clone();
    a.extend(["--node".into(), "2".into()]);
    cases.push(a);
    for a in cases {
        assert!(authoring::run(a).is_err());
        assert!(!output.exists());
        assert_eq!(fs::read(&input).unwrap(), bytes);
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
    }
    let shuffled = good
        .chunks(2)
        .rev()
        .flat_map(|p| p.iter().cloned())
        .collect::<Vec<_>>();
    authoring::run(shuffled).unwrap();
}
#[test]
fn cli_unsafe_names_ancestry_and_package_store_outputs_reject() {
    let temp = Temp::new();
    let input = temp.join("source.glb");
    fs::write(&input, fixture()).unwrap();
    for name in [
        "prop.glb",
        "prop.json",
        "prop name.model.json",
        "한글.model.json",
        "CON.model.json",
        "LPT1.model.json",
        "AUX.model.json",
    ] {
        let out = temp.join(name);
        assert!(authoring::run(args(&input, &out)).is_err());
        assert!(!out.exists());
    }
    // Win32 dot/space normalization must never turn a different spelling into
    // an installed .orr destination. This rejection precedes metadata access.
    for alias in [".orr.", ".orr ", "ordinary.", "ordinary "] {
        let error =
            authoring::run(args(&input, &temp.join(alias).join("prop.model.json"))).unwrap_err();
        assert!(error.contains("ending in a dot or space"), "{error}");
    }
    #[cfg(windows)]
    for remote in [
        r"\\server\share\prop.model.json",
        r"\\?\UNC\server\share\prop.model.json",
        r"\\.\pipe\prop.model.json",
    ] {
        let error = authoring::run(args(&input, Path::new(remote))).unwrap_err();
        assert!(error.contains("nonlocal Windows path prefixes"), "{error}");
    }
    let long = temp.join(&("x".repeat(100) + ".model.json"));
    assert!(authoring::run(args(&input, &long)).is_err());
    // Distinct parents keep both spellings exercisable on case-insensitive filesystems.
    for (case, dir) in [".orr", ".ORR"].into_iter().enumerate() {
        let parent = temp.join(&format!("package-store-case-{case}"));
        fs::create_dir(&parent).unwrap();
        let store = parent.join(dir);
        fs::create_dir(&store).unwrap();
        let output = store.join("prop.model.json");
        assert!(authoring::run(args(&input, &output)).is_err());
        assert!(!output.exists());
        assert_eq!(fs::read_dir(&store).unwrap().count(), 0);
    }
    assert!(authoring::run(args(&input, &temp.join("missing").join("prop.model.json"))).is_err());
    // PathBuf::join normalizes .. for Windows verbatim roots. Preserve the
    // literal CLI spelling and keep its normalized destination inside our fixture.
    let traversal_child = temp.join("traversal-child");
    fs::create_dir(&traversal_child).unwrap();
    let mut traversal = traversal_child.as_os_str().to_os_string();
    traversal.push(std::path::MAIN_SEPARATOR_STR);
    traversal.push("..");
    traversal.push(std::path::MAIN_SEPARATOR_STR);
    traversal.push("prop.model.json");
    let arguments = args(&input, Path::new(&traversal));
    assert!(Path::new(arguments.last().unwrap())
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir)));
    let error = authoring::run(arguments).unwrap_err();
    assert_eq!(error, "empty paths and parent traversal are unsupported");
    assert!(!temp.join("prop.model.json").exists());
    assert_eq!(fs::read_dir(&traversal_child).unwrap().count(), 0);
}
#[test]
fn cli_invalid_missing_and_oversized_inputs_leave_no_output() {
    let temp = Temp::new();
    let input = temp.join("source.glb");
    let output = temp.join("prop.model.json");
    assert!(authoring::run(args(&input, &output)).is_err());
    assert!(!output.exists());
    fs::write(&input, b"malformed").unwrap();
    assert!(authoring::run(args(&input, &output)).is_err());
    assert_eq!(fs::read(&input).unwrap(), b"malformed");
    let f = fs::File::create(&input).unwrap();
    f.set_len(orr_model::MAX_FILE_BYTES as u64 + 1).unwrap();
    drop(f);
    assert!(authoring::run(args(&input, &output)).is_err());
    assert!(!output.exists());
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
}
#[cfg(unix)]
#[test]
fn cli_symlink_sources_destinations_and_ancestors_reject() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let input = temp.join("source.glb");
    let output = temp.join("prop.model.json");
    let bytes = fixture();
    fs::write(&input, &bytes).unwrap();
    let alias = temp.join("linked.glb");
    symlink(&input, &alias).unwrap();
    assert!(authoring::run(args(&alias, &output)).is_err());
    symlink(&input, &output).unwrap();
    assert!(authoring::run(args(&input, &output)).is_err());
    fs::remove_file(&output).unwrap();
    symlink(temp.join("absent"), &output).unwrap();
    assert!(authoring::run(args(&input, &output)).is_err());
    fs::remove_file(&output).unwrap();
    let dir = temp.join("linked-parent");
    symlink(&temp.0, &dir).unwrap();
    assert!(authoring::run(args(&dir.join("source.glb"), &output)).is_err());
    assert!(authoring::run(args(&input, &dir.join("prop.model.json"))).is_err());
    assert_eq!(fs::read(&input).unwrap(), bytes);
    assert!(!output.exists());
}
#[cfg(unix)]
#[test]
fn cli_nonregular_socket_input_rejects_without_opening_or_publication() {
    use std::os::unix::net::UnixListener;
    let temp = Temp::new();
    let input = temp.join("socket.glb");
    let output = temp.join("prop.model.json");
    let _socket = UnixListener::bind(&input).unwrap();
    assert!(authoring::run(args(&input, &output)).is_err());
    assert!(!output.exists());
}
