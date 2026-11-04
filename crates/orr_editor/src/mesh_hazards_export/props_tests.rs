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

pub(crate) fn props_runtime(
    platform: RuntimePlatform,
    camera: bool,
    art: bool,
    lighting: bool,
    hud: bool,
) -> Vec<u8> {
    let mut bytes = if hud {
        hud_tests::hud_runtime(platform, camera, art, lighting)
    } else if lighting {
        lighting_tests::lighting_runtime(platform, camera, art)
    } else if art {
        art_tests::art_runtime(platform, camera)
    } else if camera {
        runtime::valid_test_camera_runtime(platform)
    } else {
        runtime::valid_test_runtime(platform)
    };
    let marker = runtime::PROPS_CAPABILITY_MARKER;
    match platform {
        RuntimePlatform::WindowsX86_64 => {
            let count = u16::from_le_bytes(bytes[70..72].try_into().unwrap());
            let section = 200 + usize::from(count) * 40;
            let offset = bytes.len();
            bytes.resize(offset + 512, 0);
            put16(&mut bytes, 70, count + 1);
            put32(&mut bytes, 144, (u32::from(count) + 2) * 4096);
            bytes[section..section + 8].copy_from_slice(b".orrhprp");
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
            let offset = 320;
            bytes.resize(section + 64, 0);
            put16(&mut bytes, 60, count + 1);
            let mut names = bytes[320..320 + name].to_vec();
            names.extend_from_slice(b".orrhprp\0");
            let names_offset = bytes.len();
            bytes.extend_from_slice(&names);
            put64(&mut bytes, 728, names_offset as u64);
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
    fn props_path(&self) -> PathBuf {
        self.scene
            .with_file_name("source.scene.yaml.mesh-props.json")
    }
    fn add_props(
        &self,
        platform: RuntimePlatform,
        camera: bool,
        art: bool,
        lighting: bool,
        hud: bool,
    ) -> Vec<u8> {
        let revision = admit_scene(&fs::read(&self.scene).unwrap()).unwrap().1;
        let art_bytes = if self.art_path().exists() {
            fs::read(self.art_path()).unwrap()
        } else {
            self.add_art(platform, camera)
        };
        let snapshot =
            crate::mesh_art::Snapshot::decode_embedded(&art_bytes, "source.scene.yaml", revision)
                .unwrap();
        if !art {
            fs::remove_file(self.art_path()).unwrap();
        }
        let document = crate::mesh_props::Document {
            props: vec![crate::mesh_props::Prop {
                id: 7,
                name: "돌 · Static prop".into(),
                art: snapshot,
                #[cfg(feature = "mesh-prop-shading")]
                shading: None,
            }],
        };
        let bytes = document.encode("source.scene.yaml", revision).unwrap();
        fs::write(self.props_path(), &bytes).unwrap();
        fs::write(
            &self.runtime,
            props_runtime(platform, camera, art, lighting, hud),
        )
        .unwrap();
        bytes
    }
}

#[test]
fn props_roundtrip_all_optional_combinations_and_relocation() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for flags in 0..16 {
            let (camera, art, lighting, hud) = (
                flags & 1 != 0,
                flags & 2 != 0,
                flags & 4 != 0,
                flags & 8 != 0,
            );
            let f = Fixture::new(platform);
            if camera {
                f.add_camera(platform);
            }
            if art {
                f.add_art(platform, camera);
            }
            if lighting {
                f.add_lighting(platform, camera, art, false);
            }
            if hud {
                let revision = admit_scene(&fs::read(&f.scene).unwrap()).unwrap().1;
                fs::write(
                    f.scene.with_file_name("source.scene.yaml.mesh-hud.json"),
                    crate::mesh_hud::Document::default()
                        .encode("source.scene.yaml", revision)
                        .unwrap(),
                )
                .unwrap();
            }
            let source = f.add_props(platform, camera, art, lighting, hud);
            let captured = capture_sources(&f.scene).unwrap();
            let dest = f.root.join("한글 props");
            let receipt = f.export(&dest, true).unwrap();
            assert_eq!(capture_sources(&f.scene).unwrap(), captured);
            assert_eq!(fs::read(f.props_path()).unwrap(), source);
            let manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(dest.join(MANIFEST_FILENAME)).unwrap()).unwrap();
            assert_eq!(manifest["schema"], 6);
            assert_eq!(
                fs::read_dir(&dest).unwrap().count(),
                4 + (flags as u32).count_ones() as usize
            );
            let bytes = fs::read(dest.join(PROPS_FILENAME)).unwrap();
            let props =
                decode_static_props(&bytes, SCENE_FILENAME, receipt.graph_revision).unwrap();
            assert_eq!(props.props.len(), 1);
            assert_eq!(props.props[0].id, 7);
            assert_eq!(receipt.props_sha256, Some(digest(&bytes)));
            assert!(f.export(&dest, true).is_err());
            fs::remove_file(f.props_path()).unwrap();
            fs::remove_file(&f.scene).unwrap();
            let moved = f.root.join("moved");
            fs::rename(&dest, &moved).unwrap();
            assert_eq!(
                open_bundle(&moved).unwrap().props_sha256,
                receipt.props_sha256
            );
        }
    }
}

#[test]
fn props_legacy_schemas_forbid_even_null_record() {
    for schema in 1..=5 {
        for value in [serde_json::Value::Null, serde_json::json!({})] {
            let f = Fixture::new(RuntimePlatform::LinuxX86_64);
            let dest = f.root.join("out");
            f.export(&dest, true).unwrap();
            mutate_manifest(&dest, |v| {
                v["schema"] = schema.into();
                v["props"] = value;
            });
            assert!(open_bundle(&dest).is_err());
        }
    }
}

#[test]
fn props_manifest_and_self_consistent_content_tampering_rejected() {
    for case in 0..24 {
        let f = Fixture::new(RuntimePlatform::LinuxX86_64);
        f.add_props(RuntimePlatform::LinuxX86_64, false, false, false, false);
        let dest = f.root.join("out");
        f.export(&dest, true).unwrap();
        mutate_manifest(&dest, |v| match case {
            0 => {
                v.as_object_mut().unwrap().remove("props");
            }
            1 => v["props"] = serde_json::Value::Null,
            2 => v["props"] = serde_json::json!([]),
            3 => v["props"]["bytes"] = (MAX_PROPS_BYTES + 1).into(),
            4 => v["props"]["name"] = "../escape".into(),
            5 => v["props"]["sha256"] = "0".repeat(64).into(),
            6 => fs::write(dest.join("extra.mesh-props.json"), b"{}").unwrap(),
            7 => fs::remove_file(dest.join(PROPS_FILENAME)).unwrap(),
            8 => v["camera"] = serde_json::Value::Null,
            9 => v["art"] = serde_json::Value::Null,
            10 => v["lighting"] = serde_json::Value::Null,
            11 => v["hud"] = serde_json::Value::Null,
            12 => v["props"]["unknown"] = true.into(),
            _ => {
                let mut props: serde_json::Value =
                    serde_json::from_slice(&fs::read(dest.join(PROPS_FILENAME)).unwrap()).unwrap();
                match case {
                    13 => props["scene_filename"] = "source.scene.yaml".into(),
                    14 => {
                        props["graph_revision"][0] =
                            (props["graph_revision"][0].as_u64().unwrap() ^ 1).into()
                    }
                    15 => props["props"][0]["art"]["unknown"] = true.into(),
                    16 => props["props"][0]["animation"] = true.into(),
                    17 => {
                        props["props"][0]["shading"] =
                            serde_json::json!({"mode":"solid","tint":[1.0,1.0,1.0,1.0]})
                    }
                    18 => props["props"][0]["shading"] = serde_json::Value::Null,
                    19 => props["props"] = serde_json::Value::Null,
                    20 => props["props"][0]["name"] = "x".repeat(65).into(),
                    21 => props["props"][0]["art"]["scene_filename"] = "source.scene.yaml".into(),
                    22 => props["props"] = vec![props["props"][0].clone(); 129].into(),
                    _ => props["schema"] = 2.into(),
                }
                let bytes = serde_json::to_vec(&props).unwrap();
                fs::write(dest.join(PROPS_FILENAME), &bytes).unwrap();
                v["props"] = serde_json::to_value(record(PROPS_FILENAME, &bytes)).unwrap();
            }
        });
        assert!(open_bundle(&dest).is_err(), "case {case}");
    }
}

#[test]
fn props_source_changes_bound_at_admission_and_publication() {
    for before_admission in [false, true] {
        for case in 0..3 {
            let f = Fixture::new(RuntimePlatform::LinuxX86_64);
            let source = f.add_props(RuntimePlatform::LinuxX86_64, false, false, false, false);
            if case == 0 {
                fs::remove_file(f.props_path()).unwrap();
            }
            let captured = capture_sources(&f.scene).unwrap();
            let change = || match case {
                0 => fs::write(f.props_path(), &source).unwrap(),
                1 => fs::remove_file(f.props_path()).unwrap(),
                _ => {
                    let mut bytes = source.clone();
                    bytes.push(b' ');
                    fs::write(f.props_path(), bytes).unwrap();
                }
            };
            let dest = f.root.join("out");
            let result = if before_admission {
                change();
                export_interactive_sources(f.request(&dest), &captured)
            } else {
                export_sources_with_recheck(f.request(&dest), &captured, change)
            };
            assert!(result.is_err());
            assert!(!dest.exists());
        }
    }
}

#[test]
fn props_sources_reject_invalid_stale_oversized_and_unknown_attachments() {
    for case in 0..8 {
        let f = Fixture::new(RuntimePlatform::LinuxX86_64);
        let source = f.add_props(RuntimePlatform::LinuxX86_64, false, false, false, false);
        match case {
            0 => fs::write(f.props_path(), b"{}").unwrap(),
            1 => fs::write(f.props_path(), b"null").unwrap(),
            2 => fs::OpenOptions::new()
                .write(true)
                .open(f.props_path())
                .unwrap()
                .set_len(MAX_PROPS_BYTES + 1)
                .unwrap(),
            3 => fs::rename(
                f.props_path(),
                f.root.join("source.scene.yaml.MESH-PROPS.JSON"),
            )
            .unwrap(),
            4 => fs::write(
                f.root.join("source.scene.yaml.mesh-props.json.extra"),
                b"{}",
            )
            .unwrap(),
            5 => fs::write(f.root.join("source.scene.yaml.mesh-animation.json"), b"{}").unwrap(),
            _ => {
                let mut v: serde_json::Value = serde_json::from_slice(&source).unwrap();
                if case == 6 {
                    v["scene_filename"] = "else.scene.yaml".into();
                } else {
                    v["graph_revision"][0] = (v["graph_revision"][0].as_u64().unwrap() ^ 1).into();
                }
                fs::write(f.props_path(), serde_json::to_vec(&v).unwrap()).unwrap();
            }
        }
        let dest = f.root.join("out");
        assert!(f.export(&dest, true).is_err(), "case {case}");
        assert!(!dest.exists());
    }
}

#[test]
fn props_require_separate_marker_and_trust_for_both_platforms() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for flags in 0..16 {
            let (c, a, l, h) = (
                flags & 1 != 0,
                flags & 2 != 0,
                flags & 4 != 0,
                flags & 8 != 0,
            );
            let bytes = props_runtime(platform, c, a, l, h);
            assert_eq!(
                runtime::validate_props_runtime(&bytes, c, a, l, h),
                Ok(platform)
            );
            for n in 0..bytes.len() {
                assert!(runtime::validate_props_runtime(&bytes[..n], c, a, l, h).is_err());
            }
            for missing in 0..4 {
                let requirements = [c, a, l, h];
                if !requirements[missing] {
                    let mut required = requirements;
                    required[missing] = true;
                    assert!(
                        runtime::validate_props_runtime(
                            &bytes,
                            required[0],
                            required[1],
                            required[2],
                            required[3]
                        )
                        .is_err()
                    );
                }
            }
        }
        let f = Fixture::new(platform);
        f.add_props(platform, false, false, false, false);
        let dest = f.root.join("out");
        assert!(f.export(&dest, false).is_err());
        let mut bytes = runtime::valid_test_runtime(platform);
        bytes.extend_from_slice(runtime::PROPS_CAPABILITY_MARKER);
        fs::write(&f.runtime, &bytes).unwrap();
        assert!(f.export(&dest, true).is_err());
        assert!(!dest.exists());
        fs::write(
            &f.runtime,
            props_runtime(platform, false, false, false, false),
        )
        .unwrap();
        f.export(&dest, true).unwrap();
        fs::write(dest.join(platform.runtime_filename()), &bytes).unwrap();
        mutate_manifest(&dest, |v| {
            v["runtime"] =
                serde_json::to_value(record(platform.runtime_filename(), &bytes)).unwrap()
        });
        assert!(open_bundle(&dest).is_err());
    }
}

#[cfg(unix)]
#[test]
fn props_symlink_fifo_sources_and_bundle_fail_closed() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new(RuntimePlatform::LinuxX86_64);
    let bytes = f.add_props(RuntimePlatform::LinuxX86_64, false, false, false, false);
    let dest = f.root.join("out");
    f.export(&dest, true).unwrap();
    let real = f.root.join("real-props");
    fs::write(&real, bytes).unwrap();
    for path in [f.props_path(), dest.join(PROPS_FILENAME)] {
        fs::remove_file(&path).unwrap();
        symlink(&real, &path).unwrap();
        assert!(f.export(&f.root.join("second"), true).is_err());
        assert!(open_bundle(&dest).is_err() || path == f.props_path());
        fs::remove_file(&path).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        if path == f.props_path() {
            assert!(capture_sources(&f.scene).is_err());
        } else {
            assert!(open_bundle(&dest).is_err());
        }
    }
}

#[test]
fn props_runtime_malformed_duplicate_and_misplaced_sections_fail_closed() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        for case in 0..6 {
            let mut bytes = props_runtime(platform, true, false, false, false);
            match platform {
                RuntimePlatform::LinuxX86_64 => {
                    let count = u16::from_le_bytes(bytes[60..62].try_into().unwrap()) as usize;
                    let section = 512 + (count - 1) * 64;
                    match case {
                        0 => bytes[320] ^= 1,
                        1 => put64(&mut bytes, section + 8, 3),
                        2 => put64(&mut bytes, section + 32, 1),
                        3 => put64(&mut bytes, section + 16, 0x500140),
                        4 => {
                            let name =
                                u32::from_le_bytes(bytes[section..section + 4].try_into().unwrap());
                            put32(&mut bytes, 768, name);
                            put64(
                                &mut bytes,
                                800,
                                runtime::PROPS_CAPABILITY_MARKER.len() as u64,
                            );
                            bytes[384..384 + runtime::PROPS_CAPABILITY_MARKER.len()]
                                .copy_from_slice(runtime::PROPS_CAPABILITY_MARKER);
                        }
                        _ => put32(&mut bytes, section + 4, 8),
                    }
                }
                RuntimePlatform::WindowsX86_64 => {
                    let count = u16::from_le_bytes(bytes[70..72].try_into().unwrap()) as usize;
                    let section = 200 + (count - 1) * 40;
                    let offset =
                        u32::from_le_bytes(bytes[section + 20..section + 24].try_into().unwrap())
                            as usize;
                    match case {
                        0 => bytes[offset] ^= 1,
                        1 => put32(&mut bytes, section + 36, 0xe000_0040),
                        2 => put32(&mut bytes, section + 8, 1),
                        3 => bytes[offset + runtime::PROPS_CAPABILITY_MARKER.len()] = 1,
                        4 => {
                            bytes[280..288].copy_from_slice(b".orrhprp");
                            put32(
                                &mut bytes,
                                288,
                                runtime::PROPS_CAPABILITY_MARKER.len() as u32,
                            );
                            bytes[1536..2048].fill(0);
                            bytes[1536..1536 + runtime::PROPS_CAPABILITY_MARKER.len()]
                                .copy_from_slice(runtime::PROPS_CAPABILITY_MARKER);
                        }
                        _ => put32(&mut bytes, section + 20, 1536),
                    }
                }
            }
            assert!(
                runtime::validate_runtime(&bytes).is_err(),
                "{platform:?} case {case}"
            );
            assert!(runtime::validate_props_runtime(&bytes, false, false, false, false).is_err());
        }
    }
}

#[test]
fn props_duplicate_manifest_and_document_fields_fail_closed() {
    let f = Fixture::new(RuntimePlatform::LinuxX86_64);
    let source = f.add_props(RuntimePlatform::LinuxX86_64, false, false, false, false);
    let dest = f.root.join("out");
    f.export(&dest, true).unwrap();
    let manifest = dest.join(MANIFEST_FILENAME);
    let text = fs::read_to_string(&manifest).unwrap();
    for duplicate in ["\"props\":null,", "\"props\":{},"] {
        fs::write(&manifest, text.replacen('{', &format!("{{{duplicate}"), 1)).unwrap();
        assert!(open_bundle(&dest).is_err());
    }
    let text = String::from_utf8(source).unwrap();
    for duplicate in [
        text.replacen('{', "{\"props\":[],", 1),
        text.replacen("\"id\":", "\"id\":7,\"id\":", 1),
    ] {
        fs::write(f.props_path(), duplicate).unwrap();
        assert!(f.export(&f.root.join("duplicate"), true).is_err());
    }
}

#[cfg(feature = "mesh-prop-shading")]
#[test]
fn props_valid_authored_shading_is_rejected_without_mutating_source() {
    let f = Fixture::new(RuntimePlatform::LinuxX86_64);
    let source = f.add_props(RuntimePlatform::LinuxX86_64, false, false, false, false);
    let mut value: serde_json::Value = serde_json::from_slice(&source).unwrap();
    value["props"][0]["shading"] =
        serde_json::json!({"version":1,"slots":[{"material":0,"metallic":0.25,"roughness":0.5}]});
    let bytes = serde_json::to_vec(&value).unwrap();
    let revision = admit_scene(&fs::read(&f.scene).unwrap()).unwrap().1;
    assert!(crate::mesh_props::Document::decode(&bytes, "source.scene.yaml", revision).is_ok());
    assert!(decode_static_props(&bytes, "source.scene.yaml", revision).is_err());
    fs::write(f.props_path(), &bytes).unwrap();
    let dest = f.root.join("out");
    assert!(f.export(&dest, true).is_err());
    assert!(!dest.exists());
    assert_eq!(fs::read(f.props_path()).unwrap(), bytes);
}
