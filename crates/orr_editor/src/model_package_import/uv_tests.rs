use super::*;
fn variant() -> UvVariant {
    UvVariant {
        material: 0,
        offset: [0.25, -0.5],
        scale: [2., 0.5],
        quarter_turns: 1,
        asset_id: "variants/tile.uv.json".into(),
    }
}
fn unopened() -> Request {
    Request {
        source: "missing.animated.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "uv-player".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Animated,
        source_format: SourceFormat::CookedAnimated,
        license_acknowledged: true,
    }
}
#[test]
fn invalid_uv_identity_numeric_ranges_material_and_turns_reject_without_io() {
    for id in [
        "../bad.uv.json",
        "bad.recolor.json",
        "/bad.uv.json",
        "CON.uv.json",
        "",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(validate_uv_variant(&v).is_err());
    }
    for bad in [f64::NAN, f64::INFINITY, f64::from(f32::MAX) * 2.] {
        let mut v = variant();
        v.offset[0] = bad;
        assert!(validate_uv_variant(&v).is_err());
    }
    for bad in [
        f64::NAN,
        f64::INFINITY,
        0.,
        -1.,
        f64::MIN_POSITIVE,
        f64::from(f32::MAX) * 2.,
    ] {
        let mut v = variant();
        v.scale[1] = bad;
        assert!(validate_uv_variant(&v).is_err());
    }
    let mut v = variant();
    v.quarter_turns = 4;
    assert!(validate_uv_variant(&v).is_err());
    let mut v = variant();
    v.material = u32::MAX;
    assert!(validate_uv_variant(&v).is_err());
    validate_uv_variant(&variant()).unwrap();
}
#[test]
fn uv_license_mode_and_expected_hash_gate_precedes_paths() {
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(import_uv_variant(r, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut r = unopened();
    r.kind = ModelKind::Static;
    assert!(import_uv_variant(r, variant())
        .err()
        .unwrap()
        .contains("Animated model"));
    let mut r = unopened();
    r.source_format = SourceFormat::RawGlb;
    assert!(import_uv_variant(r, variant())
        .err()
        .unwrap()
        .contains("CookedAnimated"));
    assert!(
        import_uv_variant_checked(unopened(), variant(), Some("bad".into()))
            .err()
            .unwrap()
            .contains("64 hexadecimal")
    );
}
#[cfg(not(feature = "room-animated-models"))]
#[test]
fn uv_feature_off_rejects_before_all_input_and_output_paths() {
    assert!(import_uv_variant(unopened(), variant())
        .err()
        .unwrap()
        .contains("requires room-animated-models"));
}
#[cfg(feature = "room-animated-models")]
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
#[cfg(feature = "room-animated-models")]
#[test]
fn all_four_rotations_match_uv_oracle_and_preserve_unselected_material_geometry_rig_and_poses() {
    use orr_model::animation::AnimatedModel;
    let original = AnimatedModel::from_bytes(SOURCE).unwrap();
    let before = original.to_bytes().unwrap();
    for turns in 0..4 {
        let mut v = variant();
        v.quarter_turns = turns;
        let bytes = uv_variant_bytes(SOURCE, &v).unwrap();
        assert_eq!(uv_variant_bytes(SOURCE, &v).unwrap(), bytes);
        let out = AnimatedModel::from_bytes(&bytes).unwrap();
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
                    let [u, w] = vertex.vertex.uv;
                    vertex.vertex.uv = match turns {
                        0 => [u * 2. + 0.25, w * 0.5 - 0.5],
                        1 => [0.25 - w * 0.5, u * 2. - 0.5],
                        2 => [0.25 - u * 2., -0.5 - w * 0.5],
                        3 => [w * 0.5 + 0.25, -0.5 - u * 2.],
                        _ => unreachable!(),
                    };
                }
            }
        }
        expected.dependencies = out.source().dependencies.clone();
        assert_eq!(&expected, out.source());
        assert_eq!(out.source().dependencies.len(), 2);
        assert_eq!(out.source().dependencies[0].uri, "$source");
        assert_eq!(out.source().dependencies[1].uri, "input.animated.json");
        assert_eq!(out.source().dependencies[1].sha256, digest(SOURCE));
        for (index, clip) in original.source().clips.iter().enumerate() {
            for time in [0., clip.duration() * 0.37, clip.duration()] {
                let a = original.sample_clip(index as u32, time).unwrap();
                let b = out.sample_clip(index as u32, time).unwrap();
                assert_eq!(a.local(), b.local());
                assert_eq!(a.global(), b.global());
                assert_eq!(a.skin_matrices(), b.skin_matrices());
            }
        }
    }
    assert_eq!(original.to_bytes().unwrap(), before);
}
#[cfg(feature = "room-animated-models")]
#[test]
fn identity_uv_transform_preserves_signed_zero_bits_on_selected_and_unselected_vertices() {
    use orr_model::animation::AnimatedModel;
    let mut source = AnimatedModel::from_bytes(SOURCE).unwrap().source().clone();
    for primitive in &mut source.primitives {
        primitive.vertices[0].vertex.uv = [-0., 0.];
    }
    let original = AnimatedModel::new(source).unwrap();
    let bytes = original.to_bytes().unwrap();
    let mut v = variant();
    v.offset = [-0., 0.];
    v.scale = [1., 1.];
    v.quarter_turns = 0;
    let out = AnimatedModel::from_bytes(&uv_variant_bytes(&bytes, &v).unwrap()).unwrap();
    for (a, b) in original
        .source()
        .primitives
        .iter()
        .zip(&out.source().primitives)
    {
        for (x, y) in a.vertices.iter().zip(&b.vertices) {
            assert_eq!(x.vertex.uv.map(f32::to_bits), y.vertex.uv.map(f32::to_bits));
        }
    }
}
#[cfg(feature = "room-animated-models")]
#[test]
fn malformed_unused_material_empty_clips_and_output_uv_overflow_reject_without_mutating_source() {
    use orr_model::animation::AnimatedModel;
    assert!(uv_variant_bytes(b"bad", &variant()).is_err());
    let original = AnimatedModel::from_bytes(SOURCE).unwrap();
    let before = original.to_bytes().unwrap();
    let mut source = original.source().clone();
    let mut v = variant();
    v.material = source.materials.len() as u32;
    source.materials.push(source.materials[0].clone());
    assert!(
        uv_variant_bytes(&AnimatedModel::new(source).unwrap().to_bytes().unwrap(), &v).is_err()
    );
    let mut source = original.source().clone();
    source.clips.clear();
    assert!(uv_variant_bytes(
        &AnimatedModel::new(source).unwrap().to_bytes().unwrap(),
        &variant()
    )
    .is_err());
    let mut v = variant();
    v.offset = [65537., 0.];
    assert!(uv_variant_bytes(SOURCE, &v).is_err());
    assert_eq!(original.to_bytes().unwrap(), before);
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
mod publication {
    use super::*;
    fn request(root: &Path) -> Request {
        let r = Request {
            source: root.join("source.animated.json"),
            license: root.join("LICENSE"),
            destination: root.join("uv-package"),
            ..unopened()
        };
        fs::write(&r.source, SOURCE).unwrap();
        fs::write(&r.license, b"Uv source license\n").unwrap();
        r
    }
    #[test]
    fn uv_publish_matches_readonly_preview_hash_and_preserves_inputs_and_foreign_output() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let r = request(&root);
        let source = r.source.clone();
        let license = r.license.clone();
        let expected = uv_variant_bytes(SOURCE, &variant()).unwrap();
        let preview = variant_preview::prepare(variant_preview::Request {
            source: source.clone(),
            spec: variant_preview::Spec::Uv(variant()),
            expected_source_sha256: Some(digest(SOURCE)),
        })
        .unwrap();
        assert_eq!(preview.asset_sha256, digest(&expected));
        assert_eq!(preview.source_sha256, digest(SOURCE));
        assert!(preview.png_sha256.is_none());
        assert_eq!(preview.model.to_bytes().unwrap(), expected);
        let report = import_uv_variant_checked(r, variant(), Some(digest(SOURCE))).unwrap();
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), expected);
        assert_eq!(report.source_sha256, digest(&expected));
        assert_eq!(report.asset, "model.animated.json");
        assert_eq!(fs::read(&source).unwrap(), SOURCE);
        assert_eq!(fs::read(&license).unwrap(), b"Uv source license\n");
        assert!(import_uv_variant(request(&root), variant()).is_err());
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), expected);
        orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime()).unwrap();
    }
    #[test]
    fn stale_inspection_and_invalid_used_slot_never_publish_or_change_source() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let r = request(&root);
        let output = r.destination.clone();
        assert!(
            import_uv_variant_checked(r, variant(), Some("a".repeat(64)))
                .err()
                .unwrap()
                .contains("source changed")
        );
        assert!(!output.exists());
        let mut spec = variant();
        spec.material = 255;
        assert!(import_uv_variant(request(&root), spec).is_err());
        assert!(!output.exists());
        assert_eq!(fs::read(root.join("source.animated.json")).unwrap(), SOURCE);
    }
}
