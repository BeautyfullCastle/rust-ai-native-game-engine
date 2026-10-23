use super::{completion_metadata_matches, saved_sidecars_match, Request};
use std::path::PathBuf;

fn request() -> Request {
    Request {
        scene: PathBuf::from(r"C:\projects\mesh.scene.yaml"),
        checksum: 0x1234,
        graph_revision: [7; 32],
        destination: PathBuf::from(r"C:\exports\mesh-game"),
        runtime: PathBuf::from(r"C:\tools\orr_mesh_player.exe"),
        input_profile: crate::mesh_export::InputProfile::MeshClickToMoveV1,
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
        props_sha256: None,
        camera_sha256: None,
        animation_sha256: None,
        lighting_sha256: None,
        hud_sha256: None,
        runtime_sha256: "runtime-hash".into(),
        input_profile: Some(crate::mesh_export::InputProfile::MeshClickToMoveV1),
    }
}

#[test]
fn completed_export_is_not_current_after_either_saved_sidecar_changes() {
    let temporary = tempfile::tempdir().unwrap();
    let scene = temporary.path().join("mesh.scene.yaml");
    std::fs::write(&scene, b"owned scene").unwrap();
    let mut receipt = receipt(&request());
    assert!(saved_sidecars_match(&scene, &receipt));
    let art = crate::mesh_art::sidecar_path(&scene).unwrap();
    let props = crate::mesh_props::sidecar_path(&scene).unwrap();
    let camera = crate::mesh_camera_store::sidecar_path(&scene).unwrap();
    let animation = scene.with_file_name("mesh.scene.yaml.mesh-animation.json");
    let camera_document = crate::mesh_camera::Document::from_orbit(
        orr_render::camera3d::OrbitCamera::new([1.0, 2.0, 3.0], 0.4, 0.2, 8.0),
    )
    .unwrap();
    crate::mesh_camera_store::save(&scene, Some(&camera_document), None, [7; 32]).unwrap();
    receipt.camera_sha256 = crate::mesh_camera_store::sidecar_stamp(&scene).unwrap();
    assert!(saved_sidecars_match(&scene, &receipt));
    std::fs::write(&camera, b"newer camera").unwrap();
    assert!(!saved_sidecars_match(&scene, &receipt));
    receipt.camera_sha256 = crate::mesh_camera_store::sidecar_stamp(&scene).unwrap();
    assert!(saved_sidecars_match(&scene, &receipt));
    std::fs::write(&art, b"saved art").unwrap();
    assert!(!saved_sidecars_match(&scene, &receipt));
    receipt.sidecar_sha256 = crate::mesh_art::sidecar_stamp(&scene).unwrap();
    assert!(saved_sidecars_match(&scene, &receipt));
    std::fs::write(&props, b"saved props").unwrap();
    assert!(!saved_sidecars_match(&scene, &receipt));
    receipt.props_sha256 = crate::mesh_props::sidecar_stamp(&scene).unwrap();
    assert!(saved_sidecars_match(&scene, &receipt));
    std::fs::write(&props, b"newer props").unwrap();
    assert!(!saved_sidecars_match(&scene, &receipt));
    std::fs::remove_file(&props).unwrap();
    receipt.props_sha256 = None;
    std::fs::remove_file(&camera).unwrap();
    receipt.camera_sha256 = None;
    std::fs::write(&animation, b"saved animation").unwrap();
    assert!(!saved_sidecars_match(&scene, &receipt));
    receipt.animation_sha256 = Some(crate::mesh_art::digest(&std::fs::read(&animation).unwrap()));
    #[cfg(feature = "mesh-animated-models")]
    assert!(saved_sidecars_match(&scene, &receipt));
    #[cfg(not(feature = "mesh-animated-models"))]
    assert!(!saved_sidecars_match(&scene, &receipt));
}

#[test]
fn completed_export_tracks_lighting_creation_change_and_removal() {
    let temporary = tempfile::tempdir().unwrap();
    let scene = temporary.path().join("mesh.scene.yaml");
    std::fs::write(&scene, b"owned scene").unwrap();
    let mut receipt = receipt(&request());
    assert!(saved_sidecars_match(&scene, &receipt));
    let document =
        crate::mesh_lighting::Document::from_lighting(orr_render::Lighting::default()).unwrap();
    let first_stamp = crate::mesh_lighting_store::save(&scene, Some(&document), None, [7; 32])
        .unwrap()
        .unwrap();
    assert!(!saved_sidecars_match(&scene, &receipt));
    receipt.lighting_sha256 = Some(first_stamp.clone());
    assert!(saved_sidecars_match(&scene, &receipt));
    let changed = crate::mesh_lighting::Document::from_lighting(orr_render::Lighting {
        intensity: 2.0,
        ..orr_render::Lighting::default()
    })
    .unwrap();
    let second_stamp =
        crate::mesh_lighting_store::save(&scene, Some(&changed), Some(&first_stamp), [7; 32])
            .unwrap()
            .unwrap();
    assert!(!saved_sidecars_match(&scene, &receipt));
    receipt.lighting_sha256 = Some(second_stamp.clone());
    assert!(saved_sidecars_match(&scene, &receipt));
    crate::mesh_lighting_store::save(&scene, None, Some(&second_stamp), [7; 32]).unwrap();
    assert!(!saved_sidecars_match(&scene, &receipt));
    receipt.lighting_sha256 = None;
    assert!(saved_sidecars_match(&scene, &receipt));
}

#[test]
fn completed_export_tracks_hud_creation_change_and_removal() {
    let temporary = tempfile::tempdir().unwrap();
    let scene = temporary.path().join("mesh.scene.yaml");
    std::fs::write(&scene, b"owned scene").unwrap();
    let mut receipt = receipt(&request());
    assert!(saved_sidecars_match(&scene, &receipt));
    let document = crate::mesh_hud::Document::default();
    let first_stamp = crate::mesh_hud_store::save(&scene, Some(&document), None, [7; 32])
        .unwrap()
        .unwrap();
    assert!(!saved_sidecars_match(&scene, &receipt));
    receipt.hud_sha256 = Some(first_stamp.clone());
    assert!(saved_sidecars_match(&scene, &receipt));
    let changed = crate::mesh_hud::Document {
        title: "Changed game title".into(),
        ..document
    };
    let second_stamp =
        crate::mesh_hud_store::save(&scene, Some(&changed), Some(&first_stamp), [7; 32])
            .unwrap()
            .unwrap();
    assert!(!saved_sidecars_match(&scene, &receipt));
    receipt.hud_sha256 = Some(second_stamp.clone());
    assert!(saved_sidecars_match(&scene, &receipt));
    crate::mesh_hud_store::save(&scene, None, Some(&second_stamp), [7; 32]).unwrap();
    assert!(!saved_sidecars_match(&scene, &receipt));
    receipt.hud_sha256 = None;
    assert!(saved_sidecars_match(&scene, &receipt));
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
fn completion_requires_the_interactive_input_profile() {
    let request = request();
    let mut receipt = receipt(&request);
    receipt.input_profile = None;

    assert!(!completion_metadata_matches(
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
