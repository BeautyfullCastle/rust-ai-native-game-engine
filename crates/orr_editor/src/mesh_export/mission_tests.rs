#[cfg(feature = "mesh-mission")]
use super::super::RUNTIME_FILENAME;
use super::super::{
    export, open_bundle, validate_game_profile, validate_runtime_game_profile, GameProfile,
    MANIFEST_FILENAME,
};
#[cfg(feature = "mesh-mission")]
use super::legacy_test_runtime;
use super::{fixture, test_runtime};
use std::fs;

#[test]
fn game_profile_support_is_an_explicit_feature_boundary() {
    #[cfg(feature = "mesh-mission")]
    {
        assert!(validate_game_profile(Some(GameProfile::MeshMissionV1)).is_ok());
        assert!(validate_game_profile(None).is_err());
        assert!(
            validate_runtime_game_profile(&test_runtime(), Some(GameProfile::MeshMissionV1))
                .is_ok()
        );
        assert!(validate_runtime_game_profile(
            &legacy_test_runtime(),
            Some(GameProfile::MeshMissionV1)
        )
        .is_err());
        let mut misplaced_marker = legacy_test_runtime();
        misplaced_marker.extend_from_slice(b"ORR_MESH_MISSION_V1");
        assert!(
            validate_runtime_game_profile(&misplaced_marker, Some(GameProfile::MeshMissionV1))
                .is_err()
        );
        let mut damaged_marker = test_runtime();
        let marker_offset = super::section_payload(&damaged_marker, b".orrmis\0");
        damaged_marker[marker_offset] = b'X';
        assert!(
            validate_runtime_game_profile(&damaged_marker, Some(GameProfile::MeshMissionV1))
                .is_err()
        );
    }
    #[cfg(not(feature = "mesh-mission"))]
    {
        assert!(validate_game_profile(None).is_ok());
        assert!(validate_game_profile(Some(GameProfile::MeshMissionV1)).is_err());
        assert!(validate_runtime_game_profile(&test_runtime(), None).is_ok());
    }
}

#[cfg(feature = "mesh-mission")]
#[test]
fn every_mission_build_export_records_profile_even_for_disabled_mission() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("mission-profile-bundle");

    export(&scene, checksum, &destination, &runtime).unwrap();

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join(MANIFEST_FILENAME)).unwrap()).unwrap();
    assert_eq!(manifest["game_profile"], "mesh-mission-v1");
    open_bundle(&destination).unwrap();
}

#[cfg(feature = "mesh-mission")]
#[test]
fn mission_export_rejects_a_player_without_its_dedicated_marker() {
    let (temporary, scene, runtime, checksum) = fixture();
    fs::write(&runtime, legacy_test_runtime()).unwrap();
    let destination = temporary.path().join("missing-mission-capability");

    assert!(export(&scene, checksum, &destination, &runtime).is_err());
    assert!(!destination.exists());
}

#[cfg(feature = "mesh-mission")]
#[test]
fn mission_open_rejects_missing_profile_and_marker_after_file_hash_recheck() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("mission-open-profile");
    export(&scene, checksum, &destination, &runtime).unwrap();
    let manifest_path = destination.join(MANIFEST_FILENAME);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest.as_object_mut().unwrap().remove("game_profile");
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(open_bundle(&destination)
        .unwrap_err()
        .contains("mesh-mission-v1"));

    let second_destination = temporary.path().join("mission-open-marker");
    export(&scene, checksum, &second_destination, &runtime).unwrap();
    let runtime_path = second_destination.join(RUNTIME_FILENAME);
    let legacy_runtime = legacy_test_runtime();
    fs::write(&runtime_path, &legacy_runtime).unwrap();
    let manifest_path = second_destination.join(MANIFEST_FILENAME);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["runtime"]["bytes"] = serde_json::json!(legacy_runtime.len());
    manifest["runtime"]["sha256"] = serde_json::json!(super::super::digest(&legacy_runtime));
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(open_bundle(&second_destination)
        .unwrap_err()
        .contains("MeshMission support"));
}

#[cfg(not(feature = "mesh-mission"))]
#[test]
fn legacy_export_omits_profile_and_rejects_mission_profile_bundles() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("legacy-profile-bundle");
    export(&scene, checksum, &destination, &runtime).unwrap();
    let manifest_path = destination.join(MANIFEST_FILENAME);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert!(manifest.get("game_profile").is_none());
    manifest["game_profile"] = serde_json::json!("mesh-mission-v1");
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(open_bundle(&destination)
        .unwrap_err()
        .contains("mesh-mission bundle"));
}
