//! Two-copy assembly CPU widgets and worker admission without GPU/window.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};

fn harness(p: Panel) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1600.0, 4000.0])
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

#[test]
fn static_assembly_widgets_default_off_exclusive_reset_busy_and_decimal_bounds() {
    for raw in ["", "NaN", "inf", "3.5e38", "-3.5e38"] {
        assert!(number(raw, false).is_err());
    }
    for raw in ["0", "-0", "-1", "1e-99"] {
        assert!(number(raw, true).is_err());
    }
    assert_eq!(
        number("0.123456789012345", false).unwrap(),
        0.123456789012345
    );
    assert!(number(&f64::from(f32::MAX).to_string(), false).is_ok());
    let mut h = harness(Panel::default());
    assert!(!h.state().static_assembly.enabled);
    assert!(h
        .query_by_label("Create static two-copy assembly")
        .is_none());
    click(&mut h, "Cooked static model");
    assert!(!h.state().static_assembly.enabled);
    assert_eq!(h.state().static_assembly.parts[0], PartControls::default());
    assert_eq!(h.state().static_assembly.parts[1].offset, ["2", "0", "0"]);
    for other in ["Create static UV variant", "Create static PNG skin"] {
        click(&mut h, other);
        click(&mut h, "Create static two-copy assembly");
        assert!(h.state().static_assembly.enabled);
        assert!(!h.state().static_uv.enabled && !h.state().static_texture.enabled);
        click(&mut h, other);
        assert!(!h.state().static_assembly.enabled);
    }
    h.state_mut().color_variant_enabled = true;
    h.state_mut().texture_variant_enabled = true;
    h.state_mut().trim.enabled = true;
    h.state_mut().sampler.enabled = true;
    h.state_mut().sequence.enabled = true;
    h.state_mut().uv.enabled = true;
    click(&mut h, "Create static two-copy assembly");
    let p = h.state();
    assert!(
        !p.color_variant_enabled
            && !p.texture_variant_enabled
            && !p.trim.enabled
            && !p.sampler.enabled
            && !p.sequence.enabled
            && !p.uv.enabled
    );
    assert!(p.static_assembly_request().is_err());
    assert!(h
        .get_by_label("Use inspected source for assembly")
        .accesskit_node()
        .is_disabled());
    assert!(h.query_by_label("Preview animated variant").is_none());
    h.state_mut().start_import("/project".into(), 4, 7);
    assert!(!h.state().busy());
    let (sender, receiver) = mpsc::channel();
    h.state_mut().pending = Some(receiver);
    h.run_steps(2);
    for label in [
        "Create static two-copy assembly",
        "Use inspected source for assembly",
        "Assembly part 1 offset X",
        "Assembly part 2 scale Z",
        "Assembly part 2 yaw",
        "Static assembly variant asset ID",
        "Inspect static source",
        "Import cooked static package",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
    drop(sender);
    h.state_mut().poll(Some(Path::new("/project")), 4, 7);
    h.run_steps(2);
    click(&mut h, "Raw GLB");
    assert_eq!(h.state().static_assembly, Controls::default());
    assert!(!h.state().license_acknowledged);
}

#[test]
fn static_assembly_actual_transform_and_package_edits_revoke_consent() {
    let mut p = Panel::default();
    p.set_source_format(SourceFormat::CookedStatic);
    p.static_assembly.enabled = true;
    let mut h = harness(p);
    for part in 1..=2 {
        for axis in ["X", "Y", "Z"] {
            for (field, value) in [("offset", "3"), ("scale", "0.5")] {
                let label = format!("Assembly part {part} {field} {axis}");
                h.state_mut().license_acknowledged = true;
                h.state_mut().destination_identity = Some("/old".into());
                let generation = h.state().authoring_generation;
                replace(&mut h, &label, value);
                assert!(!h.state().license_acknowledged, "{label}");
                assert!(h.state().destination_identity.is_none());
                assert!(h.state().authoring_generation > generation);
            }
        }
        h.state_mut().license_acknowledged = true;
        click(&mut h, &format!("Assembly part {part} yaw"));
        click(&mut h, "90 degrees");
        assert!(!h.state().license_acknowledged);
        assert_eq!(h.state().static_assembly.parts[part - 1].quarter_turns, 1);
    }
    for (label, value) in [
        (
            "Static assembly variant asset ID",
            "variants/new.assemble.json",
        ),
        ("Package destination directory", "/new-package"),
        ("Package name", "static-pair"),
        ("Package version", "1.0.0"),
        ("License file", "/license.txt"),
    ] {
        h.state_mut().license_acknowledged = true;
        let generation = h.state().authoring_generation;
        replace(&mut h, label, value);
        assert!(!h.state().license_acknowledged, "{label}");
        assert!(h.state().authoring_generation > generation);
    }
    h.state_mut().license_acknowledged = true;
    replace(
        &mut h,
        "Cooked static model file (.model.json)",
        "/other.model.json",
    );
    assert_eq!(h.state().static_assembly, Controls::default());
    assert!(!h.state().license_acknowledged);
}

#[cfg(target_os = "linux")]
fn fixture(path: &Path) {
    let model = orr_model::import::import_with_resolver(
        "props/tile.glb",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded fixture"),
    )
    .unwrap();
    std::fs::write(path, model.to_bytes().unwrap()).unwrap();
}
#[cfg(target_os = "linux")]
fn inspected(source: &Path) -> Harness<'static, Panel> {
    use std::time::{Duration, Instant};
    let mut p = Panel::default();
    p.set_source_format(SourceFormat::CookedStatic);
    p.source = source.display().to_string();
    p.license = "/license.txt".into();
    p.destination = "/package".into();
    p.name = "static-pair".into();
    p.version = "1.0.0".into();
    let mut h = harness(p);
    click(&mut h, "Create static two-copy assembly");
    click(&mut h, "Inspect static source");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.state().busy() {
        h.state_mut().poll(Some(Path::new("/project")), 4, 7);
        h.run_steps(2);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    h
}

#[cfg(target_os = "linux")]
#[test]
fn static_assembly_requires_explicit_inspected_source_and_validates_placement() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tile.model.json");
    fixture(&source);
    let original = std::fs::read(&source).unwrap();
    let mut h = inspected(&source);
    assert!(h.state().static_assembly_request().is_err());
    assert!(h.state().inspected_hash.is_none());
    assert!(h
        .get_by_label("Use material 0")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Use inspected source for assembly");
    let spec = h.state().static_assembly_request().unwrap().unwrap();
    assert_eq!(spec.parts[0].offset, [0.0; 3]);
    assert_eq!(spec.parts[1].offset, [2.0, 0.0, 0.0]);
    assert!(!h.state().license_acknowledged);
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    let p = h.state_mut();
    for id in [
        "",
        "../bad.assemble.json",
        "bad.uv.json",
        "CON.assemble.json",
    ] {
        p.static_assembly.asset_id = id.into();
        assert!(p.static_assembly_request().is_err(), "{id}");
    }
    p.static_assembly.asset_id = "variants/new.assemble.json".into();
    p.static_assembly.parts[1].quarter_turns = 4;
    assert!(p.static_assembly_request().is_err());
    p.static_assembly.parts[1].quarter_turns = 1;
    // Positive representable values can still fail the shared normal-matrix check.
    p.static_assembly.parts[1].scale = ["1e-20".into(), "1e-20".into(), "1e-20".into()];
    assert!(p.static_assembly_request().is_err());
    p.static_assembly.parts[1].scale = ["2".into(), "1".into(), "0.5".into()];
    assert!(p.static_assembly_request().unwrap().is_some());
    p.source = "/other.model.json".into();
    p.sync_inspection(Some(Path::new("/project")), 4, 7);
    p.source = source.display().to_string();
    p.sync_inspection(Some(Path::new("/project")), 4, 7);
    assert!(p.static_assembly_request().is_err());
    assert!(p.static_assembly.source_sha256.is_none());
    assert!(p.inspected_hash.is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn static_assembly_snapshot_rejects_stale_inputs_and_accepts_current_once() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tile.model.json");
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
        "choice",
        "first-offset",
        "second-offset",
        "scale",
        "yaw",
        "id",
        "disabled",
        "static-uv",
        "static-texture",
    ] {
        let mut h = inspected(&source);
        click(&mut h, "Use inspected source for assembly");
        let p = h.state_mut();
        p.submitted_static_assembly = Some(
            p.static_assembly_snapshot(Path::new("/project"), 4, 7)
                .unwrap(),
        );
        p.submitted_inspected_hash = Some(p.inspected_hash.clone());
        p.submitted_authoring_generation = Some(p.authoring_generation);
        p.license_acknowledged = true;
        match changed {
            "source" => p.source = "/other.model.json".into(),
            "license" => p.license.push('2'),
            "destination" => p.destination.push('2'),
            "name" => p.name.push('2'),
            "version" => p.version.push('2'),
            "kind" => p.kind = ModelKind::Animated,
            "format" => p.source_format = SourceFormat::RawGlb,
            "hash" => p.inspected_hash = Some("b".repeat(64)),
            "choice" => p.static_assembly.source_sha256 = None,
            "first-offset" => p.static_assembly.parts[0].offset[0] = "1".into(),
            "second-offset" => p.static_assembly.parts[1].offset[2] = "1".into(),
            "scale" => p.static_assembly.parts[1].scale[1] = "2".into(),
            "yaw" => p.static_assembly.parts[1].quarter_turns = 1,
            "id" => p.static_assembly.asset_id = "variants/other.assemble.json".into(),
            "disabled" => p.static_assembly.enabled = false,
            "static-uv" => p.static_uv.enabled = true,
            "static-texture" => p.static_texture.enabled = true,
            "authoring-generation" => p.authoring_generation += 1,
            _ => {}
        }
        let (sender, receiver) = mpsc::channel();
        p.pending = Some(receiver);
        sender
            .send(Completion {
                source_format: SourceFormat::CookedStatic,
                project_root: "/project".into(),
                expected_destination: Ok("/package".into()),
                project_generation: 4,
                source_generation: 7,
                result: Ok(Report {
                    root: "/package".into(),
                    package: "static-pair".into(),
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
        assert!(!p.license_acknowledged);
        assert!(!p.busy());
        assert!(p.poll(Some(Path::new("/project")), 4, 7).is_none());
    }
}
