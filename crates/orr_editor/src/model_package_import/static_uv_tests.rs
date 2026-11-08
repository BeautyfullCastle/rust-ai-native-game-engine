#![allow(clippy::float_arithmetic)]
use super::*;
use orr_model::{Dependency, Image, StaticModel, Wrap};

fn variant() -> StaticUvVariant {
    StaticUvVariant {
        material: 0,
        offset: [0.25, -0.5],
        scale: [2.0, 0.5],
        quarter_turns: 1,
        asset_id: "variants/static-tile.uv.json".into(),
    }
}

fn unopened() -> Request {
    Request {
        source: "missing.model.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "static-uv-prop".into(),
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
        |_| panic!("embedded fixture must not resolve dependencies"),
    )
    .unwrap();
    let mut source = model.source().clone();
    source.materials[0].base_color = [0.125, 0.5, 0.75, 0.25];
    source.materials[0].wrap_s = Wrap::Mirror;
    source.materials[1].base_color = [-0.0, 0.25, 0.75, 1.0];
    source.materials[1].image = source.materials[0].image;
    source.materials.push(source.materials[1].clone()); // An unused slot remains observable.
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![19, 37, 71, 103],
    });
    let uvs = [
        [-1.5, 2.25],
        [0.25, -0.5],
        [1.75, 0.125],
        [-0.0, 0.0],
        [0.5, 1.5],
    ];
    assert_eq!(source.primitives[0].vertices.len(), uvs.len());
    for (vertex, uv) in source.primitives[0].vertices.iter_mut().zip(uvs) {
        vertex.uv = uv;
        vertex.position[2] = -0.0;
        vertex.normal[0] = -0.0;
    }
    for vertex in &mut source.primitives[1].vertices {
        vertex.uv = [-0.0, 2.5];
    }
    let mut shared = source.primitives[0].clone();
    shared.id = format!("{}#node=17/mesh=8/primitive=3", source.asset_id);
    shared.transform[3] = [4.0, -7.0, 11.0, 1.0];
    source.primitives.push(shared);
    source.dependencies.push(Dependency {
        uri: "history/missing.png".into(),
        sha256: digest(b"history is not resolved"),
    });
    StaticModel::new(source).unwrap()
}

#[test]
fn static_uv_invalid_spec_rejects_before_any_filesystem_access() {
    for id in [
        "",
        "../bad.uv.json",
        "/bad.uv.json",
        "bad.recolor.json",
        "CON.uv.json",
        "x//bad.uv.json",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(import_static_uv_variant(unopened(), v)
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
        v.offset[0] = bad;
        assert!(import_static_uv_variant(unopened(), v)
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
        v.scale[1] = bad;
        assert!(import_static_uv_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("positive f32"));
    }
    for material in [orr_model::MAX_MATERIALS as u32, u32::MAX] {
        let mut v = variant();
        v.material = material;
        assert!(import_static_uv_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("admitted material"));
    }
    let mut v = variant();
    v.quarter_turns = 4;
    assert!(import_static_uv_variant(unopened(), v)
        .err()
        .unwrap()
        .contains("quarter turns"));
    let mut v = variant();
    v.offset = [-f64::from(f32::MAX), f64::from(f32::MAX)];
    v.scale = [f64::from(f32::from_bits(1)), f64::from(f32::MAX)];
    validate_static_uv_variant(&v).unwrap();
}

#[test]
fn static_uv_license_mode_format_and_hash_admission_precedes_paths() {
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(import_static_uv_variant(r, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut r = unopened();
    r.kind = ModelKind::Animated;
    assert!(import_static_uv_variant(r, variant())
        .err()
        .unwrap()
        .contains("Static model mode"));
    for format in [SourceFormat::RawGlb, SourceFormat::CookedAnimated] {
        let mut r = unopened();
        r.source_format = format;
        assert!(import_static_uv_variant(r, variant())
            .err()
            .unwrap()
            .contains("CookedStatic"));
    }
    for hash in ["".into(), "bad".into(), "z".repeat(64), "a".repeat(65)] {
        assert!(
            import_static_uv_variant_checked(unopened(), variant(), Some(hash))
                .err()
                .unwrap()
                .contains("64 hexadecimal")
        );
    }
}

#[test]
fn static_uv_all_rotations_match_oracle_and_preserve_all_other_model_fields() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    for turns in 0..4 {
        let mut v = variant();
        v.quarter_turns = turns;
        let bytes = static_uv_variant_bytes(&source, &v).unwrap();
        assert_eq!(static_uv_variant_bytes(&source, &v).unwrap(), bytes);
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
            if primitive.material == 0 {
                for vertex in &mut primitive.vertices {
                    let [u, w] = vertex.uv;
                    vertex.uv = match turns {
                        0 => [u * 2.0 + 0.25, w * 0.5 - 0.5],
                        1 => [0.25 - w * 0.5, u * 2.0 - 0.5],
                        2 => [0.25 - u * 2.0, -0.5 - w * 0.5],
                        3 => [w * 0.5 + 0.25, -0.5 - u * 2.0],
                        _ => unreachable!(),
                    };
                }
            }
        }
        assert_eq!(out.source().dependencies.len(), 2);
        assert_eq!(out.source().dependencies[0].uri, "$source");
        assert_eq!(out.source().dependencies[1].uri, "input.model.json");
        assert_eq!(out.source().dependencies[1].sha256, digest(&source));
        expected.dependencies = out.source().dependencies.clone();
        // Serialization compares signed-zero bits as well as geometry, material/image tables,
        // unselected UVs, transforms, indices, identities and the complete inventory.
        assert_eq!(
            serde_json::to_vec(&expected).unwrap(),
            out.to_bytes().unwrap()
        );
    }
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn static_uv_f32_identity_preserves_selected_and_unselected_signed_zero_bits() {
    let original = fixture();
    let bytes = original.to_bytes().unwrap();
    for (scale, offset) in [
        ([1.0, 1.0], [-0.0, 0.0]),
        (
            [1.0 + f64::EPSILON, 1.0],
            [f64::MIN_POSITIVE, -f64::MIN_POSITIVE],
        ),
    ] {
        let mut v = variant();
        v.scale = scale;
        v.offset = offset;
        v.quarter_turns = 0;
        let out = StaticModel::from_bytes(&static_uv_variant_bytes(&bytes, &v).unwrap()).unwrap();
        for (a, b) in original
            .source()
            .primitives
            .iter()
            .zip(&out.source().primitives)
        {
            for (x, y) in a.vertices.iter().zip(&b.vertices) {
                assert_eq!(x.uv.map(f32::to_bits), y.uv.map(f32::to_bits));
            }
        }
    }
}

#[test]
fn static_uv_malformed_unused_missing_material_and_output_overflow_reject() {
    let original = fixture();
    let bytes = original.to_bytes().unwrap();
    assert!(static_uv_variant_bytes(b"bad", &variant()).is_err());
    for material in [2, 3] {
        // Unused existing slot, then missing slot.
        let mut v = variant();
        v.material = material;
        assert!(static_uv_variant_bytes(&bytes, &v)
            .unwrap_err()
            .contains("exist and be used"));
    }
    let mut v = variant();
    v.offset = [65537.0, 0.0];
    assert!(static_uv_variant_bytes(&bytes, &v)
        .unwrap_err()
        .contains("65536"));
    let mut v = variant();
    v.scale = [f64::from(f32::MAX), 1.0];
    assert!(static_uv_variant_bytes(&bytes, &v)
        .unwrap_err()
        .contains("nonfinite intermediate"));
    assert_eq!(original.to_bytes().unwrap(), bytes);
}

#[cfg(target_os = "linux")]
mod publication {
    use super::*;
    fn request(root: &Path, bytes: &[u8]) -> Request {
        let r = Request {
            source: root.join("source.MODEL.JSON"),
            license: root.join("LICENSE"),
            destination: root.join("static-uv-package"),
            ..unopened()
        };
        fs::write(&r.source, bytes).unwrap();
        fs::write(&r.license, b"Static UV source license\n").unwrap();
        r
    }

    #[test]
    fn static_uv_publication_binds_source_hash_and_preserves_inputs_and_collision() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = fixture().to_bytes().unwrap();
        let r = request(&root, &source);
        let source_path = r.source.clone();
        let license_path = r.license.clone();
        let catalog = source_inspection::inspect_static_source(&source_path).unwrap();
        let expected = static_uv_variant_bytes(&source, &variant()).unwrap();
        let report = import_static_uv_variant_checked(
            r,
            variant(),
            Some(catalog.source_sha256.to_ascii_uppercase()),
        )
        .unwrap();
        assert_eq!(report.asset, "model.orrmodel");
        assert_eq!(report.kind, ModelKind::Static);
        assert_eq!(report.source_sha256, digest(&expected));
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), expected);
        assert_eq!(fs::read(&source_path).unwrap(), source);
        assert_eq!(
            fs::read(&license_path).unwrap(),
            b"Static UV source license\n"
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
        assert!(import_static_uv_variant(request(&root, &source), variant()).is_err());
        assert_eq!(fs::read(report.root.join("foreign")).unwrap(), b"keep");
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), expected);
    }

    #[test]
    fn static_uv_stale_inspection_and_invalid_slot_never_publish() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = fixture().to_bytes().unwrap();
        let r = request(&root, &source);
        let output = r.destination.clone();
        let catalog = source_inspection::inspect_static_source(&r.source).unwrap();
        let mut changed = fixture().source().clone();
        changed.materials[0].base_color[0] = 0.33;
        fs::write(
            &r.source,
            StaticModel::new(changed).unwrap().to_bytes().unwrap(),
        )
        .unwrap();
        assert!(
            import_static_uv_variant_checked(r, variant(), Some(catalog.source_sha256))
                .err()
                .unwrap()
                .contains("inspected static source changed")
        );
        assert!(!output.exists());
        let mut v = variant();
        v.material = 2;
        assert!(import_static_uv_variant(request(&root, &source), v).is_err());
        assert!(!output.exists());
        assert_eq!(fs::read(root.join("source.MODEL.JSON")).unwrap(), source);
    }
}
