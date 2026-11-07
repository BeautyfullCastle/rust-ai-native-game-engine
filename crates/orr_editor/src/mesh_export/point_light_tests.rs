use super::*;
fn point_runtime() -> Vec<u8> {
    runtime_with_mission_marker(test_runtime_with_sections(&[
        (b".orrmesh", b"ORR_MESH_PLAYER_V1"),
        (b".orrlit\0", b"ORR_MESH_LIGHTING_V1"),
        (b".orrpntl", b"ORR_MESH_POINT_LIGHT_V1"),
    ]))
}
#[test]
fn point_light_requires_its_own_capability_and_ignores_overlay_claims() {
    let old = test_runtime_with_lighting();
    super::super::validate_runtime_point_light(&old, false).unwrap();
    assert!(super::super::validate_runtime_point_light(&old, true).is_err());
    let mut overlay = old;
    overlay.extend_from_slice(b"ORR_MESH_POINT_LIGHT_V1");
    assert!(super::super::validate_runtime_point_light(&overlay, true).is_err());
    super::super::validate_runtime_point_light(&point_runtime(), true).unwrap();
}
#[test]
fn point_light_rejects_wrong_and_duplicate_capability_sections() {
    for sections in [
        vec![(b".orrpntl", b"ORR_MESH_LIGHTING_V1".as_slice())],
        vec![
            (b".orrpntl", b"ORR_MESH_POINT_LIGHT_V1".as_slice()),
            (b".orrpntl", b"ORR_MESH_POINT_LIGHT_V1".as_slice()),
        ],
    ] {
        let bytes = test_runtime_with_sections(&sections);
        assert!(super::super::validate_runtime_point_light(&bytes, true).is_err());
    }
}
#[cfg(target_os = "linux")]
#[test]
fn exported_point_light_reopens_source_hidden_and_rejects_rehashed_old_runtime() {
    let (tmp, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    let mut d =
        crate::mesh_lighting::Document::from_lighting(orr_render::Lighting::default()).unwrap();
    d.point_light = Some(orr_render::PointLight {
        position: [2., 3., 4.],
        color: [0.2, 0.7, 0.4],
        intensity: 3.,
        range: 8.,
    });
    crate::mesh_lighting_store::save(&scene, Some(&d), None, revision).unwrap();
    let source = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
    let bytes = fs::read(&source).unwrap();
    let dest = tmp.path().join("point-bundle");
    fs::write(&runtime, test_runtime_with_lighting()).unwrap();
    assert!(
        export(&scene, checksum, &dest, &runtime)
            .unwrap_err()
            .contains("point-light")
    );
    assert!(!dest.exists());
    fs::write(&runtime, point_runtime()).unwrap();
    export(&scene, checksum, &dest, &runtime).unwrap();
    fs::remove_file(source).unwrap();
    fs::remove_file(scene).unwrap();
    let reopened = open_bundle(&dest).unwrap();
    let (saved, _) = crate::mesh_lighting_store::load(&reopened.scene, revision)
        .unwrap()
        .unwrap();
    assert_eq!(saved, d);
    assert_eq!(
        fs::read(crate::mesh_lighting_store::sidecar_path(&reopened.scene).unwrap()).unwrap(),
        bytes
    );
    let old = test_runtime_with_lighting();
    let path = dest.join(MANIFEST_FILENAME);
    let mut m: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    m["runtime"]["bytes"] = serde_json::json!(old.len());
    m["runtime"]["sha256"] = super::super::digest(&old).into();
    fs::write(dest.join(RUNTIME_FILENAME), old).unwrap();
    fs::write(path, serde_json::to_vec(&m).unwrap()).unwrap();
    assert!(open_bundle(&dest).unwrap_err().contains("point-light"));
}
