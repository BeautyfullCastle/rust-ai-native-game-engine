use super::*;

#[cfg(feature = "mesh-prop-shading")]
fn shade(scene: &std::path::Path, revision: [u8; 32]) {
    let (mut document, stamp) = mesh_props::load(scene, revision).unwrap().unwrap();
    let model = document.props[0].art.source_model_arc();
    document.props[0].shading = Some(
        crate::mesh_prop_shading::Settings::new(
            vec![crate::mesh_prop_shading::Slot {
                material: model.source().primitives[0].material,
                metallic: 0.8,
                roughness: 0.3,
            }],
            model,
        )
        .unwrap(),
    );
    mesh_props::save(scene, Some(&document), Some(&stamp), revision).unwrap();
}

#[cfg(feature = "mesh-prop-shading")]
#[test]
fn shaded_export_refuses_and_existing_diffuse_bundle_stays_intact() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_props(&scene, revision);
    fs::write(&runtime, test_runtime_with_props()).unwrap();
    let existing = temporary.path().join("legacy-bundle");
    export(&scene, checksum, &existing, &runtime).unwrap();
    open_bundle(&existing).unwrap();
    let manifest_before = fs::read(existing.join(MANIFEST_FILENAME)).unwrap();
    shade(&scene, revision);
    let absent = temporary.path().join("shaded-bundle");
    let error = export(&scene, checksum, &absent, &runtime).unwrap_err();
    assert!(
        error.contains("Scalar-shaded props are Editor-only"),
        "{error}"
    );
    assert!(!absent.exists());
    assert!(export(&scene, checksum, &existing, &runtime).is_err());
    assert_eq!(
        fs::read(existing.join(MANIFEST_FILENAME)).unwrap(),
        manifest_before
    );
    open_bundle(&existing).unwrap();
}

#[cfg(feature = "mesh-prop-shading")]
#[test]
fn shaded_bundle_open_refuses_even_when_hashes_are_updated() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_props(&scene, revision);
    fs::write(&runtime, test_runtime_with_props()).unwrap();
    let output = temporary.path().join("bundle");
    let receipt = export(&scene, checksum, &output, &runtime).unwrap();
    open_bundle(&output).unwrap();
    shade(&receipt.scene, revision);
    let props_path = mesh_props::sidecar_path(&receipt.scene).unwrap();
    let bytes = fs::read(&props_path).unwrap();
    let path = output.join(MANIFEST_FILENAME);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["props"]["sha256"] = serde_json::json!(super::super::digest(&bytes));
    manifest["props"]["bytes"] = serde_json::json!(bytes.len());
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let error = open_bundle(&output).unwrap_err();
    assert!(
        error.contains("Scalar-shaded props are Editor-only"),
        "{error}"
    );
    assert_eq!(fs::read(props_path).unwrap(), bytes);
}

#[cfg(not(feature = "mesh-prop-shading"))]
#[test]
fn feature_off_shading_refuses_actual_export_and_bundle_open() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_props(&scene, revision);
    fs::write(&runtime, test_runtime_with_props()).unwrap();
    let bundle = temporary.path().join("legacy");
    let receipt = export(&scene, checksum, &bundle, &runtime).unwrap();
    open_bundle(&bundle).unwrap();
    let inject = |scene: &std::path::Path| {
        let path = mesh_props::sidecar_path(scene).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["props"][0]["shading"] = serde_json::json!({"version":1,"slots":[{"material":0,"metallic":0.5,"roughness":0.5}]});
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(path, &bytes).unwrap();
        bytes
    };
    inject(&scene);
    let output = temporary.path().join("unsupported");
    let error = export(&scene, checksum, &output, &runtime).unwrap_err();
    assert!(error.contains("unknown field `shading`"), "{error}");
    assert!(!output.exists());
    let bytes = inject(&receipt.scene);
    let path = bundle.join(MANIFEST_FILENAME);
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["props"]["sha256"] = serde_json::json!(super::super::digest(&bytes));
    value["props"]["bytes"] = serde_json::json!(bytes.len());
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    let error = open_bundle(&bundle).unwrap_err();
    assert!(error.contains("unknown field `shading`"), "{error}");
}
