use super::*;
use source_inspection::*;
fn request() -> Request {
    Request {
        source: "missing.animated.json".into(),
        license: "missing-license".into(),
        destination: "missing/out".into(),
        name: "catalog-player".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Animated,
        source_format: SourceFormat::CookedAnimated,
        license_acknowledged: true,
    }
}
fn color() -> ColorVariant {
    ColorVariant {
        material: 0,
        linear_rgb: [0.2, 0.7, 0.4],
        asset_id: "variants/catalog.recolor.json".into(),
    }
}
#[test]
fn expected_hash_is_strict_before_any_paths() {
    for h in ["", "abc", &"z".repeat(64), &"a".repeat(65)] {
        assert!(
            import_color_variant_checked(request(), color(), Some(h.into()))
                .err()
                .unwrap()
                .contains("64 hexadecimal")
        );
        let v = TextureVariant {
            material: 0,
            texture: "missing.png".into(),
            asset_id: "variants/catalog.retexture.json".into(),
        };
        assert!(import_texture_variant_checked(request(), v, Some(h.into()))
            .err()
            .unwrap()
            .contains("64 hexadecimal"));
    }
    validate_expected_source_hash(Some(&"A".repeat(64))).unwrap();
    validate_expected_source_hash(None).unwrap();
}
#[test]
fn checked_import_keeps_mode_gate_before_files() {
    let mut r = request();
    r.source_format = SourceFormat::RawGlb;
    assert!(
        import_color_variant_checked(r, color(), Some("a".repeat(64)))
            .err()
            .unwrap()
            .contains("CookedAnimated")
    );
}
#[cfg(not(feature = "room-animated-models"))]
#[test]
fn inspection_feature_gate_precedes_suffix_and_path_access() {
    assert!(inspect_animated_source(Path::new("missing.invalid"))
        .err()
        .unwrap()
        .contains("requires room-animated-models"));
}
#[cfg(feature = "room-animated-models")]
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
#[cfg(feature = "room-animated-models")]
#[test]
fn catalog_matches_complete_admitted_inventory_and_raw_source_hash() {
    let original = orr_model::animation::AnimatedModel::from_bytes(SOURCE).unwrap();
    let c = catalog_from_bytes(SOURCE).unwrap();
    assert_eq!(c.source_sha256, digest(SOURCE));
    assert_eq!(c.materials.len(), original.source().materials.len());
    for (i, m) in c.materials.iter().enumerate() {
        let expected = &original.source().materials[i];
        let image = &original.source().images[expected.image as usize];
        assert_eq!(m.index, i as u32);
        assert_eq!(
            m.used_primitive_count,
            original
                .source()
                .primitives
                .iter()
                .filter(|p| p.material == i as u32)
                .count() as u32
        );
        assert_eq!(
            m.base_color.map(f32::to_bits),
            expected.base_color.map(f32::to_bits)
        );
        assert_eq!(m.image, expected.image);
        assert_eq!((m.width, m.height), (image.width, image.height));
        assert_eq!(
            (m.linear_filter, m.wrap_s, m.wrap_t),
            (expected.linear_filter, expected.wrap_s, expected.wrap_t)
        );
    }
    for (i, c) in c.clips.iter().enumerate() {
        assert_eq!(c.index, i as u32);
        assert_eq!(c.name, original.source().clips[i].name);
        assert_eq!(
            c.duration.to_bits(),
            original.source().clips[i].duration().to_bits()
        );
    }
}
#[cfg(feature = "room-animated-models")]
#[test]
fn inspection_rejects_suffix_malformed_and_empty_clips() {
    assert!(inspect_animated_source(Path::new("missing.glb"))
        .err()
        .unwrap()
        .contains(".animated.json"));
    assert!(catalog_from_bytes(b"bad JSON").is_err());
    let mut v: serde_json::Value = serde_json::from_slice(SOURCE).unwrap();
    v["clips"] = serde_json::json!([]);
    assert!(catalog_from_bytes(&serde_json::to_vec(&v).unwrap())
        .err()
        .unwrap()
        .contains("at least one"));
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
mod filesystem {
    use super::*;
    #[test]
    fn inspection_is_readonly_and_checked_import_rejects_stale_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let path = root.join("source.ANIMATED.JSON");
        fs::write(&path, SOURCE).unwrap();
        let catalog = inspect_animated_source(&path).unwrap();
        assert_eq!(catalog.source_sha256, digest(SOURCE));
        assert_eq!(fs::read(&path).unwrap(), SOURCE);
        let mut changed: serde_json::Value = serde_json::from_slice(SOURCE).unwrap();
        changed["materials"][0]["base_color"][0] = serde_json::json!(0.33);
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        let license = root.join("LICENSE");
        fs::write(&license, b"User supplied license").unwrap();
        let r = Request {
            source: path,
            license,
            destination: root.join("package"),
            ..request()
        };
        let dest = r.destination.clone();
        assert!(
            import_color_variant_checked(r, color(), Some(catalog.source_sha256))
                .err()
                .unwrap()
                .contains("source changed")
        );
        assert!(!dest.exists());
    }
    #[test]
    fn matching_hash_imports_variant_but_symlink_inspection_rejects() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let path = root.join("source.animated.json");
        fs::write(&path, SOURCE).unwrap();
        let catalog = inspect_animated_source(&path).unwrap();
        let link = root.join("link.animated.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(inspect_animated_source(&link).is_err());
        let license = root.join("LICENSE");
        fs::write(&license, b"User supplied license").unwrap();
        let r = Request {
            source: path,
            license,
            destination: root.join("package"),
            ..request()
        };
        let report = import_color_variant_checked(r, color(), Some(catalog.source_sha256)).unwrap();
        let bytes = fs::read(report.root.join(report.asset)).unwrap();
        assert_eq!(report.source_sha256, digest(&bytes));
    }
}
