use super::*;
fn variant() -> ColorVariant {
    ColorVariant {
        material: 0,
        linear_rgb: [0.2, 0.7, 0.4],
        asset_id: "variants/green.recolor.json".into(),
    }
}
fn unopened() -> Request {
    Request {
        source: "missing.animated.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "color-player".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Animated,
        source_format: SourceFormat::CookedAnimated,
        license_acknowledged: true,
    }
}
#[test]
fn spec_rejects_invalid_identity_and_rgb_without_io() {
    for id in [
        "",
        "../bad.recolor.json",
        "/bad.recolor.json",
        "CON.recolor.json",
        "x.recolor.JSON",
        "x//y.recolor.json",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(validate_color_variant(&v).is_err());
    }
    for rgb in [
        [f64::NAN, 0.0, 0.0],
        [f64::INFINITY, 0.0, 0.0],
        [-0.01, 0.0, 0.0],
        [1.00000000001, 0.0, 0.0],
    ] {
        let mut v = variant();
        v.linear_rgb = rgb;
        assert!(validate_color_variant(&v).is_err());
    }
    validate_color_variant(&variant()).unwrap();
}
#[test]
fn wrong_mode_and_license_reject_before_paths() {
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(import_color_variant(r, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut r = unopened();
    r.source_format = SourceFormat::RawGlb;
    assert!(import_color_variant(r, variant())
        .err()
        .unwrap()
        .contains("CookedAnimated"));
    let mut r = unopened();
    r.kind = ModelKind::Static;
    assert!(import_color_variant(r, variant())
        .err()
        .unwrap()
        .contains("Animated model"));
}
#[cfg(not(feature = "room-animated-models"))]
#[test]
fn feature_gate_precedes_all_paths() {
    assert!(import_color_variant(unopened(), variant())
        .err()
        .unwrap()
        .contains("requires room-animated-models"));
}
#[cfg(feature = "room-animated-models")]
#[test]
fn producer_changes_rgb_only_and_binds_original_bytes() {
    let bytes = include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
    let original = orr_model::animation::AnimatedModel::from_bytes(bytes).unwrap();
    let a = color_variant_bytes(bytes, &variant()).unwrap();
    assert_eq!(a, color_variant_bytes(bytes, &variant()).unwrap());
    let out = orr_model::animation::AnimatedModel::from_bytes(&a).unwrap();
    let mut expected = original.source().clone();
    expected.asset_id = variant().asset_id;
    for p in &mut expected.primitives {
        p.id = format!(
            "{}{}",
            expected.asset_id,
            p.id.strip_prefix(&original.source().asset_id).unwrap()
        );
    }
    expected.materials[0].base_color[..3].copy_from_slice(&[0.2, 0.7, 0.4]);
    expected.dependencies = out.source().dependencies.clone();
    assert_eq!(serde_json::to_vec(&expected).unwrap(), a);
    assert_eq!(out.source().dependencies.len(), 2);
    assert_eq!(out.source().dependencies[1].uri, "input.animated.json");
    assert_eq!(out.source().dependencies[1].sha256, digest(bytes));
    let mut bad = variant();
    bad.material = u32::MAX;
    assert!(color_variant_bytes(bytes, &bad).is_err());
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
mod publication {
    use super::*;
    const SOURCE: &[u8] =
        include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
    fn request(root: &Path) -> Request {
        let source = root.join("source.animated.json");
        let license = root.join("license.txt");
        fs::write(&source, SOURCE).unwrap();
        fs::write(&license, b"User supplied license\n").unwrap();
        Request {
            source,
            license,
            destination: root.join("variant"),
            ..unopened()
        }
    }
    #[test]
    fn published_variant_preserves_inputs_and_installs_with_actual_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let r = request(&root);
        let input = r.source.clone();
        let license = r.license.clone();
        let report = import_color_variant(r, variant()).unwrap();
        let output = fs::read(report.root.join(&report.asset)).unwrap();
        assert_eq!(output, color_variant_bytes(SOURCE, &variant()).unwrap());
        assert_eq!(report.source_sha256, digest(&output));
        assert_ne!(report.source_sha256, digest(SOURCE));
        assert_eq!(fs::read(input).unwrap(), SOURCE);
        assert_eq!(fs::read(license).unwrap(), b"User supplied license\n");
        let runtime = orr_sample::room_project::compiled_runtime();
        orr_package::inspect(&report.root, &runtime).unwrap();
        let project_root = root.join("project");
        fs::create_dir(&project_root).unwrap();
        fs::write(
            project_root.join("orr.project.json"),
            serde_json::to_vec(
                &serde_json::json!({"schema":1,"engine":format!("={}",runtime.engine_version)}),
            )
            .unwrap(),
        )
        .unwrap();
        let project = orr_package::Project::open(&project_root, runtime).unwrap();
        project.install(std::slice::from_ref(&report.root)).unwrap();
        project.verify().unwrap();
        let loaded = crate::model_bindings::load_asset_from_project(
            &project,
            "color-player",
            "model.animated.json",
            ModelKind::Animated,
        )
        .unwrap();
        assert_eq!(loaded.source_hash(), digest(&output));
        assert_eq!(loaded.animated_model().unwrap().source().clips.len(), 3);
    }
    #[test]
    fn invalid_material_creates_nothing_and_existing_destination_survives() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let r = request(&root);
        let destination = r.destination.clone();
        let mut bad = variant();
        bad.material = u32::MAX;
        assert!(import_color_variant(r, bad).is_err());
        assert!(!destination.exists());
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("sentinel"), b"keep").unwrap();
        assert!(import_color_variant(request(&root), variant()).is_err());
        assert_eq!(fs::read(destination.join("sentinel")).unwrap(), b"keep");
    }
}
