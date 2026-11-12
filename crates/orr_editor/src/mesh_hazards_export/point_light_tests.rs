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

fn append_point(mut bytes: Vec<u8>, platform: RuntimePlatform) -> Vec<u8> {
    let marker = b"ORR_MESH_POINT_LIGHT_V1";
    match platform {
        RuntimePlatform::WindowsX86_64 => {
            let count = u16::from_le_bytes(bytes[70..72].try_into().unwrap());
            let section = 200 + usize::from(count) * 40;
            let offset = bytes.len();
            bytes.resize(offset + 512, 0);
            put16(&mut bytes, 70, count + 1);
            put32(&mut bytes, 144, (u32::from(count) + 2) * 4096);
            bytes[section..section + 8].copy_from_slice(b".orrpntl");
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
            bytes[320 + name..329 + name].copy_from_slice(b".orrpntl\0");
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

#[test]
fn pe_and_elf_point_light_capability_requires_own_section() {
    for p in [RuntimePlatform::WindowsX86_64, RuntimePlatform::LinuxX86_64] {
        let old = lighting_tests::lighting_runtime(p, false, false);
        runtime::validate_point_light_runtime(&old, false).unwrap();
        assert!(runtime::validate_point_light_runtime(&old, true).is_err());
        let mut overlay = old.clone();
        overlay.extend_from_slice(b"ORR_MESH_POINT_LIGHT_V1");
        assert!(runtime::validate_point_light_runtime(&overlay, true).is_err());
        runtime::validate_point_light_runtime(&append_point(old, p), true).unwrap();
    }
}
#[test]
fn pe_and_elf_point_light_rejects_wrong_or_duplicate_marker() {
    for p in [RuntimePlatform::WindowsX86_64, RuntimePlatform::LinuxX86_64] {
        let good = append_point(lighting_tests::lighting_runtime(p, false, false), p);
        let mut wrong = good.clone();
        let start = wrong
            .windows(23)
            .position(|v| v == b"ORR_MESH_POINT_LIGHT_V1")
            .unwrap();
        wrong[start] = b'X';
        assert!(runtime::validate_point_light_runtime(&wrong, true).is_err());
        assert!(runtime::validate_point_light_runtime(&append_point(good, p), true).is_err());
    }
}
#[cfg(target_os = "linux")]
#[test]
fn hazard_export_preserves_point_light_after_source_hidden_and_refuses_old_runtime() {
    for p in [RuntimePlatform::WindowsX86_64, RuntimePlatform::LinuxX86_64] {
        let f = Fixture::new(p);
        let revision = admit_scene(&fs::read(&f.scene).unwrap()).unwrap().1;
        let mut d =
            crate::mesh_lighting::Document::from_lighting(orr_render::Lighting::default()).unwrap();
        d.point_light = Some(orr_render::PointLight {
            position: [2., 3., 4.],
            color: [0.2, 0.7, 0.4],
            intensity: 3.,
            range: 8.,
        });
        let source = crate::mesh_lighting_store::sidecar_path(&f.scene).unwrap();
        fs::write(&source, d.encode("source.scene.yaml", revision).unwrap()).unwrap();
        let old = lighting_tests::lighting_runtime(p, false, false);
        fs::write(&f.runtime, &old).unwrap();
        let dest = f.root.join("point-bundle");
        assert!(f.export(&dest, true).unwrap_err().contains("point-light"));
        assert!(!dest.exists());
        fs::write(&f.runtime, append_point(old.clone(), p)).unwrap();
        f.export(&dest, true).unwrap();
        fs::remove_file(&f.scene).unwrap();
        fs::remove_file(source).unwrap();
        let reopened = open_bundle(&dest).unwrap();
        let (saved, _) = crate::mesh_lighting_store::load(&reopened.scene, revision)
            .unwrap()
            .unwrap();
        assert_eq!(saved, d);
        let path = dest.join(MANIFEST_FILENAME);
        let mut m: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        m["runtime"]["bytes"] = serde_json::json!(old.len());
        m["runtime"]["sha256"] = digest(&old).into();
        fs::write(dest.join(p.runtime_filename()), old).unwrap();
        fs::write(path, serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(open_bundle(&dest).unwrap_err().contains("point-light"));
    }
}
