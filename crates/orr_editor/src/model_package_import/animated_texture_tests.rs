use super::*;
fn variant() -> TextureVariant {
    TextureVariant {
        material: 0,
        texture: "skin.png".into(),
        asset_id: "variants/skin.retexture.json".into(),
    }
}
fn unopened() -> Request {
    Request {
        source: "missing.animated.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/out".into(),
        name: "skin-player".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Animated,
        source_format: SourceFormat::CookedAnimated,
        license_acknowledged: true,
    }
}
#[test]
fn spec_and_mode_reject_without_filesystem_access() {
    for id in [
        "",
        "../bad.retexture.json",
        "CON.retexture.json",
        "bad.recolor.json",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(validate_texture_variant(&v).is_err());
    }
    let mut v = variant();
    v.texture = "skin.jpg".into();
    assert!(validate_texture_variant(&v).is_err());
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(import_texture_variant(r, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut r = unopened();
    r.source_format = SourceFormat::RawGlb;
    assert!(import_texture_variant(r, variant())
        .err()
        .unwrap()
        .contains("CookedAnimated"));
    let mut r = unopened();
    r.kind = ModelKind::Static;
    assert!(import_texture_variant(r, variant())
        .err()
        .unwrap()
        .contains("Animated model"));
}
#[cfg(not(feature = "room-animated-models"))]
#[test]
fn missing_feature_precedes_any_input_or_destination_access() {
    assert!(import_texture_variant(unopened(), variant())
        .err()
        .unwrap()
        .contains("requires room-animated-models"));
}
#[cfg(feature = "room-animated-models")]
fn png_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, 2, 1);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()
            .unwrap()
            .write_image_data(&[21, 83, 177, 255, 229, 41, 97, 113])
            .unwrap();
    }
    out
}
#[cfg(feature = "room-animated-models")]
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
#[cfg(feature = "room-animated-models")]
#[test]
fn producer_appends_png_and_changes_only_selected_image_reference() {
    let original = orr_model::animation::AnimatedModel::from_bytes(SOURCE).unwrap();
    let png = png_bytes();
    let bytes = texture_variant_bytes(SOURCE, &png, &variant()).unwrap();
    assert_eq!(
        bytes,
        texture_variant_bytes(SOURCE, &png, &variant()).unwrap()
    );
    let out = orr_model::animation::AnimatedModel::from_bytes(&bytes).unwrap();
    let mut expected = original.source().clone();
    expected.asset_id = variant().asset_id;
    for p in &mut expected.primitives {
        p.id = format!(
            "{}{}",
            expected.asset_id,
            p.id.strip_prefix(&original.source().asset_id).unwrap()
        );
    }
    expected.materials[0].image = expected.images.len() as u32;
    expected.images.push(orr_model::Image {
        width: 2,
        height: 1,
        rgba8: vec![21, 83, 177, 255, 229, 41, 97, 113],
    });
    expected.dependencies = out.source().dependencies.clone();
    assert_eq!(serde_json::to_vec(&expected).unwrap(), bytes);
    assert_eq!(out.source().dependencies.len(), 3);
    assert_eq!(out.source().dependencies[1].uri, "input.animated.json");
    assert_eq!(out.source().dependencies[1].sha256, digest(SOURCE));
    assert_eq!(out.source().dependencies[2].uri, "input.png");
    assert_eq!(out.source().dependencies[2].sha256, digest(&png));
}
#[cfg(feature = "room-animated-models")]
#[test]
fn malformed_png_and_unused_material_reject() {
    assert!(texture_variant_bytes(SOURCE, b"bad png", &variant()).is_err());
    let mut v = variant();
    v.material = u32::MAX;
    assert!(texture_variant_bytes(SOURCE, &png_bytes(), &v).is_err());
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
mod publication {
    use super::*;
    fn request(root: &Path) -> (Request, TextureVariant) {
        let r = Request {
            source: root.join("source.animated.json"),
            license: root.join("LICENSE"),
            destination: root.join("skin-package"),
            ..unopened()
        };
        fs::write(&r.source, SOURCE).unwrap();
        fs::write(&r.license, b"Model and skin license\n").unwrap();
        let mut v = variant();
        v.texture = root.join("skin.PNG");
        fs::write(&v.texture, png_bytes()).unwrap();
        (r, v)
    }
    #[test]
    fn publishes_real_png_variant_with_correct_hash_and_immutable_inputs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let (r, v) = request(&root);
        let paths = (r.source.clone(), r.license.clone(), v.texture.clone());
        let report = import_texture_variant(r, v).unwrap();
        let bytes = fs::read(report.root.join(&report.asset)).unwrap();
        assert_eq!(report.source_sha256, digest(&bytes));
        assert_eq!(
            bytes,
            texture_variant_bytes(SOURCE, &png_bytes(), &variant()).unwrap()
        );
        assert_eq!(fs::read(paths.0).unwrap(), SOURCE);
        assert_eq!(fs::read(paths.1).unwrap(), b"Model and skin license\n");
        assert_eq!(fs::read(paths.2).unwrap(), png_bytes());
        let inspected =
            orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime())
                .unwrap();
        assert_eq!(
            inspected.manifest.capabilities,
            BTreeSet::from(["animation".into(), "models".into()])
        );
    }
    #[test]
    fn invalid_png_no_publication_existing_destination_no_replacement() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let (r, v) = request(&root);
        let dest = r.destination.clone();
        fs::write(&v.texture, b"bad png").unwrap();
        assert!(import_texture_variant(r, v).is_err());
        assert!(!dest.exists());
        let (r, v) = request(&root);
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("sentinel"), b"keep").unwrap();
        assert!(import_texture_variant(r, v).is_err());
        assert_eq!(fs::read(dest.join("sentinel")).unwrap(), b"keep");
    }
}
