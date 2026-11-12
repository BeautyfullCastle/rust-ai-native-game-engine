use super::*;
fn variant() -> SamplerVariant {
    SamplerVariant {
        material: 0,
        linear_filter: true,
        wrap_s: orr_model::Wrap::Repeat,
        wrap_t: orr_model::Wrap::Mirror,
        asset_id: "variants/pixel.sampler.json".into(),
    }
}
fn unopened() -> Request {
    Request {
        source: "missing.animated.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "sampler-player".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Animated,
        source_format: SourceFormat::CookedAnimated,
        license_acknowledged: true,
    }
}
#[test]
fn invalid_sampler_identity_and_material_limit_reject_without_io() {
    for id in [
        "../bad.sampler.json",
        "bad.recolor.json",
        "/bad.sampler.json",
        "CON.sampler.json",
        "",
    ] {
        let mut spec = variant();
        spec.asset_id = id.into();
        assert!(validate_sampler_variant(&spec).is_err());
    }
    let mut spec = variant();
    spec.material = u32::MAX;
    assert!(validate_sampler_variant(&spec).is_err());
    validate_sampler_variant(&variant()).unwrap();
}
#[test]
fn sampler_license_mode_and_expected_hash_gate_precedes_paths() {
    let mut request = unopened();
    request.license_acknowledged = false;
    assert!(import_sampler_variant(request, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut request = unopened();
    request.kind = ModelKind::Static;
    assert!(import_sampler_variant(request, variant())
        .err()
        .unwrap()
        .contains("Animated model"));
    let mut request = unopened();
    request.source_format = SourceFormat::RawGlb;
    assert!(import_sampler_variant(request, variant())
        .err()
        .unwrap()
        .contains("CookedAnimated"));
    assert!(
        import_sampler_variant_checked(unopened(), variant(), Some("bad".into()))
            .err()
            .unwrap()
            .contains("64 hexadecimal")
    );
}
#[cfg(not(feature = "room-animated-models"))]
#[test]
fn sampler_feature_off_rejects_before_all_input_and_output_paths() {
    assert!(import_sampler_variant(unopened(), variant())
        .err()
        .unwrap()
        .contains("requires room-animated-models"));
}
#[cfg(feature = "room-animated-models")]
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
#[cfg(feature = "room-animated-models")]
#[test]
fn sampler_changes_only_selected_sampler_and_identity_provenance_preserving_all_poses() {
    use orr_model::animation::AnimatedModel;
    let original = AnimatedModel::from_bytes(SOURCE).unwrap();
    let original_bytes = original.to_bytes().unwrap();
    for (linear, s, t) in [
        (true, orr_model::Wrap::Repeat, orr_model::Wrap::Mirror),
        (false, orr_model::Wrap::Clamp, orr_model::Wrap::Repeat),
        (true, orr_model::Wrap::Mirror, orr_model::Wrap::Clamp),
    ] {
        let mut spec = variant();
        spec.linear_filter = linear;
        spec.wrap_s = s;
        spec.wrap_t = t;
        let bytes = sampler_variant_bytes(SOURCE, &spec).unwrap();
        assert_eq!(sampler_variant_bytes(SOURCE, &spec).unwrap(), bytes);
        let out = AnimatedModel::from_bytes(&bytes).unwrap();
        let mut expected = original.source().clone();
        expected.asset_id = spec.asset_id.clone();
        expected.materials[0].linear_filter = linear;
        expected.materials[0].wrap_s = s;
        expected.materials[0].wrap_t = t;
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
    assert_eq!(original.to_bytes().unwrap(), original_bytes);
}
#[cfg(feature = "room-animated-models")]
#[test]
fn malformed_unused_material_and_empty_clips_fail_without_mutating_source() {
    use orr_model::animation::AnimatedModel;
    assert!(sampler_variant_bytes(b"bad", &variant()).is_err());
    let original = AnimatedModel::from_bytes(SOURCE).unwrap();
    let original_bytes = original.to_bytes().unwrap();
    let mut source = original.source().clone();
    let mut spec = variant();
    spec.material = source.materials.len() as u32;
    source.materials.push(source.materials[0].clone());
    let unused = AnimatedModel::new(source).unwrap().to_bytes().unwrap();
    assert!(sampler_variant_bytes(&unused, &spec).is_err());
    let mut source = original.source().clone();
    source.clips.clear();
    let empty = AnimatedModel::new(source).unwrap().to_bytes().unwrap();
    assert!(sampler_variant_bytes(&empty, &variant()).is_err());
    assert_eq!(original.to_bytes().unwrap(), original_bytes);
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
mod publication {
    use super::*;
    fn request(root: &Path) -> Request {
        let r = Request {
            source: root.join("source.animated.json"),
            license: root.join("LICENSE"),
            destination: root.join("sampler-package"),
            ..unopened()
        };
        fs::write(&r.source, SOURCE).unwrap();
        fs::write(&r.license, b"Sampler source license\n").unwrap();
        r
    }
    #[test]
    fn sampler_publish_matches_readonly_preview_hash_and_preserves_inputs_and_foreign_output() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let r = request(&root);
        let source = r.source.clone();
        let license = r.license.clone();
        let expected = sampler_variant_bytes(SOURCE, &variant()).unwrap();
        let preview = variant_preview::prepare(variant_preview::Request {
            source: source.clone(),
            spec: variant_preview::Spec::Sampler(variant()),
            expected_source_sha256: Some(digest(SOURCE)),
        })
        .unwrap();
        assert_eq!(preview.asset_sha256, digest(&expected));
        assert_eq!(preview.source_sha256, digest(SOURCE));
        assert!(preview.png_sha256.is_none());
        assert_eq!(preview.model.to_bytes().unwrap(), expected);
        let report = import_sampler_variant_checked(r, variant(), Some(digest(SOURCE))).unwrap();
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), expected);
        assert_eq!(report.source_sha256, digest(&expected));
        assert_eq!(report.asset, "model.animated.json");
        assert_eq!(fs::read(&source).unwrap(), SOURCE);
        assert_eq!(fs::read(&license).unwrap(), b"Sampler source license\n");
        assert!(import_sampler_variant(request(&root), variant()).is_err());
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
            import_sampler_variant_checked(r, variant(), Some("a".repeat(64)))
                .err()
                .unwrap()
                .contains("source changed")
        );
        assert!(!output.exists());
        let mut spec = variant();
        spec.material = 255;
        assert!(import_sampler_variant(request(&root), spec).is_err());
        assert!(!output.exists());
        assert_eq!(fs::read(root.join("source.animated.json")).unwrap(), SOURCE);
    }
}
