//! Source-less cylinder CPU forms and completed-result adoption; no runtime/GPU claims.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};

fn harness(p: Panel) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1600.0, 3000.0])
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
fn replace(h: &mut Harness<'_, Panel>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(2);
}
fn prepared() -> Panel {
    let mut p = Panel::default();
    p.set_cylinder_mode(true);
    p.sync_cylinder_context(Some(Path::new("/project")), 4, 7);
    p.license = "/license.txt".into();
    p.destination = "/package".into();
    p.name = "created-cylinder".into();
    p.version = "1.0.0".into();
    p
}
fn report() -> Report {
    Report {
        root: "/package".into(),
        package: "created-cylinder".into(),
        asset: "model.orrmodel".into(),
        kind: ModelKind::Static,
        source_sha256: "a".repeat(64),
        license_sha256: "b".repeat(64),
    }
}
fn queue(p: &mut Panel, result: Result<Report, String>) {
    let (sender, receiver) = mpsc::channel();
    p.pending = Some(receiver);
    sender
        .send(Completion {
            source_format: SourceFormat::CookedStatic,
            project_root: "/project".into(),
            expected_destination: Ok("/package".into()),
            project_generation: 4,
            source_generation: 7,
            result,
        })
        .unwrap();
}

#[test]
fn cylinder_mode_default_off_resets_variants_and_hides_every_external_source_route() {
    let mut h = harness(Panel::default());
    assert!(!h.state().cylinder.enabled);
    assert!(h.query_by_label("Raw GLB file").is_some());
    assert!(h.query_by_label("Cylinder radius").is_none());
    h.state_mut().source = "/external-model-that-must-not-be-opened.glb".into();
    h.state_mut().kind = ModelKind::Animated;
    h.state_mut().static_uv.enabled = true;
    h.state_mut().static_texture.enabled = true;
    h.state_mut().static_assembly.enabled = true;
    h.state_mut().static_pivot.enabled = true;
    h.state_mut().static_sampler.enabled = true;
    h.state_mut().static_recolor.enabled = true;
    h.state_mut().static_planar.enabled = true;
    h.state_mut().glb_subtree.enabled = true;
    h.state_mut().pose_bake.enabled = true;
    h.state_mut().color_variant_enabled = true;
    h.state_mut().texture_variant_enabled = true;
    h.state_mut().trim.enabled = true;
    h.state_mut().sampler.enabled = true;
    h.state_mut().sequence.enabled = true;
    h.state_mut().uv.enabled = true;
    h.state_mut().inspected_hash = Some("a".repeat(64));
    h.state_mut().license_acknowledged = true;
    h.state_mut().destination_identity = Some("/old".into());
    click(&mut h, "Create cylinder package");
    let p = h.state();
    assert!(p.cylinder.enabled);
    assert_eq!(p.kind, ModelKind::Static);
    assert_eq!(p.source_format, SourceFormat::CookedStatic);
    assert!(p.cylinder_spec().unwrap().is_some());
    assert!(!p.license_acknowledged);
    assert!(p.inspected_hash.is_none());
    assert!(p.destination_identity.is_none());
    assert!(
        !p.static_uv.enabled
            && !p.static_texture.enabled
            && !p.static_assembly.enabled
            && !p.static_pivot.enabled
            && !p.static_sampler.enabled
            && !p.static_recolor.enabled
            && !p.static_planar.enabled
            && !p.glb_subtree.enabled
            && !p.pose_bake.enabled
            && !p.color_variant_enabled
            && !p.texture_variant_enabled
            && !p.trim.enabled
            && !p.sampler.enabled
            && !p.sequence.enabled
            && !p.uv.enabled
    );
    assert!(p.inspection.catalog().is_none());
    assert!(!p.inspection.busy());
    for label in [
        "Raw GLB",
        "Cooked static model",
        "Cooked animated model",
        "Raw GLB file",
        "Cooked static model file (.model.json)",
        "Static model",
        "Animated model",
        "Inspect static source",
        "Inspect animated source",
        "Inspect GLB nodes",
        "Create static UV variant",
        "Create static planar UV variant",
        "Create static PNG skin",
        "Create static color variant",
        "Create static sampler variant",
        "Create static pivot variant",
        "Create static two-copy assembly",
        "Create static GLB subtree",
        "Bake static prop from animation",
        "Preview animated variant",
    ] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
    click(&mut h, "Create cylinder package");
    assert!(!h.state().cylinder.enabled);
    assert!(h
        .query_by_label("Cooked static model file (.model.json)")
        .is_some());
    assert!(h.state().cylinder_spec().unwrap().is_none());
    click(&mut h, "Raw GLB");
    assert!(h.query_by_label("Import raw GLB package").is_some());
}

#[test]
fn cylinder_actual_dimensions_all_segment_choices_close_popup_and_revoke_consent() {
    for raw in [
        "",
        "NaN",
        "inf",
        "-inf",
        "0",
        "-1",
        "1e-300",
        "0.0009999999999999998",
        "1000000.0000000001",
    ] {
        assert!(number(raw).is_err(), "{raw}");
    }
    assert_eq!(number("0.001").unwrap(), 0.001);
    assert_eq!(number("1000000").unwrap(), 1_000_000.0);
    let mut h = harness(prepared());
    let labels = [
        "8 cylinder segments",
        "16 cylinder segments",
        "32 cylinder segments",
        "64 cylinder segments",
        "128 cylinder segments",
    ];
    for (segments, label) in [
        Segments::Eight,
        Segments::Sixteen,
        Segments::ThirtyTwo,
        Segments::SixtyFour,
        Segments::OneTwentyEight,
    ]
    .into_iter()
    .zip(labels)
    {
        h.state_mut().license_acknowledged = true;
        let generation = h.state().authoring_generation;
        click(&mut h, "Cylinder segments");
        for option in labels {
            assert!(h.query_by_label(option).is_some(), "opened {option}");
        }
        click(&mut h, label);
        for option in labels {
            assert!(h.query_by_label(option).is_none(), "closed {option}");
        }
        assert_eq!(h.state().cylinder.segments, segments);
        assert_eq!(
            h.state().cylinder_spec().unwrap().unwrap().segments,
            segments.count()
        );
        assert!(!h.state().license_acknowledged);
        assert!(h.state().authoring_generation > generation);
    }
    for (label, value) in [
        ("Cylinder radius", "1.23456789012345"),
        ("Cylinder height", "3.5"),
        ("Cylinder asset ID", "props/new.cylinder.json"),
        ("License file", "/new-license.txt"),
        ("Package destination directory", "/new-package"),
        ("Package name", "new-cylinder"),
        ("Package version", "1.0.1"),
    ] {
        h.state_mut().license_acknowledged = true;
        h.state_mut().destination_identity = Some("/old".into());
        let generation = h.state().authoring_generation;
        replace(&mut h, label, value);
        assert!(!h.state().license_acknowledged);
        assert!(h.state().destination_identity.is_none());
        assert!(h.state().authoring_generation > generation);
    }
    assert_eq!(h.state().cylinder.radius, "1.23456789012345");
    replace(&mut h, "Cylinder radius", "1000000.0000000001");
    assert!(h.state().cylinder_spec().is_err());
    assert_eq!(h.state().cylinder.radius, "1000000.0000000001");
}

#[test]
fn cylinder_rights_readiness_no_external_source_busy_disconnect_and_context_consent() {
    let mut h = harness(prepared());
    assert!(h.state().source.is_empty());
    assert!(h
        .get_by_label("Create capped cylinder package")
        .accesskit_node()
        .is_disabled());
    h.state_mut().start_import("/project".into(), 4, 7);
    assert!(!h.state().busy());
    assert!(h.state().status().contains("acknowledgement"));
    click(&mut h, "I have reviewed rights to distribute this content");
    assert!(!h
        .get_by_label("Create capped cylinder package")
        .accesskit_node()
        .is_disabled());
    let request = h.state().cylinder_request().unwrap();
    assert_eq!(request.spec.segments, 32);
    assert_eq!(request.license, PathBuf::from("/license.txt"));
    let (sender, receiver) = mpsc::channel();
    h.state_mut().pending = Some(receiver);
    h.run_steps(2);
    for label in [
        "Create cylinder package",
        "Cylinder radius",
        "Cylinder height",
        "Cylinder segments",
        "Cylinder asset ID",
        "License file",
        "Package destination directory",
        "Package name",
        "Package version",
        "I have reviewed rights to distribute this content",
        "Create capped cylinder package",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
    assert!(h
        .state_mut()
        .poll(Some(Path::new("/project")), 4, 7)
        .is_none());
    assert!(h.state().busy());
    drop(sender);
    assert!(h
        .state_mut()
        .poll(Some(Path::new("/project")), 4, 7)
        .is_none());
    assert!(!h.state().busy());
    assert!(!h.state().license_acknowledged);
    assert!(h.state().destination_identity.is_none());
    assert!(h
        .state()
        .status()
        .contains("Publication may have completed"));
    h.state_mut().license_acknowledged = true;
    h.state_mut()
        .sync_cylinder_context(Some(Path::new("/other-project")), 4, 7);
    assert!(!h.state().license_acknowledged);
    h.state_mut().license_acknowledged = true;
    h.state_mut()
        .sync_cylinder_context(Some(Path::new("/project")), 4, 8);
    assert!(!h.state().license_acknowledged);
    h.state_mut().cylinder.asset_id = "../bad.cylinder.json".into();
    assert!(h.state().cylinder_spec().is_err());
}

#[test]
fn cylinder_complete_creation_snapshot_rejects_stale_adoption_and_accepts_static_once() {
    for changed in [
        "current",
        "dormant-source",
        "project",
        "project-generation",
        "source-generation",
        "context-aba",
        "authoring-generation",
        "generation-exhausted",
        "radius",
        "height",
        "segments",
        "id",
        "license",
        "destination",
        "name",
        "version",
        "kind",
        "format",
        "mode-off",
        "mode-aba",
        "other-operation",
        "lexical",
        "wrong-report-kind",
        "wrong-report-asset",
        "backend-error",
    ] {
        let mut p = prepared();
        p.license_acknowledged = true;
        p.submitted_cylinder = Some(p.cylinder_snapshot(Path::new("/project"), 4, 7).unwrap());
        match changed {
            "dormant-source" => p.source = "/external-model-is-irrelevant.glb".into(),
            "context-aba" => {
                p.sync_cylinder_context(Some(Path::new("/other")), 4, 7);
                p.sync_cylinder_context(Some(Path::new("/project")), 4, 7);
            }
            "authoring-generation" => p.authoring_generation += 1,
            "generation-exhausted" => p.authoring_generation = u64::MAX,
            "radius" => p.cylinder.radius = "2".into(),
            "height" => p.cylinder.height = "2".into(),
            "segments" => p.cylinder.segments = Segments::Eight,
            "id" => p.cylinder.asset_id = "props/other.cylinder.json".into(),
            "license" => p.license.push('2'),
            "destination" => p.destination.push('2'),
            "name" => p.name.push('2'),
            "version" => p.version.push('2'),
            "kind" => p.kind = ModelKind::Animated,
            "format" => p.source_format = SourceFormat::RawGlb,
            "mode-off" => p.set_cylinder_mode(false),
            "mode-aba" => {
                p.set_cylinder_mode(false);
                p.set_cylinder_mode(true);
            }
            "other-operation" => p.static_uv.enabled = true,
            "lexical" => {
                p.cylinder.radius = "0.50".into();
                p.source_inputs_changed();
            }
            _ => {}
        }
        let mut completed = report();
        if changed == "wrong-report-kind" {
            completed.kind = ModelKind::Animated;
        }
        if changed == "wrong-report-asset" {
            completed.asset = "model.glb".into();
        }
        queue(
            &mut p,
            if changed == "backend-error" {
                Err("creation failed before publication".into())
            } else {
                Ok(completed)
            },
        );
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
        let expected = matches!(changed, "current" | "dormant-source");
        assert_eq!(result.is_some(), expected, "{changed}");
        assert_eq!(p.destination_identity.is_some(), expected, "{changed}");
        if let Some(report) = result {
            assert_eq!(report.kind, ModelKind::Static);
            assert_eq!(report.asset, "model.orrmodel");
        } else if changed != "backend-error" {
            assert!(
                p.status().contains("/package"),
                "completed destination retained for recovery: {changed}: {}",
                p.status()
            );
        }
        assert!(!p.license_acknowledged);
        assert!(!p.busy());
        assert!(p.poll(Some(Path::new("/project")), 4, 7).is_none());
    }
}
