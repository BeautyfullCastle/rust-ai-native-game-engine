#![allow(clippy::float_arithmetic)]
use super::*;
use orr_model::{Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap};

fn identity() -> StaticAssemblyPart {
    StaticAssemblyPart {
        offset: [0.0; 3],
        scale: [1.0; 3],
        quarter_turns: 0,
    }
}
fn variant() -> StaticAssemblyVariant {
    StaticAssemblyVariant {
        parts: [identity(), identity()],
        asset_id: "variants/pair.assemble.json".into(),
    }
}
fn unopened() -> Request {
    Request {
        source: "missing.model.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "static-pair".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Static,
        source_format: SourceFormat::CookedStatic,
        license_acknowledged: true,
    }
}
fn fixture() -> StaticModel {
    let vertices = vec![
        Vertex {
            position: [-0.0, 0.0, 0.0],
            normal: [0.0, -0.0, 1.0],
            uv: [-0.0, 0.0],
        },
        Vertex {
            position: [1.0, 0.0, -0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, -0.0],
        },
        Vertex {
            position: [0.0, 1.0, 0.0],
            normal: [-0.0, 0.0, 1.0],
            uv: [0.0, 1.0],
        },
    ];
    let transforms = [
        [
            [-2.0, -0.0, 0.0, 0.0],
            [0.0, 3.0, 0.0, -0.0],
            [-0.0, 0.0, 0.5, 0.0],
            [4.0, -7.0, 11.0, 1.0],
        ],
        [
            [0.0, 2.0, 0.0, 0.0],
            [-3.0, 0.5, 0.0, 0.0],
            [0.0, 0.0, 4.0, -0.0],
            [-5.0, 8.0, -9.0, 1.0],
        ],
    ];
    let primitives = transforms
        .into_iter()
        .enumerate()
        .map(|(index, transform)| Primitive {
            id: format!("props/original.glb#node={index}/mesh=8/primitive=0"),
            vertices: vertices.clone(),
            indices: vec![0, 1, 2],
            material: index as u32,
            transform,
        })
        .collect();
    let material = Material {
        base_color: [-0.0, 0.5, 0.75, 0.25],
        image: 0,
        linear_filter: true,
        wrap_s: Wrap::Mirror,
        wrap_t: Wrap::Repeat,
    };
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: "props/original.glb".into(),
        dependencies: vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(b"original source"),
            },
            Dependency {
                uri: "history/missing.png".into(),
                sha256: digest(b"history never resolved"),
            },
        ],
        primitives,
        materials: vec![material.clone(), material.clone(), material],
        images: vec![
            Image {
                width: 1,
                height: 1,
                rgba8: vec![19, 37, 71, 103],
            },
            Image {
                width: 1,
                height: 1,
                rgba8: vec![2, 3, 5, 7],
            },
        ],
    })
    .unwrap()
}

#[test]
fn static_assembly_admits_full_spec_hash_license_and_mode_before_io() {
    for id in [
        "",
        "../bad.assemble.json",
        "/bad.assemble.json",
        "CON.assemble.json",
        "bad.uv.json",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(import_static_assembly_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("variant identity"));
    }
    for bad in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::from(f32::MAX) * (1.0 + f64::EPSILON),
        f64::from(f32::MAX) * 2.0,
    ] {
        let mut v = variant();
        v.parts[1].offset[0] = bad;
        assert!(import_static_assembly_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("finite f32"));
    }
    for bad in [
        f64::NAN,
        f64::INFINITY,
        -1.0,
        -0.0,
        0.0,
        f64::MIN_POSITIVE,
        f64::from(f32::MAX) * (1.0 + f64::EPSILON),
        f64::from(f32::MAX) * 2.0,
    ] {
        let mut v = variant();
        v.parts[0].scale[1] = bad;
        assert!(import_static_assembly_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("positive f32"));
    }
    for bad in [f64::from(f32::from_bits(1)), 1.0e-7, f64::from(f32::MAX)] {
        let mut v = variant();
        v.parts[0].scale = [bad; 3];
        assert!(import_static_assembly_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("placement"));
    }
    let mut v = variant();
    v.parts[1].quarter_turns = 4;
    assert!(import_static_assembly_variant(unopened(), v)
        .err()
        .unwrap()
        .contains("quarter turns"));
    let mut v = variant();
    v.parts[0].offset[2] = 1_000_001.0;
    assert!(import_static_assembly_variant(unopened(), v)
        .err()
        .unwrap()
        .contains("placement"));
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(import_static_assembly_variant(r, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut r = unopened();
    r.kind = ModelKind::Animated;
    assert!(import_static_assembly_variant(r, variant())
        .err()
        .unwrap()
        .contains("Static model mode"));
    for source_format in [SourceFormat::RawGlb, SourceFormat::CookedAnimated] {
        let mut r = unopened();
        r.source_format = source_format;
        assert!(import_static_assembly_variant(r, variant())
            .err()
            .unwrap()
            .contains("CookedStatic"));
    }
    for hash in ["".into(), "bad".into(), "z".repeat(64), "a".repeat(65)] {
        assert!(
            import_static_assembly_variant_checked(unopened(), variant(), Some(hash))
                .err()
                .unwrap()
                .contains("64 hexadecimal")
        );
    }
    validate_static_assembly_variant(&variant()).unwrap();
}

fn world(matrix: [[f32; 4]; 4], point: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| {
        matrix[0][row] * point[0]
            + matrix[1][row] * point[1]
            + matrix[2][row] * point[2]
            + matrix[3][row]
    })
}
fn close(a: [f32; 3], b: [f32; 3]) {
    for (x, y) in a.into_iter().zip(b) {
        assert!((x - y).abs() < 0.00001, "{a:?} != {b:?}");
    }
}

#[test]
fn static_assembly_all_y_yaws_match_literal_matrices_normals_world_points_and_bounds() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    let transforms = [
        [
            [-4., 0., 0., 0.],
            [0., 9., 0., 0.],
            [0., 0., 2., 0.],
            [18., -1., 74., 1.],
        ],
        [
            [0., 0., 4., 0.],
            [0., 9., 0., 0.],
            [2., 0., 0., 0.],
            [54., -1., 22., 1.],
        ],
        [
            [4., 0., 0., 0.],
            [0., 9., 0., 0.],
            [0., 0., -2., 0.],
            [2., -1., -14., 1.],
        ],
        [
            [0., 0., -4., 0.],
            [0., 9., 0., 0.],
            [-2., 0., 0., 0.],
            [-34., -1., 38., 1.],
        ],
    ];
    let normal_columns = [
        [[-0.25, 0., 0.], [0., 1. / 9., 0.], [0., 0., 0.5]],
        [[0., 0., 0.25], [0., 1. / 9., 0.], [0.5, 0., 0.]],
        [[0.25, 0., 0.], [0., 1. / 9., 0.], [0., 0., -0.5]],
        [[0., 0., -0.25], [0., 1. / 9., 0.], [-0.5, 0., 0.]],
    ];
    for turns in 0..4 {
        let mut v = variant();
        v.parts[0] = StaticAssemblyPart {
            offset: [10., 20., 30.],
            scale: [2., 3., 4.],
            quarter_turns: turns,
        };
        let bytes = static_assembly_variant_bytes(&source, &v).unwrap();
        assert_eq!(static_assembly_variant_bytes(&source, &v).unwrap(), bytes);
        let out = StaticModel::from_bytes(&bytes).unwrap();
        assert_eq!(
            out.source().primitives[0].transform,
            transforms[turns as usize]
        );
        let normal = orr_model::normal_matrix(out.source().primitives[0].transform).unwrap();
        for (column, expected) in normal.iter().take(3).zip(normal_columns[turns as usize]) {
            close([column[0], column[1], column[2]], expected);
        }
        let mut expected_min = [f32::INFINITY; 3];
        let mut expected_max = [f32::NEG_INFINITY; 3];
        let mut actual_min = [f32::INFINITY; 3];
        let mut actual_max = [f32::NEG_INFINITY; 3];
        for (before, after) in original
            .source()
            .primitives
            .iter()
            .zip(&out.source().primitives)
        {
            for vertex in &before.vertices {
                let p = world(before.transform, vertex.position);
                let [x, y, z] = [p[0] * 2., p[1] * 3., p[2] * 4.];
                let rotated = match turns {
                    0 => [x, y, z],
                    1 => [z, y, -x],
                    2 => [-x, y, -z],
                    3 => [-z, y, x],
                    _ => unreachable!(),
                };
                let expected = [rotated[0] + 10., rotated[1] + 20., rotated[2] + 30.];
                let actual = world(after.transform, vertex.position);
                close(actual, expected);
                for axis in 0..3 {
                    expected_min[axis] = expected_min[axis].min(expected[axis]);
                    expected_max[axis] = expected_max[axis].max(expected[axis]);
                    actual_min[axis] = actual_min[axis].min(actual[axis]);
                    actual_max[axis] = actual_max[axis].max(actual[axis]);
                }
            }
        }
        close(actual_min, expected_min);
        close(actual_max, expected_max);
    }
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn static_assembly_identity_keeps_bits_and_deduplicates_full_material_image_tables_and_source() {
    let mut rich = fixture().source().clone();
    rich.materials
        .resize(orr_model::MAX_MATERIALS, rich.materials[0].clone());
    rich.images
        .resize(orr_model::MAX_IMAGES, rich.images[0].clone());
    let original = StaticModel::new(rich).unwrap();
    let source = original.to_bytes().unwrap();
    let mut v = variant();
    v.parts[0].offset = [-0.0, 0.0, -0.0];
    v.parts[1].scale = [1.0 + f64::EPSILON, 1.0, 1.0];
    v.parts[1].offset = [f64::MIN_POSITIVE, -f64::MIN_POSITIVE, 0.0];
    let bytes = static_assembly_variant_bytes(&source, &v).unwrap();
    let out = StaticModel::from_bytes(&bytes).unwrap();
    let mut expected = original.source().clone();
    expected.asset_id = v.asset_id.clone();
    expected.primitives.clear();
    for key in ["first", "second"] {
        for primitive in &original.source().primitives {
            let mut copy = primitive.clone();
            copy.id = format!(
                "{}#node=part:{key}{}",
                v.asset_id,
                primitive
                    .id
                    .strip_prefix(&original.source().asset_id)
                    .unwrap()
            );
            expected.primitives.push(copy);
        }
    }
    let mut first = orr_model::IDENTITY;
    first[3] = [-0.0, 0.0, -0.0, 1.0];
    let mut second = orr_model::IDENTITY;
    second[3] = [0.0, -0.0, 0.0, 1.0];
    let recipe = serde_json::to_vec(
        &serde_json::json!({"format":"orr_static_model_assemble","version":1,
        "parts":[{"key":"first","source":"input.model.json","placement":first},
            {"key":"second","source":"input.model.json","placement":second}]}),
    )
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
    ];
    assert_eq!(serde_json::to_vec(&expected).unwrap(), bytes);
    assert_eq!(out.source().materials.len(), orr_model::MAX_MATERIALS);
    assert_eq!(out.source().images.len(), orr_model::MAX_IMAGES);
    assert_eq!(
        out.source().primitives.len(),
        original.source().primitives.len() * 2
    );
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn static_assembly_malformed_source_and_aggregate_primitive_and_input_limits_reject() {
    assert!(static_assembly_variant_bytes(b"bad model", &variant()).is_err());
    let mut many = fixture().source().clone();
    let primitive = many.primitives[0].clone();
    many.primitives.clear();
    for index in 0..(orr_model::MAX_PRIMITIVES / 2 + 1) {
        let mut p = primitive.clone();
        p.id = format!("{}#node={index}/mesh=0/primitive=0", many.asset_id);
        many.primitives.push(p);
    }
    let source = StaticModel::new(many).unwrap().to_bytes().unwrap();
    assert!(static_assembly_variant_bytes(&source, &variant())
        .unwrap_err()
        .contains("primitive count budget"));
    let mut source = fixture().to_bytes().unwrap();
    source.resize(orr_model::MAX_FILE_BYTES, b' ');
    assert!(static_assembly_variant_bytes(&source, &variant())
        .unwrap_err()
        .contains("unique input byte budget"));
}

#[test]
fn static_assembly_rejects_composed_world_bounds_and_normal_matrix_overflow() {
    let mut source = fixture().source().clone();
    source.primitives.truncate(1);
    source.primitives[0].transform = orr_model::IDENTITY;
    source.primitives[0].transform[0][0] = 1000.;
    source.primitives[0].vertices[1].position = [1_000_000., 0., 0.];
    let source = StaticModel::new(source).unwrap();
    let before = source.to_bytes().unwrap();
    let mut v = variant();
    v.parts[1].scale = [2., 1., 1.];
    assert!(static_assembly_variant_bytes(&before, &v)
        .unwrap_err()
        .contains("world vertex exceeds range"));
    assert_eq!(source.to_bytes().unwrap(), before);
    let mut source = fixture().source().clone();
    source.primitives.truncate(1);
    source.primitives[0].transform = orr_model::IDENTITY;
    source.primitives[0].transform[0][0] = 1_000_000.;
    let source = StaticModel::new(source).unwrap().to_bytes().unwrap();
    assert!(static_assembly_variant_bytes(&source, &v)
        .unwrap_err()
        .contains("composed part second"));
}

#[cfg(target_os = "linux")]
mod publication {
    use super::*;
    fn request(root: &Path) -> (Request, Vec<u8>) {
        let source = fixture().to_bytes().unwrap();
        let r = Request {
            source: root.join("source.MODEL.JSON"),
            license: root.join("LICENSE"),
            destination: root.join("assembly-package"),
            ..unopened()
        };
        fs::write(&r.source, &source).unwrap();
        fs::write(&r.license, b"Single original source license\n").unwrap();
        (r, source)
    }
    #[test]
    fn static_assembly_plain_checked_publication_preserves_inputs_and_never_overwrites() {
        for checked in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let (r, source) = request(&root);
            let paths = (r.source.clone(), r.license.clone());
            let expected = static_assembly_variant_bytes(&source, &variant()).unwrap();
            let report = if checked {
                let catalog = source_inspection::inspect_static_source(&r.source).unwrap();
                import_static_assembly_variant_checked(
                    r,
                    variant(),
                    Some(catalog.source_sha256.to_ascii_uppercase()),
                )
            } else {
                import_static_assembly_variant(r, variant())
            }
            .unwrap();
            assert_eq!(report.asset, "model.orrmodel");
            assert_eq!(report.kind, ModelKind::Static);
            assert_eq!(report.source_sha256, digest(&expected));
            assert_eq!(
                report.license_sha256,
                digest(b"Single original source license\n")
            );
            assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), expected);
            assert_eq!(fs::read(paths.0).unwrap(), source);
            assert_eq!(
                fs::read(paths.1).unwrap(),
                b"Single original source license\n"
            );
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
            let (r, _) = request(&root);
            assert!(import_static_assembly_variant(r, variant()).is_err());
            assert_eq!(fs::read(report.root.join("foreign")).unwrap(), b"keep");
            assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), expected);
        }
    }
    #[test]
    fn static_assembly_stale_hash_and_invalid_source_never_publish() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (r, mut source) = request(&root);
        let output = r.destination.clone();
        let catalog = source_inspection::inspect_static_source(&r.source).unwrap();
        source.push(b' ');
        fs::write(&r.source, &source).unwrap();
        assert!(
            import_static_assembly_variant_checked(r, variant(), Some(catalog.source_sha256))
                .err()
                .unwrap()
                .contains("inspected static source changed")
        );
        assert!(!output.exists());
        assert_eq!(fs::read(root.join("source.MODEL.JSON")).unwrap(), source);
        let (r, _) = request(&root);
        fs::write(&r.source, b"bad model").unwrap();
        assert!(import_static_assembly_variant(r, variant()).is_err());
        assert!(!output.exists());
    }
    #[test]
    fn static_assembly_safe_readers_reject_links_oversize_and_store_destination() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (mut r, _) = request(&root);
        let output = r.destination.clone();
        let link = root.join("linked.model.json");
        std::os::unix::fs::symlink(&r.source, &link).unwrap();
        r.source = link;
        assert!(import_static_assembly_variant(r, variant()).is_err());
        assert!(!output.exists());
        let (mut r, _) = request(&root);
        let link = root.join("linked-license");
        std::os::unix::fs::symlink(&r.license, &link).unwrap();
        r.license = link;
        assert!(import_static_assembly_variant(r, variant()).is_err());
        assert!(!output.exists());
        let (r, _) = request(&root);
        File::create(&r.source)
            .unwrap()
            .set_len(MAX_SOURCE_BYTES + 1)
            .unwrap();
        assert!(import_static_assembly_variant(r, variant())
            .err()
            .unwrap()
            .contains("byte limit"));
        assert!(!output.exists());
        let (mut r, _) = request(&root);
        r.destination = root.join(".orr/package");
        assert!(import_static_assembly_variant(r, variant())
            .err()
            .unwrap()
            .contains(".orr store"));
        assert!(!root.join(".orr").exists());
    }
}
