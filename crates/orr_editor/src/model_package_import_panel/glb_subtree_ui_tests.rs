//! Real CPU widgets and inspected source identities; no window/GPU.
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
fn glb_subtree_widgets_default_off_exclusive_format_reset_and_busy() {
    let mut h = harness(Panel::default());
    assert!(!h.state().glb_subtree.enabled);
    assert!(h.query_by_label("Import raw GLB package").is_some());
    assert!(h.query_by_label("Inspect GLB nodes").is_none());
    h.state_mut().static_uv.enabled = true;
    h.state_mut().static_texture.enabled = true;
    h.state_mut().static_assembly.enabled = true;
    h.state_mut().color_variant_enabled = true;
    h.state_mut().texture_variant_enabled = true;
    h.state_mut().trim.enabled = true;
    h.state_mut().sampler.enabled = true;
    h.state_mut().sequence.enabled = true;
    h.state_mut().uv.enabled = true;
    click(&mut h, "Create static GLB subtree");
    let p = h.state();
    assert!(p.glb_subtree.enabled);
    assert!(
        !p.static_uv.enabled
            && !p.static_texture.enabled
            && !p.static_assembly.enabled
            && !p.color_variant_enabled
            && !p.texture_variant_enabled
            && !p.trim.enabled
            && !p.sampler.enabled
            && !p.sequence.enabled
            && !p.uv.enabled
    );
    assert!(p.glb_subtree_request().is_err());
    assert!(h
        .get_by_label("Inspect GLB nodes")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .get_by_label("Import GLB subtree package")
        .accesskit_node()
        .is_disabled());
    assert!(h.query_by_label("Preview animated variant").is_none());
    h.state_mut().start_import("/project".into(), 4, 7);
    assert!(!h.state().busy());
    let (sender, receiver) = mpsc::channel();
    h.state_mut().pending = Some(receiver);
    h.run_steps(2);
    for label in [
        "Create static GLB subtree",
        "GLB subtree asset ID",
        "Inspect GLB nodes",
        "Import GLB subtree package",
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
    assert_eq!(h.state().glb_subtree, Controls::default());
    assert!(h.query_by_label("Create static GLB subtree").is_none());
    click(&mut h, "Raw GLB");
    assert!(!h.state().glb_subtree.enabled);
    assert!(h.query_by_label("Import raw GLB package").is_some());
}

#[test]
fn glb_subtree_actual_asset_and_package_input_edits_revoke_consent() {
    let mut p = Panel::default();
    p.glb_subtree.enabled = true;
    let mut h = harness(p);
    for (label, value) in [
        ("GLB subtree asset ID", "props/new.glb"),
        ("Package destination directory", "/new-package"),
        ("Package name", "glb-subtree"),
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
    h.state_mut().license_acknowledged = true;
    replace(&mut h, "Raw GLB file", "/other.glb");
    assert_eq!(h.state().glb_subtree, Controls::default());
    assert!(!h.state().license_acknowledged);
}

#[cfg(target_os = "linux")]
fn inspected(source: &Path) -> Harness<'static, Panel> {
    use std::time::{Duration, Instant};
    let mut p = Panel::default();
    p.source = source.display().to_string();
    p.license = "/license.txt".into();
    p.destination = "/package".into();
    p.name = "glb-subtree".into();
    p.version = "1.0.0".into();
    let mut h = harness(p);
    click(&mut h, "Create static GLB subtree");
    click(&mut h, "Inspect GLB nodes");
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
fn node(h: &Harness<'_, Panel>) -> u32 {
    h.state()
        .inspection
        .current_glb_catalog(&h.state().source, ModelKind::Static, SourceFormat::RawGlb)
        .unwrap()
        .nodes
        .last()
        .unwrap()
        .index
}

#[cfg(target_os = "linux")]
#[test]
fn glb_subtree_real_inspection_choice_identity_validation_and_pending_aba() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("model.glb");
    let bytes = include_bytes!("../../../orr_model/tests/fixtures/model.glb");
    std::fs::write(&source, bytes).unwrap();
    let mut h = inspected(&source);
    let index = node(&h);
    assert!(h.state().inspected_hash.is_none());
    assert!(h.state().glb_subtree_request().is_err());
    click(&mut h, &format!("Use node {index}"));
    assert_eq!(
        h.state().glb_subtree_request().unwrap().unwrap().node_index,
        index
    );
    assert!(!h.state().license_acknowledged);
    assert_eq!(std::fs::read(&source).unwrap().as_slice(), bytes);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    for id in ["", "../bad.glb", "bad.assemble.json", "CON.glb"] {
        h.state_mut().glb_subtree.asset_id = id.into();
        assert!(h.state().glb_subtree_request().is_err(), "{id}");
    }
    h.state_mut().glb_subtree.asset_id = "props/new.glb".into();
    h.state_mut().glb_subtree.node_index = Some(u32::MAX);
    assert!(h.state().glb_subtree_request().is_err());
    h.state_mut().glb_subtree.node_index = Some(index);
    let catalog = h
        .state()
        .inspection
        .current_glb_catalog(&h.state().source, ModelKind::Static, SourceFormat::RawGlb)
        .unwrap()
        .clone();
    let complete = h
        .state_mut()
        .hold_glb_inspection_for_test(Path::new("/project"), 4, 7, false);
    h.run_steps(2);
    assert!(h.state().busy());
    assert!(h
        .get_by_label("Inspect GLB nodes")
        .accesskit_node()
        .is_disabled());
    h.state_mut().source = "/other.glb".into();
    h.state_mut()
        .sync_inspection(Some(Path::new("/project")), 4, 7);
    h.state_mut().source = source.display().to_string();
    complete(Ok(catalog.clone()));
    h.state_mut()
        .sync_inspection(Some(Path::new("/project")), 4, 7);
    assert!(!h.state().busy());
    assert!(h.state().glb_subtree.node_index.is_none());
    assert!(h.state().inspected_hash.is_none());
    assert!(h.state().glb_subtree_request().is_err());
    h.run_steps(2);
    click(&mut h, "Create static GLB subtree");
    click(&mut h, "Create static GLB subtree");
    assert!(h.state().glb_subtree.node_index.is_none());
    assert!(h.state().glb_subtree_request().is_err());
    assert!(h.query_by_label(&format!("Use node {index}")).is_none());
    let complete = h
        .state_mut()
        .hold_glb_inspection_for_test(Path::new("/project"), 4, 7, true);
    h.state_mut().inspection.invalidate();
    h.state_mut().inspection.invalidate();
    assert!(h.state().busy());
    complete(Ok(catalog));
    h.state_mut()
        .sync_inspection(Some(Path::new("/project")), 4, 7);
    assert!(!h.state().busy());
    assert!(h.state().glb_subtree_request().is_err());
    h.run_steps(2);
    assert!(h
        .get_by_label("Inspect GLB nodes")
        .accesskit_node()
        .is_disabled());
}

#[cfg(target_os = "linux")]
#[test]
fn glb_subtree_complete_snapshot_rejects_stale_inputs_and_accepts_current_once() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("model.glb");
    std::fs::write(
        &source,
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
    )
    .unwrap();
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
        "node",
        "id",
        "disabled",
        "static-uv",
        "color",
    ] {
        let mut h = inspected(&source);
        let index = node(&h);
        click(&mut h, &format!("Use node {index}"));
        let p = h.state_mut();
        p.submitted_glb_subtree =
            Some(p.glb_subtree_snapshot(Path::new("/project"), 4, 7).unwrap());
        p.submitted_inspected_hash = Some(p.inspected_hash.clone());
        p.submitted_authoring_generation = Some(p.authoring_generation);
        p.license_acknowledged = true;
        match changed {
            "source" => p.source = "/other.glb".into(),
            "license" => p.license.push('2'),
            "destination" => p.destination.push('2'),
            "name" => p.name.push('2'),
            "version" => p.version.push('2'),
            "kind" => p.kind = ModelKind::Animated,
            "format" => p.source_format = SourceFormat::CookedStatic,
            "hash" => p.inspected_hash = Some("b".repeat(64)),
            "node" => p.glb_subtree.node_index = Some(u32::MAX),
            "id" => p.glb_subtree.asset_id = "props/other.glb".into(),
            "disabled" => p.glb_subtree.enabled = false,
            "static-uv" => p.static_uv.enabled = true,
            "color" => p.color_variant_enabled = true,
            "authoring-generation" => p.authoring_generation += 1,
            _ => {}
        }
        let (sender, receiver) = mpsc::channel();
        p.pending = Some(receiver);
        sender
            .send(Completion {
                source_format: SourceFormat::RawGlb,
                project_root: "/project".into(),
                expected_destination: Ok("/package".into()),
                project_generation: 4,
                source_generation: 7,
                result: Ok(Report {
                    root: "/package".into(),
                    package: "glb-subtree".into(),
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
