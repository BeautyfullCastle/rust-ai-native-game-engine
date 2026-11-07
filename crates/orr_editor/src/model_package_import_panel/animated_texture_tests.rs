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
fn png_skin_is_default_off_and_only_exposed_in_compiled_cooked_animated_mode() {
    let mut h = harness(Panel::default());
    assert!(!h.state().texture_variant_enabled);
    assert!(h.query_by_label("Create animated PNG skin").is_none());
    if cfg!(feature = "room-animated-models") {
        h.get_by_label("Cooked animated model").click_accesskit();
        h.run_steps(3);
        h.get_by_label("Create animated PNG skin").click_accesskit();
        h.run_steps(3);
        assert!(h.state().texture_variant_enabled);
        assert!(!h.state().color_variant_enabled);
        assert!(h.query_by_label("Variant PNG file").is_some());
        assert_eq!(
            h.state().texture_variant.asset_id,
            "variants/skin.retexture.json"
        );
        h.get_by_label("Create animated color variant")
            .click_accesskit();
        h.run_steps(3);
        assert!(h.state().color_variant_enabled);
        assert!(!h.state().texture_variant_enabled);
        assert!(h.query_by_label("Variant PNG file").is_none());
        h.get_by_label("Create animated PNG skin").click_accesskit();
        h.run_steps(3);
        assert!(!h.state().color_variant_enabled);
        assert!(h.state().texture_variant_enabled);
        h.get_by_label("Raw GLB").click_accesskit();
        h.run_steps(3);
        assert!(!h.state().texture_variant_enabled && !h.state().color_variant_enabled);
        assert_eq!(h.state().texture_variant, default_texture_variant());
    } else {
        assert!(h
            .get_by_label("Cooked animated model")
            .accesskit_node()
            .is_disabled());
    }
}
#[test]
fn png_missing_path_mixed_operation_or_wrong_mode_refuses_worker_before_io() {
    for case in 0..4 {
        let mut p = Panel {
            texture_variant_enabled: true,
            source_format: SourceFormat::CookedAnimated,
            kind: ModelKind::Animated,
            ..Default::default()
        };
        p.texture_variant.texture = PathBuf::from("/input.png");
        match case {
            0 => p.texture_variant.texture = PathBuf::new(),
            1 => p.color_variant_enabled = true,
            2 => p.source_format = SourceFormat::CookedStatic,
            _ => p.texture_variant.asset_id.clear(),
        }
        p.start_import(PathBuf::from("/project"), 4, 7);
        assert!(!p.busy());
        assert!(!p.status().is_empty());
        assert!(p.destination_identity.is_none());
    }
}
#[cfg(feature = "room-animated-models")]
#[test]
fn png_busy_controls_and_exact_path_material_identity_snapshot_reject_stale_results() {
    for changed in ["path", "material", "identity", "operation"] {
        let mut p = Panel {
            texture_variant_enabled: true,
            source_format: SourceFormat::CookedAnimated,
            kind: ModelKind::Animated,
            ..Default::default()
        };
        p.texture_variant.texture = PathBuf::from("/input.png");
        p.submitted_color = Some(None);
        p.submitted_texture = Some(Some(p.texture_variant.clone()));
        let (sender, receiver) = mpsc::channel();
        p.pending = Some(receiver);
        let mut h = harness(p);
        assert!(h
            .get_by_label("Create animated PNG skin")
            .accesskit_node()
            .is_disabled());
        assert!(h
            .get_by_label("Variant PNG file")
            .accesskit_node()
            .is_disabled());
        match changed {
            "path" => h.state_mut().texture_variant.texture = PathBuf::from("/other.png"),
            "material" => h.state_mut().texture_variant.material = 1,
            "identity" => {
                h.state_mut().texture_variant.asset_id = "variants/other.retexture.json".into()
            }
            _ => h.state_mut().texture_variant_enabled = false,
        }
        sender
            .send(Completion {
                source_format: SourceFormat::CookedAnimated,
                project_root: PathBuf::from("/project"),
                expected_destination: Ok(PathBuf::from("/new")),
                project_generation: 4,
                source_generation: 7,
                result: Ok(Report {
                    root: PathBuf::from("/new"),
                    package: "skin".into(),
                    asset: "model.animated.json".into(),
                    kind: ModelKind::Animated,
                    source_sha256: "asset".into(),
                    license_sha256: "license".into(),
                }),
            })
            .unwrap();
        assert!(
            h.state_mut()
                .poll(Some(Path::new("/project")), 4, 7)
                .is_none(),
            "{changed}"
        );
        assert!(!h.state().busy());
        assert!(h.state().destination_identity.is_none());
    }
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
#[test]
fn png_worker_refuses_missing_rights_and_bad_texture_without_publication() {
    use std::time::{Duration, Instant};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.animated.json");
    let license = temp.path().join("LICENSE.txt");
    let texture = temp.path().join("bad.png");
    std::fs::write(
        &source,
        include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json"),
    )
    .unwrap();
    std::fs::write(&license, b"Explicit fixture license").unwrap();
    std::fs::write(&texture, b"not a PNG").unwrap();
    for ack in [false, true] {
        let destination = temp.path().join(if ack { "bad-png" } else { "no-ack" });
        let mut p = Panel {
            source: source.display().to_string(),
            license: license.display().to_string(),
            destination: destination.display().to_string(),
            name: "skin".into(),
            version: "1.0.0".into(),
            source_format: SourceFormat::CookedAnimated,
            kind: ModelKind::Animated,
            texture_variant_enabled: true,
            texture_variant: TextureVariant {
                texture: texture.clone(),
                ..default_texture_variant()
            },
            license_acknowledged: ack,
            ..Default::default()
        };
        p.start_import(PathBuf::from("/project"), 4, 7);
        let deadline = Instant::now() + Duration::from_secs(10);
        while p.busy() {
            assert!(p.poll(Some(Path::new("/project")), 4, 7).is_none());
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            p.status().contains("package import failed"),
            "{}",
            p.status()
        );
        assert!(!destination.exists());
        assert!(p.destination_identity.is_none());
        assert!(!p.license_acknowledged);
    }
}
