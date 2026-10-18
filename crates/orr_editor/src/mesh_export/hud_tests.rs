use super::{fixture, graph_revision, test_runtime};
use crate::{mesh_hud::Document, mesh_hud_store};
use std::fs;

fn runtime_with_hud() -> Vec<u8> {
    super::append_runtime_section(test_runtime(), *b".orrhud\0", b"ORR_MESH_HUD_V1")
}

fn validate_hud_marker(bytes: &[u8]) -> Result<(), String> {
    super::super::validate_runtime_with_lighting(
        bytes,
        super::super::RuntimeRequirements {
            require_props: false,
            require_rotation: false,
            require_camera: false,
            require_material: false,
            require_animation: false,
            require_surface: false,
            require_lighting: false,
            require_character: false,
            require_hud: true,
        },
    )
}

fn add_hud(scene: &std::path::Path, revision: [u8; 32]) -> Vec<u8> {
    let document = Document::default();
    let digest = mesh_hud_store::save(scene, Some(&document), None, revision)
        .unwrap()
        .unwrap();
    assert_eq!(
        mesh_hud_store::load(scene, revision).unwrap().unwrap(),
        (document, digest)
    );
    fs::read(mesh_hud_store::sidecar_path(scene).unwrap()).unwrap()
}

#[test]
fn hud_sidecar_exports_as_a_four_file_bundle_and_reopens() {
    let (temporary, scene, runtime, checksum) = fixture();
    let yaml = fs::read_to_string(&scene).unwrap();
    let revision = graph_revision(&yaml);
    let hud_bytes = add_hud(&scene, revision);
    let hud_hash = super::super::digest(&hud_bytes);
    fs::write(&runtime, runtime_with_hud()).unwrap();
    let destination = temporary.path().join("hud-bundle");

    let receipt = super::export(&scene, checksum, &destination, &runtime).unwrap();
    let prepared = super::open_bundle(&destination).unwrap();
    assert_eq!(receipt.hud_sha256.as_deref(), Some(hud_hash.as_str()));
    assert_eq!(
        fs::read(destination.join("walk.scene.yaml.mesh-hud.json")).unwrap(),
        hud_bytes
    );
    let names: std::collections::BTreeSet<_> = fs::read_dir(&destination)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        std::collections::BTreeSet::from([
            "orr.mesh-bundle.json".to_owned(),
            "orr_mesh_player.exe".to_owned(),
            "walk.scene.yaml".to_owned(),
            "walk.scene.yaml.mesh-hud.json".to_owned(),
        ])
    );
    let loaded = mesh_hud_store::load(&prepared.scene, revision)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.1, hud_hash);
    assert_eq!(receipt.hud_sha256.as_deref(), Some(loaded.1.as_str()));
}

#[test]
fn malformed_hud_and_missing_runtime_capability_fail_before_output() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("invalid-hud-bundle");
    let hud_path = mesh_hud_store::sidecar_path(&scene).unwrap();
    fs::write(&hud_path, b"{\"not\":\"a HUD\"}").unwrap();
    assert!(super::export(&scene, checksum, &destination, &runtime).is_err());
    assert!(!destination.exists());

    fs::remove_file(&hud_path).unwrap();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_hud(&scene, revision);
    assert!(super::export(&scene, checksum, &destination, &runtime).is_err());
    assert!(!destination.exists());

    fs::remove_file(&hud_path).unwrap();
    let mut disabled = Document::default();
    disabled.enabled = false;
    mesh_hud_store::save(&scene, Some(&disabled), None, revision)
        .unwrap()
        .unwrap();
    let disabled_destination = temporary.path().join("disabled-hud-bundle");
    assert!(super::export(&scene, checksum, &disabled_destination, &runtime).is_err());
    assert!(!disabled_destination.exists());
}

#[test]
fn hud_marker_must_be_at_the_start_of_its_dedicated_section() {
    validate_hud_marker(&runtime_with_hud()).unwrap();
    assert!(validate_hud_marker(&test_runtime()).is_err());
    let mut misplaced = runtime_with_hud();
    let payload = super::section_payload(&misplaced, b".orrhud\0");
    misplaced[payload..payload + b"ORR_MESH_HUD_V1".len()].fill(0);
    misplaced[payload + 16..payload + 16 + b"ORR_MESH_HUD_V1".len()]
        .copy_from_slice(b"ORR_MESH_HUD_V1");
    assert!(validate_hud_marker(&misplaced).is_err());
    let mut unsectioned = test_runtime();
    unsectioned[0x100..0x100 + b"ORR_MESH_HUD_V1".len()].copy_from_slice(b"ORR_MESH_HUD_V1");
    assert!(validate_hud_marker(&unsectioned).is_err());
}

#[test]
fn unknown_hud_fields_are_rejected_without_publishing() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_hud(&scene, revision);
    let hud_path = mesh_hud_store::sidecar_path(&scene).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&hud_path).unwrap()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("extra".into(), serde_json::json!(true));
    fs::write(&hud_path, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::write(&runtime, runtime_with_hud()).unwrap();
    let destination = temporary.path().join("unknown-hud-bundle");
    assert!(super::export(&scene, checksum, &destination, &runtime).is_err());
    assert!(!destination.exists());
}

#[test]
fn downgraded_runtime_cannot_open_a_hud_bundle_even_with_updated_hashes() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_hud(&scene, revision);
    fs::write(&runtime, runtime_with_hud()).unwrap();
    let destination = temporary.path().join("downgraded-hud-bundle");
    let receipt = super::export(&scene, checksum, &destination, &runtime).unwrap();
    let legacy_runtime = test_runtime();
    fs::write(&receipt.runtime, &legacy_runtime).unwrap();
    let manifest: std::path::PathBuf = receipt.manifest;
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["runtime"]["bytes"] = serde_json::json!(legacy_runtime.len());
    value["runtime"]["sha256"] = serde_json::json!(super::super::digest(&legacy_runtime));
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(super::open_bundle(&destination).is_err());
}
