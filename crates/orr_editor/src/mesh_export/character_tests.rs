use super::*;

#[test]
fn character_capability_requires_a_dedicated_bounded_marker_section() {
    let capable = test_runtime_with_character_presentation();
    super::super::validate_runtime_with_character(&capable, true).unwrap();
    super::super::validate_runtime_with_character(&test_runtime(), false).unwrap();
    assert!(super::super::validate_runtime_with_character(&test_runtime(), true).is_err());

    let mut shifted = capable.clone();
    let payload = super::section_payload(&shifted, b".orrchar");
    shifted[payload..payload + b"ORR_MESH_CHARACTER_V1".len()].fill(0);
    shifted[0x100..0x100 + b"ORR_MESH_CHARACTER_V1".len()]
        .copy_from_slice(b"ORR_MESH_CHARACTER_V1");
    assert!(super::super::validate_runtime_with_character(&shifted, true).is_err());

    let mut wrong_section = capable.clone();
    let header = super::section_header(&wrong_section, b".orrchar");
    wrong_section[header..header + 8].copy_from_slice(b".orrlit\0");
    assert!(super::super::validate_runtime_with_character(&wrong_section, true).is_err());

    let mut corrupt = capable;
    let payload = super::section_payload(&corrupt, b".orrchar");
    corrupt[payload] ^= 1;
    assert!(super::super::validate_runtime_with_character(&corrupt, true).is_err());
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn follow_camera_and_explicit_idle_clip_require_character_runtime_for_export_and_open() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    let camera = CameraDocument::from_orbit(OrbitCamera::new([1.0, 2.0, 3.0], 0.4, 0.2, 8.0))
        .unwrap()
        .with_follow_offset(Some([0.0, 1.0, 0.0]))
        .unwrap();
    crate::mesh_camera_store::save(&scene, Some(&camera), None, revision).unwrap();

    let package = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap();
    let animation = crate::mesh_animation::Snapshot::from_package(
        &package,
        "animated.glb",
        "LICENSE.txt",
        revision,
    )
    .unwrap();
    let mut settings = animation.settings();
    settings.idle_clip = Some(0);
    let animation = animation.with_settings(settings).unwrap();
    crate::mesh_animation_store::save(&scene, Some(&animation), None, revision).unwrap();

    let legacy_runtime = test_runtime_with_all_presentations();
    fs::write(&runtime, &legacy_runtime).unwrap();
    let destination = temporary.path().join("character-bundle");
    let error = export(&scene, checksum, &destination, &runtime).unwrap_err();
    assert!(error.contains("character-follow"));
    assert!(!destination.exists());

    let capable_runtime = test_runtime_with_character_presentation();
    fs::write(&runtime, &capable_runtime).unwrap();
    let receipt = export(&scene, checksum, &destination, &runtime).unwrap();
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 5);
    let prepared = open_bundle(&destination).unwrap();
    assert_eq!(prepared.checksum, checksum);
    assert_eq!(prepared.graph_revision, revision);
    let camera_bytes =
        fs::read(crate::mesh_camera_store::sidecar_path(&prepared.scene).unwrap()).unwrap();
    let camera_value: serde_json::Value = serde_json::from_slice(&camera_bytes).unwrap();
    assert_eq!(
        camera_value["camera"]["follow_offset"],
        serde_json::json!([0.0, 1.0, 0.0])
    );
    let animation_bytes =
        fs::read(crate::mesh_animation_store::sidecar_path(&prepared.scene).unwrap()).unwrap();
    let animation_value: serde_json::Value = serde_json::from_slice(&animation_bytes).unwrap();
    assert_eq!(
        animation_value["settings"]["idle_clip"],
        serde_json::json!(0)
    );

    let manifest_path = destination.join(MANIFEST_FILENAME);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let mut corrupted_runtime = capable_runtime.clone();
    let payload = super::section_payload(&corrupted_runtime, b".orrchar");
    corrupted_runtime[payload] ^= 1;
    fs::write(&receipt.runtime, &corrupted_runtime).unwrap();
    assert!(open_bundle(&destination).is_err());

    manifest["runtime"]["bytes"] = serde_json::json!(corrupted_runtime.len());
    manifest["runtime"]["sha256"] = serde_json::json!(super::super::digest(&corrupted_runtime));
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(open_bundle(&destination).is_err());

    fs::write(&receipt.runtime, &legacy_runtime).unwrap();
    manifest["runtime"]["bytes"] = serde_json::json!(legacy_runtime.len());
    manifest["runtime"]["sha256"] = serde_json::json!(super::super::digest(&legacy_runtime));
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(open_bundle(&destination).is_err());
}
