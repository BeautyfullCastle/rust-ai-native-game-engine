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
        parse_transform(
            &["-1.25".into(), "0".into(), "3.5".into()],
            "0.75",
            &["90".into(), "0".into(), "-45".into()],
        )
        .unwrap(),
        ([-1.25, 0.0, 3.5], 0.75, [90.0, 0.0, -45.0])
    );
    let zero_rotation = &["0".into(), "0".into(), "0".into()];
    assert!(parse_transform(&["NaN".into(), "0".into(), "0".into()], "1", zero_rotation).is_err());
    assert!(parse_transform(&["0".into(), "0".into(), "inf".into()], "1", zero_rotation).is_err());
    assert!(parse_transform(&["0".into(), "0".into(), "0".into()], "NaN", zero_rotation).is_err());
    assert!(parse_transform(
        &["0".into(), "0".into(), "0".into()],
        "1",
        &["0".into(), "inf".into(), "0".into()]
    )
    .is_err());
    let (position, negative_scale, _) =
        parse_transform(&["0".into(), "0".into(), "0".into()], "-1", zero_rotation).unwrap();
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
fn viewport_drag_commits_one_history_entry_ignores_no_motion_and_cancel_restores() {
    let camera = orr_render::Camera3D::orthographic([0.0, 5.0, 5.0], [0.0; 3], 4.0);
    let viewport = (100, 100);
    let mut panel = Panel::default();
    let mut original_document = document(1);
    let original_rotation = crate::mesh_rotation::from_degrees([15.0, -25.0, 40.0]).unwrap();
    original_document.props[0].art = original_document.props[0]
        .art
        .clone()
        .with_rotation(original_rotation)
        .unwrap();
    panel.reset_history(original_document, Some(0));
    panel.graph_revision = Some(panel.document.props[0].art.graph_revision());
    panel.selected = Some(1);
    let original = panel.document.clone();
    let original_offset = panel.document.props[0].art.offset();
    let drag =
        crate::mesh_prop_view::Drag::new(&camera, [0.0, 0.0], viewport, original_offset).unwrap();

    panel.gesture = Some(super::PropGesture {
        original: original.clone(),
        selected: 1,
        drag,
    });
    panel.document.props[0].art = panel.document.props[0]
        .art
        .clone()
        .with_transform([1.0, 0.0, 2.0], 1.0)
        .unwrap();
    panel.finish_gesture();
    assert_eq!(panel.history.len(), 2);
    assert!(panel.dirty());
    assert!(panel.document.props[0]
        .art
        .rotation()
        .iter()
        .zip(original_rotation)
        .all(|(actual, expected)| (*actual - expected).abs() < 1.0e-5));
    panel.undo();
    assert_eq!(panel.document.props[0].art.offset(), original_offset);
    assert!(panel.document.props[0]
        .art
        .rotation()
        .iter()
        .zip(original_rotation)
        .all(|(actual, expected)| (*actual - expected).abs() < 1.0e-5));
    panel.redo();
    assert_eq!(panel.document.props[0].art.offset(), [1.0, 0.0, 2.0]);
    assert!(panel.document.props[0]
        .art
        .rotation()
        .iter()
        .zip(original_rotation)
        .all(|(actual, expected)| (*actual - expected).abs() < 1.0e-5));

    panel.undo();
    panel.gesture = Some(super::PropGesture {
        original: panel.document.clone(),
        selected: 1,
        drag,
    });
    let mut near_offset = original_offset;
    near_offset[0] += 0.000001;
    panel.document.props[0].art = panel.document.props[0]
        .art
        .clone()
        .with_transform(near_offset, 1.0)
        .unwrap();
    panel.finish_gesture();
    assert_eq!(panel.history.len(), 2);
    assert_eq!(panel.document.props[0].art.offset(), original_offset);

    panel.gesture = Some(super::PropGesture {
        original: panel.document.clone(),
        selected: 1,
        drag,
    });
    panel.document.props[0].art = panel.document.props[0]
        .art
        .clone()
        .with_transform([3.0, 0.0, 4.0], 1.0)
        .unwrap();
    panel.cancel_gesture();
    assert_eq!(panel.document.props[0].art.offset(), original_offset);
    assert!(!panel.dirty());
}

#[test]
fn prop_rotation_fields_round_trip_through_update_and_history() {
    let mut panel = Panel::default();
    let mut initial_document = document(1);
    let original_rotation = crate::mesh_rotation::from_degrees([10.0, 20.0, 30.0]).unwrap();
    initial_document.props[0].art = initial_document.props[0]
        .art
        .clone()
        .with_rotation(original_rotation)
        .unwrap();
    panel.reset_history(initial_document, Some(0));
    panel.graph_revision = Some(GRAPH_REVISION);
    panel.selected = Some(1);
    panel.copy_selection_fields();
    let selected_degrees = panel
        .rotation
        .each_ref()
        .map(|value| value.parse::<f32>().unwrap());
    assert!(selected_degrees
        .iter()
        .zip([10.0, 20.0, 30.0])
        .all(|(actual, expected)| (*actual - expected).abs() < 1.0e-4));

    panel.position = ["2".into(), "0".into(), "3".into()];
    panel.rotation = ["45".into(), "90".into(), "-30".into()];
    panel.update_selected();
    let updated_rotation = panel.document.props[0].art.rotation();
    let expected_rotation = crate::mesh_rotation::from_degrees([45.0, 90.0, -30.0]).unwrap();
    assert!(updated_rotation
        .iter()
        .zip(expected_rotation)
        .all(|(actual, expected)| (*actual - expected).abs() < 1.0e-5));

    panel.undo();
    assert!(panel.document.props[0]
        .art
        .rotation()
        .iter()
        .zip(original_rotation)
        .all(|(actual, expected)| (*actual - expected).abs() < 1.0e-5));
    panel.redo();
    assert!(panel.document.props[0]
        .art
        .rotation()
        .iter()
        .zip(expected_rotation)
        .all(|(actual, expected)| (*actual - expected).abs() < 1.0e-5));
}

#[test]
fn prop_tint_fields_round_trip_through_update_history_and_drag() {
    let initial_tint = [0.4, 0.6, 0.8, 0.5];
    let updated_tint = [0.2, 0.3, 0.4, 1.0];
    let mut panel = Panel::default();
    let mut initial_document = document(1);
    initial_document.props[0].art = initial_document.props[0]
        .art
        .clone()
        .with_tint(initial_tint)
        .unwrap();
    panel.reset_history(initial_document, Some(0));
    panel.graph_revision = Some(GRAPH_REVISION);
    panel.selected = Some(1);
    panel.copy_selection_fields();
    assert_eq!(super::parse_tint(&panel.tint).unwrap(), initial_tint);
    panel.position = ["2".into(), "0".into(), "3".into()];
    panel.tint = updated_tint.map(|value| value.to_string());
    panel.update_selected();
    assert_eq!(panel.document.props[0].art.tint(), updated_tint);
    panel.undo();
    assert_eq!(panel.document.props[0].art.tint(), initial_tint);
    panel.redo();
    assert_eq!(panel.document.props[0].art.tint(), updated_tint);

    let original = panel.document.clone();
    panel.gesture = Some(super::PropGesture {
        original: original.clone(),
        selected: 1,
        drag: crate::mesh_prop_view::Drag::new(
            &orr_render::Camera3D::orthographic([0.0, 5.0, 5.0], [0.0; 3], 4.0),
            [0.0, 0.0],
            (100, 100),
            [2.0, 0.0, 3.0],
        )
        .unwrap(),
    });
    panel.document.props[0].art = panel.document.props[0]
        .art
        .clone()
        .with_transform([4.0, 0.0, 5.0], 1.0)
        .unwrap();
    assert_eq!(panel.document.props[0].art.tint(), updated_tint);
    panel.cancel_gesture();
    assert_eq!(panel.document.props[0].art.tint(), updated_tint);
    assert!(super::parse_tint(&["1".into(), "1.1".into(), "1".into(), "1".into()]).is_err());
}

#[test]
fn prop_duplicate_preserves_asset_state_and_creates_one_private_history_entry() {
    let original_rotation = crate::mesh_rotation::from_degrees([12.0, 23.0, 34.0]).unwrap();
    let original_tint = [0.4, 0.6, 0.8, 0.5];
    let mut original = document(1);
    original.props[0].name = "Lantern".into();
    original.props[0].art = original.props[0]
        .art
        .clone()
        .with_transform([2.0, 1.0, 3.0], 0.75)
        .unwrap()
        .with_rotation(original_rotation)
        .unwrap()
        .with_tint(original_tint)
        .unwrap();
    let mut panel = Panel::default();
    panel.reset_history(original, Some(0));
    panel.graph_revision = Some(GRAPH_REVISION);
    panel.selected = Some(1);

    panel.duplicate_selected_prop();

    assert_eq!(panel.history.len(), 2);
    assert!(panel.dirty());
    assert_eq!(panel.selected, Some(2));
    assert_eq!(panel.document.props[1].name, "Lantern copy");
    assert_eq!(panel.document.props[1].art.offset(), [2.0, 1.0, 3.0]);
    assert_eq!(panel.document.props[1].art.scale(), 0.75);
    assert_eq!(panel.document.props[1].art.rotation(), original_rotation);
    assert_eq!(panel.document.props[1].art.tint(), original_tint);
    assert_eq!(
        panel.document.props[1].art.label(),
        panel.document.props[0].art.label()
    );

    panel.undo();
    assert_eq!(panel.document.props.len(), 1);
    assert!(!panel.dirty());
    panel.redo();
    assert_eq!(panel.document.props.len(), 2);
    assert!(panel.dirty());
}

#[test]
fn staged_prop_relocation_preserves_art_and_cancel_does_not_mutate_history() {
    let rotation = crate::mesh_rotation::from_degrees([0.0, 45.0, 0.0]).unwrap();
    let tint = [0.25, 0.5, 0.75, 1.0];
    let mut original = document(1);
    original.props[0].art = original.props[0]
        .art
        .clone()
        .with_transform([0.0, 1.0, 0.0], 1.5)
        .unwrap()
        .with_rotation(rotation)
        .unwrap()
        .with_tint(tint)
        .unwrap();
    let destination = [
        orr_fp::FP::from_raw(5 * orr_fp::FP::SCALE),
        orr_fp::FP::from_raw(2 * orr_fp::FP::SCALE),
        orr_fp::FP::from_raw(7 * orr_fp::FP::SCALE),
    ];
    let relocated =
        super::relocate_prop_document(&original, 1, destination, GRAPH_REVISION).unwrap();

    assert_eq!(original.props[0].art.offset(), [0.0, 1.0, 0.0]);
    assert_eq!(relocated.props[0].art.offset(), [5.0, 2.0, 7.0]);
    assert_eq!(relocated.props[0].art.scale(), 1.5);
    assert_eq!(relocated.props[0].art.rotation(), rotation);
    assert_eq!(relocated.props[0].art.tint(), tint);

    let mut panel = Panel::default();
    panel.reset_history(original.clone(), Some(0));
    panel.graph_revision = Some(GRAPH_REVISION);
    panel.selected = Some(1);
    panel.placement = Some(super::PropPlacement {
        prop_id: 1,
        scene: Path::new("mesh.scene.yaml").to_path_buf(),
        scene_epoch: 3,
        graph_revision: GRAPH_REVISION,
        checksum: 99,
        position: Some(destination),
    });
    assert!(panel.dirty());
    panel.cancel_placement();
    assert_eq!(panel.document.props[0].art.offset(), [0.0, 1.0, 0.0]);
    assert_eq!(panel.history.len(), 1);
    assert!(!panel.dirty());
}

#[test]
fn prop_duplicate_name_falls_back_to_bounded_id_name() {
    let mut original = document(1);
    original.props[0].name = "x".repeat(64);
    let duplicated = super::duplicate_prop_document(&original, 1, 2).unwrap();
    assert_eq!(duplicated.props[1].name, "Prop 2");
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

#[test]
fn unsaved_prop_discard_requires_confirmation_changes_and_idle_context() {
    use super::discard_changes_ready;

    assert!(discard_changes_ready(true, true, true));
    assert!(!discard_changes_ready(false, true, true));
    assert!(!discard_changes_ready(true, false, true));
    assert!(!discard_changes_ready(true, true, false));
}
