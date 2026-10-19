use super::{Completion, Panel};
use crate::model_package_import::Report;
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
