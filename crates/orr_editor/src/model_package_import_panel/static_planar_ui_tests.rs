//! Planar UV CPU widgets and immutable current-source submissions; no render claims.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};

fn harness(p: Panel) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1600.0, 3500.0])
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
fn static_planar_widgets_default_off_exclusive_reset_busy_and_numeric_admission() {
    for raw in ["", "NaN", "inf", "-inf", "1e300", "-1e300"] {
        assert!(number(raw, false).is_err(), "{raw}");
    }
    for raw in ["0", "-0", "-1", "-1e-300", "1e-300"] {
        assert!(number(raw, true).is_err(), "{raw}");
    }
    assert_eq!(
        number("0.123456789012345", false).unwrap(),
        0.123456789012345
    );
    assert!(number("-1e-300", false).is_ok());
    assert!(number(&f64::from(f32::MAX).to_string(), false).is_ok());
    assert!(number(&f64::from(f32::from_bits(1)).to_string(), true).is_ok());
    let mut h = harness(Panel::default());
    assert!(!h.state().static_planar.enabled);
    assert!(h
        .query_by_label("Create static planar UV variant")
        .is_none());
    click(&mut h, "Cooked static model");
    assert!(!h.state().static_planar.enabled);
    for other in [
        "Create static UV variant",
        "Create static PNG skin",
        "Create static two-copy assembly",
        "Create static pivot variant",
        "Create static sampler variant",
        "Create static color variant",
    ] {
        click(&mut h, other);
        click(&mut h, "Create static planar UV variant");
        assert!(h.state().static_planar.enabled);
        let p = h.state();
        assert!(
            !p.static_uv.enabled
                && !p.static_texture.enabled
                && !p.static_assembly.enabled
                && !p.static_pivot.enabled
                && !p.static_sampler.enabled
                && !p.static_recolor.enabled
        );
        click(&mut h, other);
        assert!(!h.state().static_planar.enabled);
    }
    h.state_mut().glb_subtree.enabled = true;
    h.state_mut().pose_bake.enabled = true;
    h.state_mut().color_variant_enabled = true;
    h.state_mut().texture_variant_enabled = true;
    h.state_mut().trim.enabled = true;
    h.state_mut().sampler.enabled = true;
    h.state_mut().sequence.enabled = true;
    h.state_mut().uv.enabled = true;
    click(&mut h, "Create static planar UV variant");
    let p = h.state();
    assert!(
        !p.glb_subtree.enabled
            && !p.pose_bake.enabled
            && !p.color_variant_enabled
            && !p.texture_variant_enabled
            && !p.trim.enabled
            && !p.sampler.enabled
            && !p.sequence.enabled
            && !p.uv.enabled
    );
    assert!(p.static_planar.material.is_none());
    assert!(p.static_planar_request().is_err());
    h.state_mut().start_import("/project".into(), 4, 7);
    assert!(!h.state().busy());
    let (sender, receiver) = mpsc::channel();
    h.state_mut().pending = Some(receiver);
    h.run_steps(2);
    for label in [
        "Create static planar UV variant",
        "Static planar UV plane",
        "Planar origin X",
        "Planar origin Y",
        "Planar origin Z",
        "Planar units per repeat U",
        "Planar units per repeat V",
        "Static planar UV asset ID",
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
    assert_eq!(h.state().static_planar, Controls::default());
    assert!(!h.state().license_acknowledged);
}

#[test]
fn static_planar_actual_decimal_plane_asset_and_package_edits_revoke_consent() {
    let mut p = Panel::default();
    p.set_source_format(SourceFormat::CookedStatic);
    p.static_planar.enabled = true;
    let mut h = harness(p);
    for (label, value) in [
        ("Planar origin X", "0.123456789012345"),
        ("Planar origin Y", "-0.5"),
        ("Planar origin Z", "1.25"),
        ("Planar units per repeat U", "2"),
        ("Planar units per repeat V", "0.5"),
        ("Static planar UV asset ID", "variants/new.planar-uv.json"),
        ("Package destination directory", "/new-package"),
        ("Package name", "static-planar"),
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
    assert_eq!(
        h.state().static_planar.origin,
        ["0.123456789012345", "-0.5", "1.25"]
    );
    h.state_mut().license_acknowledged = true;
    click(&mut h, "Static planar UV plane");
    click(&mut h, "XY model plane");
    assert_eq!(h.state().static_planar.plane, Plane::Xy);
    assert!(!h.state().license_acknowledged);
    h.state_mut().license_acknowledged = true;
    replace(
        &mut h,
        "Cooked static model file (.model.json)",
        "/other.model.json",
    );
    assert_eq!(h.state().static_planar, Controls::default());
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
    let mut source = model.source().clone();
    source.materials[1].image = source.materials[0].image;
    source.materials.push(source.materials[1].clone());
    std::fs::write(
        path,
        orr_model::StaticModel::new(source)
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
}
#[cfg(target_os = "linux")]
fn inspected(source: &Path) -> Harness<'static, Panel> {
    use std::time::{Duration, Instant};
    let mut p = Panel::default();
    p.set_source_format(SourceFormat::CookedStatic);
    p.source = source.display().to_string();
    p.license = "/license.txt".into();
    p.destination = "/package".into();
    p.name = "static-planar".into();
    p.version = "1.0.0".into();
    let mut h = harness(p);
    click(&mut h, "Create static planar UV variant");
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
fn static_planar_real_used_material_all_planes_invalid_units_and_source_aba() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tile.model.json");
    fixture(&source);
    let original = std::fs::read(&source).unwrap();
    let mut h = inspected(&source);
    assert!(h.state().static_planar.material.is_none());
    assert!(h.state().inspected_hash.is_none());
    assert!(h.state().static_planar_request().is_err());
    assert!(h
        .get_by_label("Use material 2")
        .accesskit_node()
        .is_disabled());
    h.state_mut().license_acknowledged = true;
    click(&mut h, "Use material 1");
    assert!(!h.state().license_acknowledged);
    for (label, value) in [
        ("Planar origin X", "0.25"),
        ("Planar origin Y", "-0.5"),
        ("Planar origin Z", "1.25"),
        ("Planar units per repeat U", "2"),
        ("Planar units per repeat V", "0.5"),
    ] {
        replace(&mut h, label, value);
    }
    let hash = h.state().inspected_hash.clone();
    for (plane, label, name) in [
        (Plane::Xy, "XY model plane", "xy"),
        (Plane::Xz, "XZ model plane", "xz"),
        (Plane::Yz, "YZ model plane", "yz"),
    ] {
        h.state_mut().license_acknowledged = true;
        click(&mut h, "Static planar UV plane");
        click(&mut h, label);
        assert_eq!(h.state().static_planar.plane, plane);
        assert!(!h.state().license_acknowledged);
        let request = h.state().static_planar_request().unwrap().unwrap();
        assert_eq!(request.material, 1);
        assert_eq!(request.plane, name);
        assert_eq!(request.origin, [0.25, -0.5, 1.25]);
        assert_eq!(request.units_per_repeat, [2.0, 0.5]);
        assert_eq!(h.state().inspected_hash, hash);
    }
    for bad in ["-1e-300", "1e-300", "0", "inf"] {
        replace(&mut h, "Planar units per repeat U", bad);
        assert!(h.state().static_planar_request().is_err());
    }
    replace(&mut h, "Planar units per repeat U", "2");
    h.state_mut().license_acknowledged = true;
    h.run_steps(2);
    assert!(!h
        .get_by_label("Import cooked static package")
        .accesskit_node()
        .is_disabled());
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    for material in [2, u32::MAX] {
        h.state_mut().static_planar.material = Some(material);
        assert!(h.state().static_planar_request().is_err());
    }
    h.state_mut().static_planar.material = Some(1);
    for id in [
        "",
        "../bad.planar-uv.json",
        "bad.uv.json",
        "CON.planar-uv.json",
    ] {
        h.state_mut().static_planar.asset_id = id.into();
        assert!(h.state().static_planar_request().is_err(), "{id}");
    }
    h.state_mut().static_planar.asset_id = "variants/new.planar-uv.json".into();
    click(&mut h, "Create static planar UV variant");
    click(&mut h, "Create static planar UV variant");
    assert!(h.state().static_planar.material.is_none());
    click(&mut h, "Use material 1");
    h.state_mut().source = "/other.model.json".into();
    h.state_mut()
        .sync_inspection(Some(Path::new("/project")), 4, 7);
    h.state_mut().source = source.display().to_string();
    h.state_mut()
        .sync_inspection(Some(Path::new("/project")), 4, 7);
    assert!(h.state().static_planar.material.is_none());
    assert!(h.state().inspected_hash.is_none());
    assert!(h.state().static_planar_request().is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn static_planar_complete_snapshot_rejects_stale_inputs_and_accepts_static_report_once() {
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
        "material",
        "plane",
        "origin-x",
        "origin-y",
        "origin-z",
        "units-u",
        "units-v",
        "lexical",
        "signed-zero",
        "id",
        "disabled",
        "static-uv",
        "pose-bake",
    ] {
        let mut h = inspected(&source);
        click(&mut h, "Use material 1");
        let p = h.state_mut();
        if changed == "signed-zero" {
            p.static_planar.origin[0] = "-0".into();
        }
        p.submitted_static_planar = Some(
            p.static_planar_snapshot(Path::new("/project"), 4, 7)
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
            "material" => p.static_planar.material = Some(0),
            "plane" => p.static_planar.plane = Plane::Xy,
            "origin-x" => p.static_planar.origin[0] = "1".into(),
            "origin-y" => p.static_planar.origin[1] = "1".into(),
            "origin-z" => p.static_planar.origin[2] = "1".into(),
            "units-u" => p.static_planar.units_per_repeat[0] = "2".into(),
            "units-v" => p.static_planar.units_per_repeat[1] = "2".into(),
            "lexical" => {
                p.static_planar.origin[0] = "0.0".into();
                p.source_inputs_changed();
            }
            "signed-zero" => {
                p.static_planar.origin[0] = "0".into();
                p.source_inputs_changed();
            }
            "id" => p.static_planar.asset_id = "variants/other.planar-uv.json".into(),
            "disabled" => p.static_planar.enabled = false,
            "static-uv" => p.static_uv.enabled = true,
            "pose-bake" => p.pose_bake.enabled = true,
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
                    package: "static-planar".into(),
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
        }
        assert!(!p.license_acknowledged);
        assert!(!p.busy());
        assert!(p.poll(Some(Path::new("/project")), 4, 7).is_none());
    }
}
