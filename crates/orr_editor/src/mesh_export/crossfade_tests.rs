use super::*;

fn runtime_with_crossfade() -> Vec<u8> {
    append_runtime_section(
        test_runtime_with_character_presentation(),
        *b".orrxfad",
        b"ORR_MESH_CROSSFADE_V1",
    )
}

#[test]
fn crossfade_capability_requires_dedicated_section_not_overlay_or_other_capability() {
    let legacy = test_runtime_with_character_presentation();
    super::super::validate_runtime_crossfade(&legacy, false).unwrap();
    assert!(super::super::validate_runtime_crossfade(&legacy, true).is_err());
    let mut overlay = legacy;
    overlay.extend_from_slice(b"ORR_MESH_CROSSFADE_V1");
    assert!(super::super::validate_runtime_crossfade(&overlay, true).is_err());
    super::super::validate_runtime_crossfade(&runtime_with_crossfade(), true).unwrap();
    for sections in [
        vec![(b".orrxfad", b"ORR_MESH_CHARACTER_V1".as_slice())],
        vec![
            (b".orrxfad", b"ORR_MESH_CROSSFADE_V1".as_slice()),
            (b".orrxfad", b"ORR_MESH_CROSSFADE_V1".as_slice()),
        ],
    ] {
        assert!(super::super::validate_runtime_crossfade(
            &test_runtime_with_sections(&sections),
            true
        )
        .is_err());
    }
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn crossfade_schema2_export_roundtrip_and_runtime_substitution_are_capability_guarded() {
    for ticks in [0, 12] {
        let (temporary, scene, runtime, checksum) = fixture();
        let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
        let package = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/animation_demo")
            .canonicalize()
            .unwrap();
        let snapshot = crate::mesh_animation::Snapshot::from_package(
            &package,
            "animated.glb",
            "LICENSE.txt",
            revision,
        )
        .unwrap();
        let settings = crate::mesh_animation::Settings {
            idle_clip: (ticks > 0).then_some(0),
            crossfade_ticks: Some(ticks),
            ..snapshot.settings()
        };
        let snapshot = snapshot.with_settings(settings).unwrap();
        crate::mesh_animation_store::save(&scene, Some(&snapshot), None, revision).unwrap();
        let source = crate::mesh_animation_store::sidecar_path(&scene).unwrap();
        let original = fs::read(&source).unwrap();
        let scene_original = fs::read(&scene).unwrap();
        let destination = temporary.path().join("crossfade-bundle");
        let legacy = test_runtime_with_character_presentation();
        fs::write(&runtime, &legacy).unwrap();
        assert!(export(&scene, checksum, &destination, &runtime)
            .unwrap_err()
            .contains("crossfade"));
        assert!(!destination.exists());
        assert_eq!(fs::read(&source).unwrap(), original);
        assert_eq!(fs::read(&scene).unwrap(), scene_original);
        assert_eq!(fs::read(&runtime).unwrap(), legacy);

        fs::write(&runtime, runtime_with_crossfade()).unwrap();
        let receipt = export(&scene, checksum, &destination, &runtime).unwrap();
        assert_eq!(
            receipt.animation_sha256,
            Some(super::super::digest(&original))
        );
        fs::remove_file(&source).unwrap();
        fs::remove_file(&scene).unwrap();
        let bundle = open_bundle(&destination).unwrap();
        let (loaded, _) = crate::mesh_animation_store::load(&bundle.scene, revision)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.settings(), settings);
        let manifest_path = destination.join(MANIFEST_FILENAME);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["runtime"]["bytes"] = serde_json::json!(legacy.len());
        manifest["runtime"]["sha256"] = super::super::digest(&legacy).into();
        fs::write(destination.join(RUNTIME_FILENAME), legacy).unwrap();
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(open_bundle(&destination).unwrap_err().contains("crossfade"));
    }
}

#[test]
fn crossfade_marker_coexists_with_post_process_without_implying_it() {
    let crossfade_only = runtime_with_crossfade();
    super::super::validate_runtime_crossfade(&crossfade_only, true).unwrap();
    assert!(super::super::validate_runtime_post_process(&crossfade_only, true).is_err());
    let both = append_runtime_section(crossfade_only, *b".orrpost", b"ORR_MESH_POST_PROCESS_V1");
    super::super::validate_runtime_crossfade(&both, true).unwrap();
    super::super::validate_runtime_post_process(&both, true).unwrap();
}
