use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn harness() -> Harness<'static, Panel> {
    Harness::builder()
        .with_size([1200.0, 1400.0])
        .build_ui_state(
            |ui, p: &mut Panel| p.show(ui, true, Some(Path::new("/project")), 4, 7),
            Panel::default(),
        )
}
fn open(h: &mut Harness<'_, Panel>) {
    h.run_steps(3);
    h.get_by_label("Import a model package").click_accesskit();
    h.run_steps(3);
}
#[test]
fn cooked_animated_selection_stages_matching_kind_and_revokes_prior_authority() {
    let mut p = Panel {
        license_acknowledged: true,
        destination_identity: Some(PathBuf::from("old")),
        ..Panel::default()
    };
    p.set_source_format(SourceFormat::CookedAnimated);
    assert_eq!(p.kind, ModelKind::Animated);
    assert!(!p.license_acknowledged);
    assert!(p.destination_identity.is_none());
    assert_eq!(p.request().source_format, SourceFormat::CookedAnimated);
    p.set_source_format(SourceFormat::RawGlb);
    assert_eq!(p.kind, ModelKind::Animated);
    p.set_source_format(SourceFormat::CookedStatic);
    assert_eq!(p.kind, ModelKind::Static);
}
#[test]
fn actual_format_widget_obeys_feature_gate_and_cooked_kind_constraints() {
    let mut h = harness();
    open(&mut h);
    let disabled = h
        .get_by_label("Cooked animated model")
        .accesskit_node()
        .is_disabled();
    assert_eq!(disabled, !cfg!(feature = "room-animated-models"));
    if !cfg!(feature = "room-animated-models") {
        assert_eq!(h.state().source_format, SourceFormat::RawGlb);
        assert!(!h.state().busy());
        assert!(h.query_by_label("Import cooked animated package").is_none());
        return;
    }
    h.state_mut().license_acknowledged = true;
    h.state_mut().destination_identity = Some(PathBuf::from("old"));
    h.get_by_label("Cooked animated model").click_accesskit();
    h.run_steps(3);
    assert_eq!(h.state().source_format, SourceFormat::CookedAnimated);
    assert_eq!(h.state().kind, ModelKind::Animated);
    assert!(!h.state().license_acknowledged);
    assert!(h.state().destination_identity.is_none());
    assert!(h
        .get_by_label("Static model")
        .accesskit_node()
        .is_disabled());
    assert!(!h
        .get_by_label("Animated model")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .query_by_label("Cooked animated model file (.animated.json)")
        .is_some());
    for label in [
        "Cooked animated model file (.animated.json)",
        "License file",
    ] {
        h.state_mut().license_acknowledged = true;
        h.state_mut().destination_identity = Some(PathBuf::from("old"));
        h.get_by_role_and_label(Role::TextInput, label).focus();
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, label)
            .type_text("changed");
        h.run_steps(3);
        assert!(!h.state().license_acknowledged);
        assert!(h.state().destination_identity.is_none());
    }
    let (_sender, receiver) = mpsc::channel();
    h.state_mut().pending = Some(receiver);
    h.run_steps(3);
    for label in [
        "Raw GLB",
        "Cooked static model",
        "Cooked animated model",
        "Static model",
        "Animated model",
        "I have reviewed rights to distribute this content",
        "Import cooked animated package",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
}
fn completion() -> Completion {
    Completion {
        source_format: SourceFormat::CookedAnimated,
        project_root: PathBuf::from("/project"),
        expected_destination: Ok(PathBuf::from("/package")),
        project_generation: 4,
        source_generation: 7,
        result: Ok(Report {
            root: PathBuf::from("/package"),
            package: "animated".into(),
            asset: "model.animated.json".into(),
            kind: ModelKind::Animated,
            source_sha256: "source".into(),
            license_sha256: "license".into(),
        }),
    }
}
#[test]
fn cooked_animated_completion_is_current_only_for_its_captured_identity() {
    for change in 0..5 {
        let mut p = Panel {
            source_format: SourceFormat::CookedAnimated,
            kind: ModelKind::Animated,
            license_acknowledged: true,
            ..Panel::default()
        };
        let (sender, receiver) = mpsc::channel();
        assert!(sender.send(completion()).is_ok());
        p.pending = Some(receiver);
        let root = if change == 1 {
            Path::new("/other")
        } else {
            Path::new("/project")
        };
        let project = if change == 2 { 5 } else { 4 };
        let source = if change == 3 { 8 } else { 7 };
        if change == 4 {
            p.set_source_format(SourceFormat::RawGlb);
        }
        let result = p.poll(Some(root), project, source);
        assert_eq!(result.is_some(), change == 0);
        assert!(!p.busy());
        assert!(!p.license_acknowledged);
        assert_eq!(p.destination_identity.is_some(), change == 0);
        if change != 0 {
            assert!(p.status().contains("prior project/source state"));
            assert!(p.status().contains("not selected"));
        }
    }
}
#[test]
fn cooked_animated_disconnection_preserves_uncertainty_without_retry() {
    let mut p = Panel {
        source_format: SourceFormat::CookedAnimated,
        kind: ModelKind::Animated,
        license_acknowledged: true,
        destination_identity: Some(PathBuf::from("old")),
        ..Panel::default()
    };
    let (sender, receiver) = mpsc::channel();
    p.pending = Some(receiver);
    drop(sender);
    assert!(p.poll(Some(Path::new("/project")), 4, 7).is_none());
    assert!(!p.busy());
    assert!(!p.license_acknowledged);
    assert!(p.destination_identity.is_none());
    assert!(p.status().contains("Publication may have completed"));
    assert!(p.status().contains("No automatic retry"));
    assert!(p.poll(Some(Path::new("/project")), 4, 7).is_none());
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
#[test]
fn actual_cooked_animated_widgets_create_exact_payload_package_without_activation() {
    use std::time::{Duration, Instant};
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let root = parent.join("project");
    std::fs::create_dir(&root).unwrap();
    let source = parent.join("courier.animated.json");
    let license = parent.join("license.txt");
    let destination = parent.join("new cooked package");
    let bytes = include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
    std::fs::write(&source, bytes).unwrap();
    std::fs::write(&license, b"CC0 explicit license").unwrap();
    let mut h = Harness::builder()
        .with_size([1200.0, 1400.0])
        .build_ui_state(
            |ui, state: &mut (Panel, PathBuf, Option<Report>)| {
                if let Some(report) = state.0.poll(Some(&state.1), 4, 7) {
                    state.2 = Some(report);
                }
                state.0.show(ui, true, Some(&state.1), 4, 7);
            },
            (Panel::default(), root.clone(), None),
        );
    h.run_steps(3);
    h.get_by_label("Import a model package").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Cooked animated model").click_accesskit();
    h.run_steps(3);
    for (label, value) in [
        (
            "Cooked animated model file (.animated.json)",
            source.to_str().unwrap(),
        ),
        ("License file", license.to_str().unwrap()),
        (
            "Package destination directory",
            destination.to_str().unwrap(),
        ),
        ("Package name", "cooked-character"),
        ("Package version", "1.0.0"),
    ] {
        h.get_by_role_and_label(Role::TextInput, label).focus();
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, label)
            .type_text(value);
        h.run_steps(3);
    }
    h.get_by_label("I have reviewed rights to distribute this content")
        .click_accesskit();
    h.run_steps(3);
    assert!(!destination.exists());
    assert!(!h
        .get_by_label("Import cooked animated package")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Import cooked animated package")
        .click_accesskit();
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().2.is_none() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "{}", h.state().0.status());
        std::thread::sleep(Duration::from_millis(5));
    }
    let report = h.state().2.as_ref().unwrap();
    assert_eq!(report.kind, ModelKind::Animated);
    assert_eq!(report.asset, "model.animated.json");
    assert_eq!(report.root, destination);
    assert_eq!(
        std::fs::read(destination.join("model.animated.json"))
            .unwrap()
            .as_slice(),
        bytes.as_slice()
    );
    assert_eq!(
        std::fs::read(destination.join("LICENSE.txt")).unwrap(),
        b"CC0 explicit license"
    );
    let manifest: orr_package::Manifest =
        serde_json::from_slice(&std::fs::read(destination.join("orr.package.json")).unwrap())
            .unwrap();
    assert!(manifest.capabilities.contains("animation"));
    assert!(manifest.dependencies.is_empty());
    assert_eq!(manifest.files.len(), 2);
    assert!(manifest.files.contains("model.animated.json"));
    assert!(!root.join(".orr").exists());
    assert!(h.state().0.status().contains("not installed or assigned"));
    assert!(!h.state().0.license_acknowledged);
}
