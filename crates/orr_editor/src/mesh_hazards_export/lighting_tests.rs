use super::*;

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub(crate) fn lighting_runtime(platform: RuntimePlatform, camera: bool, art: bool) -> Vec<u8> {
    let mut bytes = if art {
        art_tests::art_runtime(platform, camera)
    } else if camera {
        runtime::valid_test_camera_runtime(platform)
    } else {
        runtime::valid_test_runtime(platform)
    };
    let marker = runtime::LIGHTING_CAPABILITY_MARKER;
    match platform {
        RuntimePlatform::WindowsX86_64 => {
            let count = u16::from_le_bytes(bytes[70..72].try_into().unwrap());
            let section = 200 + usize::from(count) * 40;
            let offset = bytes.len();
            bytes.resize(offset + 512, 0);
            put16(&mut bytes, 70, count + 1);
            put32(&mut bytes, 144, (u32::from(count) + 2) * 4096);
            bytes[section..section + 8].copy_from_slice(b".orrhlit");
            put32(&mut bytes, section + 8, marker.len() as u32);
            put32(&mut bytes, section + 12, (u32::from(count) + 1) * 4096);
            put32(&mut bytes, section + 16, 512);
            put32(&mut bytes, section + 20, offset as u32);
            put32(&mut bytes, section + 36, 0x4000_0040);
            bytes[offset..offset + marker.len()].copy_from_slice(marker);
        }
        RuntimePlatform::LinuxX86_64 => {
            let count = u16::from_le_bytes(bytes[60..62].try_into().unwrap());
            let name = u64::from_le_bytes(bytes[736..744].try_into().unwrap()) as usize;
            let section = bytes.len();
            let offset = 384 + (usize::from(count) - 4) * 32;
            bytes.resize(section + 64, 0);
            put16(&mut bytes, 60, count + 1);
            bytes[320 + name..329 + name].copy_from_slice(b".orrhlit\0");
            put64(&mut bytes, 736, (name + 9) as u64);
            put32(&mut bytes, section, name as u32);
            put32(&mut bytes, section + 4, 1);
            put64(&mut bytes, section + 8, 2);
            put64(&mut bytes, section + 16, 0x400000 + offset as u64);
            put64(&mut bytes, section + 24, offset as u64);
            put64(&mut bytes, section + 32, marker.len() as u64);
            put64(&mut bytes, section + 48, 1);
            bytes[offset..offset + marker.len()].copy_from_slice(marker);
        }
    }
    bytes
}

impl Fixture {
    fn lighting_path(&self) -> PathBuf {
        self.scene
            .with_file_name("source.scene.yaml.mesh-lighting.json")
    }
    pub(super) fn add_lighting(
        &self,
        platform: RuntimePlatform,
        camera: bool,
        art: bool,
        post_process: bool,
    ) -> Vec<u8> {
        let revision = admit_scene(&fs::read(&self.scene).unwrap()).unwrap().1;
        let mut document = crate::mesh_lighting::Document::from_lighting(orr_render::Lighting {
            direction: [-0.4, -1.0, 0.6],
            color: [0.8, 0.7, 0.6],
            intensity: 1.6,
            sky: [0.4, 0.6, 0.7],
            ground: [0.3, 0.2, 0.1],
            ambient: 0.4,
            shadows: true,
            shadow_center: [1.0, 0.0, 0.5],
            shadow_radius: 20.0,
            tonemap: true,
            exposure: 1.4,
        })
        .unwrap();
        if post_process {
            document = document
                .with_post_process(orr_render::PostProcessSettings {
                    enabled: true,
                    bloom: true,
                    threshold: 1.5,
                    strength: 0.7,
                    radius: 3,
                    iterations: 2,
                    ..Default::default()
                })
                .unwrap();
        }
        let bytes = document.encode("source.scene.yaml", revision).unwrap();
        fs::write(self.lighting_path(), &bytes).unwrap();
        fs::write(&self.runtime, lighting_runtime(platform, camera, art)).unwrap();
        bytes
    }
}

#[test]
fn lighting_roundtrips_all_attachment_combinations_without_changing_legacy_schemas() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for camera in [false, true] {
            for art in [false, true] {
                for lighting in [false, true] {
                    for post_process in [false, true] {
                        if post_process && !lighting {
                            continue;
                        }
                        let f = Fixture::new(platform);
                        let source_scene = fs::read(&f.scene).unwrap();
                        if camera {
                            f.add_camera(platform);
                        }
                        if art {
                            f.add_art(platform, camera);
                        }
                        let source =
                            lighting.then(|| f.add_lighting(platform, camera, art, post_process));
                        let source_attachments = capture_sources(&f.scene).unwrap();
                        let dest = f.root.join("Game lighting 한글");
                        let receipt = f.export(&dest, true).unwrap();
                        let manifest: serde_json::Value = serde_json::from_slice(
                            &fs::read(dest.join(MANIFEST_FILENAME)).unwrap(),
                        )
                        .unwrap();
                        assert_eq!(
                            manifest["schema"],
                            if lighting {
                                4
                            } else if art {
                                3
                            } else if camera {
                                2
                            } else {
                                1
                            }
                        );
                        assert_eq!(
                            fs::read_dir(&dest).unwrap().count(),
                            3 + usize::from(camera) + usize::from(art) + usize::from(lighting)
                        );
                        assert_eq!(fs::read(&f.scene).unwrap(), source_scene);
                        assert_eq!(capture_sources(&f.scene).unwrap(), source_attachments);
                        assert_eq!(receipt.camera_sha256.is_some(), camera);
                        assert_eq!(receipt.art_sha256.is_some(), art);
                        assert_eq!(receipt.lighting_sha256.is_some(), lighting);
                        if let Some(source) = source {
                            let exported = fs::read(dest.join(LIGHTING_FILENAME)).unwrap();
                            let mut original: serde_json::Value =
                                serde_json::from_slice(&source).unwrap();
                            original["scene_filename"] = SCENE_FILENAME.into();
                            assert_eq!(
                                serde_json::from_slice::<serde_json::Value>(&exported).unwrap(),
                                original
                            );
                            let decoded = crate::mesh_lighting::Document::decode(
                                &exported,
                                SCENE_FILENAME,
                                receipt.graph_revision,
                            )
                            .unwrap();
                            assert_eq!(decoded.has_post_process(), post_process);
                            assert_eq!(decoded.post_process_settings().enabled, post_process);
                            assert_eq!(receipt.lighting_sha256, Some(digest(&exported)));
                            fs::remove_file(f.lighting_path()).unwrap();
                        } else {
                            assert!(manifest.get("lighting").is_none());
                        }
                        assert!(f.export(&dest, true).is_err());
                        fs::remove_file(&f.scene).unwrap();
                        if camera {
                            fs::remove_file(f.camera_path()).unwrap();
                        }
                        if art {
                            fs::remove_file(f.art_path()).unwrap();
                            fs::remove_dir_all(f.root.join("art-package")).unwrap();
                        }
                        let moved = f.root.join("relocated");
                        fs::rename(&dest, &moved).unwrap();
                        let admitted = open_bundle(&moved).unwrap();
                        assert_eq!(admitted.checksum, receipt.checksum);
                        assert_eq!(admitted.graph_revision, receipt.graph_revision);
                        assert_eq!(admitted.camera_sha256, receipt.camera_sha256);
                        assert_eq!(admitted.art_sha256, receipt.art_sha256);
                        assert_eq!(admitted.lighting_sha256, receipt.lighting_sha256);
                    }
                }
            }
        }
    }
}

#[test]
fn lighting_capabilities_are_independent_and_every_truncation_rejected() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for camera in [false, true] {
            for art in [false, true] {
                let bytes = lighting_runtime(platform, camera, art);
                assert_eq!(runtime::validate_runtime(&bytes), Ok(platform));
                assert_eq!(
                    runtime::validate_lighting_runtime(&bytes, camera, art),
                    Ok(platform)
                );
                if !camera {
                    assert!(runtime::validate_lighting_runtime(&bytes, true, art).is_err());
                }
                if !art {
                    assert!(runtime::validate_lighting_runtime(&bytes, camera, true).is_err());
                }
                for length in 0..bytes.len() {
                    assert!(
                        runtime::validate_lighting_runtime(&bytes[..length], camera, art).is_err()
                    );
                }
            }
        }
        for bytes in [
            runtime::valid_test_runtime(platform),
            runtime::valid_test_camera_runtime(platform),
            art_tests::art_runtime(platform, false),
            art_tests::art_runtime(platform, true),
        ] {
            assert!(
                runtime::validate_lighting_runtime(&bytes, false, false)
                    .unwrap_err()
                    .contains("lighting capability section is missing")
            );
        }
        // A lighting declaration is never a replacement for the player capability.
        let mut missing_player = lighting_runtime(platform, false, false);
        match platform {
            RuntimePlatform::WindowsX86_64 => missing_player[240] = b'x',
            RuntimePlatform::LinuxX86_64 => missing_player[327] = b'x',
        }
        assert!(runtime::validate_lighting_runtime(&missing_player, false, false).is_err());
    }
}

#[test]
fn lighting_runtime_rejects_malformed_sections_even_when_not_required() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for case in 0..7 {
            let mut bytes = lighting_runtime(platform, true, true);
            match platform {
                RuntimePlatform::LinuxX86_64 => match case {
                    0 => bytes[448] ^= 1,
                    1 => put64(&mut bytes, 904, 3), // Writable section.
                    2 => put64(&mut bytes, 928, 20),
                    3 => put64(&mut bytes, 912, 0x5001c0), // Unmapped address.
                    4 => {
                        // Make the earlier camera section a valid lighting section first.
                        put32(&mut bytes, 832, 43);
                        put64(
                            &mut bytes,
                            864,
                            runtime::LIGHTING_CAPABILITY_MARKER.len() as u64,
                        );
                        bytes[416..416 + runtime::LIGHTING_CAPABILITY_MARKER.len()]
                            .copy_from_slice(runtime::LIGHTING_CAPABILITY_MARKER);
                    }
                    5 => put32(&mut bytes, 900, 8),   // SHT_NOBITS.
                    _ => put64(&mut bytes, 920, 416), // Overlapping payload.
                },
                RuntimePlatform::WindowsX86_64 => match case {
                    0 => bytes[2560] ^= 1,
                    1 => put32(&mut bytes, 396, 0xe000_0040),
                    2 => put32(&mut bytes, 368, 20),
                    3 => bytes[2560 + runtime::LIGHTING_CAPABILITY_MARKER.len()] = 1,
                    4 => {
                        bytes[320..328].copy_from_slice(b".orrhlit");
                        put32(
                            &mut bytes,
                            328,
                            runtime::LIGHTING_CAPABILITY_MARKER.len() as u32,
                        );
                        bytes[2048..2560].fill(0);
                        bytes[2048..2048 + runtime::LIGHTING_CAPABILITY_MARKER.len()]
                            .copy_from_slice(runtime::LIGHTING_CAPABILITY_MARKER);
                    }
                    5 => put32(&mut bytes, 380, 2048), // Overlapping payload.
                    _ => put32(&mut bytes, 372, 0xffff_f000),
                },
            }
            if case == 4 {
                let duplicate = Err("runtime has duplicate capability sections".to_owned());
                assert_eq!(runtime::validate_runtime(&bytes), duplicate, "{platform:?}");
                assert_eq!(
                    runtime::validate_lighting_runtime(&bytes, false, false),
                    duplicate
                );
            } else {
                assert!(
                    runtime::validate_runtime(&bytes).is_err(),
                    "{platform:?} case {case}"
                );
                assert!(runtime::validate_lighting_runtime(&bytes, false, false).is_err());
            }
        }
        let mut absent = lighting_runtime(platform, false, false);
        match platform {
            RuntimePlatform::WindowsX86_64 => absent[280] = b'x',
            RuntimePlatform::LinuxX86_64 => absent[345] = b'x',
        }
        absent.extend_from_slice(runtime::LIGHTING_CAPABILITY_MARKER);
        assert!(runtime::validate_runtime(&absent).is_ok());
        assert!(runtime::validate_lighting_runtime(&absent, false, false).is_err());
    }
}

#[test]
fn lighting_export_requires_capabilities_for_each_attachment_and_explicit_trust() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for missing in ["lighting", "camera", "art"] {
            let f = Fixture::new(platform);
            f.add_camera(platform);
            f.add_art(platform, true);
            f.add_lighting(platform, true, true, false);
            let dest = f.root.join("out");
            assert!(f.export(&dest, false).is_err());
            let runtime = match missing {
                "lighting" => art_tests::art_runtime(platform, true),
                "camera" => lighting_runtime(platform, false, true),
                _ => lighting_runtime(platform, true, false),
            };
            fs::write(&f.runtime, &runtime).unwrap();
            assert!(f.export(&dest, true).is_err());
            assert!(!dest.exists());
            fs::write(&f.runtime, lighting_runtime(platform, true, true)).unwrap();
            f.export(&dest, true).unwrap();
            // Even a self-consistent replacement runtime digest cannot waive a capability.
            fs::write(dest.join(platform.runtime_filename()), &runtime).unwrap();
            mutate_manifest(&dest, |value| {
                value["runtime"] =
                    serde_json::to_value(record(platform.runtime_filename(), &runtime)).unwrap();
            });
            assert!(open_bundle(&dest).is_err());
        }
    }
}

#[test]
fn lighting_manifest_schema_shape_inventory_and_tampering_fail_closed() {
    for case in 0..23 {
        let f = Fixture::new(RuntimePlatform::LinuxX86_64);
        f.add_lighting(RuntimePlatform::LinuxX86_64, false, false, true);
        let dest = f.root.join("out");
        f.export(&dest, true).unwrap();
        mutate_manifest(&dest, |v| match case {
            0..=2 => v["schema"] = (case + 1).into(),
            3 => v["schema"] = 5.into(),
            4 => {
                v.as_object_mut().unwrap().remove("lighting");
            }
            5 => v["lighting"] = serde_json::Value::Null,
            6 => v["lighting"] = serde_json::json!([]),
            7 => v["lighting"]["name"] = "../escape".into(),
            8 => v["lighting"]["bytes"] = (MAX_LIGHTING_BYTES + 1).into(),
            9 => v["lighting"]["bytes"] = 0.into(),
            10 => v["lighting"]["sha256"] = "0".repeat(64).into(),
            11 => v["lighting"]["unknown"] = true.into(),
            12 => v["camera"] = serde_json::Value::Null,
            13 => v["art"] = serde_json::Value::Null,
            14 => v["unknown"] = true.into(),
            15 => {
                fs::remove_file(dest.join(LIGHTING_FILENAME)).unwrap();
            }
            16 => {
                fs::write(dest.join("extra.mesh-lighting.json"), b"{}").unwrap();
            }
            17 => {
                fs::write(dest.join(LIGHTING_FILENAME), b"{}").unwrap();
            }
            18 => {
                fs::OpenOptions::new()
                    .write(true)
                    .open(dest.join(LIGHTING_FILENAME))
                    .unwrap()
                    .set_len(MAX_LIGHTING_BYTES + 1)
                    .unwrap();
            }
            _ => {
                let mut lighting: serde_json::Value =
                    serde_json::from_slice(&fs::read(dest.join(LIGHTING_FILENAME)).unwrap())
                        .unwrap();
                match case {
                    19 => lighting["scene_filename"] = "source.scene.yaml".into(),
                    20 => {
                        lighting["graph_revision"][0] =
                            (lighting["graph_revision"][0].as_u64().unwrap() ^ 1).into()
                    }
                    21 => lighting["lighting"]["exposure"] = 0.into(),
                    _ => lighting["lighting"]["post_process"] = serde_json::Value::Null,
                }
                let bytes = serde_json::to_vec(&lighting).unwrap();
                fs::write(dest.join(LIGHTING_FILENAME), &bytes).unwrap();
                v["lighting"] = serde_json::to_value(record(LIGHTING_FILENAME, &bytes)).unwrap();
            }
        });
        assert!(open_bundle(&dest).is_err(), "case {case}");
    }
}

#[test]
fn lighting_is_forbidden_in_legacy_manifests_even_when_null() {
    for camera in [false, true] {
        for art in [false, true] {
            for value in [serde_json::Value::Null, serde_json::json!({})] {
                let f = Fixture::new(RuntimePlatform::LinuxX86_64);
                if camera {
                    f.add_camera(RuntimePlatform::LinuxX86_64);
                }
                if art {
                    f.add_art(RuntimePlatform::LinuxX86_64, camera);
                }
                let dest = f.root.join("out");
                f.export(&dest, true).unwrap();
                mutate_manifest(&dest, |v| v["lighting"] = value);
                assert!(open_bundle(&dest).is_err());
            }
        }
    }
}

#[test]
fn lighting_sources_reject_malformed_stale_unknown_null_bounds_and_aliases() {
    for case in 0..16 {
        let f = Fixture::new(RuntimePlatform::LinuxX86_64);
        let bytes = f.add_lighting(RuntimePlatform::LinuxX86_64, false, false, true);
        let path = f.lighting_path();
        match case {
            0 => fs::write(&path, b"{}").unwrap(),
            1 => fs::write(&path, b"null").unwrap(),
            2 => fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(MAX_LIGHTING_BYTES + 1)
                .unwrap(),
            3 => {
                fs::rename(&path, f.root.join("source.scene.yaml.MESH-LIGHTING.JSON")).unwrap();
            }
            4 => fs::write(
                f.root.join("source.scene.yaml.mesh-lighting.json.extra"),
                b"extra",
            )
            .unwrap(),
            _ => {
                let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                match case {
                    5 => v["scene_filename"] = "other.scene.yaml".into(),
                    6 => {
                        v["graph_revision"][0] =
                            (v["graph_revision"][0].as_u64().unwrap() ^ 1).into()
                    }
                    7 => v["unknown"] = true.into(),
                    8 => v["lighting"]["unknown"] = true.into(),
                    9 => v["lighting"] = serde_json::Value::Null,
                    10 => v["lighting"]["post_process"] = serde_json::Value::Null,
                    11 => v["lighting"]["exposure"] = 0.into(),
                    12 => v["lighting"]["direction"] = serde_json::json!([0, 0, 0]),
                    13 => v["lighting"]["post_process"]["radius"] = 9.into(),
                    14 => v["lighting"]["intensity"] = 10_001.into(),
                    _ => v["schema"] = 1.into(),
                }
                fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
            }
        }
        let dest = f.root.join("out");
        assert!(f.export(&dest, true).is_err(), "case {case}");
        assert!(!dest.exists());
    }
}

#[test]
fn lighting_source_presence_and_exact_bytes_bound_at_admission_and_publication() {
    for before_admission in [false, true] {
        for case in 0..3 {
            let f = Fixture::new(RuntimePlatform::LinuxX86_64);
            let bytes = f.add_lighting(RuntimePlatform::LinuxX86_64, false, false, true);
            if case == 0 {
                fs::remove_file(f.lighting_path()).unwrap();
            }
            let snapshot = capture_sources(&f.scene).unwrap();
            let change = || match case {
                0 => fs::write(f.lighting_path(), &bytes).unwrap(),
                1 => fs::remove_file(f.lighting_path()).unwrap(),
                _ => {
                    let mut changed = bytes.clone();
                    *changed.last_mut().unwrap() = b' ';
                    fs::write(f.lighting_path(), changed).unwrap();
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
            assert!(!fs::read_dir(&f.root).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".orr-hazards-stage-")
            }));
        }
    }
    let f = Fixture::new(RuntimePlatform::LinuxX86_64);
    f.add_lighting(RuntimePlatform::LinuxX86_64, false, false, false);
    assert!(capture_camera_source(&f.scene).is_err());
    assert!(export_interactive_bound(f.request(&f.root.join("out")), &None).is_err());
}

#[test]
fn lighting_duplicate_records_and_fields_are_rejected() {
    let f = Fixture::new(RuntimePlatform::LinuxX86_64);
    let source = f.add_lighting(RuntimePlatform::LinuxX86_64, false, false, false);
    let dest = f.root.join("out");
    f.export(&dest, true).unwrap();
    let manifest = dest.join(MANIFEST_FILENAME);
    let original = fs::read_to_string(&manifest).unwrap();
    for prefix in ["\"lighting\":null,", "\"lighting\":{},"] {
        fs::write(&manifest, original.replacen('{', &format!("{{{prefix}"), 1)).unwrap();
        assert!(open_bundle(&dest).is_err());
    }
    let text = String::from_utf8(source).unwrap();
    for duplicate in [
        text.replacen('{', "{\"scene_filename\":\"source.scene.yaml\",", 1),
        text.replace("\"intensity\":", "\"intensity\":1.0,\"intensity\":"),
    ] {
        fs::write(f.lighting_path(), duplicate).unwrap();
        assert!(f.export(&f.root.join("duplicate"), true).is_err());
    }
}

#[cfg(unix)]
#[test]
fn lighting_symlink_and_fifo_sources_and_bundle_reject_without_blocking() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new(RuntimePlatform::LinuxX86_64);
    let bytes = f.add_lighting(RuntimePlatform::LinuxX86_64, false, false, false);
    let dest = f.root.join("out");
    f.export(&dest, true).unwrap();
    let real = f.root.join("real-lighting");
    fs::write(&real, bytes).unwrap();
    fs::remove_file(dest.join(LIGHTING_FILENAME)).unwrap();
    symlink(&real, dest.join(LIGHTING_FILENAME)).unwrap();
    assert!(open_bundle(&dest).is_err());
    fs::remove_file(f.lighting_path()).unwrap();
    symlink(&real, f.lighting_path()).unwrap();
    assert!(capture_sources(&f.scene).is_err());
    fs::remove_file(f.lighting_path()).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(f.lighting_path())
            .status()
            .unwrap()
            .success()
    );
    assert!(capture_sources(&f.scene).is_err());
    assert!(f.export(&f.root.join("fifo"), true).is_err());
    fs::remove_file(dest.join(LIGHTING_FILENAME)).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(dest.join(LIGHTING_FILENAME))
            .status()
            .unwrap()
            .success()
    );
    assert!(open_bundle(&dest).is_err());
}
