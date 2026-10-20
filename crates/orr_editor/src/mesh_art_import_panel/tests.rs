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

#[test]
fn import_completion_requires_matching_scene_epoch_graph_and_inputs() {
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
    assert!(!context_matches(&context, None, 4, Some([7; 32]), 9,));
}

#[test]
fn current_worker_failure_is_visible_and_releases_the_busy_guard() {
    let context = stamp();
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Completion {
            context,
            result: Err("GLB validation failed".into()),
        })
        .unwrap();
    drop(sender);
    let mut panel = Panel {
        input_generation: 9,
        pending: Some(receiver),
        ..Default::default()
    };

    assert!(panel.busy());
    assert!(panel
        .poll(
            Some(Path::new("C:/project/mesh.scene.yaml")),
            4,
            Some([7; 32]),
        )
        .is_none());
    assert!(!panel.busy());
    assert_eq!(
        panel.status,
        "Agent-art import failed: GLB validation failed"
    );
}

#[test]
fn stale_worker_failure_is_reported_as_prior_context() {
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Completion {
            context: stamp(),
            result: Err("GLB validation failed".into()),
        })
        .unwrap();
    drop(sender);
    let mut panel = Panel {
        pending: Some(receiver),
        ..Default::default()
    };

    assert!(panel
        .poll(
            Some(Path::new("C:/project/other.scene.yaml")),
            5,
            Some([8; 32]),
        )
        .is_none());
    assert!(!panel.busy());
    assert!(panel
        .status
        .starts_with("Agent-art import for a prior scene or input state failed:"));
}

#[test]
fn successful_completion_is_selected_once_only_for_the_current_context() {
    for current in [true, false] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let source = root.join("agent.glb");
        let license = root.join("license.txt");
        let destination = root.join("generated-package");
        std::fs::write(
            &source,
            include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        )
        .unwrap();
        std::fs::write(&license, "Example license\n").unwrap();
        let report = crate::mesh_art_import::import_package(crate::mesh_art_import::Request {
            source,
            license,
            destination: destination.clone(),
            name: "agent-art".into(),
            version: "1.0.0".into(),
            license_acknowledged: true,
            graph_revision: [7; 32],
        })
        .unwrap();
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
        let epoch = if current { 4 } else { 5 };
        let selected = panel.poll(
            Some(Path::new("C:/project/mesh.scene.yaml")),
            epoch,
            Some([7; 32]),
        );
        assert_eq!(selected.is_some(), current);
        assert!(!panel.busy());
        assert!(destination.join("model.glb").is_file());
        assert!(destination.join("orr.package.json").is_file());
        assert!(panel.status.contains(if current {
            "Not applied"
        } else {
            "it was not selected"
        }));
        assert!(panel
            .poll(
                Some(Path::new("C:/project/mesh.scene.yaml")),
                epoch,
                Some([7; 32]),
            )
            .is_none());
    }
}
