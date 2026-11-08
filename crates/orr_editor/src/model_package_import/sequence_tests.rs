use super::*;
fn variant() -> SequenceVariant {
    SequenceVariant {
        first_clip: 0,
        second_clip: 2,
        transition_seconds: 0.25,
        name: "search-then-escape".into(),
        asset_id: "variants/action.sequence.json".into(),
    }
}
fn unopened() -> Request {
    Request {
        source: "missing.animated.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "sequence-player".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Animated,
        source_format: SourceFormat::CookedAnimated,
        license_acknowledged: true,
    }
}
#[test]
fn invalid_sequence_identity_name_and_transition_reject_without_io() {
    for id in [
        "../bad.sequence.json",
        "bad.recolor.json",
        "/bad.sequence.json",
        "CON.sequence.json",
        "",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(validate_sequence_variant(&v).is_err());
    }
    for name in ["".into(), "   ".into(), "bad\nname".into(), "a".repeat(129)] {
        let mut v = variant();
        v.name = name;
        assert!(validate_sequence_variant(&v).is_err());
    }
    for seconds in [f64::NAN, f64::INFINITY, -0.01, 0., f64::MIN_POSITIVE, 60.01] {
        let mut v = variant();
        v.transition_seconds = seconds;
        assert!(validate_sequence_variant(&v).is_err());
    }
    validate_sequence_variant(&variant()).unwrap();
    #[cfg(feature = "room-animated-models")]
    for first in [false, true] {
        let mut v = variant();
        if first {
            v.first_clip = u32::MAX;
        } else {
            v.second_clip = u32::MAX;
        }
        assert!(validate_sequence_variant(&v).is_err());
    }
}
#[test]
fn sequence_license_mode_and_expected_hash_gate_precedes_paths() {
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(import_sequence_variant(r, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut r = unopened();
    r.kind = ModelKind::Static;
    assert!(import_sequence_variant(r, variant())
        .err()
        .unwrap()
        .contains("Animated model"));
    let mut r = unopened();
    r.source_format = SourceFormat::RawGlb;
    assert!(import_sequence_variant(r, variant())
        .err()
        .unwrap()
        .contains("CookedAnimated"));
    assert!(
        import_sequence_variant_checked(unopened(), variant(), Some("bad".into()))
            .err()
            .unwrap()
            .contains("64 hexadecimal")
    );
}
#[cfg(not(feature = "room-animated-models"))]
#[test]
fn sequence_feature_off_rejects_before_all_input_and_output_paths() {
    assert!(import_sequence_variant(unopened(), variant())
        .err()
        .unwrap()
        .contains("requires room-animated-models"));
}
#[cfg(feature = "room-animated-models")]
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
#[cfg(feature = "room-animated-models")]
#[test]
fn sequence_preserves_old_clips_and_source_fields_and_matches_first_bridge_second_pose_oracle() {
    use orr_model::animation::AnimatedModel;
    let original = AnimatedModel::from_bytes(SOURCE).unwrap();
    let original_bytes = original.to_bytes().unwrap();
    let bytes = sequence_variant_bytes(SOURCE, &variant()).unwrap();
    assert_eq!(bytes, sequence_variant_bytes(SOURCE, &variant()).unwrap());
    let out = AnimatedModel::from_bytes(&bytes).unwrap();
    let n = original.source().clips.len();
    assert_eq!(out.source().clips.len(), n + 1);
    assert_eq!(out.source().clips[n].name, variant().name);
    assert_eq!(out.source().clips[n].duration(), 3.25);
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
    assert_eq!(&expected, out.source());
    assert_eq!(out.source().dependencies.len(), 2);
    assert_eq!(out.source().dependencies[0].uri, "$source");
    assert_eq!(out.source().dependencies[1].uri, "input.animated.json");
    assert_eq!(out.source().dependencies[1].sha256, digest(SOURCE));
    for (t, clip, local) in [
        (0., 0, 0.),
        (0.75, 0, 0.75),
        (2., 0, 2.),
        (2.25, 2, 0.),
        (2.75, 2, 0.5),
        (3.25, 2, 1.),
    ] {
        let a = out.sample_clip(n as u32, t).unwrap();
        let b = original.sample_clip(clip, local).unwrap();
        assert_eq!(a.local(), b.local());
        assert_eq!(a.global(), b.global());
        assert_eq!(a.skin_matrices(), b.skin_matrices());
    }
    let left = original.sample_clip(0, 2.).unwrap();
    let right = original.sample_clip(2, 0.).unwrap();
    let bridge = out.sample_clip(n as u32, 2.125).unwrap();
    for ((a, b), actual) in left.local().iter().zip(right.local()).zip(bridge.local()) {
        for axis in 0..3 {
            assert!(
                (actual.translation[axis] - (a.translation[axis] + b.translation[axis]) * 0.5)
                    .abs()
                    < 1e-5
            );
            assert!((actual.scale[axis] - (a.scale[axis] + b.scale[axis]) * 0.5).abs() < 1e-5);
        }
        let dot = (0..4).map(|i| a.rotation[i] * b.rotation[i]).sum::<f32>();
        let sum: Vec<f64> = (0..4)
            .map(|i| {
                f64::from(a.rotation[i])
                    + f64::from(b.rotation[i]) * if dot < 0. { -1. } else { 1. }
            })
            .collect();
        let length = sum.iter().map(|v| v * v).sum::<f64>().sqrt();
        let expected_dot = (0..4)
            .map(|i| f64::from(actual.rotation[i]) * sum[i] / length)
            .sum::<f64>()
            .abs();
        assert!((1. - expected_dot).abs() < 1e-5);
    }
    let expected = original.blend_poses(&left, &right, 0.5).unwrap();
    for (a, b) in bridge
        .global()
        .iter()
        .flatten()
        .flatten()
        .zip(expected.global().iter().flatten().flatten())
    {
        assert!((a - b).abs() < 1e-5);
    }
    for (a, b) in bridge
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
        assert!((a - b).abs() < 1e-5);
    }
    assert_eq!(original.to_bytes().unwrap(), original_bytes);
}
#[cfg(feature = "room-animated-models")]
#[test]
fn sequence_rejects_malformed_source_duplicate_name_step_mismatched_targets_and_clip_budget() {
    use orr_model::animation::{AnimatedModel, Interpolation, MAX_CLIPS};
    assert!(sequence_variant_bytes(b"bad", &variant()).is_err());
    let original = AnimatedModel::from_bytes(SOURCE).unwrap();
    let mut v = variant();
    v.name = original.source().clips[0].name.clone();
    assert!(sequence_variant_bytes(SOURCE, &v).is_err());
    let mut v = variant();
    v.second_clip = 31;
    assert!(sequence_variant_bytes(SOURCE, &v).is_err());
    let mut source = original.source().clone();
    source.clips[2].channels[0].interpolation = Interpolation::Step;
    assert!(sequence_variant_bytes(
        &AnimatedModel::new(source).unwrap().to_bytes().unwrap(),
        &variant()
    )
    .is_err());
    let mut source = original.source().clone();
    source.clips[2].channels.pop();
    assert!(sequence_variant_bytes(
        &AnimatedModel::new(source).unwrap().to_bytes().unwrap(),
        &variant()
    )
    .is_err());
    let mut source = original.source().clone();
    while source.clips.len() < MAX_CLIPS {
        let mut clip = source.clips[0].clone();
        clip.name = format!("copy{}", source.clips.len());
        source.clips.push(clip);
    }
    assert!(sequence_variant_bytes(
        &AnimatedModel::new(source).unwrap().to_bytes().unwrap(),
        &variant()
    )
    .is_err());
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
mod publication {
    use super::*;
    fn request(root: &Path) -> Request {
        let r = Request {
            source: root.join("source.animated.json"),
            license: root.join("LICENSE"),
            destination: root.join("sequence-package"),
            ..unopened()
        };
        fs::write(&r.source, SOURCE).unwrap();
        fs::write(&r.license, b"Sequence source license\n").unwrap();
        r
    }
    #[test]
    fn sequence_publish_matches_readonly_preview_hash_and_preserves_inputs_and_foreign_output() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let r = request(&root);
        let source = r.source.clone();
        let license = r.license.clone();
        let expected = sequence_variant_bytes(SOURCE, &variant()).unwrap();
        let preview = variant_preview::prepare(variant_preview::Request {
            source: source.clone(),
            spec: variant_preview::Spec::Sequence(variant()),
            expected_source_sha256: Some(digest(SOURCE)),
        })
        .unwrap();
        assert_eq!(preview.asset_sha256, digest(&expected));
        assert_eq!(preview.source_sha256, digest(SOURCE));
        assert!(preview.png_sha256.is_none());
        assert_eq!(preview.model.to_bytes().unwrap(), expected);
        let report = import_sequence_variant_checked(r, variant(), Some(digest(SOURCE))).unwrap();
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), expected);
        assert_eq!(report.source_sha256, digest(&expected));
        assert_eq!(report.asset, "model.animated.json");
        assert_eq!(fs::read(&source).unwrap(), SOURCE);
        assert_eq!(fs::read(&license).unwrap(), b"Sequence source license\n");
        assert!(import_sequence_variant(request(&root), variant()).is_err());
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), expected);
        orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime()).unwrap();
    }
    #[test]
    fn stale_inspection_and_invalid_clip_never_publish_or_change_source() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let r = request(&root);
        let output = r.destination.clone();
        assert!(
            import_sequence_variant_checked(r, variant(), Some("a".repeat(64)))
                .err()
                .unwrap()
                .contains("source changed")
        );
        assert!(!output.exists());
        let mut spec = variant();
        spec.second_clip = 31;
        assert!(import_sequence_variant(request(&root), spec).is_err());
        assert!(!output.exists());
        assert_eq!(fs::read(root.join("source.animated.json")).unwrap(), SOURCE);
    }
}
