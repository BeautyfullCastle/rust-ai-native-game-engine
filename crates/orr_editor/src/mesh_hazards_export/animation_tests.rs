use super::*;

impl Fixture {
    fn animation_path(&self) -> PathBuf {
        self.scene
            .with_file_name("source.scene.yaml.mesh-animation.json")
    }

    #[cfg(feature = "mesh-animated-models")]
    fn add_animation(&self, platform: RuntimePlatform) -> Vec<u8> {
        let revision = admit_scene(&fs::read(&self.scene).unwrap()).unwrap().1;
        let package = Path::new(env!("CARGO_MANIFEST_DIR"))
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
        let bytes = snapshot.encode_embedded("source.scene.yaml").unwrap();
        fs::write(self.animation_path(), &bytes).unwrap();
        fs::write(
            &self.runtime,
            runtime::valid_test_animation_runtime(platform, false, false, false, false),
        )
        .unwrap();
        bytes
    }
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn animation_bundle_rebinds_only_filename_and_relocates_for_both_platforms() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        let fixture = Fixture::new(platform);
        let source = fixture.add_animation(platform);
        let dest = fixture.root.join("animated game 한글");
        let receipt = fixture.export(&dest, true).unwrap();
        assert_eq!(fs::read(fixture.animation_path()).unwrap(), source);
        let mut expected: serde_json::Value = serde_json::from_slice(&source).unwrap();
        expected["scene_filename"] = SCENE_FILENAME.into();
        let saved = fs::read(dest.join(ANIMATION_FILENAME)).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&saved).unwrap(),
            expected
        );
        assert_eq!(
            receipt.animation_sha256.as_deref(),
            Some(digest(&saved).as_str())
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(dest.join(MANIFEST_FILENAME)).unwrap()).unwrap();
        assert_eq!(manifest["schema"], 7);
        assert!(manifest.get("art").is_none());
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 4);
        fs::remove_file(fixture.animation_path()).unwrap();
        fs::remove_file(&fixture.scene).unwrap();
        fs::remove_file(&fixture.runtime).unwrap();
        let relocated = fixture.root.join("relocated");
        fs::rename(&dest, &relocated).unwrap();
        let bundle = open_bundle(&relocated).unwrap();
        assert_eq!(bundle.animation_sha256, receipt.animation_sha256);
        assert_eq!(bundle.checksum, fixture.checksum);
        assert_eq!(bundle.platform, platform);
    }
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn animation_schema_requires_object_and_refuses_legacy_or_static_art_mix() {
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    fixture.add_animation(RuntimePlatform::LinuxX86_64);
    let dest = fixture.root.join("out");
    fixture.export(&dest, true).unwrap();
    let original = fs::read(dest.join(MANIFEST_FILENAME)).unwrap();
    for case in 0..12 {
        fs::write(dest.join(MANIFEST_FILENAME), &original).unwrap();
        mutate_manifest(&dest, |value| match case {
            0..=5 => value["schema"] = (case + 1).into(),
            6 => value["animation"] = serde_json::Value::Null,
            7 => {
                value.as_object_mut().unwrap().remove("animation");
            }
            8 => value["art"] = value["animation"].clone(),
            9 => value["art"] = serde_json::Value::Null,
            10 => value["props"] = serde_json::Value::Null,
            _ => value["animation"]["name"] = "else.mesh-animation.json".into(),
        });
        assert!(open_bundle(&dest).is_err(), "case {case}");
    }
    fs::write(dest.join(MANIFEST_FILENAME), &original).unwrap();
    assert!(open_bundle(&dest).is_ok());
    fs::write(
        fixture
            .scene
            .with_file_name("source.scene.yaml.mesh-art.json"),
        b"{}",
    )
    .unwrap();
    let error = fixture
        .export(&fixture.root.join("mixed"), true)
        .unwrap_err();
    assert!(error.contains("mutually exclusive"));
    assert!(!fixture.root.join("mixed").exists());
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn animation_admission_validates_binding_provenance_settings_and_exact_inventory() {
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    let original = fixture.add_animation(RuntimePlatform::LinuxX86_64);
    for case in 0..6 {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        match case {
            0 => value["scene_filename"] = "wrong.scene.yaml".into(),
            1 => {
                value["graph_revision"][0] =
                    (value["graph_revision"][0].as_u64().unwrap() ^ 1).into()
            }
            2 => value["license_text"] = "changed".into(),
            3 => value["cooked_sha256"] = "0".repeat(64).into(),
            4 => value["settings"]["rate"] = 100.into(),
            _ => value["extra"] = true.into(),
        }
        fs::write(
            fixture.animation_path(),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        let dest = fixture.root.join(format!("bad-{case}"));
        assert!(fixture.export(&dest, true).is_err(), "case {case}");
        assert!(!dest.exists());
    }
    fs::write(fixture.animation_path(), &original).unwrap();
    let dest = fixture.root.join("out");
    fixture.export(&dest, true).unwrap();
    let saved = fs::read(dest.join(ANIMATION_FILENAME)).unwrap();
    fs::write(dest.join(ANIMATION_FILENAME), b"{}").unwrap();
    assert!(open_bundle(&dest).is_err());
    fs::write(dest.join(ANIMATION_FILENAME), saved).unwrap();
    fs::write(dest.join("extra.mesh-animation.json"), b"{}").unwrap();
    assert!(open_bundle(&dest).is_err());
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn animation_source_presence_and_exact_bytes_are_pinned_until_publication() {
    for case in 0..4 {
        let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
        if case != 0 {
            fixture.add_animation(RuntimePlatform::LinuxX86_64);
        } else {
            // The addition case must isolate attachment presence, not runtime changes.
            fs::write(
                &fixture.runtime,
                runtime::valid_test_animation_runtime(
                    RuntimePlatform::LinuxX86_64,
                    false,
                    false,
                    false,
                    false,
                ),
            )
            .unwrap();
        }
        let sources = capture_sources(&fixture.scene).unwrap();
        let dest = fixture.root.join("out");
        let result = export_sources_with_recheck(
            ExportRequest {
                scene: &fixture.scene,
                expected_checksum: fixture.checksum,
                destination: &dest,
                runtime: &fixture.runtime,
                trusted_runtime: true,
            },
            &sources,
            || match case {
                0 => {
                    fixture.add_animation(RuntimePlatform::LinuxX86_64);
                }
                1 => {
                    fs::remove_file(fixture.animation_path()).unwrap();
                }
                2 => {
                    fs::write(fixture.animation_path(), b"{}").unwrap();
                }
                _ => {
                    let mut bytes = fs::read(fixture.animation_path()).unwrap();
                    bytes.push(b' ');
                    fs::write(fixture.animation_path(), bytes).unwrap();
                }
            },
        );
        assert!(result.is_err(), "case {case}");
        assert!(!dest.exists());
        assert!(!fs::read_dir(&fixture.root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".orr-hazards-stage-")
        }));
    }
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn animation_bundle_retains_optional_camera_lighting_hud_and_props() {
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    fixture.add_animation(RuntimePlatform::LinuxX86_64);
    fixture.add_camera(RuntimePlatform::LinuxX86_64);
    let revision = admit_scene(&fs::read(&fixture.scene).unwrap()).unwrap().1;
    for (suffix, bytes) in [
        (
            "mesh-lighting.json",
            crate::mesh_lighting::Document::from_lighting(orr_render::Lighting::default())
                .unwrap()
                .encode("source.scene.yaml", revision)
                .unwrap(),
        ),
        (
            "mesh-hud.json",
            crate::mesh_hud::Document::default()
                .encode("source.scene.yaml", revision)
                .unwrap(),
        ),
        (
            "mesh-props.json",
            crate::mesh_props::Document::default()
                .encode("source.scene.yaml", revision)
                .unwrap(),
        ),
    ] {
        fs::write(
            fixture
                .scene
                .with_file_name(format!("source.scene.yaml.{suffix}")),
            bytes,
        )
        .unwrap();
    }
    fs::write(
        &fixture.runtime,
        runtime::valid_test_animation_runtime(RuntimePlatform::LinuxX86_64, true, true, true, true),
    )
    .unwrap();
    let dest = fixture.root.join("all");
    let receipt = fixture.export(&dest, true).unwrap();
    assert!(receipt.camera_sha256.is_some());
    assert!(receipt.lighting_sha256.is_some());
    assert!(receipt.hud_sha256.is_some());
    assert!(receipt.props_sha256.is_some());
    assert!(receipt.art_sha256.is_none());
    assert_eq!(fs::read_dir(&dest).unwrap().count(), 8);
    assert_eq!(
        open_bundle(&dest).unwrap().animation_sha256,
        receipt.animation_sha256
    );
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn animation_export_requires_animation_capability_without_output() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        let fixture = Fixture::new(platform);
        fixture.add_animation(platform);
        fs::write(&fixture.runtime, runtime::valid_test_runtime(platform)).unwrap();
        let dest = fixture.root.join("out");
        assert!(fixture.export(&dest, true).is_err());
        assert!(!dest.exists());
    }
}

#[cfg(not(feature = "mesh-animated-models"))]
#[test]
fn animation_export_feature_off_refuses_without_creating_output() {
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    fs::write(fixture.animation_path(), b"{}").unwrap();
    let dest = fixture.root.join("out");
    let error = fixture.export(&dest, true).unwrap_err();
    assert!(error.contains("mesh-animated-models"));
    assert!(!dest.exists());
}

#[test]
fn animation_source_limits_and_aliases_fail_closed() {
    for case in 0..3 {
        let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
        let path = match case {
            0 => fixture.animation_path(),
            1 => fixture
                .scene
                .with_file_name("source.scene.yaml.MESH-ANIMATION.JSON"),
            _ => fixture
                .scene
                .with_file_name("source.scene.yaml.mesh-animation.json.extra"),
        };
        fs::write(&path, b"{}").unwrap();
        if case == 0 {
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(MAX_ANIMATION_BYTES + 1)
                .unwrap();
        }
        assert!(capture_sources(&fixture.scene).is_err());
    }
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn animation_bundle_semantics_are_checked_even_with_matching_file_digest() {
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    fixture.add_animation(RuntimePlatform::LinuxX86_64);
    let dest = fixture.root.join("out");
    fixture.export(&dest, true).unwrap();
    let original = fs::read(dest.join(ANIMATION_FILENAME)).unwrap();
    for case in 0..5 {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        match case {
            0 => value["scene_filename"] = "source.scene.yaml".into(),
            1 => {
                value["graph_revision"][0] =
                    (value["graph_revision"][0].as_u64().unwrap() ^ 1).into()
            }
            2 => value["license_text"] = "different license".into(),
            3 => value["settings"]["clip"] = u32::MAX.into(),
            _ => value["cooked_sha256"] = "f".repeat(64).into(),
        }
        let changed = serde_json::to_vec(&value).unwrap();
        fs::write(dest.join(ANIMATION_FILENAME), &changed).unwrap();
        mutate_manifest(&dest, |manifest| {
            manifest["animation"]["bytes"] = (changed.len() as u64).into();
            manifest["animation"]["sha256"] = digest(&changed).into();
        });
        assert!(open_bundle(&dest).is_err(), "case {case}");
    }
}
