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

pub(crate) fn hud_runtime(
    platform: RuntimePlatform,
    camera: bool,
    art: bool,
    lighting: bool,
) -> Vec<u8> {
    let mut bytes = if lighting {
        lighting_tests::lighting_runtime(platform, camera, art)
    } else if art {
        art_tests::art_runtime(platform, camera)
    } else if camera {
        runtime::valid_test_camera_runtime(platform)
    } else {
        runtime::valid_test_runtime(platform)
    };
    let marker = runtime::HUD_CAPABILITY_MARKER;
    match platform {
        RuntimePlatform::WindowsX86_64 => {
            let count = u16::from_le_bytes(bytes[70..72].try_into().unwrap());
            let section = 200 + usize::from(count) * 40;
            let offset = bytes.len();
            bytes.resize(offset + 512, 0);
            put16(&mut bytes, 70, count + 1);
            put32(&mut bytes, 144, (u32::from(count) + 2) * 4096);
            bytes[section..section + 8].copy_from_slice(b".orrhhud");
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
            bytes[320 + name..329 + name].copy_from_slice(b".orrhhud\0");
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
    fn hud_path(&self) -> PathBuf {
        self.scene.with_file_name("source.scene.yaml.mesh-hud.json")
    }
    fn add_hud(
        &self,
        platform: RuntimePlatform,
        camera: bool,
        art: bool,
        lighting: bool,
    ) -> Vec<u8> {
        let revision = admit_scene(&fs::read(&self.scene).unwrap()).unwrap().1;
        let document = crate::mesh_hud::Document {
            title: "위험 지대 · Hazard course".into(),
            enabled: false,
            font_size: 24.0,
            ..Default::default()
        };
        let bytes = document.encode("source.scene.yaml", revision).unwrap();
        fs::write(self.hud_path(), &bytes).unwrap();
        fs::write(&self.runtime, hud_runtime(platform, camera, art, lighting)).unwrap();
        bytes
    }
}

#[test]
fn hud_roundtrips_all_attachment_combinations_and_relocates() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for camera in [false, true] {
            for art in [false, true] {
                for lighting in [false, true] {
                    let f = Fixture::new(platform);
                    if camera {
                        f.add_camera(platform);
                    }
                    if art {
                        f.add_art(platform, camera);
                    }
                    if lighting {
                        f.add_lighting(platform, camera, art, true);
                    }
                    let source = f.add_hud(platform, camera, art, lighting);
                    let snapshot = capture_sources(&f.scene).unwrap();
                    let dest = f.root.join("HUD game 한글");
                    let receipt = f.export(&dest, true).unwrap();
                    let manifest: serde_json::Value =
                        serde_json::from_slice(&fs::read(dest.join(MANIFEST_FILENAME)).unwrap())
                            .unwrap();
                    assert_eq!(manifest["schema"], 5);
                    assert_eq!(
                        fs::read_dir(&dest).unwrap().count(),
                        4 + usize::from(camera) + usize::from(art) + usize::from(lighting)
                    );
                    assert_eq!(capture_sources(&f.scene).unwrap(), snapshot);
                    let exported = fs::read(dest.join(HUD_FILENAME)).unwrap();
                    let mut original: serde_json::Value = serde_json::from_slice(&source).unwrap();
                    original["scene_filename"] = SCENE_FILENAME.into();
                    assert_eq!(
                        serde_json::from_slice::<serde_json::Value>(&exported).unwrap(),
                        original
                    );
                    assert_eq!(receipt.hud_sha256, Some(digest(&exported)));
                    assert!(
                        !crate::mesh_hud::Document::decode(
                            &exported,
                            SCENE_FILENAME,
                            receipt.graph_revision
                        )
                        .unwrap()
                        .enabled
                    );
                    assert!(f.export(&dest, true).is_err());
                    fs::remove_file(&f.scene).unwrap();
                    fs::remove_file(f.hud_path()).unwrap();
                    let moved = f.root.join("moved");
                    fs::rename(dest, &moved).unwrap();
                    let admitted = open_bundle(&moved).unwrap();
                    assert_eq!(admitted.hud_sha256, receipt.hud_sha256);
                    assert_eq!(admitted.checksum, receipt.checksum);
                    assert_eq!(admitted.camera_sha256, receipt.camera_sha256);
                    assert_eq!(admitted.art_sha256, receipt.art_sha256);
                    assert_eq!(admitted.lighting_sha256, receipt.lighting_sha256);
                }
            }
        }
    }
}

#[test]
fn hud_capability_requires_every_selected_dependency_and_rejects_truncation() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for camera in [false, true] {
            for art in [false, true] {
                for lighting in [false, true] {
                    let bytes = hud_runtime(platform, camera, art, lighting);
                    assert_eq!(runtime::validate_runtime(&bytes), Ok(platform));
                    assert_eq!(
                        runtime::validate_hud_runtime(&bytes, camera, art, lighting),
                        Ok(platform)
                    );
                    if !camera {
                        assert!(
                            runtime::validate_hud_runtime(&bytes, true, art, lighting).is_err()
                        );
                    }
                    if !art {
                        assert!(
                            runtime::validate_hud_runtime(&bytes, camera, true, lighting).is_err()
                        );
                    }
                    if !lighting {
                        assert!(runtime::validate_hud_runtime(&bytes, camera, art, true).is_err());
                    }
                    for length in 0..bytes.len() {
                        assert!(
                            runtime::validate_hud_runtime(&bytes[..length], camera, art, lighting)
                                .is_err()
                        );
                    }
                }
            }
        }
        assert!(
            runtime::validate_hud_runtime(
                &lighting_tests::lighting_runtime(platform, true, true),
                true,
                true,
                true
            )
            .is_err()
        );
        let mut missing = hud_runtime(platform, false, false, false);
        match platform {
            RuntimePlatform::WindowsX86_64 => missing[240] = b'x',
            RuntimePlatform::LinuxX86_64 => missing[327] = b'x',
        }
        assert!(runtime::validate_hud_runtime(&missing, false, false, false).is_err());
    }
}

#[test]
fn hud_export_and_reopen_require_trust_and_each_capability() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for missing in 0..4 {
            let f = Fixture::new(platform);
            f.add_camera(platform);
            f.add_art(platform, true);
            f.add_lighting(platform, true, true, false);
            f.add_hud(platform, true, true, true);
            let dest = f.root.join("out");
            assert!(f.export(&dest, false).is_err());
            let bytes = if missing == 0 {
                lighting_tests::lighting_runtime(platform, true, true)
            } else {
                hud_runtime(platform, missing != 1, missing != 2, missing != 3)
            };
            fs::write(&f.runtime, &bytes).unwrap();
            assert!(f.export(&dest, true).is_err());
            assert!(!dest.exists());
            fs::write(&f.runtime, hud_runtime(platform, true, true, true)).unwrap();
            f.export(&dest, true).unwrap();
            fs::write(dest.join(platform.runtime_filename()), &bytes).unwrap();
            mutate_manifest(&dest, |v| {
                v["runtime"] =
                    serde_json::to_value(record(platform.runtime_filename(), &bytes)).unwrap()
            });
            assert!(open_bundle(&dest).is_err());
        }
    }
}

#[test]
fn hud_manifest_shape_schema_inventory_and_self_consistent_tampering_fail_closed() {
    for case in 0..27 {
        let f = Fixture::new(RuntimePlatform::LinuxX86_64);
        f.add_hud(RuntimePlatform::LinuxX86_64, false, false, false);
        let dest = f.root.join("out");
        f.export(&dest, true).unwrap();
        mutate_manifest(&dest, |v| match case {
            0..=3 => v["schema"] = (case + 1).into(),
            4 => v["schema"] = 6.into(),
            5 => {
                v.as_object_mut().unwrap().remove("hud");
            }
            6 => v["hud"] = serde_json::Value::Null,
            7 => v["hud"] = serde_json::json!([]),
            8 => v["hud"]["name"] = "../escape".into(),
            9 => v["hud"]["bytes"] = (MAX_HUD_BYTES + 1).into(),
            10 => v["hud"]["bytes"] = 0.into(),
            11 => v["hud"]["sha256"] = "0".repeat(64).into(),
            12 => v["hud"]["unknown"] = true.into(),
            13 => v["camera"] = serde_json::Value::Null,
            14 => v["art"] = serde_json::Value::Null,
            15 => v["lighting"] = serde_json::Value::Null,
            16 => v["unknown"] = true.into(),
            17 => fs::remove_file(dest.join(HUD_FILENAME)).unwrap(),
            18 => fs::write(dest.join("extra.mesh-hud.json"), b"{}").unwrap(),
            19 => fs::write(dest.join(HUD_FILENAME), b"{}").unwrap(),
            20 => fs::OpenOptions::new()
                .write(true)
                .open(dest.join(HUD_FILENAME))
                .unwrap()
                .set_len(MAX_HUD_BYTES + 1)
                .unwrap(),
            _ => {
                let mut hud: serde_json::Value =
                    serde_json::from_slice(&fs::read(dest.join(HUD_FILENAME)).unwrap()).unwrap();
                match case {
                    21 => hud["scene_filename"] = "source.scene.yaml".into(),
                    22 => {
                        hud["graph_revision"][0] =
                            (hud["graph_revision"][0].as_u64().unwrap() ^ 1).into()
                    }
                    23 => hud["hud"]["font_size"] = 0.into(),
                    24 => hud["hud"] = serde_json::Value::Null,
                    25 => hud["hud"]["title"] = "x".repeat(81).into(),
                    _ => hud["hud"]["unknown"] = true.into(),
                }
                let bytes = serde_json::to_vec(&hud).unwrap();
                fs::write(dest.join(HUD_FILENAME), &bytes).unwrap();
                v["hud"] = serde_json::to_value(record(HUD_FILENAME, &bytes)).unwrap();
            }
        });
        assert!(open_bundle(&dest).is_err(), "case {case}");
    }
}

#[test]
fn hud_key_is_forbidden_in_all_legacy_schemas_even_when_null() {
    for schema in 1..=4 {
        for hud in [serde_json::Value::Null, serde_json::json!({})] {
            let f = Fixture::new(RuntimePlatform::LinuxX86_64);
            match schema {
                2 => {
                    f.add_camera(RuntimePlatform::LinuxX86_64);
                }
                3 => {
                    f.add_art(RuntimePlatform::LinuxX86_64, false);
                }
                4 => {
                    f.add_lighting(RuntimePlatform::LinuxX86_64, false, false, false);
                }
                _ => {}
            }
            let dest = f.root.join("out");
            f.export(&dest, true).unwrap();
            mutate_manifest(&dest, |v| v["hud"] = hud);
            assert!(open_bundle(&dest).is_err());
        }
    }
}

#[test]
fn hud_sources_reject_malformed_stale_unknown_null_bounds_and_aliases() {
    for case in 0..16 {
        let f = Fixture::new(RuntimePlatform::LinuxX86_64);
        let bytes = f.add_hud(RuntimePlatform::LinuxX86_64, false, false, false);
        let path = f.hud_path();
        match case {
            0 => fs::write(&path, b"{}").unwrap(),
            1 => fs::write(&path, b"null").unwrap(),
            2 => fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(MAX_HUD_BYTES + 1)
                .unwrap(),
            3 => {
                fs::rename(&path, f.root.join("source.scene.yaml.MESH-HUD.JSON")).unwrap();
            }
            4 => fs::write(
                f.root.join("source.scene.yaml.mesh-hud.json.extra"),
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
                    8 => v["hud"]["unknown"] = true.into(),
                    9 => v["hud"] = serde_json::Value::Null,
                    10 => v["hud"]["title"] = serde_json::Value::Null,
                    11 => v["hud"]["font_size"] = 0.into(),
                    12 => v["hud"]["text_color"] = serde_json::json!([0, 0, 0]),
                    13 => v["hud"]["title"] = "x".repeat(81).into(),
                    14 => v["hud"]["font_size"] = 49.into(),
                    _ => v["schema"] = 2.into(),
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
fn hud_source_presence_and_exact_bytes_bound_at_admission_and_publication() {
    for before_admission in [false, true] {
        for case in 0..3 {
            let f = Fixture::new(RuntimePlatform::LinuxX86_64);
            let bytes = f.add_hud(RuntimePlatform::LinuxX86_64, false, false, false);
            if case == 0 {
                fs::remove_file(f.hud_path()).unwrap();
            }
            let snapshot = capture_sources(&f.scene).unwrap();
            let change = || match case {
                0 => fs::write(f.hud_path(), &bytes).unwrap(),
                1 => fs::remove_file(f.hud_path()).unwrap(),
                _ => {
                    let mut changed = bytes.clone();
                    *changed.last_mut().unwrap() = b' ';
                    fs::write(f.hud_path(), changed).unwrap();
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
    f.add_hud(RuntimePlatform::LinuxX86_64, false, false, false);
    assert!(capture_camera_source(&f.scene).is_err());
    assert!(export_interactive_bound(f.request(&f.root.join("out")), &None).is_err());
}

#[test]
fn hud_duplicate_records_and_fields_are_rejected() {
    let f = Fixture::new(RuntimePlatform::LinuxX86_64);
    let source = f.add_hud(RuntimePlatform::LinuxX86_64, false, false, false);
    let dest = f.root.join("out");
    f.export(&dest, true).unwrap();
    let manifest = dest.join(MANIFEST_FILENAME);
    let original = fs::read_to_string(&manifest).unwrap();
    for prefix in ["\"hud\":null,", "\"hud\":{},"] {
        fs::write(&manifest, original.replacen('{', &format!("{{{prefix}"), 1)).unwrap();
        assert!(open_bundle(&dest).is_err());
    }
    let text = String::from_utf8(source).unwrap();
    for duplicate in [
        text.replacen('{', "{\"scene_filename\":\"source.scene.yaml\",", 1),
        text.replace("\"font_size\":", "\"font_size\":1.0,\"font_size\":"),
    ] {
        fs::write(f.hud_path(), duplicate).unwrap();
        assert!(f.export(&f.root.join("duplicate"), true).is_err());
    }
}

#[cfg(unix)]
#[test]
fn hud_symlink_and_fifo_sources_and_bundle_reject_without_blocking() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new(RuntimePlatform::LinuxX86_64);
    let bytes = f.add_hud(RuntimePlatform::LinuxX86_64, false, false, false);
    let dest = f.root.join("out");
    f.export(&dest, true).unwrap();
    let real = f.root.join("real-hud");
    fs::write(&real, bytes).unwrap();
    fs::remove_file(dest.join(HUD_FILENAME)).unwrap();
    symlink(&real, dest.join(HUD_FILENAME)).unwrap();
    assert!(open_bundle(&dest).is_err());
    fs::remove_file(f.hud_path()).unwrap();
    symlink(&real, f.hud_path()).unwrap();
    assert!(capture_sources(&f.scene).is_err());
    fs::remove_file(f.hud_path()).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(f.hud_path())
            .status()
            .unwrap()
            .success()
    );
    assert!(capture_sources(&f.scene).is_err());
    assert!(f.export(&f.root.join("fifo"), true).is_err());
    fs::remove_file(dest.join(HUD_FILENAME)).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(dest.join(HUD_FILENAME))
            .status()
            .unwrap()
            .success()
    );
    assert!(open_bundle(&dest).is_err());
}

#[test]
fn hud_runtime_rejects_malformed_sections_even_when_not_required() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for case in 0..7 {
            let mut bytes = hud_runtime(platform, true, true, false);
            match platform {
                RuntimePlatform::LinuxX86_64 => match case {
                    0 => bytes[448] ^= 1,
                    1 => put64(&mut bytes, 904, 3), // Writable section.
                    2 => put64(&mut bytes, 928, 20),
                    3 => put64(&mut bytes, 912, 0x5001c0), // Unmapped address.
                    4 => {
                        // Make the earlier camera section a valid hud section first.
                        put32(&mut bytes, 832, 43);
                        put64(&mut bytes, 864, runtime::HUD_CAPABILITY_MARKER.len() as u64);
                        bytes[416..416 + runtime::HUD_CAPABILITY_MARKER.len()]
                            .copy_from_slice(runtime::HUD_CAPABILITY_MARKER);
                    }
                    5 => put32(&mut bytes, 900, 8),   // SHT_NOBITS.
                    _ => put64(&mut bytes, 920, 416), // Overlapping payload.
                },
                RuntimePlatform::WindowsX86_64 => match case {
                    0 => bytes[2560] ^= 1,
                    1 => put32(&mut bytes, 396, 0xe000_0040),
                    2 => put32(&mut bytes, 368, 20),
                    3 => bytes[2560 + runtime::HUD_CAPABILITY_MARKER.len()] = 1,
                    4 => {
                        bytes[320..328].copy_from_slice(b".orrhhud");
                        put32(&mut bytes, 328, runtime::HUD_CAPABILITY_MARKER.len() as u32);
                        bytes[2048..2560].fill(0);
                        bytes[2048..2048 + runtime::HUD_CAPABILITY_MARKER.len()]
                            .copy_from_slice(runtime::HUD_CAPABILITY_MARKER);
                    }
                    5 => put32(&mut bytes, 380, 2048), // Overlapping payload.
                    _ => put32(&mut bytes, 372, 0xffff_f000),
                },
            }
            if case == 4 {
                let duplicate = Err("runtime has duplicate capability sections".to_owned());
                assert_eq!(runtime::validate_runtime(&bytes), duplicate, "{platform:?}");
                assert_eq!(
                    runtime::validate_hud_runtime(&bytes, false, false, false),
                    duplicate
                );
            } else {
                assert!(
                    runtime::validate_runtime(&bytes).is_err(),
                    "{platform:?} case {case}"
                );
                assert!(runtime::validate_hud_runtime(&bytes, false, false, false).is_err());
            }
        }
        let mut absent = hud_runtime(platform, false, false, false);
        match platform {
            RuntimePlatform::WindowsX86_64 => absent[280] = b'x',
            RuntimePlatform::LinuxX86_64 => absent[345] = b'x',
        }
        absent.extend_from_slice(runtime::HUD_CAPABILITY_MARKER);
        assert!(runtime::validate_runtime(&absent).is_ok());
        assert!(runtime::validate_hud_runtime(&absent, false, false, false).is_err());
    }
}
