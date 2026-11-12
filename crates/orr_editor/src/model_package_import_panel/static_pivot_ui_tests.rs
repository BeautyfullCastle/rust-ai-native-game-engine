//! Real pivot CPU widgets, source choice and immutable worker identity.
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
fn static_pivot_widgets_default_off_exclusive_reset_busy_and_cli_number_bounds() {
    for raw in ["", "NaN", "inf", "-inf", "1e300", "1000000.1", "-1000000.1"] {
        assert!(number(raw).is_err(), "{raw}");
    }
    assert_eq!(number("1000000.01").unwrap() as f32, 1_000_000.0);
    assert_eq!(number("-1000000.01").unwrap() as f32, -1_000_000.0);
    assert_eq!(
        (number("-1e-300").unwrap() as f32).to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(number("0.123456789012345").unwrap(), 0.123456789012345);
    let mut h = harness(Panel::default());
    assert!(!h.state().static_pivot.enabled);
    assert!(h.query_by_label("Create static pivot variant").is_none());
    click(&mut h, "Cooked static model");
    assert!(!h.state().static_pivot.enabled);
    for other in [
        "Create static UV variant",
        "Create static PNG skin",
        "Create static two-copy assembly",
    ] {
        click(&mut h, other);
        click(&mut h, "Create static pivot variant");
        assert!(h.state().static_pivot.enabled);
        assert!(
            !h.state().static_uv.enabled
                && !h.state().static_texture.enabled
                && !h.state().static_assembly.enabled
        );
        click(&mut h, other);
        assert!(!h.state().static_pivot.enabled);
    }
    h.state_mut().glb_subtree.enabled = true;
    h.state_mut().color_variant_enabled = true;
    h.state_mut().texture_variant_enabled = true;
    h.state_mut().trim.enabled = true;
    h.state_mut().sampler.enabled = true;
    h.state_mut().sequence.enabled = true;
    h.state_mut().uv.enabled = true;
    click(&mut h, "Create static pivot variant");
    let p = h.state();
    assert!(
        !p.glb_subtree.enabled
            && !p.color_variant_enabled
            && !p.texture_variant_enabled
            && !p.trim.enabled
            && !p.sampler.enabled
            && !p.sequence.enabled
            && !p.uv.enabled
    );
    assert!(p.static_pivot_request().is_err());
    assert!(h
        .get_by_label("Use inspected source for pivot")
        .accesskit_node()
        .is_disabled());
    assert!(h.query_by_label("Preview animated variant").is_none());
    h.state_mut().start_import("/project".into(), 4, 7);
    assert!(!h.state().busy());
    let (sender, receiver) = mpsc::channel();
    h.state_mut().pending = Some(receiver);
    h.run_steps(2);
    for label in [
        "Create static pivot variant",
        "Use inspected source for pivot",
        "Static pivot X",
        "Static pivot Y",
        "Static pivot Z",
        "Static pivot variant asset ID",
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
    assert_eq!(h.state().static_pivot, Controls::default());
    assert!(!h.state().license_acknowledged);
}

#[test]
fn static_pivot_actual_xyz_asset_and_package_edits_revoke_consent_without_clamping() {
    let mut p = Panel::default();
    p.set_source_format(SourceFormat::CookedStatic);
    p.static_pivot.enabled = true;
    let mut h = harness(p);
    for (label, value) in [
        ("Static pivot X", "1000000.01"),
        ("Static pivot Y", "-2.25"),
        ("Static pivot Z", "0.5"),
        ("Static pivot variant asset ID", "variants/new.pivot.json"),
        ("Package destination directory", "/new-package"),
        ("Package name", "static-pivot"),
        ("Package version", "1.0.0"),
        ("License file", "/license.txt"),
    ] {
        h.state_mut().license_acknowledged = true;
        h.state_mut().destination_identity = Some("/old".into());
        let generation = h.state().authoring_generation;
        replace(&mut h, label, value);
        assert!(!h.state().license_acknowledged, "{label}");
        assert!(h.state().destination_identity.is_none());
        assert!(h.state().authoring_generation > generation);
    }
    assert_eq!(h.state().static_pivot.pivot, ["1000000.01", "-2.25", "0.5"]);
    h.state_mut().license_acknowledged = true;
    replace(
        &mut h,
        "Cooked static model file (.model.json)",
        "/other.model.json",
    );
    assert_eq!(h.state().static_pivot, Controls::default());
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
    p.name = "static-pivot".into();
    p.version = "1.0.0".into();
    let mut h = harness(p);
    click(&mut h, "Create static pivot variant");
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
fn static_pivot_real_inspection_requires_source_choice_valid_id_and_fresh_source_after_aba() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tile.model.json");
    fixture(&source);
    let original = std::fs::read(&source).unwrap();
    let mut h = inspected(&source);
    assert!(h.state().static_pivot_request().is_err());
    assert!(h.state().inspected_hash.is_none());
    assert!(h
        .get_by_label("Use material 0")
        .accesskit_node()
        .is_disabled());
    h.state_mut().license_acknowledged = true;
    click(&mut h, "Use inspected source for pivot");
    assert_eq!(
        h.state().static_pivot_request().unwrap().unwrap().pivot,
        [0.0; 3]
    );
    assert!(!h.state().license_acknowledged);
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    for id in ["", "../bad.pivot.json", "bad.uv.json", "CON.pivot.json"] {
        h.state_mut().static_pivot.asset_id = id.into();
        assert!(h.state().static_pivot_request().is_err(), "{id}");
    }
    h.state_mut().static_pivot.asset_id = "variants/new.pivot.json".into();
    h.state_mut().static_pivot.pivot[0] = "1000000.01".into();
    assert!(h.state().static_pivot_request().is_ok());
    h.state_mut().static_pivot.pivot[0] = "1000000.1".into();
    assert!(h.state().static_pivot_request().is_err());
    h.state_mut().static_pivot.pivot[0] = "0".into();
    click(&mut h, "Create static pivot variant");
    click(&mut h, "Create static pivot variant");
    assert!(h.state().static_pivot.source_sha256.is_none());
    assert!(h.state().static_pivot_request().is_err());
    click(&mut h, "Use inspected source for pivot");
    assert!(h.state().static_pivot_request().is_ok());
    h.state_mut().source = "/other.model.json".into();
    h.state_mut()
        .sync_inspection(Some(Path::new("/project")), 4, 7);
    h.state_mut().source = source.display().to_string();
    h.state_mut()
        .sync_inspection(Some(Path::new("/project")), 4, 7);
    assert!(h.state().static_pivot.source_sha256.is_none());
    assert!(h.state().inspected_hash.is_none());
    assert!(h.state().static_pivot_request().is_err());
    h.run_steps(2);
    assert!(h
        .get_by_label("Use inspected source for pivot")
        .accesskit_node()
        .is_disabled());
}

#[cfg(target_os = "linux")]
#[test]
fn static_pivot_complete_snapshot_rejects_stale_inputs_and_accepts_current_once() {
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
        "x",
        "y",
        "z",
        "lexical",
        "signed-zero",
        "id",
        "disabled",
        "static-uv",
        "glb-subtree",
        "color",
    ] {
        let mut h = inspected(&source);
        click(&mut h, "Use inspected source for pivot");
        let p = h.state_mut();
        p.submitted_static_pivot = Some(
            p.static_pivot_snapshot(Path::new("/project"), 4, 7)
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
            "choice" => p.static_pivot.source_sha256 = None,
            "x" => p.static_pivot.pivot[0] = "1".into(),
            "y" => p.static_pivot.pivot[1] = "1".into(),
            "z" => p.static_pivot.pivot[2] = "1".into(),
            "lexical" => {
                p.static_pivot.pivot[0] = "0.0".into();
                p.source_inputs_changed();
            }
            "signed-zero" => {
                p.static_pivot.pivot[0] = "-0".into();
                p.source_inputs_changed();
            }
            "id" => p.static_pivot.asset_id = "variants/other.pivot.json".into(),
            "disabled" => p.static_pivot.enabled = false,
            "static-uv" => p.static_uv.enabled = true,
            "glb-subtree" => p.glb_subtree.enabled = true,
            "color" => p.color_variant_enabled = true,
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
                    package: "static-pivot".into(),
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
