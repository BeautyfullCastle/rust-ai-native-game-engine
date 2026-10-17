use super::{completion_metadata_matches, Request};
use std::path::PathBuf;

fn request() -> Request {
    Request {
        scene: PathBuf::from(r"C:\projects\mesh.scene.yaml"),
        checksum: 0x1234,
        graph_revision: [7; 32],
        destination: PathBuf::from(r"C:\exports\mesh-game"),
        runtime: PathBuf::from(r"C:\tools\orr_mesh_player.exe"),
    }
}

fn receipt(request: &Request) -> crate::mesh_export::Receipt {
    crate::mesh_export::Receipt {
        root: request.destination.clone(),
        scene: request.destination.join("scene.scene.yaml"),
        runtime: request.destination.join("orr_mesh_player.exe"),
        manifest: request.destination.join("orr.mesh-game.json"),
        checksum: request.checksum,
        graph_revision: request.graph_revision,
        scene_sha256: "scene-hash".into(),
        sidecar_sha256: None,
        runtime_sha256: "runtime-hash".into(),
    }
}

#[test]
fn matching_export_request_and_receipt_are_current() {
    let request = request();
    let receipt = receipt(&request);

    assert!(completion_metadata_matches(
        &request,
        &request.runtime.display().to_string(),
        &request.destination.display().to_string(),
        None,
        &receipt,
    ));
}

#[test]
fn changed_inputs_receipt_identity_or_blocker_keep_bundle_stale() {
    let request = request();
    let receipt = receipt(&request);

    assert!(!completion_metadata_matches(
        &request,
        r"C:\tools\different.exe",
        &request.destination.display().to_string(),
        None,
        &receipt,
    ));
    assert!(!completion_metadata_matches(
        &request,
        &request.runtime.display().to_string(),
        r"C:\exports\different",
        None,
        &receipt,
    ));
    assert!(!completion_metadata_matches(
        &request,
        &request.runtime.display().to_string(),
        &request.destination.display().to_string(),
        Some("the current mesh is not playable"),
        &receipt,
    ));

    let mut changed_receipt = receipt;
    changed_receipt.checksum = 0x5678;
    assert!(!completion_metadata_matches(
        &request,
        &request.runtime.display().to_string(),
        &request.destination.display().to_string(),
        None,
        &changed_receipt,
    ));
}
