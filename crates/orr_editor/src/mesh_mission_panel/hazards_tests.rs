use super::*;

fn fixture() -> (tempfile::TempDir, Editor, Panel) {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("hazards-mission.scene.yaml");
    drop(
        crate::mesh_workspace::mission::prepare(
            &scene,
            &crate::mesh_workspace::mission::Settings::default(),
        )
        .unwrap(),
    );
    let mut editor = Editor::open_game(&scene, EditorGame::MeshHazardsV1).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.sync(&editor);
    assert!(panel.can_edit(&editor));
    (directory, editor, panel)
}

fn config(editor: &Editor) -> MeshMission {
    mission::from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .config
}

fn stage(panel: &mut Panel) {
    panel.points[0][3] = "0.25".into();
    panel.exit[3] = "0.75".into();
    panel.form_dirty = true;
}

#[test]
fn hazard_mission_apply_is_atomic_and_roundtrips_undo_redo_save() {
    let (_directory, mut editor, mut panel) = fixture();
    let before = config(&editor);
    let history = editor.history().entries.len();
    stage(&mut panel);
    assert!(panel.play_blocker(&editor).is_some());
    panel.apply(&mut editor);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert!(!panel.dirty());
    assert!(panel.play_blocker(&editor).is_none());
    assert_eq!(editor.history().entries.len(), history + 1);
    let after = config(&editor);
    assert_ne!(before, after);
    assert_eq!(
        after.points[0].radius,
        parse_fixed("0.25", "radius").unwrap()
    );
    assert_eq!(after.exit.radius, parse_fixed("0.75", "radius").unwrap());
    assert!(editor.undo());
    editor.sync();
    panel.sync(&editor);
    assert_eq!(config(&editor), before);
    assert!(editor.redo());
    editor.sync();
    panel.sync(&editor);
    assert_eq!(config(&editor), after);
    assert!(editor.save());
    let mut reopened =
        Editor::open_game(&editor.path().unwrap(), EditorGame::MeshHazardsV1).unwrap();
    reopened.sync();
    assert_eq!(config(&reopened), after);
    assert_eq!(reopened.game(), EditorGame::MeshHazardsV1);
}

#[test]
fn hazard_mission_invalid_fields_and_play_never_mutate() {
    let (_directory, mut editor, mut panel) = fixture();
    let before = editor.checksum();
    let history = editor.history().entries.len();
    for value in ["0", "-1", "4.1", "NaN"] {
        panel.points[0][3] = value.into();
        panel.form_dirty = true;
        panel.apply(&mut editor);
        assert!(panel.error.is_some());
        assert_eq!(editor.checksum(), before);
        assert_eq!(editor.history().entries.len(), history);
    }
    panel.reload(&editor);
    editor.play();
    editor.sync();
    assert_eq!(editor.mode(), Mode::Play);
    stage(&mut panel);
    assert!(!panel.can_edit(&editor));
    assert!(!panel.can_reload(&editor));
    let playing = editor.checksum();
    panel.apply(&mut editor);
    assert!(panel.error.is_some());
    assert_eq!(editor.checksum(), playing);
}

#[test]
fn hazard_mission_stale_checksum_rejected_without_panel_sync() {
    let (_directory, mut editor, mut panel) = fixture();
    stage(&mut panel);
    panel.apply(&mut editor);
    assert!(panel.error.is_none());
    panel.points[0][3] = "0.3".into();
    panel.form_dirty = true;
    assert!(editor.undo());
    editor.sync();
    let before = editor.checksum();
    panel.apply(&mut editor);
    assert!(panel.error.is_some());
    assert_eq!(editor.checksum(), before);
    panel.sync(&editor);
    assert!(panel.context_stale);
    assert!(panel.play_blocker(&editor).is_some());
    panel.reload(&editor);
    assert!(!panel.dirty());
    assert!(panel.can_edit(&editor));
}

#[test]
fn hazard_mission_exact_profile_epoch_graph_and_readback_are_required() {
    let (_directory, editor, mut panel) = fixture();
    let expected = config(&editor);
    panel.game = Some(EditorGame::MeshNavigation);
    assert!(!panel.can_edit(&editor));
    assert!(panel.refresh_after_apply(&editor, expected).is_err());
    panel.reload(&editor);
    panel.scene_epoch = Some(0);
    assert!(!panel.can_edit(&editor));
    assert!(panel.refresh_after_apply(&editor, expected).is_err());
    panel.reload(&editor);
    panel.graph_revision = Some([0; 32]);
    assert!(!panel.can_edit(&editor));
    assert!(panel.refresh_after_apply(&editor, expected).is_err());
    panel.reload(&editor);
    stage(&mut panel);
    let changed = panel.parse_form(&editor).unwrap();
    assert!(panel.refresh_after_apply(&editor, changed).is_err());
    assert!(panel.dirty());
}

#[test]
fn hazard_mission_remove_has_authoritative_readback_and_legacy_tools_stay_closed() {
    let (_directory, mut editor, mut panel) = fixture();
    assert!(!panel.can_stage_route_checkpoints(&editor));
    let before = editor.checksum();
    panel.stage_route_checkpoints(&editor);
    assert!(panel.error.is_some());
    assert_eq!(editor.checksum(), before);
    panel.confirm_remove = true;
    panel.remove(&mut editor);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert!(!panel.dirty());
    assert!(!panel.current_mission_enabled());
    assert!(editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<MeshMission>()
        .disabled());
    assert!(editor.undo());
    editor.sync();
    panel.sync(&editor);
    assert!(panel.current_mission_enabled());
}

#[test]
fn mission_identity_does_not_admit_implicit_or_mismatched_profiles() {
    use crate::backend::HostSpec;
    use crate::mesh_context::reported_scene_matches_request;
    let path = std::path::Path::new("mission.scene.yaml");
    assert!(!reported_scene_matches_request(
        &HostSpec::local(path),
        EditorGame::MeshHazardsV1,
        path
    ));
    let explicit = HostSpec::local_game(path, EditorGame::MeshHazardsV1);
    assert!(!reported_scene_matches_request(
        &explicit,
        EditorGame::MeshNavigation,
        path
    ));
    assert!(!reported_scene_matches_request(
        &explicit,
        EditorGame::MeshHazardsV1,
        std::path::Path::new("other.scene.yaml")
    ));
}
