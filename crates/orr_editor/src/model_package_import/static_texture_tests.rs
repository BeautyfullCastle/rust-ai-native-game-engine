use super::*;
use orr_model::{Dependency, Image, StaticModel, Wrap};

const PIXELS: [u8; 8] = [21, 83, 177, 255, 229, 41, 97, 113];

fn variant() -> StaticTextureVariant {
    StaticTextureVariant {
        material: 1,
        texture: if cfg!(windows) {
            "C:\\unopened\\skin.png"
        } else {
            "/unopened/skin.png"
        }
        .into(),
        asset_id: "variants/static-skin.retexture.json".into(),
    }
}

fn unopened() -> Request {
    Request {
        source: "missing.model.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "static-skin-prop".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Static,
        source_format: SourceFormat::CookedStatic,
        license_acknowledged: true,
    }
}

fn fixture() -> StaticModel {
    let original = orr_model::import::import_with_resolver(
        "props/original.glb",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded fixture must not resolve dependencies"),
    )
    .unwrap();
    let mut source = original.source().clone();
    source.materials[1].image = source.materials[0].image;
    source.materials[1].base_color = [-0.0, 0.25, 0.75, 0.5];
    source.materials[1].linear_filter = true;
    source.materials[1].wrap_s = Wrap::Mirror;
    source.materials[1].wrap_t = Wrap::Repeat;
    source.materials.push(source.materials[1].clone());
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![19, 37, 71, 103],
    });
    source.primitives[0].vertices[0].uv = [-0.0, 0.0];
    source.primitives[0].vertices[0].position[2] = -0.0;
    let mut shared = source.primitives[1].clone();
    shared.id = format!("{}#node=17/mesh=8/primitive=3", source.asset_id);
    shared.transform[3] = [4.0, -7.0, 11.0, 1.0];
    source.primitives.push(shared);
    source.dependencies.push(Dependency {
        uri: "history/missing-skin.png".into(),
        sha256: digest(b"historical input is not resolved"),
    });
    StaticModel::new(source).unwrap()
}

fn png(width: u32, color: png::ColorType, data: &[u8], animated: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, 1);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        if animated {
            encoder.set_animated(1, 0).unwrap();
        }
        encoder
            .write_header()
            .unwrap()
            .write_image_data(data)
            .unwrap();
    }
    bytes
}

fn skin() -> Vec<u8> {
    png(2, png::ColorType::Rgba, &PIXELS, false)
}

#[test]
fn static_texture_spec_hash_license_mode_and_paths_reject_before_io() {
    for id in [
        "",
        "../bad.retexture.json",
        "/bad.retexture.json",
        "CON.retexture.json",
        "bad.uv.json",
        "x//bad.retexture.json",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(import_static_texture_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("variant identity"));
    }
    for path in ["relative.png", "../skin.png", "", "C:drive-relative.png"] {
        let mut v = variant();
        v.texture = path.into();
        assert!(import_static_texture_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("path must"));
    }
    let mut v = variant();
    v.texture.set_extension("jpg");
    assert!(import_static_texture_variant(unopened(), v)
        .err()
        .unwrap()
        .contains(".png"));
    for material in [orr_model::MAX_MATERIALS as u32, u32::MAX] {
        let mut v = variant();
        v.material = material;
        assert!(import_static_texture_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("material limit"));
    }
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(import_static_texture_variant(r, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut r = unopened();
    r.kind = ModelKind::Animated;
    assert!(import_static_texture_variant(r, variant())
        .err()
        .unwrap()
        .contains("Static model mode"));
    for format in [SourceFormat::RawGlb, SourceFormat::CookedAnimated] {
        let mut r = unopened();
        r.source_format = format;
        assert!(import_static_texture_variant(r, variant())
            .err()
            .unwrap()
            .contains("CookedStatic"));
    }
    for hash in ["".into(), "bad".into(), "z".repeat(64), "a".repeat(65)] {
        assert!(
            import_static_texture_variant_checked(unopened(), variant(), Some(hash))
                .err()
                .unwrap()
                .contains("64 hexadecimal")
        );
    }
    validate_static_texture_variant(&variant()).unwrap();
}

#[test]
fn static_texture_payload_pixels_provenance_and_shared_original_are_exact() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    let png = skin();
    let v = variant();
    let bytes = static_texture_variant_bytes(&source, &png, &v).unwrap();
    assert_eq!(
        static_texture_variant_bytes(&source, &png, &v).unwrap(),
        bytes
    );
    let out = StaticModel::from_bytes(&bytes).unwrap();
    let mut expected = original.source().clone();
    expected.asset_id = v.asset_id;
    for primitive in &mut expected.primitives {
        primitive.id = format!(
            "{}{}",
            expected.asset_id,
            primitive
                .id
                .strip_prefix(&original.source().asset_id)
                .unwrap()
        );
    }
    expected.materials[1].image = expected.images.len() as u32;
    expected.images.push(Image {
        width: 2,
        height: 1,
        rgba8: PIXELS.to_vec(),
    });
    let recipe = serde_json::to_vec(&serde_json::json!({
        "format": "orr_static_material_texture", "version": 1, "source": "input.model.json",
        "texture": "skin.png", "material": 1,
    }))
    .unwrap();
    expected.dependencies = vec![
        Dependency {
            uri: "$source".into(),
            sha256: digest(&recipe),
        },
        Dependency {
            uri: "input.model.json".into(),
            sha256: digest(&source),
        },
        Dependency {
            uri: "skin.png".into(),
            sha256: digest(&png),
        },
    ];
    // Full serialization checks geometry, signed zeros, samplers, original image tables,
    // unused material/image entries and unselected materials sharing the original image.
    assert_eq!(serde_json::to_vec(&expected).unwrap(), bytes);
    assert_eq!(
        out.source().materials[0].image,
        original.source().materials[0].image
    );
    assert_eq!(
        &out.source().images[..original.source().images.len()],
        original.source().images.as_slice()
    );
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn static_texture_malformed_missing_unused_slots_and_combined_budget_reject() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    assert!(static_texture_variant_bytes(b"bad model", &skin(), &variant()).is_err());
    assert!(static_texture_variant_bytes(&source, b"bad PNG", &variant()).is_err());
    for material in [2, 3] {
        let mut v = variant();
        v.material = material;
        assert!(static_texture_variant_bytes(&source, &skin(), &v)
            .unwrap_err()
            .contains("exist and be used"));
    }
    let mut at_limit = source.clone();
    at_limit.resize(orr_model::MAX_FILE_BYTES, b' ');
    assert!(static_texture_variant_bytes(&at_limit, &skin(), &variant())
        .unwrap_err()
        .contains("64 MiB input budget"));
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn static_texture_reuses_strict_png_decoder_and_image_inventory_limits() {
    let source = fixture().to_bytes().unwrap();
    for png in [
        png(2, png::ColorType::Rgb, &[1, 2, 3, 4, 5, 6], false),
        png(2, png::ColorType::Rgba, &PIXELS, true),
        png(
            orr_model::MAX_IMAGE_EXTENT + 1,
            png::ColorType::Rgba,
            &vec![0; (orr_model::MAX_IMAGE_EXTENT as usize + 1) * 4],
            false,
        ),
    ] {
        assert!(static_texture_variant_bytes(&source, &png, &variant()).is_err());
    }
    let mut full = fixture().source().clone();
    full.images
        .resize(orr_model::MAX_IMAGES, full.images[0].clone());
    assert!(static_texture_variant_bytes(
        &StaticModel::new(full).unwrap().to_bytes().unwrap(),
        &skin(),
        &variant()
    )
    .unwrap_err()
    .contains("image count limit"));
}

#[cfg(target_os = "linux")]
mod publication {
    use super::*;
    fn request(root: &Path) -> (Request, StaticTextureVariant, Vec<u8>) {
        let source = fixture().to_bytes().unwrap();
        let r = Request {
            source: root.join("source.MODEL.JSON"),
            license: root.join("LICENSE"),
            destination: root.join("static-skin-package"),
            ..unopened()
        };
        let mut v = variant();
        v.texture = root.join("skin.PNG");
        fs::write(&r.source, &source).unwrap();
        fs::write(&r.license, b"Static model and PNG license\n").unwrap();
        fs::write(&v.texture, skin()).unwrap();
        (r, v, source)
    }

    #[test]
    fn static_texture_plain_checked_publish_immutable_inputs_and_preserve_collisions() {
        for checked in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let (r, v, source) = request(&root);
            let paths = (r.source.clone(), r.license.clone(), v.texture.clone());
            let expected = static_texture_variant_bytes(&source, &skin(), &v).unwrap();
            let report = if checked {
                let catalog = source_inspection::inspect_static_source(&r.source).unwrap();
                import_static_texture_variant_checked(
                    r,
                    v,
                    Some(catalog.source_sha256.to_ascii_uppercase()),
                )
            } else {
                import_static_texture_variant(r, v)
            }
            .unwrap();
            assert_eq!(report.asset, "model.orrmodel");
            assert_eq!(report.kind, ModelKind::Static);
            assert_eq!(report.source_sha256, digest(&expected));
            assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), expected);
            assert_eq!(fs::read(&paths.0).unwrap(), source);
            assert_eq!(
                fs::read(&paths.1).unwrap(),
                b"Static model and PNG license\n"
            );
            assert_eq!(fs::read(&paths.2).unwrap(), skin());
            let package =
                orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime())
                    .unwrap();
            assert_eq!(
                package.manifest.capabilities,
                BTreeSet::from(["models".into()])
            );
            assert_eq!(
                package.manifest.files,
                BTreeSet::from(["LICENSE.txt".into(), "model.orrmodel".into()])
            );
            fs::write(report.root.join("foreign"), b"keep").unwrap();
            let (r, v, _) = request(&root);
            assert!(import_static_texture_variant(r, v).is_err());
            assert_eq!(fs::read(report.root.join("foreign")).unwrap(), b"keep");
            assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), expected);
        }
    }

    #[test]
    fn static_texture_stale_hash_precedes_png_path_or_read_and_never_publishes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (r, v, _) = request(&root);
        let output = r.destination.clone();
        let catalog = source_inspection::inspect_static_source(&r.source).unwrap();
        let mut changed = fixture().source().clone();
        changed.materials[0].base_color[0] = 0.33;
        let changed = StaticModel::new(changed).unwrap().to_bytes().unwrap();
        fs::write(&r.source, &changed).unwrap();
        fs::remove_file(&v.texture).unwrap(); // No PNG exists: stale-source error must win.
        assert!(
            import_static_texture_variant_checked(r, v, Some(catalog.source_sha256))
                .err()
                .unwrap()
                .contains("inspected static source changed")
        );
        assert!(!output.exists());
        assert_eq!(fs::read(root.join("source.MODEL.JSON")).unwrap(), changed);
    }

    #[test]
    fn static_texture_safe_readers_reject_linked_and_nonregular_png_and_source() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (r, mut v, _) = request(&root);
        let output = r.destination.clone();
        let link = root.join("linked.png");
        std::os::unix::fs::symlink(&v.texture, &link).unwrap();
        v.texture = link;
        assert!(import_static_texture_variant(r, v).is_err());
        assert!(!output.exists());
        let (mut r, v, _) = request(&root);
        let link = root.join("linked.model.json");
        std::os::unix::fs::symlink(&r.source, &link).unwrap();
        r.source = link;
        assert!(import_static_texture_variant(r, v).is_err());
        assert!(!output.exists());
        let (r, v, _) = request(&root);
        fs::remove_file(&v.texture).unwrap();
        fs::create_dir(&v.texture).unwrap();
        assert!(import_static_texture_variant(r, v).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn static_texture_invalid_oversized_png_and_store_destination_never_publish() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (r, v, source) = request(&root);
        let output = r.destination.clone();
        fs::write(&v.texture, b"bad PNG").unwrap();
        assert!(import_static_texture_variant(r, v).is_err());
        assert!(!output.exists());
        let (r, v, _) = request(&root);
        File::create(&v.texture)
            .unwrap()
            .set_len(MAX_SOURCE_BYTES + 1)
            .unwrap();
        assert!(import_static_texture_variant(r, v)
            .err()
            .unwrap()
            .contains("byte limit"));
        assert!(!output.exists());
        let (mut r, v, _) = request(&root);
        r.destination = root.join(".orr/package");
        assert!(import_static_texture_variant(r, v)
            .err()
            .unwrap()
            .contains(".orr store"));
        assert!(!root.join(".orr").exists());
        assert_eq!(fs::read(root.join("source.MODEL.JSON")).unwrap(), source);
    }
}
