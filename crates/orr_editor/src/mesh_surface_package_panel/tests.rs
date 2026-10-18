use super::{context_matches, Completion, ContextStamp, Panel};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
};

fn stamp() -> ContextStamp {
    ContextStamp {
        scene: Some(PathBuf::from("C:/project/mesh.scene.yaml")),
        scene_epoch: 4,
        graph_revision: Some([7; 32]),
        input_generation: 9,
    }
}

fn imported_report(destination: &Path) -> crate::mesh_art_import::Report {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = root.join("walkable.glb");
    let license = root.join("license.txt");
    std::fs::write(
        &source,
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../orr_model/tests/fixtures/model.glb"
        )),
    )
    .unwrap();
    std::fs::write(&license, "Example license\n").unwrap();
    crate::mesh_art_import::import_package(crate::mesh_art_import::Request {
        source,
        license,
        destination: destination.to_path_buf(),
        name: "walkable-surface".into(),
        version: "1.0.0".into(),
        license_acknowledged: true,
        graph_revision: [7; 32],
    })
    .unwrap()
}

#[test]
fn cook_completion_requires_matching_scene_epoch_graph_and_inputs() {
    let context = stamp();
    assert!(context_matches(
        &context,
        Some(Path::new("C:/project/mesh.scene.yaml")),
        4,
        Some([7; 32]),
        9,
    ));
    assert!(!context_matches(
        &context,
        Some(Path::new("C:/project/other.scene.yaml")),
        4,
        Some([7; 32]),
        9,
    ));
    assert!(!context_matches(
        &context,
        Some(Path::new("C:/project/mesh.scene.yaml")),
        5,
        Some([7; 32]),
        9,
    ));
    assert!(!context_matches(
        &context,
        Some(Path::new("C:/project/mesh.scene.yaml")),
        4,
        Some([8; 32]),
        9,
    ));
    assert!(!context_matches(
        &context,
        Some(Path::new("C:/project/mesh.scene.yaml")),
        4,
        Some([7; 32]),
        10,
    ));
    assert!(!context_matches(&context, None, 4, Some([7; 32]), 9));
}

#[test]
fn current_cook_completion_returns_package_once_without_staging_geometry() {
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary
        .path()
        .canonicalize()
        .unwrap()
        .join("cooked-package");
    let report = imported_report(&destination);
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Completion {
            context: stamp(),
            result: Ok(report),
        })
        .unwrap();
    drop(sender);
    let mut panel = Panel {
        input_generation: 9,
        pending: Some(receiver),
        ..Panel::default()
    };

    assert!(panel.busy());
    let report = panel
        .poll(
            Some(Path::new("C:/project/mesh.scene.yaml")),
            4,
            Some([7; 32]),
        )
        .unwrap();
    assert!(!panel.busy());
    assert_eq!(report.root, destination);
    assert!(destination.join("model.glb").is_file());
    assert!(destination.join("LICENSE.txt").is_file());
    assert!(destination.join("orr.package.json").is_file());
    assert!(panel.status.contains("Geometry was not staged or applied"));
    assert!(panel
        .poll(
            Some(Path::new("C:/project/mesh.scene.yaml")),
            4,
            Some([7; 32]),
        )
        .is_none());
}

#[test]
fn stale_cook_completion_keeps_package_but_does_not_select_it() {
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("cooked-package");
    let report = imported_report(&destination);
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Completion {
            context: stamp(),
            result: Ok(report),
        })
        .unwrap();
    drop(sender);
    let mut panel = Panel {
        input_generation: 9,
        pending: Some(receiver),
        ..Panel::default()
    };

    assert!(panel
        .poll(
            Some(Path::new("C:/project/other.scene.yaml")),
            5,
            Some([8; 32]),
        )
        .is_none());
    assert!(!panel.busy());
    assert!(destination.join("orr.package.json").is_file());
    assert!(panel.status.contains("package retained"));
    assert!(panel.status.contains("it was not selected"));
}

#[test]
fn current_failure_and_worker_disconnect_release_busy_without_success_status() {
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Completion {
            context: stamp(),
            result: Err("walkable model contains triangles outside the agent slope limit".into()),
        })
        .unwrap();
    drop(sender);
    let mut panel = Panel {
        input_generation: 9,
        pending: Some(receiver),
        ..Panel::default()
    };

    assert!(panel
        .poll(
            Some(Path::new("C:/project/mesh.scene.yaml")),
            4,
            Some([7; 32]),
        )
        .is_none());
    assert!(!panel.busy());
    assert!(panel.status.contains("walkable model contains triangles"));
    assert!(!panel.status.contains("package created"));

    let (sender, receiver) = mpsc::channel::<Completion>();
    drop(sender);
    panel.pending = Some(receiver);
    assert!(panel
        .poll(
            Some(Path::new("C:/project/mesh.scene.yaml")),
            4,
            Some([7; 32]),
        )
        .is_none());
    assert!(!panel.busy());
    assert!(panel.status.contains("worker disconnected"));
    assert!(!panel.status.contains("package created"));
}
