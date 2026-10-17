use super::{
    document_frame_ready, parse_rotation, parse_transform, reported_scene_matches_request, Panel,
    MAX_HISTORY,
};
use crate::{backend::HostSpec, game::EditorGame, mesh_art::Snapshot, model::Mode};
use orr_package::Manifest;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const STATIC_GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");

fn static_package(root: &Path) -> PathBuf {
    let package_root = root.join("verified-static-package");
    fs::create_dir(&package_root).unwrap();
    fs::write(package_root.join("model.glb"), STATIC_GLB).unwrap();
    fs::write(package_root.join("LICENSE.txt"), b"Example license text\n").unwrap();
    let runtime = orr_package::Runtime::content_only();
    let manifest = Manifest {
        schema: 1,
        name: "verified-static".into(),
        version: "1.0.0".into(),
        engine: format!("={}", runtime.engine_version),
        capabilities: BTreeSet::from(["models".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["LICENSE.txt".into(), "model.glb".into()]),
    };
    fs::write(
        package_root.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    package_root
}

#[test]
fn transform_parser_accepts_finite_values_and_rejects_invalid_values() {
    let offset = ["-1.25".into(), "0".into(), "3.5".into()];
    assert_eq!(
        parse_transform(&offset, "0.75").unwrap(),
        ([-1.25, 0.0, 3.5], 0.75)
    );

    assert!(parse_transform(&["NaN".into(), "0".into(), "0".into()], "1").is_err());
    assert!(parse_transform(&["0".into(), "0".into(), "0".into()], "inf").is_err());
    assert!(parse_transform(&["bad".into(), "0".into(), "0".into()], "1").is_err());
}

#[test]
fn rotation_parser_accepts_intrinsic_xyz_degrees_and_rejects_invalid_values() {
    let quaternion = parse_rotation(&["90".into(), "0".into(), "0".into()]).unwrap();
    let expected = std::f32::consts::FRAC_1_SQRT_2;
    assert!((quaternion[0] - expected).abs() < 1.0e-6);
    assert!(quaternion[1].abs() < 1.0e-6);
    assert!(quaternion[2].abs() < 1.0e-6);
    assert!((quaternion[3] - expected).abs() < 1.0e-6);

    assert!(parse_rotation(&["NaN".into(), "0".into(), "0".into()]).is_err());
    assert!(parse_rotation(&["0".into(), "inf".into(), "0".into()]).is_err());
    assert!(parse_rotation(&["360001".into(), "0".into(), "0".into()]).is_err());
    assert!(parse_rotation(&["bad".into(), "0".into(), "0".into()]).is_err());
}

#[test]
fn private_undo_redo_restores_agent_art_rotation_fields() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = static_package(&root);
    let first_rotation = crate::mesh_rotation::from_degrees([20.0, 35.0, -15.0]).unwrap();
    let second_rotation = crate::mesh_rotation::from_degrees([-45.0, 10.0, 75.0]).unwrap();
    let first = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", [0x5a; 32])
        .unwrap()
        .with_rotation(first_rotation)
        .unwrap();
    let second = first.clone().with_rotation(second_rotation).unwrap();
    let mut panel = Panel::default();

    panel.push_state(Some(first.clone()));
    panel.saved_id = Some(panel.current_entry().unwrap().id);
    panel.push_state(Some(second.clone()));
    panel.undo();
    assert_quaternion_close(panel.current_snapshot().unwrap().rotation(), first_rotation);
    assert_quaternion_close(parse_rotation(&panel.rotation).unwrap(), first_rotation);

    panel.redo();
    assert_quaternion_close(
        panel.current_snapshot().unwrap().rotation(),
        second_rotation,
    );
    assert_quaternion_close(parse_rotation(&panel.rotation).unwrap(), second_rotation);
}

fn assert_quaternion_close(actual: [f32; 4], expected: [f32; 4]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1.0e-5);
    }
}

#[test]
fn undo_redo_tracks_saved_state_and_history_is_bounded() {
    let mut panel = Panel::default();
    assert!(!panel.dirty());

    panel.push_state(None);
    assert!(panel.dirty());
    panel.undo();
    assert!(!panel.dirty());
    panel.redo();
    assert!(panel.dirty());

    panel.undo();
    panel.push_state(None);
    assert!(panel.dirty());
    assert_eq!(panel.cursor + 1, panel.history.len());

    for _ in 0..MAX_HISTORY + 2 {
        panel.push_state(None);
    }
    assert_eq!(panel.history.len(), MAX_HISTORY);
    assert_eq!(panel.cursor + 1, panel.history.len());
    assert!(panel.dirty());
}

#[test]
fn reported_scene_must_match_requested_mesh_navigation_scene() {
    let requested = PathBuf::from(r"C:\workspace\mesh.scene.yaml");
    let spec = HostSpec::local_game(requested.clone(), EditorGame::MeshNavigation);

    assert!(reported_scene_matches_request(
        EditorGame::MeshNavigation,
        &spec,
        Some(&requested)
    ));
    assert!(!reported_scene_matches_request(
        EditorGame::MeshNavigation,
        &spec,
        Some(std::path::Path::new(r"C:\workspace\old.scene.yaml"))
    ));
    assert!(!reported_scene_matches_request(
        EditorGame::PhysGame,
        &spec,
        Some(&requested)
    ));

    let wrong_game = HostSpec::local_game(requested.clone(), EditorGame::PhysGame);
    assert!(!reported_scene_matches_request(
        EditorGame::MeshNavigation,
        &wrong_game,
        Some(&requested)
    ));
    let remote = HostSpec::Remote {
        url: "ws://localhost:7777".into(),
        token: None,
    };
    assert!(!reported_scene_matches_request(
        EditorGame::MeshNavigation,
        &remote,
        Some(&requested)
    ));
}

#[test]
fn reported_scene_path_accepts_canonical_equivalence() {
    let temporary = tempfile::tempdir().unwrap();
    let scene = temporary.path().join("mesh.scene.yaml");
    std::fs::write(&scene, b"scene").unwrap();
    let canonical_scene = scene.canonicalize().unwrap();
    let requested = temporary.path().join(".").join("mesh.scene.yaml");
    let spec = HostSpec::local_game(requested, EditorGame::MeshNavigation);

    assert!(reported_scene_matches_request(
        EditorGame::MeshNavigation,
        &spec,
        Some(&canonical_scene)
    ));
}

#[test]
fn edit_frame_must_match_state_but_play_frame_can_advance_between_state_polls() {
    let scene = PathBuf::from(r"C:\workspace\mesh.scene.yaml");
    let spec = HostSpec::local_game(scene.clone(), EditorGame::MeshNavigation);

    assert!(document_frame_ready(
        EditorGame::MeshNavigation,
        &spec,
        Some(&scene),
        Mode::Edit,
        true,
        7,
        7,
    ));
    assert!(!document_frame_ready(
        EditorGame::MeshNavigation,
        &spec,
        Some(&scene),
        Mode::Edit,
        true,
        7,
        8,
    ));
    assert!(!document_frame_ready(
        EditorGame::MeshNavigation,
        &spec,
        Some(&scene),
        Mode::Edit,
        false,
        7,
        7,
    ));
    assert!(document_frame_ready(
        EditorGame::MeshNavigation,
        &spec,
        Some(&scene),
        Mode::Play,
        true,
        7,
        8,
    ));
    assert!(!document_frame_ready(
        EditorGame::MeshNavigation,
        &spec,
        Some(&scene),
        Mode::Play,
        false,
        7,
        8,
    ));
}
