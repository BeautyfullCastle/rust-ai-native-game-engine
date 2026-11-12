fn absolute(name: &str) -> std::path::PathBuf {
    if cfg!(windows) {
        std::path::Path::new(r"C:\unopened").join(name)
    } else {
        std::path::Path::new("/unopened").join(name)
    }
}
use super::*;
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
fn color() -> Request {
    Request {
        source: absolute("missing.animated.json"),
        spec: Spec::Color(ColorVariant {
            material: 0,
            linear_rgb: [0.2, 0.7, 0.4],
            asset_id: "variants/preview.recolor.json".into(),
        }),
        expected_source_sha256: None,
    }
}
fn texture() -> Request {
    Request {
        spec: Spec::Texture(TextureVariant {
            material: 0,
            texture: absolute("missing.png"),
            asset_id: "variants/preview.retexture.json".into(),
        }),
        ..color()
    }
}
fn png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, 1, 1);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()
            .unwrap()
            .write_image_data(&[21, 83, 177, 113])
            .unwrap();
    }
    out
}
#[test]
fn color_preview_matches_existing_producer_and_raw_hashes() {
    let r = color();
    let prepared = from_captured(&r, SOURCE, None).unwrap();
    let Spec::Color(spec) = &r.spec else {
        unreachable!()
    };
    let expected = color_variant_bytes(SOURCE, spec).unwrap();
    assert_eq!(prepared.model.to_bytes().unwrap(), expected);
    assert_eq!(prepared.asset_sha256, digest(&expected));
    assert_eq!(prepared.source_sha256, digest(SOURCE));
    assert_eq!(prepared.png_sha256, None);
    assert_eq!(prepared.model.source().clips.len(), 3);
}
#[test]
fn png_preview_matches_existing_producer_and_three_dependencies() {
    let r = texture();
    let png = png();
    let p = from_captured(&r, SOURCE, Some(&png)).unwrap();
    let Spec::Texture(spec) = &r.spec else {
        unreachable!()
    };
    let expected = texture_variant_bytes(SOURCE, &png, spec).unwrap();
    assert_eq!(p.model.to_bytes().unwrap(), expected);
    assert_eq!(p.asset_sha256, digest(&expected));
    assert_eq!(p.png_sha256, Some(digest(&png)));
    assert_eq!(p.model.source().dependencies.len(), 3);
    assert_eq!(p.model.source().clips.len(), 3);
}
#[test]
fn invalid_spec_hash_suffix_and_source_fail_before_publication() {
    let mut r = color();
    r.expected_source_sha256 = Some("bad".into());
    assert!(prepare(r.clone()).err().unwrap().contains("64 hexadecimal"));
    r.expected_source_sha256 = Some("a".repeat(64));
    assert!(
        from_captured(&r, SOURCE, None)
            .err()
            .unwrap()
            .contains("source changed")
    );
    r = color();
    r.source = absolute("bad.glb");
    assert!(prepare(r).err().unwrap().contains(".animated.json"));
    let mut r = color();
    let Spec::Color(spec) = &mut r.spec else {
        unreachable!()
    };
    spec.linear_rgb[0] = f64::NAN;
    assert!(prepare(r).err().unwrap().contains("linear_rgb"));
    assert!(from_captured(&texture(), SOURCE, Some(b"bad png")).is_err());
    assert!(from_captured(&color(), b"bad model", None).is_err());
}
#[cfg(target_os = "linux")]
#[test]
fn readonly_preparation_preserves_inputs_and_creates_no_package() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let source = root.join("source.animated.json");
    let image = root.join("skin.png");
    fs::write(&source, SOURCE).unwrap();
    fs::write(&image, png()).unwrap();
    let mut r = texture();
    r.source = source.clone();
    let Spec::Texture(spec) = &mut r.spec else {
        unreachable!()
    };
    spec.texture = image.clone();
    let p = prepare(r).unwrap();
    assert_eq!(p.source_sha256, digest(SOURCE));
    assert_eq!(fs::read(&source).unwrap(), SOURCE);
    assert_eq!(fs::read(&image).unwrap(), png());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
}
#[cfg(target_os = "linux")]
#[test]
fn stale_source_and_symlink_inputs_are_rejected_readonly() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let source = root.join("source.animated.json");
    fs::write(&source, SOURCE).unwrap();
    let mut r = color();
    r.source = source.clone();
    r.expected_source_sha256 = Some("a".repeat(64));
    assert!(prepare(r).err().unwrap().contains("source changed"));
    let link = root.join("link.animated.json");
    std::os::unix::fs::symlink(source, &link).unwrap();
    let mut r = color();
    r.source = link;
    assert!(prepare(r).is_err());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
}

fn trim() -> Request {
    Request {
        spec: Spec::Trim(TrimVariant {
            clip: 0,
            start_seconds: 0.25,
            end_seconds: 0.75,
            name: "preview-short".into(),
            asset_id: "clips/preview.trim.json".into(),
        }),
        ..color()
    }
}
#[test]
fn trim_preview_matches_import_bytes_appends_clip_and_captures_source_hash() {
    let r = trim();
    let p = from_captured(&r, SOURCE, None).unwrap();
    let Spec::Trim(spec) = &r.spec else {
        unreachable!()
    };
    let expected = trim_variant_bytes(SOURCE, spec).unwrap();
    assert_eq!(p.model.to_bytes().unwrap(), expected);
    assert_eq!(p.asset_sha256, digest(&expected));
    assert_eq!(p.source_sha256, digest(SOURCE));
    assert_eq!(p.png_sha256, None);
    assert_eq!(p.model.source().clips.len(), 4);
    assert_eq!(p.model.source().clips[3].name, "preview-short");
    assert_eq!(p.model.source().clips[3].duration(), 0.5);
}
#[test]
fn trim_preview_rejects_stale_source_and_invalid_spec_before_producer() {
    let mut r = trim();
    r.expected_source_sha256 = Some("a".repeat(64));
    assert!(
        from_captured(&r, SOURCE, None)
            .err()
            .unwrap()
            .contains("source changed")
    );
    let mut r = trim();
    let Spec::Trim(spec) = &mut r.spec else {
        unreachable!()
    };
    spec.start_seconds = 1.0;
    spec.end_seconds = 1.0;
    assert!(prepare(r).err().unwrap().contains("trim range"));
}
#[cfg(target_os = "linux")]
#[test]
fn readonly_trim_preparation_creates_no_package_or_other_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("source.animated.json");
    fs::write(&source, SOURCE).unwrap();
    let mut r = trim();
    r.source = source.clone();
    let prepared = prepare(r).unwrap();
    assert_eq!(prepared.model.source().clips.len(), 4);
    assert_eq!(fs::read(source).unwrap(), SOURCE);
    assert_eq!(fs::read_dir(root).unwrap().count(), 1);
}
