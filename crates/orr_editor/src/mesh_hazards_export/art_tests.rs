use super::*;

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
pub(crate) fn art_runtime(platform: RuntimePlatform, camera: bool) -> Vec<u8> {
    let mut bytes = runtime::valid_test_camera_runtime(platform);
    let marker = b"ORR_MESH_HAZARDS_ART_V1";
    match platform {
        RuntimePlatform::WindowsX86_64 => {
            bytes[280..288].copy_from_slice(b".orrhart");
            put32(&mut bytes, 288, marker.len() as u32);
            bytes[1536..2048].fill(0);
            bytes[1536..1536 + marker.len()].copy_from_slice(marker);
            if camera {
                let source = runtime::valid_test_camera_runtime(platform);
                bytes.resize(2560, 0);
                bytes[70..72].copy_from_slice(&4u16.to_le_bytes());
                put32(&mut bytes, 144, 20480);
                bytes[320..360].copy_from_slice(&source[280..320]);
                put32(&mut bytes, 332, 16384);
                put32(&mut bytes, 340, 2048);
                bytes[2048..2560].copy_from_slice(&source[1536..2048]);
            }
        }
        RuntimePlatform::LinuxX86_64 => {
            bytes[345..353].copy_from_slice(b".orrhart");
            bytes[384..408].fill(0);
            bytes[384..384 + marker.len()].copy_from_slice(marker);
            put64(&mut bytes, 800, marker.len() as u64);
            if camera {
                bytes.resize(896, 0);
                bytes[60..62].copy_from_slice(&6u16.to_le_bytes());
                bytes[354..363].copy_from_slice(b".orrhcam\0");
                put64(&mut bytes, 736, 43);
                bytes.copy_within(768..832, 832);
                put32(&mut bytes, 832, 34);
                put64(&mut bytes, 848, 0x4001a0);
                put64(&mut bytes, 856, 416);
                let camera_marker = b"ORR_MESH_HAZARDS_CAMERA_V1";
                put64(&mut bytes, 864, camera_marker.len() as u64);
                bytes[416..416 + camera_marker.len()].copy_from_slice(camera_marker);
            }
        }
    }
    bytes
}

impl Fixture {
    pub(super) fn art_path(&self) -> PathBuf {
        self.scene.with_file_name("source.scene.yaml.mesh-art.json")
    }
    pub(super) fn add_art(&self, platform: RuntimePlatform, camera: bool) -> Vec<u8> {
        use std::collections::{BTreeMap, BTreeSet};
        let root = self.root.join("art-package");
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("model.glb"),
            include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        )
        .unwrap();
        fs::write(root.join("LICENSE.txt"), b"Example license text\n").unwrap();
        let manifest = orr_package::Manifest {
            schema: 1,
            name: "verified-static".into(),
            version: "1.0.0".into(),
            engine: format!("={}", orr_package::Runtime::content_only().engine_version),
            capabilities: BTreeSet::from(["models".into()]),
            dependencies: BTreeMap::new(),
            files: BTreeSet::from(["LICENSE.txt".into(), "model.glb".into()]),
        };
        fs::write(
            root.join("orr.package.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let graph = admit_scene(&fs::read(&self.scene).unwrap()).unwrap().1;
        let art = crate::mesh_art::Snapshot::from_package(&root, "model.glb", "LICENSE.txt", graph)
            .unwrap()
            .with_transform([1.25, 2.0, 3.0], 0.75)
            .unwrap()
            .with_tint([0.4, 0.5, 0.6, 1.0])
            .unwrap()
            .with_facing(crate::mesh_facing::Facing::Route)
            .unwrap();
        let bytes = art.encode_embedded("source.scene.yaml").unwrap();
        fs::write(self.art_path(), &bytes).unwrap();
        fs::write(&self.runtime, art_runtime(platform, camera)).unwrap();
        bytes
    }
}

#[test]
fn art_roundtrips_rebinds_only_filename_and_relocates_with_optional_camera() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for camera in [false, true] {
            let f = Fixture::new(platform);
            if camera {
                f.add_camera(platform);
            }
            let source = f.add_art(platform, camera);
            let dest = f.root.join("out");
            let receipt = f.export(&dest, true).unwrap();
            let manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(dest.join(MANIFEST_FILENAME)).unwrap()).unwrap();
            assert_eq!(manifest["schema"], 3);
            assert_eq!(
                fs::read_dir(&dest).unwrap().count(),
                if camera { 5 } else { 4 }
            );
            let exported = fs::read(dest.join(ART_FILENAME)).unwrap();
            let mut original: serde_json::Value = serde_json::from_slice(&source).unwrap();
            original["scene_filename"] = SCENE_FILENAME.into();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&exported).unwrap(),
                original
            );
            assert_eq!(fs::read(f.art_path()).unwrap(), source);
            assert_eq!(receipt.art_sha256, Some(digest(&exported)));
            assert!(f.export(&dest, true).is_err());
            fs::remove_file(f.art_path()).unwrap();
            fs::remove_dir_all(f.root.join("art-package")).unwrap();
            let moved = f.root.join("relocated");
            fs::rename(&dest, &moved).unwrap();
            assert_eq!(open_bundle(&moved).unwrap().art_sha256, receipt.art_sha256);
        }
    }
}

#[test]
fn art_capabilities_are_independent_and_malformed_sections_rejected() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for camera in [false, true] {
            let bytes = art_runtime(platform, camera);
            assert_eq!(runtime::validate_art_runtime(&bytes, camera), Ok(platform));
            if !camera {
                assert!(runtime::validate_art_runtime(&bytes, true).is_err());
            }
            for length in 0..bytes.len() {
                assert!(runtime::validate_art_runtime(&bytes[..length], camera).is_err());
            }
            let mut malformed = bytes;
            let payload = if platform == RuntimePlatform::LinuxX86_64 {
                384
            } else {
                1536
            };
            malformed[payload] ^= 1;
            assert!(runtime::validate_runtime(&malformed).is_err());
            assert!(runtime::validate_art_runtime(&malformed, camera).is_err());
        }
        assert!(
            runtime::validate_art_runtime(&runtime::valid_test_camera_runtime(platform), false)
                .is_err()
        );
        let f = Fixture::new(platform);
        f.add_art(platform, false);
        fs::write(&f.runtime, runtime::valid_test_camera_runtime(platform)).unwrap();
        assert!(f.export(&f.root.join("out"), true).is_err());
        assert!(!f.root.join("out").exists());
    }
}

#[test]
fn art_manifest_schema_record_inventory_and_tampering_fail_closed() {
    for case in 0..14 {
        let f = Fixture::new(RuntimePlatform::LinuxX86_64);
        f.add_art(RuntimePlatform::LinuxX86_64, false);
        let dest = f.root.join("out");
        f.export(&dest, true).unwrap();
        mutate_manifest(&dest, |v| match case {
            0 => v["schema"] = 1.into(),
            1 => v["schema"] = 2.into(),
            2 => v["art"] = serde_json::Value::Null,
            3 => {
                v.as_object_mut().unwrap().remove("art");
            }
            4 => v["camera"] = serde_json::Value::Null,
            5 => v["art"]["name"] = "../escape".into(),
            6 => v["art"]["bytes"] = (MAX_ART_BYTES + 1).into(),
            7 => v["art"]["sha256"] = "0".repeat(64).into(),
            8 => v["art"]["unknown"] = true.into(),
            9 => v["art"] = serde_json::json!([]),
            10 => {
                fs::remove_file(dest.join(ART_FILENAME)).unwrap();
            }
            11 => {
                fs::write(dest.join("extra.mesh-art.json"), b"{}").unwrap();
            }
            12 => {
                fs::write(dest.join(ART_FILENAME), b"{}").unwrap();
            }
            _ => {
                let mut art: serde_json::Value =
                    serde_json::from_slice(&fs::read(dest.join(ART_FILENAME)).unwrap()).unwrap();
                art["scene_filename"] = "source.scene.yaml".into();
                let bytes = serde_json::to_vec(&art).unwrap();
                fs::write(dest.join(ART_FILENAME), &bytes).unwrap();
                v["art"] = serde_json::to_value(record(ART_FILENAME, &bytes)).unwrap();
            }
        });
        assert!(open_bundle(&dest).is_err(), "case {case}");
    }
    for camera in [false, true] {
        let f = Fixture::new(RuntimePlatform::LinuxX86_64);
        if camera {
            f.add_camera(RuntimePlatform::LinuxX86_64);
        }
        let dest = f.root.join("out");
        f.export(&dest, true).unwrap();
        mutate_manifest(&dest, |v| v["art"] = serde_json::Value::Null);
        assert!(open_bundle(&dest).is_err());
    }
}

#[test]
fn art_sources_reject_stale_malformed_oversized_and_aliases_without_publication() {
    for case in 0..7 {
        let f = Fixture::new(RuntimePlatform::LinuxX86_64);
        let bytes = f.add_art(RuntimePlatform::LinuxX86_64, false);
        match case {
            0 => fs::write(f.art_path(), b"{}").unwrap(),
            1..=3 => {
                let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                match case {
                    1 => value["scene_filename"] = "other.scene.yaml".into(),
                    2 => {
                        value["graph_revision"][0] =
                            (value["graph_revision"][0].as_u64().unwrap() ^ 1).into()
                    }
                    _ => value["unexpected"] = true.into(),
                }
                fs::write(f.art_path(), serde_json::to_vec(&value).unwrap()).unwrap();
            }
            4 => fs::OpenOptions::new()
                .write(true)
                .open(f.art_path())
                .unwrap()
                .set_len(MAX_ART_BYTES + 1)
                .unwrap(),
            5 => {
                fs::rename(f.art_path(), f.root.join("source.scene.yaml.MESH-ART.JSON")).unwrap();
            }
            _ => fs::write(f.root.join("source.scene.yaml.extra"), b"extra").unwrap(),
        }
        let dest = f.root.join("out");
        assert!(f.export(&dest, true).is_err(), "case {case}");
        assert!(!dest.exists());
    }
}

#[test]
fn art_original_bytes_and_absence_bound_at_admission_and_publication() {
    for before_admission in [false, true] {
        for case in 0..3 {
            let f = Fixture::new(RuntimePlatform::LinuxX86_64);
            let bytes = f.add_art(RuntimePlatform::LinuxX86_64, false);
            if case == 0 {
                fs::remove_file(f.art_path()).unwrap();
            }
            let snapshot = capture_sources(&f.scene).unwrap();
            let change = || match case {
                0 => fs::write(f.art_path(), &bytes).unwrap(),
                1 => fs::remove_file(f.art_path()).unwrap(),
                _ => {
                    let mut changed = bytes.clone();
                    changed.push(b' ');
                    fs::write(f.art_path(), changed).unwrap();
                }
            };
            let dest = f.root.join("out");
            let result = if before_admission {
                change();
                export_interactive_sources(f.request(&dest), &snapshot)
            } else {
                export_sources_with_recheck(f.request(&dest), &snapshot, change)
            };
            assert!(result.is_err());
            assert!(!dest.exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn art_symlink_and_fifo_are_rejected_without_blocking() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new(RuntimePlatform::LinuxX86_64);
    let bytes = f.add_art(RuntimePlatform::LinuxX86_64, false);
    let dest = f.root.join("out");
    f.export(&dest, true).unwrap();
    let real = f.root.join("real-art");
    fs::write(&real, bytes).unwrap();
    fs::remove_file(dest.join(ART_FILENAME)).unwrap();
    symlink(&real, dest.join(ART_FILENAME)).unwrap();
    assert!(open_bundle(&dest).is_err());
    fs::remove_file(f.art_path()).unwrap();
    symlink(&real, f.art_path()).unwrap();
    assert!(capture_sources(&f.scene).is_err());
    fs::remove_file(f.art_path()).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(f.art_path())
            .status()
            .unwrap()
            .success()
    );
    assert!(capture_sources(&f.scene).is_err());
}

#[test]
fn art_duplicate_manifest_records_and_source_fields_are_rejected() {
    let f = Fixture::new(RuntimePlatform::LinuxX86_64);
    let source = f.add_art(RuntimePlatform::LinuxX86_64, false);
    let dest = f.root.join("out");
    f.export(&dest, true).unwrap();
    let manifest = dest.join(MANIFEST_FILENAME);
    let original = fs::read_to_string(&manifest).unwrap();
    for prefix in ["\"art\":null,", "\"art\":{},"] {
        let duplicate = original.replacen('{', &format!("{{{prefix}"), 1);
        fs::write(&manifest, duplicate).unwrap();
        assert!(open_bundle(&dest).is_err());
    }
    let text = String::from_utf8(source).unwrap();
    fs::write(
        f.art_path(),
        text.replacen('{', "{\"scene_filename\":\"source.scene.yaml\",", 1),
    )
    .unwrap();
    assert!(f.export(&f.root.join("duplicate"), true).is_err());
}

#[test]
fn art_runtime_rejects_duplicate_flags_sizes_and_mapping_even_when_optional() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for case in 0..4 {
            let mut bytes = art_runtime(platform, true);
            match platform {
                RuntimePlatform::LinuxX86_64 => match case {
                    0 => put32(&mut bytes, 832, 25), // Duplicate art section.
                    1 => put64(&mut bytes, 776, 3),  // Writable.
                    2 => put64(&mut bytes, 800, 20),
                    _ => put64(&mut bytes, 784, 0x500180),
                },
                RuntimePlatform::WindowsX86_64 => match case {
                    0 => bytes[320..328].copy_from_slice(b".orrhart"),
                    1 => put32(&mut bytes, 316, 0xe000_0040),
                    2 => put32(&mut bytes, 288, 20),
                    _ => bytes[1536 + b"ORR_MESH_HAZARDS_ART_V1".len()] = 1,
                },
            }
            assert!(runtime::validate_art_runtime(&bytes, false).is_err());
            assert!(runtime::validate_runtime(&bytes).is_err());
        }
    }
}
