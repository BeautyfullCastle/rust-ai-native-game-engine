use super::{
    document_frame_ready, parse_transform, reported_scene_matches_request, Panel, MAX_HISTORY,
};
use crate::{backend::HostSpec, game::EditorGame, model::Mode};
use std::path::PathBuf;

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
