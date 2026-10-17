use super::{
    discard_invalid_ready, document_frame_ready, parse_transform, reported_scene_matches_request,
    validate_document, validate_name, Panel, MAX_HISTORY, MAX_PROPS,
};
use crate::{
    backend::HostSpec,
    game::EditorGame,
    mesh_art::Snapshot,
    mesh_props::{Document, Prop},
    model::Mode,
};
use std::path::Path;

const GRAPH_REVISION: [u8; 32] = [0x53; 32];

fn snapshot() -> Snapshot {
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    Snapshot::from_package(&package, "foreground.glb", "LICENSE.txt", GRAPH_REVISION).unwrap()
}

fn document(count: usize) -> Document {
    let art = snapshot();
    Document {
        props: (0..count)
            .map(|index| Prop {
                id: index as u64 + 1,
                name: format!("Prop {index}"),
                art: art.clone(),
            })
            .collect(),
    }
}

#[test]
fn prop_transform_parser_accepts_finite_positions_and_rejects_invalid_values() {
    assert_eq!(
        parse_transform(&["-1.25".into(), "0".into(), "3.5".into()], "0.75").unwrap(),
        ([-1.25, 0.0, 3.5], 0.75)
    );
    assert!(parse_transform(&["NaN".into(), "0".into(), "0".into()], "1").is_err());
    assert!(parse_transform(&["0".into(), "0".into(), "inf".into()], "1").is_err());
    assert!(parse_transform(&["0".into(), "0".into(), "0".into()], "NaN").is_err());
    let (position, negative_scale) =
        parse_transform(&["0".into(), "0".into(), "0".into()], "-1").unwrap();
    assert!(snapshot().with_transform(position, negative_scale).is_err());
}

#[test]
fn prop_names_are_trimmed_bounded_and_printable() {
    assert_eq!(validate_name("  Lamp  ").unwrap(), "Lamp");
    assert!(validate_name("").is_err());
    assert!(validate_name(&"p".repeat(65)).is_err());
    assert!(validate_name("bad\nname").is_err());
}

#[test]
fn document_validation_checks_prop_count_ids_names_and_graph_revision() {
    let valid = document(2);
    validate_document(&valid, GRAPH_REVISION).unwrap();
    assert_eq!(valid.placements().unwrap().len(), 2);

    let too_many = document(MAX_PROPS + 1);
    assert!(validate_document(&too_many, GRAPH_REVISION).is_err());

    let mut duplicate_ids = document(2);
    duplicate_ids.props[1].id = duplicate_ids.props[0].id;
    assert!(validate_document(&duplicate_ids, GRAPH_REVISION).is_err());

    let mut duplicate_names = document(2);
    duplicate_names.props[1].name = duplicate_names.props[0].name.clone();
    assert!(validate_document(&duplicate_names, GRAPH_REVISION).is_err());

    let wrong_revision = document(1);
    let other_revision = [0x24; 32];
    assert!(validate_document(&wrong_revision, other_revision).is_err());
}

#[test]
fn prop_history_tracks_saved_state_and_keeps_a_bounded_cursor() {
    let mut panel = Panel::default();
    assert!(!panel.dirty());

    panel.push_document(document(1));
    assert!(panel.dirty());
    panel.undo();
    assert!(!panel.dirty());
    panel.redo();
    assert!(panel.dirty());

    panel.undo();
    panel.push_document(document(2));
    assert!(panel.dirty());
    assert_eq!(panel.cursor + 1, panel.history.len());

    for count in 0..MAX_HISTORY + 2 {
        panel.push_document(document(count % (MAX_PROPS + 1)));
    }
    assert_eq!(panel.history.len(), MAX_HISTORY);
    assert_eq!(panel.cursor + 1, panel.history.len());
    assert!(panel.dirty());
}

#[test]
fn mesh_scene_identity_requires_the_requested_local_mesh_scene() {
    let requested = Path::new(r"C:\workspace\mesh.scene.yaml");
    let spec = HostSpec::local_game(requested.to_path_buf(), EditorGame::MeshNavigation);
    assert!(reported_scene_matches_request(
        EditorGame::MeshNavigation,
        &spec,
        Some(requested)
    ));
    assert!(!reported_scene_matches_request(
        EditorGame::MeshNavigation,
        &spec,
        Some(Path::new(r"C:\workspace\other.scene.yaml"))
    ));
    assert!(!reported_scene_matches_request(
        EditorGame::PhysGame,
        &spec,
        Some(requested)
    ));
    assert!(!reported_scene_matches_request(
        EditorGame::MeshNavigation,
        &HostSpec::Remote {
            url: "ws://localhost:7777".into(),
            token: None,
        },
        Some(requested)
    ));

    assert!(document_frame_ready(
        EditorGame::MeshNavigation,
        &spec,
        Some(requested),
        Mode::Edit,
        true,
        17,
        17,
    ));
    assert!(!document_frame_ready(
        EditorGame::MeshNavigation,
        &spec,
        Some(requested),
        Mode::Edit,
        true,
        17,
        18,
    ));
    assert!(document_frame_ready(
        EditorGame::MeshNavigation,
        &spec,
        Some(requested),
        Mode::Play,
        true,
        17,
        18,
    ));

    let temporary = tempfile::tempdir().unwrap();
    let scene = temporary.path().join("mesh.scene.yaml");
    std::fs::write(&scene, b"mesh scene").unwrap();
    let canonical_scene = scene.canonicalize().unwrap();
    let equivalent_spec = HostSpec::local_game(
        temporary.path().join(".").join("mesh.scene.yaml"),
        EditorGame::MeshNavigation,
    );
    assert!(reported_scene_matches_request(
        EditorGame::MeshNavigation,
        &equivalent_spec,
        Some(&canonical_scene)
    ));
}

#[test]
fn invalid_sidecar_discard_requires_confirmation_and_every_safety_guard() {
    assert!(discard_invalid_ready(true, true, true, true, true, true));
    assert!(!discard_invalid_ready(false, true, true, true, true, true));
    assert!(!discard_invalid_ready(true, false, true, true, true, true));
    assert!(!discard_invalid_ready(true, true, false, true, true, true));
    assert!(!discard_invalid_ready(true, true, true, false, true, true));
    assert!(!discard_invalid_ready(true, true, true, true, false, true));
    assert!(!discard_invalid_ready(true, true, true, true, true, false));
}
