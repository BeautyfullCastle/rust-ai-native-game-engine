#![allow(clippy::float_arithmetic)]
use super::*;
use orr_model::{Dependency, Image, StaticModel};

fn variant() -> StaticPivotVariant {
    StaticPivotVariant {
        pivot: [1.25, -2.5, 0.75],
        asset_id: "variants/prop.pivot.json".into(),
    }
}

fn unopened() -> Request {
    Request {
        source: "missing.model.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "pivot-prop".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Static,
        source_format: SourceFormat::CookedStatic,
        license_acknowledged: true,
    }
}

fn fixture() -> StaticModel {
    let model = orr_model::import::import_with_resolver(
        "props/original.glb",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded fixture"),
    )
    .unwrap();
    let mut source = model.source().clone();
    source.primitives[0].transform = [
        [-2.0, 0.0, 0.0, 0.0],
        [0.0, 3.0, 0.0, 0.0],
        [0.0, 0.0, 0.5, 0.0],
        [4.0, -5.0, -0.0, 1.0],
    ];
    source.primitives[1].transform[3] = [-3.0, 7.0, 2.0, 1.0];
    source.primitives[0].vertices[0].uv = [-0.0, 0.0];
    source.primitives[0].vertices[0].normal[0] = -0.0;
    source.materials.push(source.materials[0].clone());
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![19, 37, 71, 103],
    });
    source.dependencies.push(Dependency {
        uri: "history/missing.png".into(),
        sha256: digest(b"never resolved"),
    });
    StaticModel::new(source).unwrap()
}

fn request(root: &Path, source: &[u8]) -> Request {
    let source_path = root.join("source.model.json");
    let license = root.join("LICENSE.txt");
    fs::write(&source_path, source).unwrap();
    fs::write(&license, b"Redistribution license\n").unwrap();
    Request {
        source: source_path,
        license,
        destination: root.join("new-package"),
        ..unopened()
    }
}

fn recipe(pivot: [f64; 3]) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"format":"orr_static_model_pivot","version":1,"source":"input.model.json","pivot":pivot.map(|v|v as f32)})).unwrap()
}

#[test]
fn pivot_invalid_identity_numeric_license_mode_and_hash_admission_precedes_paths() {
    for id in [
        "",
        "../bad.pivot.json",
        "CON.pivot.json",
        "bad.uv.json",
        "x//bad.pivot.json",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(import_static_pivot_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("variant identity"));
    }
    for bad in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        1000000.1,
        -1000000.1,
    ] {
        let mut v = variant();
        v.pivot[2] = bad;
        assert!(import_static_pivot_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("finite f32"));
    }
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(import_static_pivot_variant(r, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut r = unopened();
    r.kind = ModelKind::Animated;
    assert!(import_static_pivot_variant(r, variant())
        .err()
        .unwrap()
        .contains("Static model mode"));
    for format in [SourceFormat::RawGlb, SourceFormat::CookedAnimated] {
        let mut r = unopened();
        r.source_format = format;
        assert!(import_static_pivot_variant(r, variant())
            .err()
            .unwrap()
            .contains("CookedStatic"));
    }
    for hash in ["".into(), "bad".into(), "z".repeat(64), "a".repeat(65)] {
        assert!(
            import_static_pivot_variant_checked(unopened(), variant(), Some(hash))
                .err()
                .unwrap()
                .contains("64 hexadecimal")
        );
    }
}

#[test]
fn pivot_rebases_only_world_translation_and_captures_exact_recipe_and_source() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    let v = variant();
    let bytes = static_pivot_variant_bytes(&source, &v).unwrap();
    assert_eq!(bytes, static_pivot_variant_bytes(&source, &v).unwrap());
    let out = StaticModel::from_bytes(&bytes).unwrap();
    let mut expected = original.source().clone();
    expected.asset_id = v.asset_id.clone();
    for primitive in &mut expected.primitives {
        primitive.id = format!(
            "{}{}",
            v.asset_id,
            primitive
                .id
                .strip_prefix(&original.source().asset_id)
                .unwrap()
        );
        // Independent hand oracle: T(-[1.25,-2.5,0.75]) * M. Linear entries,
        // reflected/nonuniform scales, vertex bytes and local UVs stay exact.
        primitive.transform[3][0] -= 1.25;
        primitive.transform[3][1] += 2.5;
        primitive.transform[3][2] -= 0.75;
    }
    expected.dependencies = vec![
        Dependency {
            uri: "$source".into(),
            sha256: digest(br#"{"format":"orr_static_model_pivot","version":1,"source":"input.model.json","pivot":[1.25,-2.5,0.75]}"#),
        },
        Dependency {
            uri: "input.model.json".into(),
            sha256: digest(&source),
        },
    ];
    assert_eq!(serde_json::to_vec(&expected).unwrap(), bytes);
    assert_eq!(
        out.source().primitives[0].transform[3],
        [2.75, -2.5, -0.75, 1.0]
    );
    assert_eq!(
        out.source().primitives[1].transform[3],
        [-4.25, 9.5, 1.25, 1.0]
    );
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn pivot_zero_and_f32_underflow_preserve_all_transform_bits() {
    let source = fixture().to_bytes().unwrap();
    let original = StaticModel::from_bytes(&source).unwrap();
    for pivot in [
        [0.0, -0.0, 0.0],
        [f64::MIN_POSITIVE, -f64::MIN_POSITIVE, 0.0],
    ] {
        let mut v = variant();
        v.pivot = pivot;
        let out =
            StaticModel::from_bytes(&static_pivot_variant_bytes(&source, &v).unwrap()).unwrap();
        for (old, new) in original
            .source()
            .primitives
            .iter()
            .zip(&out.source().primitives)
        {
            assert_eq!(
                old.transform.map(|column| column.map(f32::to_bits)),
                new.transform.map(|column| column.map(f32::to_bits))
            );
        }
    }
}

#[test]
fn pivot_bound_matches_existing_f32_recipe_rounding_and_rejects_result_overflow() {
    let mut v = variant();
    v.pivot = [1000000.01, -1000000.01, f64::MIN_POSITIVE];
    validate_static_pivot_variant(&v).unwrap();
    let mut source = fixture().source().clone();
    for p in &mut source.primitives {
        p.transform[3] = [0.0, 0.0, -0.0, 1.0];
    }
    let bytes = StaticModel::new(source.clone())
        .unwrap()
        .to_bytes()
        .unwrap();
    let out = StaticModel::from_bytes(&static_pivot_variant_bytes(&bytes, &v).unwrap()).unwrap();
    assert_eq!(
        out.source().primitives[0].transform[3].map(f32::to_bits),
        [-1000000.0f32, 1000000.0, -0.0, 1.0].map(f32::to_bits)
    );
    assert_eq!(
        out.source().dependencies[0].sha256,
        digest(&recipe(v.pivot))
    );
    // Input pivot is admitted but its resulting translation would exceed bounds.
    source.primitives[0].transform[3][0] = -1.0;
    let bytes = StaticModel::new(source).unwrap().to_bytes().unwrap();
    assert!(static_pivot_variant_bytes(&bytes, &v)
        .unwrap_err()
        .contains("transform bounds"));
}

#[test]
fn pivot_package_is_immutable_cooked_static_with_original_license_and_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source_bytes = fixture().to_bytes().unwrap();
    let req = request(&root, &source_bytes);
    let source = req.source.clone();
    let license = req.license.clone();
    let report = import_static_pivot_variant_checked(
        req,
        variant(),
        Some(digest(&source_bytes).to_uppercase()),
    )
    .unwrap();
    assert_eq!(report.asset, "model.orrmodel");
    assert_eq!(report.kind, ModelKind::Static);
    let bytes = fs::read(report.root.join(&report.asset)).unwrap();
    assert_eq!(report.source_sha256, digest(&bytes));
    assert_eq!(
        bytes,
        static_pivot_variant_bytes(&source_bytes, &variant()).unwrap()
    );
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(
        fs::read(report.root.join("LICENSE.txt")).unwrap(),
        fs::read(&license).unwrap()
    );
    assert_eq!(report.license_sha256, digest(&fs::read(&license).unwrap()));
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
fn pivot_stale_source_rejects_before_license_payload_and_publication() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let bytes = fixture().to_bytes().unwrap();
    let req = request(&root, &bytes);
    let destination = req.destination.clone();
    fs::write(&req.source, b"changed cooked payload").unwrap();
    fs::write(&req.license, [0xff]).unwrap();
    let error = import_static_pivot_variant_checked(req, variant(), Some(digest(&bytes)))
        .err()
        .unwrap();
    assert!(error.contains("inspected static source changed"), "{error}");
    assert!(!destination.exists());
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
}

#[test]
fn pivot_invalid_source_license_output_transform_or_existing_destination_publish_nothing() {
    for case in 0..4 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root, &fixture().to_bytes().unwrap());
        let mut v = variant();
        let destination = req.destination.clone();
        match case {
            0 => fs::write(&req.source, b"invalid cooked model").unwrap(),
            1 => fs::write(&req.license, [0xff]).unwrap(),
            2 => v.pivot = [-1000000.0, 0.0, 0.0],
            _ => {
                fs::create_dir(&destination).unwrap();
                fs::write(destination.join("keep"), b"untouched").unwrap();
            }
        }
        assert!(import_static_pivot_variant(req, v).is_err());
        if case == 3 {
            assert_eq!(fs::read(destination.join("keep")).unwrap(), b"untouched");
        } else {
            assert!(!destination.exists());
            assert_eq!(fs::read_dir(root).unwrap().count(), 2);
        }
    }
}

#[cfg(unix)]
#[test]
fn pivot_source_and_license_links_are_rejected_without_publication() {
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
        assert!(import_static_pivot_variant(req, variant()).is_err());
        assert!(!destination.exists());
    }
}
