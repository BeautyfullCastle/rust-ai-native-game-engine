use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};

fn harness(panel: Panel) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1400.0, 1800.0])
        .build_ui_state(
            |ui, p: &mut Panel| p.show(ui, true, Some(Path::new("/project")), 4, 7),
            panel,
        );
    h.run_steps(3);
    h.get_by_label("Import a model package").click_accesskit();
    h.run_steps(3);
    h
}
#[test]
fn color_option_is_explicit_default_off_and_compiled_animated_only() {
    let mut h = harness(Panel::default());
    assert!(h.query_by_label("Create animated color variant").is_none());
    assert!(!h.state().color_variant_enabled);
    if cfg!(feature = "room-animated-models") {
        h.get_by_label("Cooked animated model").click_accesskit();
        h.run_steps(3);
        assert!(!h.state().color_variant_enabled);
        assert!(h.query_by_label("Variant asset ID").is_none());
        h.get_by_label("Create animated color variant")
            .click_accesskit();
        h.run_steps(3);
        assert!(h.state().color_variant_enabled);
        assert_eq!(
            h.state().color_variant.asset_id,
            "variants/color.recolor.json"
        );
        for label in [
            "Variant material index",
            "Variant linear R (0..1)",
            "Variant linear G (0..1)",
            "Variant linear B (0..1)",
        ] {
            assert!(h.query_by_label(label).is_some());
        }
        h.get_by_label("I have reviewed rights to distribute this content")
            .click_accesskit();
        h.run_steps(3);
        h.get_by_label("Raw GLB").click_accesskit();
        h.run_steps(3);
        assert!(!h.state().color_variant_enabled && !h.state().license_acknowledged);
        assert!(h.query_by_label("Create animated color variant").is_none());
        assert_eq!(h.state().color_variant, default_color_variant());
    } else {
        assert!(h
            .get_by_label("Cooked animated model")
            .accesskit_node()
            .is_disabled());
    }
}
#[test]
fn incompatible_or_invalid_color_request_never_starts_worker() {
    for case in 0..4 {
        let mut p = Panel {
            color_variant_enabled: true,
            ..Default::default()
        };
        p.source_format = SourceFormat::CookedAnimated;
        p.kind = ModelKind::Animated;
        match case {
            0 => p.source_format = SourceFormat::RawGlb,
            1 => p.kind = ModelKind::Static,
            2 => p.color_variant.linear_rgb[0] = f64::NAN,
            _ => p.color_variant.asset_id.clear(),
        }
        p.start_import(PathBuf::from("/absent/project"), 4, 7);
        assert!(!p.busy());
        assert!(p.destination_identity.is_none());
        assert!(!p.status().is_empty());
    }
}
#[cfg(feature = "room-animated-models")]
#[test]
fn changed_exact_color_spec_rejects_completed_worker_and_busy_disables_controls() {
    let mut p = Panel {
        source_format: SourceFormat::CookedAnimated,
        kind: ModelKind::Animated,
        color_variant_enabled: true,
        ..Default::default()
    };
    let submitted = p.color_variant.clone();
    p.submitted_color = Some(Some(submitted));
    let (sender, receiver) = mpsc::channel();
    p.pending = Some(receiver);
    let mut h = harness(p);
    assert!(h
        .get_by_label("Create animated color variant")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .get_by_label("Variant asset ID")
        .accesskit_node()
        .is_disabled());
    h.state_mut().color_variant.linear_rgb[0] = 0.2;
    sender
        .send(Completion {
            source_format: SourceFormat::CookedAnimated,
            project_root: PathBuf::from("/project"),
            expected_destination: Ok(PathBuf::from("/new-package")),
            project_generation: 4,
            source_generation: 7,
            result: Ok(Report {
                root: PathBuf::from("/new-package"),
                package: "color".into(),
                asset: "model.animated.json".into(),
                kind: ModelKind::Animated,
                source_sha256: "asset".into(),
                license_sha256: "license".into(),
            }),
        })
        .unwrap();
    assert!(h
        .state_mut()
        .poll(Some(Path::new("/project")), 4, 7)
        .is_none());
    assert!(!h.state().busy());
    assert!(h.state().destination_identity.is_none());
    assert!(h.state().status().contains("prior project/source state"));
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
#[test]
fn actual_color_checkbox_and_import_worker_publish_variant_and_report_asset_digest() {
    use sha2::{Digest, Sha256};
    use std::time::{Duration, Instant};
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let root = parent.join("project");
    std::fs::create_dir(&root).unwrap();
    let source = parent.join("courier.animated.json");
    let license = parent.join("LICENSE.txt");
    let destination = parent.join("color-package");
    let bytes = include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
    std::fs::write(&source, bytes).unwrap();
    std::fs::write(&license, b"Explicit fixture license").unwrap();
    let panel = Panel {
        source: source.display().to_string(),
        license: license.display().to_string(),
        destination: destination.display().to_string(),
        name: "color-character".into(),
        version: "1.0.0".into(),
        source_format: SourceFormat::CookedAnimated,
        kind: ModelKind::Animated,
        color_variant: ColorVariant {
            linear_rgb: [0.2, 0.7, 0.4],
            ..default_color_variant()
        },
        ..Default::default()
    };
    let mut h = harness(panel);
    h.get_by_label("Create animated color variant")
        .click_accesskit();
    h.run_steps(3);
    assert!(h
        .get_by_label("Import cooked animated package")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("I have reviewed rights to distribute this content")
        .click_accesskit();
    h.run_steps(3);
    h.get_by_label("Import cooked animated package")
        .click_accesskit();
    h.run_steps(1);
    let deadline = Instant::now() + Duration::from_secs(10);
    let report = loop {
        if let Some(report) = h.state_mut().poll(Some(Path::new("/project")), 4, 7) {
            break report;
        }
        assert!(h.state().busy(), "{}", h.state().status());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    let asset = std::fs::read(destination.join("model.animated.json")).unwrap();
    let model = orr_model::animation::AnimatedModel::from_bytes(&asset).unwrap();
    assert_eq!(model.source().materials[0].base_color, [0.2, 0.7, 0.4, 1.0]);
    let hash: String = Sha256::digest(&asset)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(report.source_sha256, hash);
    assert!(h
        .state()
        .status()
        .contains(&format!("Asset SHA-256: {hash}")));
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    assert!(!root.join(".orr").exists());
    assert!(!h.state().license_acknowledged);
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
#[test]
fn color_worker_license_and_missing_input_errors_do_not_publish_or_select() {
    use std::time::{Duration, Instant};
    let temp = tempfile::tempdir().unwrap();
    for acknowledged in [false, true] {
        let destination = temp.path().join(if acknowledged {
            "missing-input"
        } else {
            "no-license-ack"
        });
        let mut panel = Panel {
            source: temp
                .path()
                .join("absent.animated.json")
                .display()
                .to_string(),
            license: temp.path().join("absent-license.txt").display().to_string(),
            destination: destination.display().to_string(),
            name: "color".into(),
            version: "1.0.0".into(),
            source_format: SourceFormat::CookedAnimated,
            kind: ModelKind::Animated,
            color_variant_enabled: true,
            license_acknowledged: acknowledged,
            ..Default::default()
        };
        panel.start_import(PathBuf::from("/project"), 4, 7);
        let deadline = Instant::now() + Duration::from_secs(10);
        while panel.busy() {
            assert!(panel.poll(Some(Path::new("/project")), 4, 7).is_none());
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            panel.status().contains("package import failed"),
            "{}",
            panel.status()
        );
        if !acknowledged {
            assert!(panel.status().contains("acknowledgement"));
        }
        assert!(!destination.exists());
        assert!(panel.destination_identity.is_none());
        assert!(!panel.license_acknowledged);
    }
}
