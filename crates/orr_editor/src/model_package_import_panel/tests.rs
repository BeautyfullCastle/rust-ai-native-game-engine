use super::{Completion, Panel};
use crate::model_package_import::{Report, SourceFormat};
use orr_model_bindings::model_bindings::ModelKind;
use std::{path::PathBuf, sync::mpsc};

fn report(root: &str) -> Report {
    Report {
        root: PathBuf::from(root),
        package: "sample.model".into(),
        asset: "model.glb".into(),
        kind: ModelKind::Static,
        source_sha256: "source-digest".into(),
        license_sha256: "license-digest".into(),
    }
}

fn queue(panel: &mut Panel, completion: Completion) {
    let (sender, receiver) = mpsc::channel();
    assert!(sender.send(completion).is_ok());
    panel.pending = Some(receiver);
}

fn completion(
    project_root: &str,
    expected_destination: Result<&str, &str>,
    project_generation: u64,
    source_generation: u64,
    result: Result<Report, &str>,
) -> Completion {
    Completion {
        source_format: SourceFormat::RawGlb,
        project_root: PathBuf::from(project_root),
        expected_destination: expected_destination
            .map(PathBuf::from)
            .map_err(str::to_owned),
        project_generation,
        source_generation,
        result: result.map_err(str::to_owned),
    }
}

#[test]
fn matching_canonical_completion_is_returned_once_and_clears_busy() {
    let mut panel = Panel {
        license_acknowledged: true,
        ..Panel::default()
    };
    queue(
        &mut panel,
        completion(
            r"C:\projects\room",
            Ok(r"\\?\C:\packages\sample.model"),
            4,
            7,
            Ok(report(r"\\?\C:\packages\sample.model")),
        ),
    );

    assert!(panel.busy());
    let imported = panel
        .poll(Some(PathBuf::from(r"C:\projects\room").as_path()), 4, 7)
        .expect("current matching result");
    assert_eq!(
        imported.root,
        PathBuf::from(r"\\?\C:\packages\sample.model")
    );
    assert!(!panel.busy());
    assert!(!panel.license_acknowledged);
    assert_eq!(panel.destination_identity, Some(imported.root.clone()));
    assert!(panel.status().contains("not installed or assigned"));
    assert!(panel.poll(Some(imported.root.as_path()), 4, 7).is_none());
}

#[test]
fn unexpected_root_is_fail_closed() {
    let mut panel = Panel::default();
    queue(
        &mut panel,
        completion(
            r"C:\projects\room",
            Ok(r"\\?\C:\packages\expected"),
            4,
            7,
            Ok(report(r"\\?\C:\other\unexpected")),
        ),
    );

    assert!(panel
        .poll(Some(PathBuf::from(r"C:\projects\room").as_path()), 4, 7)
        .is_none());
    assert!(!panel.busy());
    assert_eq!(panel.destination_identity, None);
    assert!(panel.status().contains("unexpected package root"));
}

#[test]
fn stale_project_or_source_completion_is_never_selected() {
    for (current_root, project_generation, source_generation) in [
        (r"C:\projects\other", 4, 7),
        (r"C:\projects\room", 5, 7),
        (r"C:\projects\room", 4, 8),
    ] {
        let mut panel = Panel::default();
        queue(
            &mut panel,
            completion(
                r"C:\projects\room",
                Ok(r"\\?\C:\packages\sample.model"),
                4,
                7,
                Ok(report(r"\\?\C:\packages\sample.model")),
            ),
        );

        assert!(panel
            .poll(
                Some(PathBuf::from(current_root).as_path()),
                project_generation,
                source_generation
            )
            .is_none());
        assert!(!panel.busy());
        assert_eq!(panel.destination_identity, None);
        assert!(panel.status().contains("prior project/source state"));
    }
}

#[test]
fn import_error_and_worker_disconnect_are_visible_without_success_identity() {
    let project_root = PathBuf::from(r"C:\projects\room");
    let mut failed = Panel::default();
    queue(
        &mut failed,
        completion(
            r"C:\projects\room",
            Ok(r"\\?\C:\packages\sample.model"),
            4,
            7,
            Err("invalid GLB"),
        ),
    );
    assert!(failed.poll(Some(&project_root), 4, 7).is_none());
    assert!(!failed.busy());
    assert_eq!(failed.destination_identity, None);
    assert!(failed.status().contains("import failed: invalid GLB"));

    let mut disconnected = Panel {
        license_acknowledged: true,
        ..Panel::default()
    };
    let (sender, receiver) = mpsc::channel();
    disconnected.pending = Some(receiver);
    drop(sender);
    assert!(disconnected.poll(Some(&project_root), 4, 7).is_none());
    assert!(!disconnected.busy());
    assert!(!disconnected.license_acknowledged);
    assert_eq!(disconnected.destination_identity, None);
    assert!(disconnected.status().contains("worker disconnected"));
}

#[test]
fn format_switch_revokes_rights_and_last_destination_and_requires_static() {
    let mut panel = Panel {
        kind: ModelKind::Animated,
        license_acknowledged: true,
        destination_identity: Some(PathBuf::from("previous-package")),
        ..Panel::default()
    };
    assert_eq!(panel.source_format, SourceFormat::RawGlb);
    panel.set_source_format(SourceFormat::CookedStatic);
    assert_eq!(panel.kind, ModelKind::Static);
    assert!(!panel.license_acknowledged);
    assert!(panel.destination_identity.is_none());

    panel.license_acknowledged = true;
    panel.destination_identity = Some(PathBuf::from("cooked-package"));
    panel.set_source_format(SourceFormat::CookedStatic);
    assert!(
        panel.license_acknowledged,
        "unchanged format is not an edit"
    );
    panel.set_source_format(SourceFormat::RawGlb);
    assert!(!panel.license_acknowledged);
    assert!(panel.destination_identity.is_none());
    assert_eq!(panel.kind, ModelKind::Static);
}

#[test]
fn request_snapshot_pins_format_kind_paths_and_rights() {
    let mut panel = Panel {
        source: "  /input/variant.model.json  ".into(),
        license: " /input/LICENSE.txt ".into(),
        destination: " /packages/variant ".into(),
        name: " sample.variant ".into(),
        version: " 1.2.3 ".into(),
        source_format: SourceFormat::CookedStatic,
        license_acknowledged: true,
        ..Panel::default()
    };
    let request = panel.request();
    panel.set_source_format(SourceFormat::RawGlb);
    panel.source.clear();
    panel.license.clear();
    panel.destination.clear();
    panel.name.clear();
    panel.version.clear();
    panel.kind = ModelKind::Animated;
    assert_eq!(request.source_format, SourceFormat::CookedStatic);
    assert_eq!(request.kind, ModelKind::Static);
    assert_eq!(request.source, PathBuf::from("/input/variant.model.json"));
    assert_eq!(request.license, PathBuf::from("/input/LICENSE.txt"));
    assert_eq!(request.destination, PathBuf::from("/packages/variant"));
    assert_eq!(request.name, "sample.variant");
    assert_eq!(request.version, "1.2.3");
    assert!(request.license_acknowledged);
}

#[test]
fn current_cooked_completion_is_returned_without_installing_or_assigning() {
    let mut panel = Panel {
        source_format: SourceFormat::CookedStatic,
        ..Panel::default()
    };
    let mut cooked = report("/packages/cooked");
    cooked.asset = "model.orrmodel".into();
    let mut completed = completion("/project", Ok("/packages/cooked"), 1, 2, Ok(cooked));
    completed.source_format = SourceFormat::CookedStatic;
    queue(&mut panel, completed);
    let result = panel
        .poll(Some(PathBuf::from("/project").as_path()), 1, 2)
        .unwrap();
    assert_eq!(result.asset, "model.orrmodel");
    assert!(panel.status().contains("not installed or assigned"));
    assert_eq!(panel.destination_identity, Some(result.root));
}

#[test]
fn cooked_worker_uses_captured_request_and_retains_published_output_when_stale() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let model = orr_model::import::import_with_resolver(
        "authored/variant.model.json",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("self-contained test fixture"),
    )
    .unwrap();
    let bytes = model.to_bytes().unwrap();
    let source = root.join("source.model.json");
    let license = root.join("LICENSE.txt");
    let destination = root.join("cooked-package");
    std::fs::write(&source, &bytes).unwrap();
    std::fs::write(&license, b"explicit license").unwrap();
    let mut panel = Panel {
        source: source.display().to_string(),
        license: license.display().to_string(),
        destination: destination.display().to_string(),
        name: "cooked-package".into(),
        version: "1.0.0".into(),
        source_format: SourceFormat::CookedStatic,
        license_acknowledged: true,
        ..Panel::default()
    };
    panel.start_import(root.clone(), 1, 2);
    assert!(panel.busy());
    // UI fields are disabled while busy. Simulate a newer external state after
    // dispatch to prove that the worker owns its original immutable request.
    panel.set_source_format(SourceFormat::RawGlb);
    panel.source = "unavailable.glb".into();
    panel.license.clear();
    let completed = panel
        .pending
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert_eq!(completed.source_format, SourceFormat::CookedStatic);
    assert_eq!(
        completed.result.as_ref().ok().unwrap().asset,
        "model.orrmodel"
    );
    queue(&mut panel, completed);
    assert!(panel.poll(Some(&root), 1, 2).is_none());
    assert!(panel.destination_identity.is_none());
    assert!(!panel.license_acknowledged);
    assert!(panel.status().contains("Imported package"));
    assert!(panel.status().contains("prior project/source state"));
    assert!(panel.status().contains("not selected"));
    assert_eq!(
        std::fs::read(destination.join("model.orrmodel")).unwrap(),
        bytes
    );
}

#[test]
fn cooked_stale_project_or_source_generation_is_not_selected() {
    for (current_root, project_generation, source_generation) in [
        ("/other-project", 4, 7),
        ("/project", 5, 7),
        ("/project", 4, 8),
    ] {
        let mut panel = Panel {
            source_format: SourceFormat::CookedStatic,
            license_acknowledged: true,
            ..Panel::default()
        };
        let mut cooked = report("/packages/cooked");
        cooked.asset = "model.orrmodel".into();
        let mut completed = completion("/project", Ok("/packages/cooked"), 4, 7, Ok(cooked));
        completed.source_format = SourceFormat::CookedStatic;
        queue(&mut panel, completed);
        assert!(panel
            .poll(
                Some(PathBuf::from(current_root).as_path()),
                project_generation,
                source_generation
            )
            .is_none());
        assert!(!panel.busy());
        assert!(!panel.license_acknowledged);
        assert!(panel.destination_identity.is_none());
        assert!(panel.status().contains("Imported package /packages/cooked"));
        assert!(panel.status().contains("not selected"));
    }
}

#[test]
fn source_format_and_file_widgets_revoke_rights_and_disable_while_busy() {
    use egui::accesskit::Role;
    use egui_kittest::{
        kittest::{NodeT, Queryable},
        Harness,
    };

    let mut harness = Harness::builder()
        .with_size([1000.0, 1000.0])
        .build_ui_state(
            |ui, panel: &mut Panel| {
                panel.show(ui, true, Some(std::path::Path::new("/project")), 1, 2)
            },
            Panel::default(),
        );
    harness.run_steps(3);
    harness.get_by_label("Import a model package").click();
    harness.run_steps(3);
    assert_eq!(harness.state().source_format, SourceFormat::RawGlb);
    assert!(harness.query_by_label("Import raw GLB package").is_some());
    harness.state_mut().license_acknowledged = true;
    harness.state_mut().destination_identity = Some(PathBuf::from("/previous"));
    harness.get_by_label("Cooked static model").click();
    harness.run_steps(3);
    assert_eq!(harness.state().source_format, SourceFormat::CookedStatic);
    assert!(!harness.state().license_acknowledged);
    assert!(harness.state().destination_identity.is_none());
    assert!(harness
        .query_by_label("Import cooked static package")
        .is_some());
    assert!(harness
        .get_by_label("Animated model")
        .accesskit_node()
        .is_disabled());

    for label in ["Cooked static model file (.model.json)", "License file"] {
        harness.state_mut().license_acknowledged = true;
        harness.state_mut().destination_identity = Some(PathBuf::from("/previous"));
        harness
            .get_by_role_and_label(Role::TextInput, label)
            .focus();
        harness.run_steps(2);
        harness
            .get_by_role_and_label(Role::TextInput, label)
            .type_text("changed");
        harness.run_steps(3);
        assert!(!harness.state().license_acknowledged, "{label}");
        assert!(harness.state().destination_identity.is_none(), "{label}");
    }
    let (_sender, receiver) = mpsc::channel();
    harness.state_mut().pending = Some(receiver);
    harness.run_steps(3);
    for label in [
        "Raw GLB",
        "Cooked static model",
        "Static model",
        "Animated model",
        "I have reviewed rights to distribute this content",
        "Import cooked static package",
    ] {
        assert!(
            harness.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
    for label in [
        "Cooked static model file (.model.json)",
        "License file",
        "Package destination directory",
        "Package name",
        "Package version",
    ] {
        assert!(
            harness
                .get_by_role_and_label(Role::TextInput, label)
                .accesskit_node()
                .is_disabled(),
            "{label}"
        );
    }
    harness.get_by_label("Raw GLB").click();
    harness.run_steps(3);
    assert_eq!(harness.state().source_format, SourceFormat::CookedStatic);
    assert!(harness.state().busy());
}
