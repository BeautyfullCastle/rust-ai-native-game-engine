#![allow(clippy::float_arithmetic)]
use super::*;
use orr_model::{Dependency, Image, StaticModel, Wrap};

fn variant() -> StaticColorVariant {
    StaticColorVariant {
        material: 1,
        linear_rgb: [0.1, 0.2, 0.3],
        asset_id: "variants/prop.recolor.json".into(),
    }
}

fn unopened() -> Request {
    Request {
        source: "missing.model.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "recolor-prop".into(),
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
        |_| panic!("embedded fixture"),
    )
    .unwrap();
    let mut source = original.source().clone();
    assert_eq!(source.materials.len(), 2);
    assert!(source.primitives.iter().any(|p| p.material == 1));
    source.materials[0].base_color = [-0.0, 0.25, 0.75, 1.0];
    source.materials[1].base_color = [0.125, 0.5, 0.75, -0.0];
    source.materials[1].image = source.materials[0].image;
    source.materials[1].linear_filter = false;
    source.materials[1].wrap_s = Wrap::Mirror;
    source.materials[1].wrap_t = Wrap::Repeat;
    source.materials.push(source.materials[0].clone());
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![19, 37, 71, 103],
    });
    source.primitives[0].transform[3][2] = -0.0;
    for p in &mut source.primitives {
        p.vertices[0].position[2] = -0.0;
        p.vertices[0].normal[0] = -0.0;
        p.vertices[0].uv = [-0.0, f32::from_bits(1)];
    }
    source.dependencies.push(Dependency {
        uri: "history/missing.png".into(),
        sha256: digest(b"never resolved"),
    });
    StaticModel::new(source).unwrap()
}

fn request(root: &Path, bytes: &[u8]) -> Request {
    let source = root.join("source.model.json");
    let license = root.join("LICENSE.txt");
    fs::write(&source, bytes).unwrap();
    fs::write(&license, b"Redistribution license\n").unwrap();
    Request {
        source,
        license,
        destination: root.join("new-package"),
        ..unopened()
    }
}

#[test]
fn static_color_used_slot_preserves_every_other_payload_bit_and_exact_recipe_hash() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    let v = variant();
    let bytes = static_color_variant_bytes(&source, &v).unwrap();
    assert_eq!(bytes, static_color_variant_bytes(&source, &v).unwrap());
    let out = StaticModel::from_bytes(&bytes).unwrap();
    let mut expected = original.source().clone();
    expected.asset_id = v.asset_id.clone();
    for p in &mut expected.primitives {
        p.id = format!(
            "{}{}",
            v.asset_id,
            p.id.strip_prefix(&original.source().asset_id).unwrap()
        );
    }
    expected.materials[1].base_color = [0.1, 0.2, 0.3, -0.0];
    expected.dependencies=vec![Dependency {uri:"$source".into(),sha256:digest(br#"{"format":"orr_static_material_color","version":1,"source":"input.model.json","material":1,"linear_rgb":[0.1,0.2,0.3]}"#)},Dependency {uri:"input.model.json".into(),sha256:digest(&source)}];
    // Serialized payload equality includes signed zero/subnormal geometry,
    // alpha, samplers, shared images, unused slots and source-world transforms.
    assert_eq!(serde_json::to_vec(&expected).unwrap(), bytes);
    assert_eq!(
        out.source().materials[0].image,
        out.source().materials[1].image
    );
    assert_eq!(
        out.source().materials[1].base_color[3].to_bits(),
        (-0.0f32).to_bits()
    );
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn static_color_keeps_original_f64_recipe_values_before_f32_conversion() {
    let bytes = fixture().to_bytes().unwrap();
    let v = variant();
    let out = StaticModel::from_bytes(&static_color_variant_bytes(&bytes, &v).unwrap()).unwrap();
    let mut rounded = v.clone();
    rounded.linear_rgb = v.linear_rgb.map(|x| f64::from(x as f32));
    let narrowed =
        StaticModel::from_bytes(&static_color_variant_bytes(&bytes, &rounded).unwrap()).unwrap();
    assert_eq!(out.source().materials, narrowed.source().materials);
    assert_ne!(
        out.source().dependencies[0].sha256,
        narrowed.source().dependencies[0].sha256
    );
    assert_eq!(
        out.source().dependencies[1],
        narrowed.source().dependencies[1]
    );
}

#[test]
fn static_color_rgb_boundaries_and_alpha_bits_survive_without_sampler_or_image_changes() {
    for alpha in [0.0f32, -0.0, f32::from_bits(1), 0.25, 1.0] {
        let mut source = fixture().source().clone();
        source.materials[1].base_color[3] = alpha;
        let original = StaticModel::new(source).unwrap();
        let bytes = original.to_bytes().unwrap();
        for rgb in [
            [0.0, 1.0, -0.0],
            [f64::MIN_POSITIVE, 1.0 - f64::EPSILON, 0.5],
        ] {
            let mut v = variant();
            v.linear_rgb = rgb;
            let out =
                StaticModel::from_bytes(&static_color_variant_bytes(&bytes, &v).unwrap()).unwrap();
            assert_eq!(
                out.source().materials[1].base_color.map(f32::to_bits),
                [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, alpha].map(f32::to_bits)
            );
            let mut actual = out.source().materials[1].clone();
            actual.base_color = original.source().materials[1].base_color;
            assert_eq!(actual, original.source().materials[1]);
            assert_eq!(out.source().images, original.source().images);
        }
    }
}

#[test]
fn static_color_invalid_f64_identity_index_mode_license_and_hash_reject_before_paths() {
    for id in [
        "",
        "../bad.recolor.json",
        "CON.recolor.json",
        "bad.uv.json",
        "x//bad.recolor.json",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(import_static_color_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("variant identity"));
    }
    for bad in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -f64::MIN_POSITIVE,
        -f64::from_bits(1),
        1.0 + f64::EPSILON,
        -0.1,
        1.1,
    ] {
        let mut v = variant();
        v.linear_rgb[1] = bad;
        assert!(import_static_color_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("finite linear-light"));
    }
    let mut v = variant();
    v.material = u32::MAX;
    assert!(import_static_color_variant(unopened(), v)
        .err()
        .unwrap()
        .contains("material limit"));
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(import_static_color_variant(r, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut r = unopened();
    r.kind = ModelKind::Animated;
    assert!(import_static_color_variant(r, variant())
        .err()
        .unwrap()
        .contains("Static model mode"));
    for format in [SourceFormat::RawGlb, SourceFormat::CookedAnimated] {
        let mut r = unopened();
        r.source_format = format;
        assert!(import_static_color_variant(r, variant())
            .err()
            .unwrap()
            .contains("CookedStatic"));
    }
    for hash in ["".into(), "bad".into(), "z".repeat(64), "a".repeat(65)] {
        assert!(
            import_static_color_variant_checked(unopened(), variant(), Some(hash))
                .err()
                .unwrap()
                .contains("64 hexadecimal")
        );
    }
}

#[test]
fn static_color_unused_or_absent_material_never_publishes() {
    for index in [2, 3] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root, &fixture().to_bytes().unwrap());
        let destination = req.destination.clone();
        let mut v = variant();
        v.material = index;
        let error = import_static_color_variant(req, v).err().unwrap();
        assert!(error.contains("must exist and be used"), "{error}");
        assert!(!destination.exists());
        assert_eq!(fs::read_dir(root).unwrap().count(), 2);
    }
}

#[test]
fn static_color_package_preserves_original_source_license_and_exact_cooked_payload() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let bytes = fixture().to_bytes().unwrap();
    let req = request(&root, &bytes);
    let source = req.source.clone();
    let license = req.license.clone();
    let report =
        import_static_color_variant_checked(req, variant(), Some(digest(&bytes).to_uppercase()))
            .unwrap();
    assert_eq!(report.asset, "model.orrmodel");
    assert_eq!(report.kind, ModelKind::Static);
    let payload = fs::read(report.root.join(&report.asset)).unwrap();
    assert_eq!(
        payload,
        static_color_variant_bytes(&bytes, &variant()).unwrap()
    );
    assert_eq!(report.source_sha256, digest(&payload));
    assert_eq!(fs::read(source).unwrap(), bytes);
    assert_eq!(
        fs::read(report.root.join("LICENSE.txt")).unwrap(),
        fs::read(&license).unwrap()
    );
    assert_eq!(report.license_sha256, digest(&fs::read(license).unwrap()));
    let package =
        orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime()).unwrap();
    assert_eq!(
        package.manifest.files,
        BTreeSet::from(["LICENSE.txt".into(), "model.orrmodel".into()])
    );
    assert_eq!(
        package.manifest.capabilities,
        BTreeSet::from(["models".into()])
    );
}

#[test]
fn static_color_stale_hash_precedes_license_payload_and_source_producer() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let bytes = fixture().to_bytes().unwrap();
    let req = request(&root, &bytes);
    let destination = req.destination.clone();
    fs::write(&req.source, b"changed cooked bytes").unwrap();
    fs::write(&req.license, [0xff]).unwrap();
    let error = import_static_color_variant_checked(req, variant(), Some(digest(&bytes)))
        .err()
        .unwrap();
    assert!(error.contains("inspected static source changed"), "{error}");
    assert!(!destination.exists());
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
}

#[test]
fn static_color_invalid_source_license_and_existing_destination_never_publish_or_overwrite() {
    for case in 0..3 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root, &fixture().to_bytes().unwrap());
        let destination = req.destination.clone();
        match case {
            0 => fs::write(&req.source, b"not cooked").unwrap(),
            1 => fs::write(&req.license, [0xff]).unwrap(),
            _ => {
                fs::create_dir(&destination).unwrap();
                fs::write(destination.join("keep"), b"untouched").unwrap();
            }
        }
        assert!(import_static_color_variant(req, variant()).is_err());
        if case == 2 {
            assert_eq!(fs::read(destination.join("keep")).unwrap(), b"untouched");
        } else {
            assert!(!destination.exists());
            assert_eq!(fs::read_dir(root).unwrap().count(), 2);
        }
    }
}

#[cfg(unix)]
#[test]
fn static_color_linked_source_and_license_reject_without_publication() {
    for source_link in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut req = request(&root, &fixture().to_bytes().unwrap());
        let link = root.join(if source_link {
            "link.model.json"
        } else {
            "link.txt"
        });
        std::os::unix::fs::symlink(
            if source_link {
                &req.source
            } else {
                &req.license
            },
            &link,
        )
        .unwrap();
        if source_link {
            req.source = link;
        } else {
            req.license = link;
        }
        let destination = req.destination.clone();
        assert!(import_static_color_variant(req, variant()).is_err());
        assert!(!destination.exists());
    }
}
