//! Focused import admission/publication tests. Windows executes no publication.
#![allow(clippy::float_arithmetic)]
use super::*;
fn variant() -> TrimVariant {
    TrimVariant {
        clip: 0,
        start_seconds: 0.25,
        end_seconds: 0.75,
        name: "short-search".into(),
        asset_id: "clips/short.trim.json".into(),
    }
}
fn unopened() -> Request {
    Request {
        source: "missing.animated.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "trim-player".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Animated,
        source_format: SourceFormat::CookedAnimated,
        license_acknowledged: true,
    }
}
#[test]
fn invalid_trim_spec_rejects_names_ranges_and_identity_without_io() {
    for name in ["", "   ", "bad\nname"] {
        let mut v = variant();
        v.name = name.into();
        assert!(validate_trim_variant(&v).is_err());
    }
    let mut v = variant();
    v.name = "a".repeat(129);
    assert!(validate_trim_variant(&v).is_err());
    for (start, end) in [
        (f64::NAN, 1.0),
        (0.0, f64::INFINITY),
        (-0.01, 1.0),
        (1.0, 1.0),
        (2.0, 1.0),
        (0.0, 3600.1),
        (1.0, 1.00000000001),
    ] {
        let mut v = variant();
        v.start_seconds = start;
        v.end_seconds = end;
        assert!(validate_trim_variant(&v).is_err());
    }
    for id in [
        "../bad.trim.json",
        "bad.recolor.json",
        "/bad.trim.json",
        "CON.trim.json",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(validate_trim_variant(&v).is_err());
    }
    validate_trim_variant(&variant()).unwrap();
}
#[test]
fn trim_license_mode_and_expected_hash_gate_precedes_paths() {
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(
        import_trim_variant(r, variant())
            .err()
            .unwrap()
            .contains("acknowledgement")
    );
    let mut r = unopened();
    r.source_format = SourceFormat::RawGlb;
    assert!(
        import_trim_variant(r, variant())
            .err()
            .unwrap()
            .contains("CookedAnimated")
    );
    let mut r = unopened();
    r.kind = ModelKind::Static;
    assert!(
        import_trim_variant(r, variant())
            .err()
            .unwrap()
            .contains("Animated model")
    );
    assert!(
        import_trim_variant_checked(unopened(), variant(), Some("bad".into()))
            .err()
            .unwrap()
            .contains("64 hexadecimal")
    );
}
#[cfg(not(feature = "room-animated-models"))]
#[test]
fn trim_feature_off_rejects_before_all_input_or_output_paths() {
    assert!(
        import_trim_variant(unopened(), variant())
            .err()
            .unwrap()
            .contains("requires room-animated-models")
    );
}
#[cfg(feature = "room-animated-models")]
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
#[cfg(feature = "room-animated-models")]
#[test]
fn trim_appends_clip_preserves_original_indices_and_direct_provenance() {
    use orr_model::animation::AnimatedModel;
    let original = AnimatedModel::from_bytes(SOURCE).unwrap();
    let bytes = trim_variant_bytes(SOURCE, &variant()).unwrap();
    assert_eq!(trim_variant_bytes(SOURCE, &variant()).unwrap(), bytes);
    let out = AnimatedModel::from_bytes(&bytes).unwrap();
    let n = original.source().clips.len();
    assert_eq!(out.source().clips.len(), n + 1);
    let mut expected = original.source().clone();
    expected.asset_id = variant().asset_id;
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
    expected.clips.push(out.source().clips[n].clone());
    assert_eq!(serde_json::to_vec(&expected).unwrap(), bytes);
    assert_eq!(out.source().clips[n].name, "short-search");
    assert_eq!(out.source().clips[n].duration(), 0.5);
    for (target, source) in [(0.0, 0.25), (0.25, 0.5), (0.5, 0.75)] {
        let got = out.sample_clip(n as u32, target).unwrap();
        let expected = original.sample_clip(0, source).unwrap();
        // Re-sampling SLERP boundary keys can introduce roundoff; original
        // clip channels above remain bit-exact, and their old-index samples do too.
        for (a, b) in got.local().iter().zip(expected.local()) {
            for (a, b) in a
                .translation
                .iter()
                .chain(&a.rotation)
                .chain(&a.scale)
                .zip(b.translation.iter().chain(&b.rotation).chain(&b.scale))
            {
                assert!((a - b).abs() < 0.00001);
            }
        }
        for (a, b) in got
            .global()
            .iter()
            .flatten()
            .flatten()
            .zip(expected.global().iter().flatten().flatten())
        {
            assert!((a - b).abs() < 0.00001);
        }
        for (a, b) in got
            .skin_matrices()
            .iter()
            .flatten()
            .flatten()
            .flatten()
            .zip(
                expected
                    .skin_matrices()
                    .iter()
                    .flatten()
                    .flatten()
                    .flatten(),
            )
        {
            assert!((a - b).abs() < 0.00001);
        }
        let old_index = out.sample_clip(0, source).unwrap();
        assert_eq!(old_index.local(), expected.local());
        assert_eq!(old_index.global(), expected.global());
        assert_eq!(old_index.skin_matrices(), expected.skin_matrices());
    }
    assert_eq!(out.source().dependencies.len(), 2);
    assert_eq!(out.source().dependencies[1].uri, "input.animated.json");
    assert_eq!(out.source().dependencies[1].sha256, digest(SOURCE));
}
#[cfg(feature = "room-animated-models")]
#[test]
fn trim_rejects_bad_source_missing_clip_duplicate_name_and_outside_source_range() {
    assert!(trim_variant_bytes(b"bad model", &variant()).is_err());
    let mut v = variant();
    v.clip = u32::MAX;
    assert!(trim_variant_bytes(SOURCE, &v).is_err());
    let original = orr_model::animation::AnimatedModel::from_bytes(SOURCE).unwrap();
    let mut v = variant();
    v.name = original.source().clips[0].name.clone();
    assert!(trim_variant_bytes(SOURCE, &v).is_err());
    let mut v = variant();
    v.end_seconds = 100.0;
    assert!(trim_variant_bytes(SOURCE, &v).is_err());
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
mod publication {
    use super::*;
    fn request(root: &Path) -> Request {
        let r = Request {
            source: root.join("source.animated.json"),
            license: root.join("LICENSE"),
            destination: root.join("trim-package"),
            ..unopened()
        };
        fs::write(&r.source, SOURCE).unwrap();
        fs::write(&r.license, b"Trim source license\n").unwrap();
        r
    }
    #[test]
    fn trim_publishes_exact_preview_payload_and_preserves_inputs_and_foreign_output() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let r = request(&root);
        let source = r.source.clone();
        let license = r.license.clone();
        let expected = trim_variant_bytes(SOURCE, &variant()).unwrap();
        let report = import_trim_variant_checked(r, variant(), Some(digest(SOURCE))).unwrap();
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), expected);
        assert_eq!(report.source_sha256, digest(&expected));
        assert_eq!(report.asset, "model.animated.json");
        assert_eq!(report.kind, ModelKind::Animated);
        assert_eq!(fs::read(&source).unwrap(), SOURCE);
        assert_eq!(fs::read(&license).unwrap(), b"Trim source license\n");
        let before = fs::read(report.root.join(&report.asset)).unwrap();
        assert!(import_trim_variant(request(&root), variant()).is_err());
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), before);
        orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime()).unwrap();
    }
    #[test]
    fn stale_source_and_invalid_range_never_publish_or_mutate_input() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let r = request(&root);
        let output = r.destination.clone();
        assert!(
            import_trim_variant_checked(r, variant(), Some("a".repeat(64)))
                .err()
                .unwrap()
                .contains("source changed")
        );
        assert!(!output.exists());
        let r = request(&root);
        let mut bad = variant();
        bad.end_seconds = 100.0;
        assert!(import_trim_variant(r, bad).is_err());
        assert!(!output.exists());
        assert_eq!(fs::read(root.join("source.animated.json")).unwrap(), SOURCE);
    }
}
