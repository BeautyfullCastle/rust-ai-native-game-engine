//! Static UV CPU widget and worker identity coverage, with no GPU/window.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};

fn harness(p: Panel) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1600.0, 3200.0])
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
fn static_uv_decimal_admission_preserves_f64_and_rejects_f32_overflow_underflow() {
    for raw in ["", "NaN", "inf", "-inf", "3.5e38", "-3.5e38"] {
        assert!(number(raw, false).is_err(), "{raw}");
    }
    for raw in ["0", "-0", "-1", "1e-99", "-1e-99", "3.5e38"] {
        assert!(number(raw, true).is_err(), "{raw}");
    }
    assert_eq!(
        number("0.123456789012345", false).unwrap(),
        0.123456789012345
    );
    assert_eq!(number("-1000000", false).unwrap(), -1000000.0);
    assert!(number(&f64::from(f32::MAX).to_string(), false).is_ok());
    assert!(number(&f64::from(f32::from_bits(1)).to_string(), true).is_ok());
}

#[test]
fn static_uv_actual_widgets_default_off_format_reset_and_busy_fields() {
    let mut h = harness(Panel::default());
    assert!(!h.state().static_uv.enabled);
    assert!(h.query_by_label("Create static UV variant").is_none());
    click(&mut h, "Cooked static model");
    assert!(!h.state().static_uv.enabled);
    assert_eq!(h.state().kind, ModelKind::Static);
    assert!(h.query_by_label("Inspect static source").is_some());
    assert!(h.query_by_label("Preview animated variant").is_none());
    assert!(h
        .get_by_label("Inspect static source")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Create static UV variant");
    assert!(h.state().static_uv_request().is_err());
    assert!(h
        .get_by_label("Import cooked static package")
        .accesskit_node()
        .is_disabled());
    h.state_mut().start_import("/project".into(), 4, 7);
    assert!(!h.state().busy());
    let (sender, receiver) = mpsc::channel();
    h.state_mut().pending = Some(receiver);
    h.run_steps(2);
    for label in [
        "Create static UV variant",
        "Static UV offset U",
        "Static UV offset V",
        "Static UV scale U",
        "Static UV scale V",
        "Static UV quarter-turn rotation",
        "Static UV variant asset ID",
        "Inspect static source",
        "Import cooked static package",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
    drop(sender);
    assert!(h
        .state_mut()
        .poll(Some(Path::new("/project")), 4, 7)
        .is_none());
    h.run_steps(2);
    click(&mut h, "Raw GLB");
    assert_eq!(h.state().static_uv, Controls::default());
    assert!(!h.state().license_acknowledged);
    assert!(h.query_by_label("Create static UV variant").is_none());
}

#[test]
fn static_uv_editing_actual_settings_and_package_inputs_revokes_consent() {
    let mut p = Panel::default();
    p.set_source_format(SourceFormat::CookedStatic);
    p.static_uv.enabled = true;
    let mut h = harness(p);
    for (label, value) in [
        ("Static UV offset U", "0.25"),
        ("Static UV offset V", "-0.5"),
        ("Static UV scale U", "2"),
        ("Static UV scale V", "0.5"),
        ("Static UV variant asset ID", "variants/tile.uv.json"),
        ("Package destination directory", "/new-package"),
        ("Package name", "static-tile"),
        ("Package version", "1.0.0"),
        ("License file", "/license.txt"),
    ] {
        h.state_mut().license_acknowledged = true;
        h.state_mut().destination_identity = Some("/old-package".into());
        let generation = h.state().authoring_generation;
        replace(&mut h, label, value);
        assert!(!h.state().license_acknowledged, "{label}");
        assert!(h.state().destination_identity.is_none(), "{label}");
        assert!(h.state().authoring_generation > generation, "{label}");
    }
    h.state_mut().license_acknowledged = true;
    click(&mut h, "Static UV quarter-turn rotation");
    click(&mut h, "90 degrees");
    assert_eq!(h.state().static_uv.quarter_turns, 1);
    assert!(!h.state().license_acknowledged);
    h.state_mut().license_acknowledged = true;
    replace(
        &mut h,
        "Cooked static model file (.model.json)",
        "/other.model.json",
    );
    assert_eq!(h.state().static_uv, Controls::default());
    assert!(!h.state().license_acknowledged);
}

#[cfg(target_os = "linux")]
fn inspected(source: &Path) -> Harness<'static, Panel> {
    use std::time::{Duration, Instant};
    let mut p = Panel::default();
    p.set_source_format(SourceFormat::CookedStatic);
    p.source = source.display().to_string();
    p.license = "/license.txt".into();
    p.destination = "/package".into();
    p.name = "static-tile".into();
    p.version = "1.0.0".into();
    let mut h = harness(p);
    click(&mut h, "Create static UV variant");
    click(&mut h, "Inspect static source");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.state().busy() {
        h.state_mut().poll(Some(Path::new("/project")), 4, 7);
        h.run_steps(2);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    click(&mut h, "Use material 0");
    assert!(h.state().static_uv_request().unwrap().is_some());
    h
}

#[cfg(target_os = "linux")]
#[test]
fn static_uv_inspected_snapshot_rejects_every_stale_input_and_accepts_current_once() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tile.model.json");
    let model = orr_model::import::import_with_resolver(
        "props/tile.glb",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded fixture"),
    )
    .unwrap();
    std::fs::write(&source, model.to_bytes().unwrap()).unwrap();
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
        "material",
        "offset",
        "scale",
        "rotation",
        "id",
        "disabled",
    ] {
        let mut h = inspected(&source);
        let p = h.state_mut();
        let snapshot = p.static_uv_snapshot(Path::new("/project"), 4, 7).unwrap();
        p.submitted_static_uv = Some(snapshot);
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
            "material" => p.static_uv.material = Some(u32::MAX),
            "offset" => p.static_uv.offset[0] = "0.25".into(),
            "scale" => p.static_uv.scale[1] = "0.5".into(),
            "rotation" => p.static_uv.quarter_turns = 1,
            "id" => p.static_uv.asset_id = "variants/other.uv.json".into(),
            "disabled" => p.static_uv.enabled = false,
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
                    package: "static-tile".into(),
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
