use super::{export, open_bundle, validate_runtime, MANIFEST_FILENAME, RUNTIME_FILENAME};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::{authoring::create_mesh_scene, AgentSpec};
use std::fs;

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
    validate_runtime(&runtime).unwrap();
    runtime[0x44..0x46].copy_from_slice(&0x14cu16.to_le_bytes());
    assert!(validate_runtime(&runtime).is_err());
}

#[test]
fn runtime_marker_outside_executable_section_is_not_accepted() {
    let mut runtime = test_runtime();
    runtime[0x100..0x100 + b"ORR_MESH_PLAYER_V1".len()].copy_from_slice(b"ORR_MESH_PLAYER_V1");
    runtime[0x180..0x180 + b"ORR_MESH_PLAYER_V1".len()].fill(0);

    assert!(validate_runtime(&runtime).is_err());
}

#[test]
fn open_bundle_rejects_unlisted_files() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("bundle");
    export(&scene, checksum, &destination, &runtime).unwrap();
    fs::write(destination.join("extra.txt"), b"not in the manifest").unwrap();

    assert!(open_bundle(&destination).is_err());
}
