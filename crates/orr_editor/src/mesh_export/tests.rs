use super::{
    export, open_bundle, validate_runtime, MANIFEST_FILENAME, PROPS_SUFFIX, RUNTIME_FILENAME,
};
use crate::{
    mesh_art::Snapshot,
    mesh_props::{self, Document, Prop},
};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::{authoring::create_mesh_scene, AgentSpec};
use std::fs;

const STATIC_GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");
const LICENSE: &[u8] = b"Example license text\n";

fn admitted_scene() -> (String, [u8; 32], u64) {
    let graph = StaticMeshGraph::bake(
        &[
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [fp!(2), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(2)],
            [fp!(2), FP::ZERO, fp!(2)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let spec = AgentSpec {
        start: [fp!(0.25), FP::ZERO, fp!(0.25)],
        goal: [fp!(1.75), FP::ZERO, fp!(1.75)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.25),
    };
    let mut document = create_mesh_scene(&graph, spec).unwrap();
    let checksum = document.frame().checksum();
    (document.save_yaml(), graph.revision(), checksum)
}

fn test_runtime() -> Vec<u8> {
    let mut bytes = vec![0; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
    bytes[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[0x46..0x48].copy_from_slice(&1u16.to_le_bytes());
    bytes[0x54..0x56].copy_from_slice(&240u16.to_le_bytes());
    bytes[0x56..0x58].copy_from_slice(&0x0002u16.to_le_bytes());
    bytes[0x58..0x5a].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[0x148..0x150].copy_from_slice(b".orrmesh");
    bytes[0x158..0x15c].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x15c..0x160].copy_from_slice(&0x180u32.to_le_bytes());
    bytes[0x16c..0x170].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x180..0x180 + b"ORR_MESH_PLAYER_V1".len()].copy_from_slice(b"ORR_MESH_PLAYER_V1");
    bytes
}

fn test_runtime_with_props() -> Vec<u8> {
    let mut bytes = vec![0; 640];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
    bytes[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[0x46..0x48].copy_from_slice(&2u16.to_le_bytes());
    bytes[0x54..0x56].copy_from_slice(&240u16.to_le_bytes());
    bytes[0x56..0x58].copy_from_slice(&0x0002u16.to_le_bytes());
    bytes[0x58..0x5a].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[0x148..0x150].copy_from_slice(b".orrmesh");
    bytes[0x158..0x15c].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x15c..0x160].copy_from_slice(&0x1c0u32.to_le_bytes());
    bytes[0x16c..0x170].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x170..0x178].copy_from_slice(b".orrprop");
    bytes[0x180..0x184].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x184..0x188].copy_from_slice(&0x200u32.to_le_bytes());
    bytes[0x198..0x19c].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x1c0..0x1c0 + b"ORR_MESH_PLAYER_V1".len()].copy_from_slice(b"ORR_MESH_PLAYER_V1");
    bytes[0x200..0x200 + b"ORR_MESH_PROPS_V1".len()].copy_from_slice(b"ORR_MESH_PROPS_V1");
    bytes
}

fn graph_revision(yaml: &str) -> [u8; 32] {
    let document = orr_remote::mesh_navigation::mesh_doc_from_yaml(yaml).unwrap();
    orr_remote::mesh_navigation::mesh_from_view(orr_bridge::FrameView::of(document.frame()))
        .unwrap()
        .graph
        .revision()
}

fn add_props(scene: &std::path::Path, revision: [u8; 32]) {
    let root = scene.parent().unwrap();
    let package_root = root.join("verified-static-package");
    fs::create_dir(&package_root).unwrap();
    fs::write(package_root.join("model.glb"), STATIC_GLB).unwrap();
    fs::write(package_root.join("LICENSE.txt"), LICENSE).unwrap();
    let runtime = orr_package::Runtime::content_only();
    let manifest = orr_package::Manifest {
        schema: 1,
        name: "verified-static".into(),
        version: "1.0.0".into(),
        engine: format!("={}", runtime.engine_version),
        capabilities: std::collections::BTreeSet::from(["models".into()]),
        dependencies: std::collections::BTreeMap::new(),
        files: std::collections::BTreeSet::from(["LICENSE.txt".into(), "model.glb".into()]),
    };
    fs::write(
        package_root.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let art = Snapshot::from_package(&package_root, "model.glb", "LICENSE.txt", revision).unwrap();
    let document = Document {
        props: vec![Prop {
            id: 1,
            name: "crate".into(),
            art,
        }],
    };
    mesh_props::save(scene, Some(&document), None, revision).unwrap();
}

fn fixture() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    u64,
) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene = root.join("walk.scene.yaml");
    let runtime = root.join("player.exe");
    let (yaml, _, checksum) = admitted_scene();
    fs::write(&scene, yaml).unwrap();
    fs::write(&runtime, test_runtime()).unwrap();
    (temporary, scene, runtime, checksum)
}

#[test]
fn export_opens_self_contained_bundle_without_changing_sources() {
    let (temporary, scene, runtime, checksum) = fixture();
    let scene_before = fs::read(&scene).unwrap();
    let runtime_before = fs::read(&runtime).unwrap();
    let destination = temporary.path().join("bundle");

    let receipt = export(&scene, checksum, &destination, &runtime).unwrap();
    let prepared = open_bundle(&destination).unwrap();

    assert_eq!(receipt.checksum, checksum);
    assert_eq!(prepared.checksum, checksum);
    assert_eq!(receipt.graph_revision, prepared.graph_revision);
    assert_eq!(prepared.runtime_sha256, receipt.runtime_sha256);
    assert_eq!(fs::read(&scene).unwrap(), scene_before);
    assert_eq!(fs::read(&runtime).unwrap(), runtime_before);
    assert!(receipt.scene.is_file());
    assert!(receipt.runtime.is_file());
    assert!(receipt.manifest.is_file());
    assert_eq!(receipt.manifest.file_name().unwrap(), MANIFEST_FILENAME);
    assert_eq!(receipt.scene.file_name().unwrap(), "walk.scene.yaml");
    assert_eq!(receipt.runtime.file_name().unwrap(), RUNTIME_FILENAME);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt.manifest).unwrap()).unwrap();
    assert!(manifest.get("props").is_none());
}

#[test]
fn export_refuses_existing_destination_without_changing_it() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("bundle");
    fs::create_dir(&destination).unwrap();
    let sentinel = destination.join("sentinel");
    fs::write(&sentinel, b"keep").unwrap();

    assert!(export(&scene, checksum, &destination, &runtime).is_err());
    assert_eq!(fs::read(sentinel).unwrap(), b"keep");
}

#[test]
fn open_bundle_rejects_tampered_scene_and_unknown_manifest_fields() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("bundle");
    export(&scene, checksum, &destination, &runtime).unwrap();
    let bundle_scene = destination.join("walk.scene.yaml");
    fs::write(&bundle_scene, b"tampered").unwrap();
    assert!(open_bundle(&destination).is_err());
    fs::write(&bundle_scene, fs::read(&scene).unwrap()).unwrap();

    let manifest_path = destination.join(MANIFEST_FILENAME);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let mut object = manifest.as_object().unwrap().clone();
    object.insert("unexpected".into(), serde_json::Value::Bool(true));
    fs::write(&manifest_path, serde_json::to_vec(&object).unwrap()).unwrap();
    assert!(open_bundle(&destination).is_err());
}

#[test]
fn runtime_validation_requires_x64_pe32_plus_and_marker() {
    let mut runtime = test_runtime();
    validate_runtime(&runtime, false).unwrap();
    runtime[0x44..0x46].copy_from_slice(&0x14cu16.to_le_bytes());
    assert!(validate_runtime(&runtime, false).is_err());
}

#[test]
fn runtime_marker_outside_executable_section_is_not_accepted() {
    let mut runtime = test_runtime();
    runtime[0x100..0x100 + b"ORR_MESH_PLAYER_V1".len()].copy_from_slice(b"ORR_MESH_PLAYER_V1");
    runtime[0x180..0x180 + b"ORR_MESH_PLAYER_V1".len()].fill(0);

    assert!(validate_runtime(&runtime, false).is_err());
}

#[test]
fn open_bundle_rejects_unlisted_files() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("bundle");
    export(&scene, checksum, &destination, &runtime).unwrap();
    fs::write(destination.join("extra.txt"), b"not in the manifest").unwrap();

    assert!(open_bundle(&destination).is_err());
}

#[test]
fn props_sidecar_is_admitted_and_requires_props_capable_runtime() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_props(&scene, revision);
    let props_path = mesh_props::sidecar_path(&scene).unwrap();
    let props_before = fs::read(&props_path).unwrap();
    let destination = temporary.path().join("with-props");

    assert!(export(&scene, checksum, &destination, &runtime).is_err());
    assert!(!destination.exists());
    let props_runtime = temporary.path().join("props-player.exe");
    fs::write(&props_runtime, test_runtime_with_props()).unwrap();
    let receipt = export(&scene, checksum, &destination, &props_runtime).unwrap();
    let prepared = open_bundle(&destination).unwrap();
    let bundle_props = destination.join(format!("walk.scene.yaml{PROPS_SUFFIX}"));
    let expected_props_hash = super::digest(&props_before);
    assert_eq!(
        receipt.props_sha256.as_deref(),
        Some(expected_props_hash.as_str())
    );
    assert_eq!(fs::read(&bundle_props).unwrap(), props_before);
    assert_eq!(fs::read(&props_path).unwrap(), props_before);
    assert_eq!(prepared.graph_revision, revision);
}

#[test]
fn props_bundle_rejects_tampering_and_missing_inventory() {
    let (temporary, scene, _, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_props(&scene, revision);
    let props_runtime = temporary.path().join("props-player.exe");
    fs::write(&props_runtime, test_runtime_with_props()).unwrap();
    let destination = temporary.path().join("bundle");
    export(&scene, checksum, &destination, &props_runtime).unwrap();
    let props_path = destination.join(format!("walk.scene.yaml{PROPS_SUFFIX}"));
    let original = fs::read(&props_path).unwrap();
    fs::write(&props_path, b"tampered").unwrap();
    assert!(open_bundle(&destination).is_err());
    fs::write(&props_path, original).unwrap();
    fs::remove_file(&props_path).unwrap();
    assert!(open_bundle(&destination).is_err());
}

#[test]
fn props_runtime_marker_must_be_in_a_distinct_pe_section() {
    validate_runtime(&test_runtime_with_props(), true).unwrap();
    assert!(validate_runtime(&test_runtime(), true).is_err());
    let mut bytes = test_runtime_with_props();
    bytes[0x200..0x200 + b"ORR_MESH_PROPS_V1".len()].fill(0);
    bytes[0x100..0x100 + b"ORR_MESH_PROPS_V1".len()].copy_from_slice(b"ORR_MESH_PROPS_V1");
    assert!(validate_runtime(&bytes, true).is_err());
    validate_runtime(&test_runtime(), false).unwrap();
}

#[test]
fn props_bundle_rejects_invalid_admission_even_with_updated_file_hash() {
    let (temporary, scene, _, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_props(&scene, revision);
    let runtime = temporary.path().join("props-player.exe");
    fs::write(&runtime, test_runtime_with_props()).unwrap();
    let destination = temporary.path().join("bundle");
    export(&scene, checksum, &destination, &runtime).unwrap();
    let props_path = destination.join(format!("walk.scene.yaml{PROPS_SUFFIX}"));
    let mut props: serde_json::Value =
        serde_json::from_slice(&fs::read(&props_path).unwrap()).unwrap();
    props["graph_revision"] = serde_json::to_value([0_u8; 32]).unwrap();
    let bytes = serde_json::to_vec(&props).unwrap();
    fs::write(&props_path, &bytes).unwrap();
    let manifest_path = destination.join(MANIFEST_FILENAME);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["props"]["bytes"] = serde_json::json!(bytes.len());
    manifest["props"]["sha256"] = serde_json::json!(super::digest(&bytes));
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(open_bundle(&destination).is_err());
}
