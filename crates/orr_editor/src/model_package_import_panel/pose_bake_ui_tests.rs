//! Animated-source/static-result CPU widgets, exact clip/time and ticket admission.
use super::*;
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};

fn harness(p: Panel) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1600.0, 3400.0])
        .build_ui_state(
            |ui, p: &mut Panel| p.show(ui, true, Some(Path::new("/project")), 4, 7),
            p,
        );
    h.run_steps(2);
    click(&mut h, "Import a model package");
    h
}
fn click(h: &mut Harness<'_, Panel>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
fn replace(h: &mut Harness<'_, Panel>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(2);
}

#[test]
fn pose_bake_time_validates_original_f64_endpoint_and_positive_underflow() {
    let duration = f64::from(0.1_f32);
    assert_eq!(number(&duration.to_string(), duration).unwrap(), duration);
    let above = f64::from_bits(duration.to_bits() + 1);
    assert_eq!(above as f32, duration as f32);
    assert!(number(&above.to_string(), duration).is_err());
    for raw in ["", "NaN", "inf", "-inf", "-1e-300", "1e-300", "1e300"] {
        assert!(number(raw, duration).is_err(), "{raw}");
    }
    assert_eq!(
        number("-0", duration).unwrap().to_bits(),
        (-0.0_f64).to_bits()
    );
    assert_eq!(number("0", 0.0).unwrap(), 0.0);
    assert!(number("0.000000000000001", duration).is_ok());
}

#[test]
fn pose_bake_widgets_default_off_feature_gate_exclusive_reset_and_busy() {
    let mut h = harness(Panel::default());
    assert!(!h.state().pose_bake.enabled);
    assert!(h
        .query_by_label("Bake static prop from animation")
        .is_none());
    if !cfg!(feature = "room-animated-models") {
        assert!(h
            .get_by_label("Cooked animated model")
            .accesskit_node()
            .is_disabled());
        let p = h.state_mut();
        p.source_format = SourceFormat::CookedAnimated;
        p.kind = ModelKind::Animated;
        p.source = "/missing.animated.json".into();
        p.pose_bake.enabled = true;
        p.pose_bake.clip = Some(0);
        assert!(p.pose_bake_request().is_err());
        p.start_import("/missing-project".into(), 4, 7);
        assert!(!p.busy());
        assert!(p.destination_identity.is_none());
        return;
    }
    click(&mut h, "Cooked animated model");
    assert!(!h.state().pose_bake.enabled);
    for other in [
        "Create animated color variant",
        "Create animated PNG skin",
        "Create trimmed animation clip",
        "Create animated sampler variant",
        "Create animation clip sequence",
        "Create animated UV variant",
    ] {
        click(&mut h, other);
        click(&mut h, "Bake static prop from animation");
        assert!(h.state().pose_bake.enabled);
        let p = h.state();
        assert!(
            !p.color_variant_enabled
                && !p.texture_variant_enabled
                && !p.trim.enabled
                && !p.sampler.enabled
                && !p.sequence.enabled
                && !p.uv.enabled
        );
        click(&mut h, other);
        assert!(!h.state().pose_bake.enabled);
    }
    h.state_mut().static_uv.enabled = true;
    h.state_mut().static_texture.enabled = true;
    h.state_mut().static_assembly.enabled = true;
    h.state_mut().static_pivot.enabled = true;
    h.state_mut().static_sampler.enabled = true;
    h.state_mut().static_recolor.enabled = true;
    h.state_mut().glb_subtree.enabled = true;
    click(&mut h, "Bake static prop from animation");
    let p = h.state();
    assert!(
        !p.static_uv.enabled
            && !p.static_texture.enabled
            && !p.static_assembly.enabled
            && !p.static_pivot.enabled
            && !p.static_sampler.enabled
            && !p.static_recolor.enabled
            && !p.glb_subtree.enabled
    );
    assert!(p.pose_bake.clip.is_none());
    assert!(p.pose_bake_request().is_err());
    assert!(h
        .get_by_label("Preview animated variant")
        .accesskit_node()
        .is_disabled());
    h.state_mut().start_import("/project".into(), 4, 7);
    assert!(!h.state().busy());
    let (sender, receiver) = mpsc::channel();
    h.state_mut().pending = Some(receiver);
    h.run_steps(2);
    for label in [
        "Bake static prop from animation",
        "Pose sample time (seconds)",
        "Static pose asset ID",
        "Inspect animated source",
        "Import baked static pose package",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
    drop(sender);
    h.state_mut().poll(Some(Path::new("/project")), 4, 7);
    h.run_steps(2);
    click(&mut h, "Cooked static model");
    assert_eq!(h.state().pose_bake, Controls::default());
    assert!(h
        .query_by_label("Bake static prop from animation")
        .is_none());
}

#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
fn fixture(path: &Path) {
    let model = orr_model::animation::AnimatedModel::from_bytes(include_bytes!(
        "../../../../assets/room_character_demo/courier.orrmodel.json"
    ))
    .unwrap();
    let mut source = model.source().clone();
    let mut clip = source.clips[0].clone();
    clip.name = "same advisory name".into();
    source.clips = vec![clip.clone(), clip];
    std::fs::write(
        path,
        orr_model::animation::AnimatedModel::new(source)
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
fn inspected(source: &Path) -> Harness<'static, Panel> {
    use std::time::{Duration, Instant};
    let mut p = Panel::default();
    p.set_source_format(SourceFormat::CookedAnimated);
    p.source = source.display().to_string();
    p.license = "/license.txt".into();
    p.destination = "/package".into();
    p.name = "static-pose".into();
    p.version = "1.0.0".into();
    let mut h = harness(p);
    click(&mut h, "Bake static prop from animation");
    click(&mut h, "Inspect animated source");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.state().busy() {
        h.state_mut().poll(Some(Path::new("/project")), 4, 7);
        h.run_steps(2);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    h
}

#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
#[test]
fn pose_bake_actual_nonzero_clip_shared_names_endpoint_widgets_consent_and_pending_aba_max() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("courier.animated.json");
    fixture(&source);
    let original = std::fs::read(&source).unwrap();
    let mut h = inspected(&source);
    assert!(h.state().pose_bake.clip.is_none());
    assert!(h.state().inspected_hash.is_none());
    assert!(h.state().pose_bake_request().is_err());
    let catalog = h.state().inspection.catalog().unwrap().clone();
    assert_eq!(catalog.clips[0].name, catalog.clips[1].name);
    h.state_mut().license_acknowledged = true;
    click(&mut h, "Use clip 1");
    assert!(!h.state().license_acknowledged);
    assert_eq!(h.state().pose_bake_request().unwrap().unwrap().clip, 1);
    let duration = f64::from(catalog.clips[1].duration);
    let above = f64::from_bits(duration.to_bits() + 1).to_string();
    replace(&mut h, "Pose sample time (seconds)", &above);
    assert!(h.state().pose_bake_request().is_err());
    replace(&mut h, "Pose sample time (seconds)", "1e-300");
    assert!(h.state().pose_bake_request().is_err());
    h.state_mut().license_acknowledged = true;
    replace(&mut h, "Pose sample time (seconds)", &duration.to_string());
    assert!(!h.state().license_acknowledged);
    assert_eq!(
        h.state().pose_bake_request().unwrap().unwrap().time_seconds,
        duration
    );
    for (label, value) in [
        ("Static pose asset ID", "variants/new.pose.json"),
        ("Package destination directory", "/new-package"),
        ("Package name", "pose-prop"),
        ("Package version", "1.0.1"),
        ("License file", "/new-license.txt"),
    ] {
        h.state_mut().license_acknowledged = true;
        h.state_mut().destination_identity = Some("/old".into());
        let generation = h.state().authoring_generation;
        replace(&mut h, label, value);
        assert!(!h.state().license_acknowledged);
        assert!(h.state().destination_identity.is_none());
        assert!(h.state().authoring_generation > generation);
    }
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    h.state_mut().pose_bake.clip = Some(u32::MAX);
    assert!(h.state().pose_bake_request().is_err());
    h.state_mut().pose_bake.clip = Some(1);
    for id in ["", "../bad.pose.json", "bad.recolor.json", "CON.pose.json"] {
        h.state_mut().pose_bake.asset_id = id.into();
        assert!(h.state().pose_bake_request().is_err(), "{id}");
    }
    h.state_mut().pose_bake.asset_id = "variants/new.pose.json".into();
    let complete = h
        .state_mut()
        .hold_pose_inspection_for_test(Path::new("/project"), 4, 7, false);
    h.run_steps(2);
    assert!(h.state().busy());
    assert!(h.query_by_label("Use clip 1").is_none());
    h.state_mut().source = "/other.animated.json".into();
    h.state_mut()
        .sync_inspection(Some(Path::new("/project")), 4, 7);
    h.state_mut().source = source.display().to_string();
    complete(Ok(catalog.clone()));
    h.state_mut()
        .sync_inspection(Some(Path::new("/project")), 4, 7);
    assert!(!h.state().busy());
    assert!(h.state().pose_bake.clip.is_none());
    assert!(h.state().inspected_hash.is_none());
    assert!(h.state().pose_bake_request().is_err());
    let complete = h
        .state_mut()
        .hold_pose_inspection_for_test(Path::new("/project"), 4, 7, true);
    h.state_mut().inspection.invalidate();
    h.state_mut().inspection.invalidate();
    complete(Ok(catalog));
    h.state_mut()
        .sync_inspection(Some(Path::new("/project")), 4, 7);
    assert!(!h.state().busy());
    assert!(h.state().pose_bake_request().is_err());
    h.run_steps(2);
    assert!(h
        .get_by_label("Inspect animated source")
        .accesskit_node()
        .is_disabled());
}

#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
#[test]
fn pose_bake_full_animated_source_snapshot_accepts_static_report_once_and_rejects_stale_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("courier.animated.json");
    fixture(&source);
    for changed in [
        "current",
        "project",
        "project-generation",
        "source-generation",
        "authoring-generation",
        "source",
        "license",
        "destination",
        "name",
        "version",
        "kind",
        "format",
        "hash",
        "clip",
        "time",
        "lexical",
        "signed-zero",
        "id",
        "disabled",
        "animated-color",
        "static-recolor",
    ] {
        let mut h = inspected(&source);
        click(&mut h, "Use clip 1");
        let p = h.state_mut();
        if changed == "signed-zero" {
            p.pose_bake.time_seconds = "-0".into();
        }
        p.submitted_pose_bake = Some(p.pose_bake_snapshot(Path::new("/project"), 4, 7).unwrap());
        p.submitted_inspected_hash = Some(p.inspected_hash.clone());
        p.submitted_authoring_generation = Some(p.authoring_generation);
        p.license_acknowledged = true;
        match changed {
            "source" => p.source = "/other.animated.json".into(),
            "license" => p.license.push('2'),
            "destination" => p.destination.push('2'),
            "name" => p.name.push('2'),
            "version" => p.version.push('2'),
            "kind" => p.kind = ModelKind::Static,
            "format" => p.source_format = SourceFormat::CookedStatic,
            "hash" => p.inspected_hash = Some("b".repeat(64)),
            "clip" => p.pose_bake.clip = Some(0),
            "time" => p.pose_bake.time_seconds = "0.000001".into(),
            "lexical" => {
                p.pose_bake.time_seconds = "0.0".into();
                p.source_inputs_changed();
            }
            "signed-zero" => {
                p.pose_bake.time_seconds = "0".into();
                p.source_inputs_changed();
            }
            "id" => p.pose_bake.asset_id = "variants/other.pose.json".into(),
            "disabled" => p.pose_bake.enabled = false,
            "animated-color" => p.color_variant_enabled = true,
            "static-recolor" => p.static_recolor.enabled = true,
            "authoring-generation" => p.authoring_generation += 1,
            _ => {}
        }
        let (sender, receiver) = mpsc::channel();
        p.pending = Some(receiver);
        sender
            .send(Completion {
                source_format: SourceFormat::CookedAnimated,
                project_root: "/project".into(),
                expected_destination: Ok("/package".into()),
                project_generation: 4,
                source_generation: 7,
                result: Ok(Report {
                    root: "/package".into(),
                    package: "static-pose".into(),
                    asset: "model.orrmodel".into(),
                    kind: ModelKind::Static,
                    source_sha256: "a".repeat(64),
                    license_sha256: "c".repeat(64),
                }),
            })
            .unwrap();
        let root = if changed == "project" {
            "/other-project"
        } else {
            "/project"
        };
        let result = p.poll(
            Some(Path::new(root)),
            if changed == "project-generation" {
                5
            } else {
                4
            },
            if changed == "source-generation" { 8 } else { 7 },
        );
        assert_eq!(result.is_some(), changed == "current", "{changed}");
        assert_eq!(
            p.destination_identity.is_some(),
            changed == "current",
            "{changed}"
        );
        if let Some(report) = result {
            assert_eq!(report.kind, ModelKind::Static);
            assert_eq!(p.kind, ModelKind::Animated);
            assert_eq!(p.source_format, SourceFormat::CookedAnimated);
        }
        assert!(!p.license_acknowledged);
        assert!(!p.busy());
        assert!(p.poll(Some(Path::new("/project")), 4, 7).is_none());
    }
}
