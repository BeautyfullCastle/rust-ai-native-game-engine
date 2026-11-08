#![allow(clippy::float_arithmetic)]
use super::*;
use orr_model::{Dependency, Image, StaticModel, Wrap};
fn variant() -> StaticPlanarUvVariant {
    StaticPlanarUvVariant {
        material: 1,
        plane: "xz".into(),
        origin: [0.25, -0.5, 1.0],
        units_per_repeat: [2.0, 4.0],
        asset_id: "variants/prop.planar-uv.json".into(),
    }
}
fn unopened() -> Request {
    Request {
        source: "missing.model.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "planar-prop".into(),
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
    assert_eq!(source.materials.len(), 2);
    assert!(source.primitives.iter().any(|p| p.material == 1));
    source.materials[0].base_color = [-0.0, 0.25, 0.75, 1.0];
    source.materials[1].base_color = [0.125, 0.5, 0.75, 0.25];
    source.materials[1].image = source.materials[0].image;
    source.materials[0].linear_filter = true;
    source.materials[0].wrap_s = Wrap::Mirror;
    source.materials[0].wrap_t = Wrap::Clamp;
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
        sha256: digest(b"not resolved"),
    });
    source.primitives[1].transform = [
        [-2.0, 0.0, 0.0, 0.0],
        [0.0, 3.0, 0.0, 0.0],
        [0.0, 0.0, 4.0, 0.0],
        [10.0, -20.0, 30.0, 1.0],
    ];
    let mut extra = source.primitives[1].clone();
    extra.id = "props/original.glb#node=1/mesh=999/primitive=0".into();
    extra.transform = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 2.0, 0.0, 0.0],
        [0.0, 0.0, -1.0, 0.0],
        [4.0, 8.0, -12.0, 1.0],
    ];
    source.primitives.push(extra);
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

#[test]
fn planar_all_planes_project_used_material_from_each_primitive_transform_and_preserve_all_else() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    for plane in ["xy", "xz", "yz"] {
        let mut v = variant();
        v.plane = plane.into();
        let bytes = static_planar_uv_variant_bytes(&source, &v).unwrap();
        assert_eq!(bytes, static_planar_uv_variant_bytes(&source, &v).unwrap());
        let mut expected = original.source().clone();
        expected.asset_id = v.asset_id.clone();
        for (index, p) in expected.primitives.iter_mut().enumerate() {
            p.id = format!(
                "{}{}",
                v.asset_id,
                p.id.strip_prefix(&original.source().asset_id).unwrap()
            );
            if p.material == 1 {
                for vertex in &mut p.vertices {
                    let [x, y, z] = vertex.position.map(f64::from);
                    // Independent hand affine oracle for the fixture's two selected
                    // mirrored/nonuniformly scaled and translated primitives.
                    let world = if index == 1 {
                        [-2.0 * x + 10.0, 3.0 * y - 20.0, 4.0 * z + 30.0]
                    } else {
                        [x + 4.0, 2.0 * y + 8.0, -z - 12.0]
                    };
                    vertex.uv = match plane {
                        "xy" => [
                            ((world[0] - 0.25) / 2.0) as f32,
                            ((world[1] + 0.5) / 4.0) as f32,
                        ],
                        "xz" => [
                            ((world[0] - 0.25) / 2.0) as f32,
                            ((world[2] - 1.0) / 4.0) as f32,
                        ],
                        "yz" => [
                            ((world[1] + 0.5) / 2.0) as f32,
                            ((world[2] - 1.0) / 4.0) as f32,
                        ],
                        _ => unreachable!(),
                    };
                }
            }
        }
        let recipe = format!(
            r#"{{"format":"orr_static_planar_uv","version":1,"source":"input.model.json","material":1,"origin":[0.25,-0.5,1.0],"units_per_repeat":[2.0,4.0],"plane":"{plane}"}}"#
        );
        expected.dependencies = vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(recipe.as_bytes()),
            },
            Dependency {
                uri: "input.model.json".into(),
                sha256: digest(&source),
            },
        ];
        // Full serialized bytes verify unchanged normals, positions, matrices,
        // unselected UV sign bits, indices, material/image tables and unused slots.
        assert_eq!(serde_json::to_vec(&expected).unwrap(), bytes);
        let out = StaticModel::from_bytes(&bytes).unwrap();
        assert_eq!(
            out.source().materials[0].image,
            out.source().materials[1].image
        );
    }
    assert_eq!(original.to_bytes().unwrap(), source);
}
#[test]
fn planar_original_f64_recipe_values_retain_provenance_while_producer_decodes_f32() {
    let source = fixture().to_bytes().unwrap();
    let mut v = variant();
    v.origin = [0.1, 0.2, 0.3];
    v.units_per_repeat = [0.7, 0.9];
    let out =
        StaticModel::from_bytes(&static_planar_uv_variant_bytes(&source, &v).unwrap()).unwrap();
    let recipe=br#"{"format":"orr_static_planar_uv","version":1,"source":"input.model.json","material":1,"origin":[0.1,0.2,0.3],"units_per_repeat":[0.7,0.9],"plane":"xz"}"#;
    assert_eq!(out.source().dependencies[0].sha256, digest(recipe));
    let mut narrowed = v.clone();
    narrowed.origin = v.origin.map(|x| f64::from(x as f32));
    narrowed.units_per_repeat = v.units_per_repeat.map(|x| f64::from(x as f32));
    let other =
        StaticModel::from_bytes(&static_planar_uv_variant_bytes(&source, &narrowed).unwrap())
            .unwrap();
    assert_eq!(
        serde_json::to_vec(&out.source().primitives).unwrap(),
        serde_json::to_vec(&other.source().primitives).unwrap()
    );
    assert_ne!(
        out.source().dependencies[0].sha256,
        other.source().dependencies[0].sha256
    );
}
#[test]
fn planar_spec_identity_plane_index_and_numeric_rejections_precede_any_io() {
    for id in [
        "",
        "../bad.planar-uv.json",
        "CON.planar-uv.json",
        "bad.uv.json",
        "x//bad.planar-uv.json",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(import_static_planar_uv_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("variant identity"));
    }
    for plane in ["", "XY", "zy", "x", " xy", "xy "] {
        let mut v = variant();
        v.plane = plane.into();
        assert!(import_static_planar_uv_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("plane"));
    }
    let mut v = variant();
    v.material = u32::MAX;
    assert!(import_static_planar_uv_variant(unopened(), v)
        .err()
        .unwrap()
        .contains("material limit"));
    for bad in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::from(f32::MAX) * 2.0,
    ] {
        let mut v = variant();
        v.origin[0] = bad;
        assert!(import_static_planar_uv_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("origin"));
    }
    for bad in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        -1.0,
        -0.0,
        0.0,
        f64::MIN_POSITIVE,
    ] {
        let mut v = variant();
        v.units_per_repeat[1] = bad;
        assert!(import_static_planar_uv_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("units_per_repeat"));
    }
    let mut v = variant();
    v.origin = [
        -f64::from(f32::MAX),
        f64::MIN_POSITIVE,
        f64::from(f32::MAX) * (1.0 + f64::EPSILON),
    ];
    v.units_per_repeat = [f64::from(f32::from_bits(1)), f64::from(f32::MAX)];
    validate_static_planar_uv_variant(&v).unwrap();
}
#[test]
fn planar_license_mode_format_and_hash_admission_precede_paths() {
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(import_static_planar_uv_variant(r, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut r = unopened();
    r.kind = ModelKind::Animated;
    assert!(import_static_planar_uv_variant(r, variant())
        .err()
        .unwrap()
        .contains("Static model mode"));
    for format in [SourceFormat::RawGlb, SourceFormat::CookedAnimated] {
        let mut r = unopened();
        r.source_format = format;
        assert!(import_static_planar_uv_variant(r, variant())
            .err()
            .unwrap()
            .contains("CookedStatic"));
    }
    for hash in ["".into(), "bad".into(), "z".repeat(64), "a".repeat(65)] {
        assert!(
            import_static_planar_uv_variant_checked(unopened(), variant(), Some(hash))
                .err()
                .unwrap()
                .contains("64 hexadecimal")
        );
    }
}
#[test]
fn planar_unknown_unused_and_out_of_range_projection_never_publish() {
    for case in 0..3 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root, &fixture().to_bytes().unwrap());
        let destination = req.destination.clone();
        let mut v = variant();
        match case {
            0 => v.material = 2,
            1 => v.material = 3,
            _ => v.units_per_repeat = [1.0e-10, 1.0e-10],
        }
        let error = import_static_planar_uv_variant(req, v).err().unwrap();
        if case < 2 {
            assert!(error.contains("must exist and be used"), "{error}");
        } else {
            assert!(error.contains("projected UV"), "{error}");
        }
        assert!(!destination.exists());
        assert_eq!(fs::read_dir(root).unwrap().count(), 2);
    }
}
#[test]
fn planar_package_is_cooked_static_with_immutable_source_license_and_exact_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let bytes = fixture().to_bytes().unwrap();
    let req = request(&root, &bytes);
    let source = req.source.clone();
    let license = req.license.clone();
    let report = import_static_planar_uv_variant_checked(
        req,
        variant(),
        Some(digest(&bytes).to_uppercase()),
    )
    .unwrap();
    assert_eq!(report.kind, ModelKind::Static);
    assert_eq!(report.asset, "model.orrmodel");
    let output = fs::read(report.root.join(&report.asset)).unwrap();
    assert_eq!(
        output,
        static_planar_uv_variant_bytes(&bytes, &variant()).unwrap()
    );
    assert_eq!(report.source_sha256, digest(&output));
    assert_eq!(fs::read(source).unwrap(), bytes);
    assert_eq!(
        fs::read(report.root.join("LICENSE.txt")).unwrap(),
        fs::read(&license).unwrap()
    );
    assert_eq!(report.license_sha256, digest(&fs::read(license).unwrap()));
    let package =
        orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime()).unwrap();
    assert_eq!(
        package.manifest.capabilities,
        BTreeSet::from(["models".into()])
    );
    assert_eq!(
        package.manifest.files,
        BTreeSet::from(["LICENSE.txt".into(), "model.orrmodel".into()])
    );
}
#[test]
fn planar_stale_source_hash_precedes_license_payload_and_publication() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let bytes = fixture().to_bytes().unwrap();
    let req = request(&root, &bytes);
    let destination = req.destination.clone();
    fs::write(&req.source, b"changed cooked bytes").unwrap();
    fs::write(&req.license, [0xff]).unwrap();
    let error = import_static_planar_uv_variant_checked(req, variant(), Some(digest(&bytes)))
        .err()
        .unwrap();
    assert!(error.contains("inspected static source changed"), "{error}");
    assert!(!destination.exists());
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
}
#[test]
fn planar_bad_source_license_or_existing_destination_never_publish_or_overwrite() {
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
        assert!(import_static_planar_uv_variant(req, variant()).is_err());
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
fn planar_linked_source_and_license_reject_without_publication() {
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
        assert!(import_static_planar_uv_variant(req, variant()).is_err());
        assert!(!destination.exists());
    }
}
