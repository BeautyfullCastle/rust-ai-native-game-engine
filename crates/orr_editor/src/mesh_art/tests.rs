use super::{load, save, sidecar_path, sidecar_stamp, Snapshot};
use orr_package::Manifest;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const STATIC_GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");
const LICENSE: &[u8] = b"Example license text\n";

fn package(root: &Path) -> PathBuf {
    let package_root = root.join("verified-static-package");
    fs::create_dir(&package_root).unwrap();
    fs::write(package_root.join("model.glb"), STATIC_GLB).unwrap();
    fs::write(package_root.join("LICENSE.txt"), LICENSE).unwrap();
    let runtime = orr_package::Runtime::content_only();
    let manifest = Manifest {
        schema: 1,
        name: "verified-static".into(),
        version: "1.0.0".into(),
        engine: format!("={}", runtime.engine_version),
        capabilities: BTreeSet::from(["models".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["LICENSE.txt".into(), "model.glb".into()]),
    };
    fs::write(
        package_root.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    package_root
}

fn scene(root: &Path, filename: &str) -> PathBuf {
    let path = root.join(filename);
    fs::write(&path, b"scene\n").unwrap();
    path
}

fn revision() -> [u8; 32] {
    [0x5a; 32]
}

#[test]
fn legacy_sidecars_default_to_identity_without_adding_rotation_or_tint_fields() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = package(&root);
    let snapshot = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", revision()).unwrap();
    let bytes = snapshot.encode_embedded("legacy.scene.yaml").unwrap();
    let encoded: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(encoded.get("rotation").is_none());
    assert!(encoded.get("tint").is_none());
    let loaded = Snapshot::decode_embedded(&bytes, "legacy.scene.yaml", revision()).unwrap();
    assert_eq!(loaded.rotation(), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(loaded.tint(), [1.0; 4]);
    assert_eq!(loaded.encode_embedded("legacy.scene.yaml").unwrap(), bytes);
}

#[test]
fn tint_round_trips_while_original_cooked_digest_and_model_provenance_stay_fixed() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = package(&root);
    let original = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", revision()).unwrap();
    let original_bytes = original.model.to_bytes().unwrap();
    let cooked_digest = original.cooked_sha256.clone();
    let tinted = original.clone().with_tint([0.5, 0.75, 1.0, 0.25]).unwrap();
    assert_eq!(tinted.model.to_bytes().unwrap(), original_bytes);
    assert_eq!(tinted.cooked_sha256, cooked_digest);
    assert_eq!(tinted.tint(), [0.5, 0.75, 1.0, 0.25]);
    let bytes = tinted.encode_embedded("tinted.scene.yaml").unwrap();
    let encoded: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(encoded["tint"], serde_json::json!([0.5, 0.75, 1.0, 0.25]));
    let loaded = Snapshot::decode_embedded(&bytes, "tinted.scene.yaml", revision()).unwrap();
    assert_eq!(loaded.tint(), tinted.tint());
    assert_eq!(loaded.model.to_bytes().unwrap(), original_bytes);
    assert_eq!(loaded.cooked_sha256, cooked_digest);
    assert_eq!(loaded.encode_embedded("tinted.scene.yaml").unwrap(), bytes);
    assert_eq!(
        loaded
            .placement([orr_fp::FP::ZERO; 3])
            .unwrap()
            .model
            .source()
            .materials[0]
            .base_color,
        tinted
            .placement([orr_fp::FP::ZERO; 3])
            .unwrap()
            .model
            .source()
            .materials[0]
            .base_color
    );
    for invalid in [
        serde_json::json!([2, 1, 1, 1]),
        serde_json::json!([1, -1, 1, 1]),
        serde_json::json!([1, 1, 1, "invalid"]),
    ] {
        let mut corrupted = encoded.clone();
        corrupted["tint"] = invalid;
        assert!(Snapshot::decode_embedded(
            &serde_json::to_vec(&corrupted).unwrap(),
            "tinted.scene.yaml",
            revision()
        )
        .is_err());
    }
    assert!(tinted.clone().with_tint([f32::NAN, 1.0, 1.0, 1.0]).is_err());
    assert!(tinted.with_tint([1.01, 1.0, 1.0, 1.0]).is_err());
}

#[test]
fn rotated_snapshot_reopens_without_source_and_translation_updates_preserve_orientation() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = package(&root);
    let scene_path = scene(&root, "rotated.scene.yaml");
    let rotation = crate::mesh_rotation::from_degrees([20.0, 35.0, -15.0]).unwrap();
    let snapshot = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", revision())
        .unwrap()
        .with_rotation(rotation)
        .unwrap()
        .with_transform([1.0, 2.0, 3.0], 0.75)
        .unwrap();
    assert_eq!(
        snapshot
            .placement([orr_fp::FP::ZERO; 3])
            .unwrap()
            .instance
            .rotation,
        snapshot.rotation()
    );
    let saved_hash = save(&scene_path, Some(&snapshot), None).unwrap().unwrap();
    fs::rename(&source, root.join("source-hidden")).unwrap();
    let (loaded, loaded_hash) = load(&scene_path, revision()).unwrap().unwrap();
    assert_eq!(loaded_hash, saved_hash);
    for (actual, expected) in loaded.rotation().into_iter().zip(rotation) {
        assert!((actual - expected).abs() < 1.0e-6);
    }
    assert_eq!(loaded.offset(), [1.0, 2.0, 3.0]);
    assert_eq!(loaded.scale(), 0.75);
    let encoded: serde_json::Value =
        serde_json::from_slice(&loaded.encode_embedded("rotated.scene.yaml").unwrap()).unwrap();
    assert_eq!(encoded["rotation"].as_array().unwrap().len(), 4);
    for invalid in [
        serde_json::json!([0, 0, 0, 0]),
        serde_json::json!([1000001, 0, 0, 1]),
    ] {
        let mut corrupted = encoded.clone();
        corrupted["rotation"] = invalid;
        assert!(Snapshot::decode_embedded(
            &serde_json::to_vec(&corrupted).unwrap(),
            "rotated.scene.yaml",
            revision()
        )
        .is_err());
    }
    assert!(loaded
        .clone()
        .with_rotation([f32::NAN, 0.0, 0.0, 1.0])
        .is_err());
    assert!(loaded.with_rotation([0.0; 4]).is_err());
}

#[test]
fn embedded_snapshot_reopens_without_source_and_preserves_transform_and_license() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = package(&root);
    let scene_path = scene(&root, "room.scene.yaml");
    let snapshot = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", revision())
        .unwrap()
        .with_transform([1.0, 2.0, 3.0], 2.0)
        .unwrap();
    let placement = snapshot
        .placement([
            orr_fp::FP::from_raw(4 * orr_fp::FP::SCALE),
            orr_fp::FP::ZERO,
            orr_fp::FP::from_raw(-2 * orr_fp::FP::SCALE),
        ])
        .unwrap();
    assert_eq!(placement.instance.translation, [5.0, 2.0, 1.0]);
    assert_eq!(placement.instance.scale, [2.0; 3]);

    let sidecar_hash = save(&scene_path, Some(&snapshot), None).unwrap().unwrap();
    assert_eq!(
        sidecar_path(&scene_path).unwrap(),
        root.join("room.scene.yaml.mesh-art.json")
    );
    fs::remove_dir_all(source).unwrap();
    let (reopened, loaded_hash) = load(&scene_path, revision()).unwrap().unwrap();
    assert_eq!(sidecar_hash, loaded_hash);
    assert_eq!(reopened.label(), "verified-static/model.glb");
    assert_eq!(reopened.offset(), [1.0, 2.0, 3.0]);
    assert_eq!(reopened.scale(), 2.0);
    assert_eq!(reopened.graph_revision(), revision());
    assert!(reopened.placement([orr_fp::FP::ZERO; 3]).is_ok());

    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(sidecar_path(&scene_path).unwrap()).unwrap()).unwrap();
    assert_eq!(persisted["license_text"], "Example license text\n");
    assert_eq!(persisted["asset"], "model.glb");
    assert!(persisted["model"]["primitives"]
        .as_array()
        .is_some_and(|items| !items.is_empty()));
}

#[test]
fn verified_cooked_static_package_assets_are_embedded() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let model = orr_model::import::import_with_resolver("model.glb", STATIC_GLB, |_| {
        Err(orr_model::Error::message(
            "external resources are unsupported",
        ))
    })
    .unwrap();
    let asset_bytes = model.to_bytes().unwrap();
    let package_root = root.join("cooked-package");
    fs::create_dir(&package_root).unwrap();
    fs::write(package_root.join("model.orrmodel"), &asset_bytes).unwrap();
    fs::write(package_root.join("LICENSE.txt"), LICENSE).unwrap();
    let runtime = orr_package::Runtime::content_only();
    let manifest = Manifest {
        schema: 1,
        name: "cooked-static".into(),
        version: "1.0.0".into(),
        engine: format!("={}", runtime.engine_version),
        capabilities: BTreeSet::from(["models".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["LICENSE.txt".into(), "model.orrmodel".into()]),
    };
    fs::write(
        package_root.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let snapshot =
        Snapshot::from_package(&package_root, "model.orrmodel", "LICENSE.txt", revision()).unwrap();
    let scene_path = scene(&root, "cooked.scene.yaml");
    save(&scene_path, Some(&snapshot), None).unwrap();
    fs::remove_dir_all(package_root).unwrap();
    assert!(load(&scene_path, revision()).unwrap().is_some());
}

#[test]
fn graph_revision_and_scene_filename_are_bound_to_the_saved_snapshot() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = package(&root);
    let original = scene(&root, "room.scene.yaml");
    let snapshot = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", revision()).unwrap();
    save(&original, Some(&snapshot), None).unwrap();
    assert!(load(&original, [0x5b; 32]).is_err());

    let renamed = scene(&root, "other.scene.yaml");
    fs::copy(
        sidecar_path(&original).unwrap(),
        sidecar_path(&renamed).unwrap(),
    )
    .unwrap();
    assert!(load(&renamed, revision()).is_err());
}

#[test]
fn optimistic_save_and_remove_preserve_a_conflicting_sidecar() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = package(&root);
    let scene_path = scene(&root, "room.scene.yaml");
    let snapshot = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", revision()).unwrap();
    let first_hash = save(&scene_path, Some(&snapshot), None).unwrap().unwrap();
    let sidecar = sidecar_path(&scene_path).unwrap();
    let before = fs::read(&sidecar).unwrap();
    let changed = snapshot
        .clone()
        .with_transform([1.0, 0.0, 0.0], 1.5)
        .unwrap();

    assert!(save(&scene_path, Some(&changed), Some("0".repeat(64).as_str())).is_err());
    assert_eq!(fs::read(&sidecar).unwrap(), before);
    let second_hash = save(&scene_path, Some(&changed), Some(&first_hash))
        .unwrap()
        .unwrap();
    assert_ne!(first_hash, second_hash);
    let before_remove = fs::read(&sidecar).unwrap();
    assert!(save(&scene_path, None, Some(&first_hash)).is_err());
    assert_eq!(fs::read(&sidecar).unwrap(), before_remove);
    assert_eq!(save(&scene_path, None, Some(&second_hash)).unwrap(), None);
    assert!(!sidecar.exists());
}

#[test]
fn missing_sidecar_is_distinct_from_stale_or_malformed_sidecars() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root, "room.scene.yaml");
    assert!(load(&scene_path, revision()).unwrap().is_none());
    let sidecar = sidecar_path(&scene_path).unwrap();
    fs::write(&sidecar, b"not-json").unwrap();
    assert!(load(&scene_path, revision()).is_err());
    let stamp = sidecar_stamp(&scene_path).unwrap().unwrap();
    assert_eq!(stamp.len(), 64);
    assert!(save(&scene_path, None, Some(&"0".repeat(64))).is_err());
    assert!(sidecar.exists());
    assert_eq!(save(&scene_path, None, Some(&stamp)).unwrap(), None);
    assert!(!sidecar.exists());
}

#[test]
fn embedded_cooked_model_digest_detects_sidecar_payload_changes() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = package(&root);
    let scene_path = scene(&root, "room.scene.yaml");
    let snapshot = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", revision()).unwrap();
    save(&scene_path, Some(&snapshot), None).unwrap();
    let sidecar = sidecar_path(&scene_path).unwrap();
    let mut document: serde_json::Value =
        serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    let vertex_x = &mut document["model"]["primitives"][0]["vertices"][0]["position"][0];
    let original_x = vertex_x.as_f64().unwrap();
    *vertex_x = serde_json::json!(original_x + 0.25);
    fs::write(&sidecar, serde_json::to_vec(&document).unwrap()).unwrap();
    assert!(load(&scene_path, revision()).is_err());
}

#[test]
fn transform_and_declared_file_inputs_fail_closed() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = package(&root);
    assert!(Snapshot::from_package(&source, "../model.glb", "LICENSE.txt", revision()).is_err());
    assert!(Snapshot::from_package(&source, "model.glb", "unlisted.txt", revision()).is_err());
    let snapshot = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", revision()).unwrap();
    assert!(snapshot
        .clone()
        .with_transform([f32::INFINITY, 0.0, 0.0], 1.0)
        .is_err());
    assert!(snapshot.with_transform([0.0; 3], 0.0).is_err());
}

#[test]
fn decoded_work_budget_rejects_oversized_geometry() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source_package = package(&root);
    let snapshot =
        Snapshot::from_package(&source_package, "model.glb", "LICENSE.txt", revision()).unwrap();
    let mut source = snapshot.model.source().clone();
    let vertex = source.primitives[0].vertices[0].clone();
    source.primitives[0]
        .vertices
        .resize_with(super::MAX_WORK_VERTICES + 1, || vertex.clone());
    assert!(super::validate_source_budget(&source).is_err());
}

#[cfg(windows)]
#[test]
fn directory_validation_rejects_junctions() {
    use std::process::Command;

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let target = root.join("target");
    let junction = root.join("junction");
    fs::create_dir(&target).unwrap();
    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&target)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(super::checked_directory(&junction, "junction").is_err());
    fs::remove_dir(&junction).unwrap();
}

#[test]
fn snapshot_rejects_unsupported_package_capabilities_and_external_glb_resources() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let unsupported = package(&root);
    let manifest_path = unsupported.join("orr.package.json");
    let mut manifest: Manifest =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest.capabilities.insert("animation".into());
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(Snapshot::from_package(&unsupported, "model.glb", "LICENSE.txt", revision()).is_err());

    let external_root = root.join("external");
    fs::create_dir(&external_root).unwrap();
    let external = package(&external_root);
    let asset_path = external.join("model.glb");
    let json_length = u32::from_le_bytes(STATIC_GLB[12..16].try_into().unwrap()) as usize;
    let json_end = 20 + json_length;
    let mut document: serde_json::Value =
        serde_json::from_slice(&STATIC_GLB[20..json_end]).unwrap();
    document["buffers"][0]["uri"] = serde_json::Value::String("external.bin".into());
    let mut json = serde_json::to_vec(&document).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let binary = &STATIC_GLB[json_end..];
    let total_length = 12 + 8 + json.len() + binary.len();
    let mut glb = Vec::with_capacity(total_length);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total_length as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    glb.extend_from_slice(&json);
    glb.extend_from_slice(binary);
    fs::write(asset_path, glb).unwrap();
    assert!(Snapshot::from_package(&external, "model.glb", "LICENSE.txt", revision()).is_err());
}
