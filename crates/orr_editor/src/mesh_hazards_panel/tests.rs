use super::*;
use crate::game::EditorGame;

fn fixture() -> (tempfile::TempDir, Editor, Panel) {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("hazards.scene.yaml");
    let legacy = crate::mesh_workspace::mission::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    drop(legacy);
    let mut editor = Editor::open_game(&scene, EditorGame::MeshHazardsV1).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.sync(&editor);
    assert!(panel.can_edit(&editor));
    (directory, editor, panel)
}

fn stage(panel: &mut Panel) {
    panel.enabled = true;
    panel.count = "1".into();
    panel.spheres = default_spheres();
    panel.form_dirty = true;
}

#[test]
fn fixed_point_fields_roundtrip_and_count_bounds() {
    let sphere = parse_sphere(&["-0.1", "0.25", "1.5", "0.4"].map(str::to_owned)).unwrap();
    assert_eq!(parse_sphere(&format_sphere(sphere)).unwrap(), sphere);
    assert!(parse_sphere(&["NaN", "0", "0", "1"].map(str::to_owned)).is_err());
    assert!(parse_sphere(&["1".repeat(49), "0".into(), "0".into(), "1".into()]).is_err());
    for value in ["", "0", "9", "01", "-1", "1.0", "+1"] {
        assert!(parse_count(value).is_err());
    }
    assert_eq!(parse_count("8").unwrap(), 8);
}

#[test]
fn apply_roundtrips_exact_configuration_and_undo_redo() {
    let (_directory, mut editor, mut panel) = fixture();
    stage(&mut panel);
    panel.spheres[0][0] = "-0.1".into();
    let before = editor.checksum();
    panel.apply(&mut editor);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert!(!panel.dirty());
    let after = editor.checksum();
    assert_ne!(before, after);
    let (_, view) = current(&editor).unwrap();
    let config = view.unwrap().config;
    assert_eq!(config.spheres[0], parse_sphere(&panel.spheres[0]).unwrap());
    assert!(config.spheres[1..]
        .iter()
        .all(|sphere| *sphere == HazardSphere::default()));
    assert!(panel.play_blocker(&editor).is_none());
    assert!(editor.undo());
    editor.sync();
    panel.sync(&editor);
    assert_eq!(editor.checksum(), before);
    assert!(!panel.enabled);
    assert!(editor.redo());
    editor.sync();
    panel.sync(&editor);
    assert_eq!(editor.checksum(), after);
    assert!(panel.enabled);
}

#[test]
fn disabled_apply_ignores_invalid_draft_geometry_and_is_canonical_zero() {
    let (_directory, mut editor, mut panel) = fixture();
    stage(&mut panel);
    panel.apply(&mut editor);
    assert!(panel.error.is_none());
    panel.enabled = false;
    panel.count = "invalid".into();
    panel.spheres[0][3] = "NaN".into();
    panel.form_dirty = true;
    panel.apply(&mut editor);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert!(!panel.dirty());
    assert!(current(&editor).unwrap().1.is_none());
    assert!(editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<MeshHazards>()
        .disabled());
}

#[test]
fn invalid_geometry_does_not_mutate_host_or_history() {
    let (_directory, mut editor, mut panel) = fixture();
    stage(&mut panel);
    let before = editor.checksum();
    let history = editor.history().entries.len();
    for (axis, value) in [(0, "256.1"), (3, "0"), (3, "-1"), (3, "256.1"), (3, "NaN")] {
        panel.spheres = default_spheres();
        panel.spheres[0][axis] = value.into();
        panel.apply(&mut editor);
        assert!(panel.error.is_some());
        assert!(panel.dirty());
        assert_eq!(editor.checksum(), before);
        assert_eq!(editor.history().entries.len(), history);
    }
    panel.spheres = default_spheres();
    panel.spheres[0][3] = "256".into();
    assert!(panel.parse_form(&current(&editor).unwrap().0).is_ok());
}

#[test]
fn stale_checksum_rejected_even_without_panel_sync_and_reload_discards_draft() {
    let (_directory, mut editor, mut panel) = fixture();
    stage(&mut panel);
    panel.apply(&mut editor);
    stage(&mut panel);
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
fn scene_epoch_and_graph_staleness_reject_apply_and_readback() {
    let (_directory, mut editor, mut panel) = fixture();
    stage(&mut panel);
    let expected = panel.parse_form(&current(&editor).unwrap().0).unwrap();
    panel.identity.as_mut().unwrap().epoch = 0;
    assert!(!panel.can_edit(&editor));
    assert!(panel.refresh_after_apply(&editor, expected).is_err());
    assert!(panel.dirty());
    panel.reload(&editor);
    panel.identity.as_mut().unwrap().revision = [0; 32];
    stage(&mut panel);
    panel.apply(&mut editor);
    assert!(panel.error.is_some());
    panel.sync(&editor);
    assert!(panel.context_stale);
}

#[test]
fn mismatched_authoritative_readback_keeps_draft_and_blocks_play() {
    let (_directory, editor, mut panel) = fixture();
    stage(&mut panel);
    let expected = panel.parse_form(&current(&editor).unwrap().0).unwrap();
    assert!(panel.refresh_after_apply(&editor, expected).is_err());
    assert!(panel.context_stale);
    assert!(panel.form_dirty);
    assert!(panel.play_blocker(&editor).is_some());
}

#[test]
fn play_rejects_authoring_and_reload() {
    let (_directory, mut editor, mut panel) = fixture();
    editor.play();
    editor.sync();
    assert_eq!(editor.mode(), Mode::Play);
    stage(&mut panel);
    assert!(!panel.can_edit(&editor));
    assert!(!panel.can_reload(&editor));
    panel.apply(&mut editor);
    assert!(panel.error.is_some());
}

#[test]
fn legacy_profile_never_reads_hazard_singletons() {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("legacy.scene.yaml");
    let mut editor = crate::mesh_workspace::mission::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    let mut panel = Panel::default();
    panel.sync(&editor);
    assert!(current(&editor).is_err());
    assert!(!panel.can_edit(&editor));
    assert!(panel.play_blocker(&editor).is_none());
    panel.apply(&mut editor);
    assert!(panel.error.is_some());
}

#[test]
fn headless_widgets_stage_apply_and_reload() {
    use egui_kittest::{kittest::Queryable, Harness};
    let (directory, editor, panel) = fixture();
    struct State {
        _directory: tempfile::TempDir,
        editor: Editor,
        panel: Panel,
    }
    let mut harness = Harness::builder()
        .with_size([900.0, 1000.0])
        .build_ui_state(
            |ui, state: &mut State| state.panel.show(ui, &mut state.editor),
            State {
                _directory: directory,
                editor,
                panel,
            },
        );
    harness.run_steps(3);
    harness.get_by_label("Mesh hazards").click();
    harness.run_steps(3);
    harness.get_by_label("Enable mesh hazards").click();
    harness.run_steps(3);
    assert!(harness.state().panel.dirty());
    harness.get_by_label("Apply mesh hazards").click();
    harness.run_steps(3);
    assert!(harness.state().panel.error.is_none());
    assert!(!harness.state().panel.dirty());
    assert!(current(&harness.state().editor).unwrap().1.is_some());
    harness.get_by_label("Enable mesh hazards").click();
    harness.run_steps(3);
    harness.get_by_label("Reload hazard fields").click();
    harness.run_steps(3);
    assert!(harness.state().panel.enabled);
    assert!(!harness.state().panel.dirty());
}

#[test]
fn pending_unadmitted_graph_cannot_apply_reload_or_play() {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("blank.scene.yaml");
    std::fs::write(
        &scene,
        "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::MeshHazardsV1).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.sync(&editor);
    assert!(panel.document_pending);
    assert!(!panel.can_edit(&editor));
    assert!(!panel.can_reload(&editor));
    assert!(panel.play_blocker(&editor).is_some());
    stage(&mut panel);
    let before = editor.checksum();
    panel.apply(&mut editor);
    assert!(panel.error.is_some());
    assert_eq!(editor.checksum(), before);
}
