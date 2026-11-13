use super::*;

fn runtime_with_post_process() -> Vec<u8> {
    runtime_with_mission_marker(test_runtime_with_sections(&[
        (b".orrmesh", b"ORR_MESH_PLAYER_V1"),
        (b".orrlit\0", b"ORR_MESH_LIGHTING_V1"),
        (b".orrpost", b"ORR_MESH_POST_PROCESS_V1"),
    ]))
}

#[test]
fn post_process_runtime_requires_its_own_section_not_lighting_or_overlay_bytes() {
    let old = test_runtime_with_lighting();
    super::super::validate_runtime_post_process(&old, false).unwrap();
    assert!(super::super::validate_runtime_post_process(&old, true)
        .unwrap_err()
        .contains("post-processing"));
    let mut overlay = old;
    overlay.extend_from_slice(b"ORR_MESH_POST_PROCESS_V1");
    assert!(super::super::validate_runtime_post_process(&overlay, true).is_err());
    super::super::validate_runtime_post_process(&runtime_with_post_process(), true).unwrap();
}

#[test]
fn post_process_runtime_rejects_wrong_and_duplicate_sections() {
    for sections in [
        vec![(b".orrpost", b"ORR_MESH_LIGHTING_V1".as_slice())],
        vec![
            (b".orrpost", b"ORR_MESH_POST_PROCESS_V1".as_slice()),
            (b".orrpost", b"ORR_MESH_POST_PROCESS_V1".as_slice()),
        ],
    ] {
        let runtime = test_runtime_with_sections(&sections);
        assert!(super::super::validate_runtime_post_process(&runtime, true).is_err());
    }
}

#[test]
fn post_process_bundle_roundtrip_requires_new_runtime_and_preserves_settings() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    let settings = orr_render::PostProcessSettings {
        enabled: true,
        bloom: true,
        threshold: 0.7,
        strength: 0.8,
        radius: 3,
        iterations: 1,
        ..Default::default()
    };
    let document = crate::mesh_lighting::Document::from_lighting(orr_render::Lighting::default())
        .unwrap()
        .with_post_process(settings)
        .unwrap();
    crate::mesh_lighting_store::save(&scene, Some(&document), None, revision).unwrap();
    let source = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
    let original = fs::read(&source).unwrap();
    let destination = temporary.path().join("post-process-bundle");
    fs::write(&runtime, test_runtime_with_lighting()).unwrap();
    let error = export(&scene, checksum, &destination, &runtime).unwrap_err();
    assert!(error.contains("post-processing"));
    assert!(!destination.exists());
    assert_eq!(fs::read(&source).unwrap(), original);

    fs::write(&runtime, runtime_with_post_process()).unwrap();
    let receipt = export(&scene, checksum, &destination, &runtime).unwrap();
    assert_eq!(
        receipt.lighting_sha256,
        Some(super::super::digest(&original))
    );
    fs::remove_file(source).unwrap();
    fs::remove_file(scene).unwrap();
    let reopened = open_bundle(&destination).unwrap();
    assert_eq!(reopened.checksum, checksum);
    let (reloaded, _) = crate::mesh_lighting_store::load(&reopened.scene, revision)
        .unwrap()
        .unwrap();
    assert!(reloaded.has_post_process());
    assert_eq!(reloaded.post_process_settings(), settings);

    // Re-hashing a substituted legacy runtime does not bypass capability admission.
    let manifest_path = destination.join(MANIFEST_FILENAME);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let old = test_runtime_with_lighting();
    manifest["runtime"]["bytes"] = serde_json::json!(old.len());
    manifest["runtime"]["sha256"] = super::super::digest(&old).into();
    fs::write(destination.join(RUNTIME_FILENAME), old).unwrap();
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(open_bundle(&destination)
        .unwrap_err()
        .contains("post-processing"));
}

#[test]
fn explicitly_disabled_post_process_document_still_requires_schema_aware_runtime() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    let document = crate::mesh_lighting::Document::from_lighting(orr_render::Lighting::default())
        .unwrap()
        .with_post_process(orr_render::PostProcessSettings::default())
        .unwrap();
    crate::mesh_lighting_store::save(&scene, Some(&document), None, revision).unwrap();
    fs::write(&runtime, test_runtime_with_lighting()).unwrap();
    let destination = temporary.path().join("disabled-post-process");
    assert!(export(&scene, checksum, &destination, &runtime)
        .unwrap_err()
        .contains("post-processing"));
    assert!(!destination.exists());
}
