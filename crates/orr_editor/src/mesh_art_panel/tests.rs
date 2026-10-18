use super::{
    document_frame_ready, parse_rotation, parse_surface, parse_transform,
    reported_scene_matches_request, Panel, MAX_HISTORY,
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
fn surface_parser_preserves_identity_and_validates_numeric_fields() {
    assert_eq!(
        parse_surface(
            false,
            "invalid",
            &["NaN".into(), "bad".into()],
            &["0".into(), "0".into()],
            "-1",
            &["bad".into(), "0".into(), "0".into()],
        )
        .unwrap(),
        None
    );
    assert_eq!(
        parse_surface(
            true,
            "0",
            &["0.25".into(), "-0.5".into()],
            &["2".into(), "0.5".into()],
            "1",
            &["0.5".into(), "0".into(), "-1".into()],
        )
        .unwrap(),
        Some(crate::mesh_surface::Settings {
            material: 0,
            uv_offset: [0.25, -0.5],
            uv_scale: [2.0, 0.5],
            uv_quarter_turns: 1,
            pivot: [0.5, 0.0, -1.0],
        })
    );
    assert!(parse_surface(
        true,
        "-1",
        &["0".into(), "0".into()],
        &["1".into(), "1".into()],
        "0",
        &["0".into(), "0".into(), "0".into()]
    )
    .is_err());
    assert!(parse_surface(
        true,
        "0",
        &["NaN".into(), "0".into()],
        &["1".into(), "1".into()],
        "0",
        &["0".into(), "0".into(), "0".into()]
    )
    .is_err());
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

#[test]
fn route_facing_apply_undo_redo_and_sidecar_reopen_preserve_selection() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = static_package(&root);
    let scene = root.join("facing.scene.yaml");
    fs::write(&scene, b"scene\n").unwrap();
    let fixed = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", [0x5a; 32]).unwrap();
    let route = fixed
        .clone()
        .with_facing(crate::mesh_facing::Facing::Route)
        .unwrap();
    let mut panel = Panel::default();

    panel.push_state(Some(fixed));
    panel.saved_id = Some(panel.current_entry().unwrap().id);
    assert!(!panel.dirty());
    panel.face_route_direction = true;
    assert!(panel.facing_form_dirty());
    assert!(panel.dirty());
    panel.face_route_direction = false;
    assert!(!panel.facing_form_dirty());
    assert!(!panel.dirty());
    panel.push_state(Some(route.clone()));
    assert!(panel.face_route_direction);
    assert!(panel.dirty());
    panel.undo();
    assert!(!panel.face_route_direction);
    panel.redo();
    assert!(panel.face_route_direction);
    assert_eq!(
        panel.current_snapshot().unwrap().facing(),
        crate::mesh_facing::Facing::Route
    );

    crate::mesh_art::save(&scene, Some(&route), None).unwrap();
    fs::rename(&source, root.join("hidden-package-source")).unwrap();
    let (reopened, _) = crate::mesh_art::load(&scene, [0x5a; 32]).unwrap().unwrap();
    assert_eq!(reopened.facing(), crate::mesh_facing::Facing::Route);
    assert!(reopened.encode_embedded("facing.scene.yaml").is_ok());
}

#[test]
fn workspace_save_detects_unapplied_art_fields_and_different_candidates() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = static_package(&root);
    let snapshot = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", [0x5a; 32]).unwrap();
    let mut panel = Panel::default();
    panel.push_state(Some(snapshot.clone()));
    panel.saved_id = Some(panel.current_entry().unwrap().id);
    panel.scene = Some(root.join("workspace-art.scene.yaml"));
    panel.copy_transform(&snapshot);

    assert!(!panel.unapplied_form_dirty());
    panel.offset[0] = "0.5".into();
    assert!(panel.unapplied_form_dirty());
    panel.copy_transform(&snapshot);
    panel.candidate = Some(snapshot.clone());
    assert!(!panel.unapplied_form_dirty());
    panel.candidate = None;
    panel.tint[0] = "invalid".into();
    assert!(panel.unapplied_form_dirty());
    panel.copy_transform(&snapshot);
    panel.candidate =
        Some(Snapshot::from_package(&source, "model.glb", "LICENSE.txt", [0x5b; 32]).unwrap());
    assert!(panel.unapplied_form_dirty());

    for angles in [[17.0, 31.0, 53.0], [0.0, 90.0, 0.0], [0.0, -90.0, 0.0]] {
        let rotated = snapshot
            .clone()
            .with_rotation(crate::mesh_rotation::from_degrees(angles).unwrap())
            .unwrap();
        let mut rotated_panel = Panel::default();
        rotated_panel.push_state(Some(rotated.clone()));
        rotated_panel.saved_id = Some(rotated_panel.current_entry().unwrap().id);
        rotated_panel.scene = Some(root.join("rotated-art.scene.yaml"));
        rotated_panel.copy_transform(&rotated);
        rotated_panel.candidate = Some(rotated);
        assert!(!rotated_panel.unapplied_form_dirty());

        let applied_from_original_angles = snapshot
            .clone()
            .with_rotation(crate::mesh_rotation::from_degrees(angles).unwrap())
            .unwrap();
        let mut applied_panel = Panel::default();
        applied_panel.push_state(Some(applied_from_original_angles.clone()));
        applied_panel.saved_id = Some(applied_panel.current_entry().unwrap().id);
        applied_panel.scene = Some(root.join("applied-rotation.scene.yaml"));
        applied_panel.copy_transform(&applied_from_original_angles);
        applied_panel.rotation = angles.map(|angle| angle.to_string());
        applied_panel.candidate = Some(applied_from_original_angles);
        assert!(!applied_panel.unapplied_form_dirty());
        applied_panel.rotation[0] = (angles[0] + 0.25).to_string();
        assert!(applied_panel.unapplied_form_dirty());
    }
}

#[test]
fn private_undo_redo_restores_agent_art_surface_settings() {
    let temporary = tempfile::tempdir().unwrap();
    let source = static_package(&temporary.path().canonicalize().unwrap());
    let material = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", [0x5a; 32])
        .unwrap()
        .material_choices()[0];
    let first_surface = crate::mesh_surface::Settings {
        material,
        uv_offset: [0.25, -0.5],
        uv_scale: [2.0, 0.5],
        uv_quarter_turns: 1,
        pivot: [0.5, 0.0, -1.0],
    };
    let second_surface = crate::mesh_surface::Settings {
        material,
        uv_offset: [-0.25, 0.75],
        uv_scale: [0.75, 1.5],
        uv_quarter_turns: 3,
        pivot: [-0.5, 0.25, 1.0],
    };
    let first = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", [0x5a; 32])
        .unwrap()
        .with_surface(Some(first_surface))
        .unwrap();
    let second = first.clone().with_surface(Some(second_surface)).unwrap();
    let mut panel = Panel::default();

    panel.push_state(Some(first.clone()));
    panel.saved_id = Some(panel.current_entry().unwrap().id);
    panel.push_state(Some(second.clone()));
    panel.undo();
    assert_eq!(
        panel.current_snapshot().unwrap().surface(),
        Some(first_surface)
    );
    assert_eq!(
        parse_surface(
            panel.surface_enabled,
            &panel.surface_material,
            &panel.surface_uv_offset,
            &panel.surface_uv_scale,
            &panel.surface_quarter_turns,
            &panel.surface_pivot,
        )
        .unwrap(),
        Some(first_surface)
    );

    panel.redo();
    assert_eq!(
        panel.current_snapshot().unwrap().surface(),
        Some(second_surface)
    );
    assert_eq!(
        parse_surface(
            panel.surface_enabled,
            &panel.surface_material,
            &panel.surface_uv_offset,
            &panel.surface_uv_scale,
            &panel.surface_quarter_turns,
            &panel.surface_pivot,
        )
        .unwrap(),
        Some(second_surface)
    );
    assert_eq!(second.clone().with_surface(None).unwrap().surface(), None);
}

#[test]
fn art_tint_parser_and_undo_redo_preserve_snapshot_tint() {
    let first_tint = [0.5, 0.75, 1.0, 0.25];
    let second_tint = [1.0, 0.25, 0.5, 0.75];
    assert_eq!(
        super::parse_tint(&first_tint.map(|value| value.to_string())).unwrap(),
        first_tint
    );
    assert!(super::parse_tint(&["NaN".into(), "1".into(), "1".into(), "1".into()]).is_err());
    assert!(super::parse_tint(&["-0.1".into(), "1".into(), "1".into(), "1".into()]).is_err());

    let temporary = tempfile::tempdir().unwrap();
    let source = static_package(&temporary.path().canonicalize().unwrap());
    let first = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", [0x5a; 32])
        .unwrap()
        .with_tint(first_tint)
        .unwrap();
    let second = first.clone().with_tint(second_tint).unwrap();
    let mut panel = Panel::default();
    panel.push_state(Some(first.clone()));
    panel.saved_id = Some(panel.current_entry().unwrap().id);
    panel.push_state(Some(second.clone()));
    panel.undo();
    assert_eq!(panel.current_snapshot().unwrap().tint(), first_tint);
    assert_eq!(super::parse_tint(&panel.tint).unwrap(), first_tint);
    panel.redo();
    assert_eq!(panel.current_snapshot().unwrap().tint(), second_tint);
    assert_eq!(super::parse_tint(&panel.tint).unwrap(), second_tint);
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
