use super::{History, MAX_GRAPH_BYTES, MAX_HISTORY_STATES};
use crate::{editor::Editor, game::EditorGame};
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use std::{fs, path::PathBuf};

fn offset_floor(offset: i64) -> StaticMeshGraph {
    let unit = FP::SCALE;
    let point = |coordinate_x: i64, coordinate_y: i64, coordinate_z: i64| {
        [
            FP::from_raw(coordinate_x * unit),
            FP::from_raw(coordinate_y * unit),
            FP::from_raw(coordinate_z * unit),
        ]
    };
    StaticMeshGraph::bake(
        &[
            point(offset, 0, 0),
            point(offset + 1, 0, 0),
            point(offset, 0, 1),
            point(offset + 1, 0, 1),
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap()
}

fn oversized_graph() -> StaticMeshGraph {
    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    for triangle_index in 0..120i64 {
        let first_vertex = u32::try_from(vertices.len()).unwrap();
        let base_x = triangle_index * 3 * FP::SCALE;
        vertices.extend([
            [FP::from_raw(base_x), FP::ZERO, FP::ZERO],
            [FP::from_raw(base_x + FP::SCALE), FP::ZERO, FP::ZERO],
            [FP::from_raw(base_x), FP::ZERO, FP::from_raw(FP::SCALE)],
        ]);
        faces.push([first_vertex, first_vertex + 2, first_vertex + 1]);
    }
    let graph = StaticMeshGraph::bake(&vertices, &faces, AgentProfile::default()).unwrap();
    assert!(graph.to_bytes().len() > MAX_GRAPH_BYTES);
    graph
}

#[test]
fn reset_tracks_empty_origin_and_roundtrips_graph_states() {
    let initial_graph = offset_floor(0);
    let edited_graph = offset_floor(2);
    let mut history = History::default();

    assert_eq!(history.current(), None);
    assert!(!history.can_undo());
    assert!(!history.can_redo());
    assert!(!history.at_origin());
    assert!(history.push(Some(&initial_graph)).is_err());

    history.reset(None).unwrap();
    assert_eq!(history.current(), Some(None));
    assert!(history.at_origin());
    assert_eq!(history.push(Some(&initial_graph)), Ok(true));
    assert_eq!(history.current(), Some(Some(&initial_graph)));
    assert!(!history.at_origin());
    assert_eq!(history.undo(), Some(None));
    assert!(history.at_origin());
    assert_eq!(history.redo(), Some(Some(initial_graph.clone())));
    assert_eq!(history.current(), Some(Some(&initial_graph)));

    assert_eq!(history.push(Some(&edited_graph)), Ok(true));
    assert_eq!(history.undo(), Some(Some(initial_graph.clone())));
    assert_eq!(history.redo(), Some(Some(edited_graph.clone())));
    assert_eq!(history.current(), Some(Some(&edited_graph)));
}

#[test]
fn reset_graph_origin_is_preserved_and_identical_pushes_are_noops() {
    let initial_graph = offset_floor(0);
    let mut history = History::default();
    history.reset(Some(&initial_graph)).unwrap();

    assert!(history.at_origin());
    assert_eq!(history.push(Some(&initial_graph)), Ok(false));
    assert!(!history.can_undo());
    assert!(!history.can_redo());
    assert_eq!(history.current(), Some(Some(&initial_graph)));
}

#[test]
fn branching_after_undo_discards_redo_states() {
    let first_graph = offset_floor(0);
    let second_graph = offset_floor(2);
    let branch_graph = offset_floor(4);
    let mut history = History::default();
    history.reset(Some(&first_graph)).unwrap();
    history.push(Some(&second_graph)).unwrap();
    history.undo();

    assert!(history.can_redo());
    assert_eq!(history.push(Some(&branch_graph)), Ok(true));
    assert!(!history.can_redo());
    assert_eq!(history.current(), Some(Some(&branch_graph)));
}

#[test]
fn pruning_is_bounded_without_redefining_the_origin() {
    let origin_graph = offset_floor(0);
    let mut history = History::default();
    history.reset(Some(&origin_graph)).unwrap();

    for offset in 1..24 {
        let graph = offset_floor(offset * 2);
        history.push(Some(&graph)).unwrap();
    }

    assert_eq!(history.states.len(), MAX_HISTORY_STATES);
    for _ in 1..MAX_HISTORY_STATES {
        assert!(history.undo().is_some());
    }
    assert!(!history.can_undo());
    assert!(!history.at_origin());
    assert!(history.current().unwrap().unwrap().revision() != origin_graph.revision());

    assert_eq!(history.push(Some(&origin_graph)), Ok(true));
    assert!(history.at_origin());
}

#[test]
fn oversized_graph_refusal_preserves_the_entire_history_cursor_and_redo() {
    let initial_graph = offset_floor(0);
    let next_graph = offset_floor(2);
    let oversized = oversized_graph();
    let mut history = History::default();
    history.reset(Some(&initial_graph)).unwrap();
    history.push(Some(&next_graph)).unwrap();
    history.undo();
    let previous_states = history.states.clone();
    let previous_bytes = history.state_bytes.clone();
    let previous_cursor = history.cursor;
    let previous_origin = history.origin_bytes.clone();

    assert!(history.push(Some(&oversized)).is_err());
    assert_eq!(history.states, previous_states);
    assert_eq!(history.state_bytes, previous_bytes);
    assert_eq!(history.cursor, previous_cursor);
    assert_eq!(history.origin_bytes, previous_origin);
    assert!(history.can_redo());
    assert_eq!(history.current(), Some(Some(&initial_graph)));
}

#[test]
fn reset_refusal_preserves_existing_history() {
    let initial_graph = offset_floor(0);
    let next_graph = offset_floor(2);
    let oversized = oversized_graph();
    let mut history = History::default();
    history.reset(Some(&initial_graph)).unwrap();
    history.push(Some(&next_graph)).unwrap();
    let previous_states = history.states.clone();
    let previous_bytes = history.state_bytes.clone();
    let previous_cursor = history.cursor;
    let previous_origin = history.origin_bytes.clone();

    assert!(history.reset(Some(&oversized)).is_err());
    assert_eq!(history.states, previous_states);
    assert_eq!(history.state_bytes, previous_bytes);
    assert_eq!(history.cursor, previous_cursor);
    assert_eq!(history.origin_bytes, previous_origin);
}

struct PanelScene {
    _directory: tempfile::TempDir,
    path: PathBuf,
}

impl PanelScene {
    fn create() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let scene = directory.path().join("geometry-history.scene.yaml");
        fs::write(
            &scene,
            "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
        )
        .unwrap();
        Self {
            _directory: directory,
            path: scene,
        }
    }

    fn editor(&self) -> (Editor, super::super::Panel) {
        let mut editor = Editor::open_game(&self.path, EditorGame::MeshNavigation).unwrap();
        editor.sync();
        let mut panel = super::super::Panel::default();
        panel.sync(&editor);
        (editor, panel)
    }
}

fn append_quad(origin_x: i64) -> super::super::geometry_panel::Action {
    super::super::geometry_panel::Action::AppendQuad(super::super::geometry::Quad {
        origin: [FP::from_raw(origin_x * FP::SCALE), FP::ZERO, FP::ZERO],
        width: FP::from_raw(4 * FP::SCALE),
        depth: FP::from_raw(4 * FP::SCALE),
        rise: FP::ZERO,
    })
}

#[test]
fn panel_preview_undo_redo_and_place_keep_host_history_separate() {
    let scene = PanelScene::create();
    let (mut editor, mut panel) = scene.editor();
    let initial_checksum = editor.checksum();
    let initial_history_len = editor.history().entries.len();

    panel.stage_geometry(append_quad(0), &editor).unwrap();
    let staged_revision = panel.graph.as_ref().unwrap().revision();
    assert!(panel.geometry_preview_active());
    assert!(panel.model().is_some());
    assert_eq!(
        panel
            .geometry_history
            .current()
            .unwrap()
            .unwrap()
            .revision(),
        staged_revision
    );
    assert_eq!(editor.checksum(), initial_checksum);
    assert_eq!(editor.history().entries.len(), initial_history_len);

    panel.change_geometry_history(&editor, false).unwrap();
    assert!(panel.geometry_history.at_origin());
    assert_eq!(panel.geometry_history.current(), Some(None));
    assert!(panel.graph.is_none());
    assert!(!panel.geometry_staged);
    assert!(!panel.geometry_preview_active());
    assert!(panel.model().is_none());
    assert_eq!(editor.checksum(), initial_checksum);
    assert_eq!(editor.history().entries.len(), initial_history_len);

    panel.change_geometry_history(&editor, true).unwrap();
    assert_eq!(panel.graph.as_ref().unwrap().revision(), staged_revision);
    assert!(panel.geometry_staged);
    assert!(panel.geometry_preview_active());
    assert!(panel.model().is_some());
    assert_eq!(editor.checksum(), initial_checksum);
    assert_eq!(editor.history().entries.len(), initial_history_len);

    panel.place_agent(&mut editor).unwrap();
    assert_eq!(editor.history().entries.len(), initial_history_len + 1);
    assert!(panel.geometry_history.current().is_none());
    assert!(!panel.geometry_history.can_undo());
    assert!(!panel.geometry_history.can_redo());
    assert!(!panel.geometry_staged);
    assert!(!panel.geometry_preview_active());
}

#[test]
fn undo_to_preexisting_unplaced_graph_retains_agent_text_and_dirty_state() {
    let scene = PanelScene::create();
    let (editor, mut panel) = scene.editor();
    panel.create_ramp_and_bridge().unwrap();
    let origin_revision = panel.graph.as_ref().unwrap().revision();
    assert!(panel.draft_dirty);

    panel.start[0] = "1.25".into();
    panel.stage_geometry(append_quad(10), &editor).unwrap();
    panel.start[0] = "1.5".into();
    panel.change_geometry_history(&editor, false).unwrap();

    assert_eq!(panel.graph.as_ref().unwrap().revision(), origin_revision);
    assert_eq!(panel.start[0], "1.5");
    assert!(panel.draft_dirty);
    assert!(!panel.geometry_staged);
    assert!(!panel.geometry_preview_active());
    assert!(panel.geometry_history.at_origin());
}

#[test]
fn external_host_change_blocks_local_undo_without_advancing_history() {
    let scene = PanelScene::create();
    let (mut editor, mut original) = scene.editor();
    original.create_ramp_and_bridge().unwrap();
    original.place_agent(&mut editor).unwrap();
    original.sync(&editor);
    original.stage_geometry(append_quad(10), &editor).unwrap();
    let staged_revision = original.graph.as_ref().unwrap().revision();
    let staged_cursor = original.geometry_history.cursor;

    let mut external = super::super::Panel::default();
    external.sync(&editor);
    external.create_ramp_and_bridge().unwrap();
    external.distance_per_tick = "0.0625".into();
    external.place_agent(&mut editor).unwrap();
    let external_checksum = editor.checksum();
    let external_history_len = editor.history().entries.len();
    assert_ne!(external_checksum, original.geometry_host_context.unwrap().1);
    original.sync(&editor);
    assert!(!original.geometry_preview_active());
    assert!(original.geometry_staged);
    assert_eq!(original.geometry_history.cursor, staged_cursor);

    let error = original
        .change_geometry_history(&editor, false)
        .unwrap_err();
    assert!(error.contains("Host mesh changed while geometry was staged"));
    assert_eq!(original.graph.as_ref().unwrap().revision(), staged_revision);
    assert_eq!(original.geometry_history.cursor, staged_cursor);
    assert!(original.geometry_staged);
    assert!(!original.geometry_preview_active());
    assert_eq!(editor.checksum(), external_checksum);
    assert_eq!(editor.history().entries.len(), external_history_len);
}

#[test]
fn replacing_staged_graph_with_builtin_or_asset_clears_local_history_and_preview() {
    let scene = PanelScene::create();
    let (editor, mut panel) = scene.editor();
    panel.stage_geometry(append_quad(0), &editor).unwrap();
    let staged_revision = panel.graph.as_ref().unwrap().revision();
    let staged_cursor = panel.geometry_history.cursor;
    let initial_checksum = editor.checksum();
    let initial_history_len = editor.history().entries.len();
    assert!(panel.geometry_preview_active());
    assert!(panel.geometry_history.can_undo());

    let replacement_error = panel.create_ramp_and_bridge().unwrap_err();
    assert_eq!(
        replacement_error,
        "Discard unplaced geometry changes before creating or opening another mesh source"
    );
    assert_eq!(panel.graph.as_ref().unwrap().revision(), staged_revision);
    assert_eq!(panel.geometry_history.cursor, staged_cursor);
    assert!(panel.geometry_staged);
    assert!(panel.geometry_preview_active());
    assert_eq!(editor.checksum(), initial_checksum);
    assert_eq!(editor.history().entries.len(), initial_history_len);

    panel.asset_path.clear();
    let asset_guard_error = panel.open_mesh_asset().unwrap_err();
    assert_eq!(asset_guard_error, replacement_error);
    assert_eq!(panel.graph.as_ref().unwrap().revision(), staged_revision);
    assert_eq!(panel.geometry_history.cursor, staged_cursor);
    assert!(panel.geometry_staged);
    assert!(panel.geometry_preview_active());

    panel.change_geometry_history(&editor, false).unwrap();
    assert!(panel.graph.is_none());
    assert!(!panel.geometry_staged);
    assert!(!panel.geometry_preview_active());
    assert!(panel.geometry_history.at_origin());
    assert!(panel.geometry_history.can_redo());

    panel.create_ramp_and_bridge().unwrap();
    assert!(panel.graph.is_some());
    assert!(!panel.geometry_staged);
    assert!(!panel.geometry_preview_active());
    assert!(panel.geometry_history.current().is_none());
    assert!(!panel.geometry_history.can_undo());
    assert!(!panel.geometry_history.can_redo());

    let asset_path = scene._directory.path().join("replacement.orrmesh");
    fs::write(&asset_path, panel.graph.as_ref().unwrap().to_bytes()).unwrap();

    panel.stage_geometry(append_quad(10), &editor).unwrap();
    let staged_asset_revision = panel.graph.as_ref().unwrap().revision();
    let staged_asset_cursor = panel.geometry_history.cursor;
    panel.asset_path = asset_path.display().to_string();
    let asset_replacement_error = panel.open_mesh_asset().unwrap_err();
    assert_eq!(asset_replacement_error, replacement_error);
    assert_eq!(
        panel.graph.as_ref().unwrap().revision(),
        staged_asset_revision
    );
    assert_eq!(panel.geometry_history.cursor, staged_asset_cursor);
    assert!(panel.geometry_staged);
    assert!(panel.geometry_preview_active());
    panel.asset_path.clear();

    panel.change_geometry_history(&editor, false).unwrap();
    assert!(panel.geometry_history.at_origin());
    assert!(panel.geometry_history.can_redo());
    assert!(!panel.geometry_staged);
    assert!(!panel.geometry_preview_active());

    panel.asset_path = asset_path.display().to_string();
    panel.open_mesh_asset().unwrap();
    assert!(!panel.geometry_staged);
    assert!(!panel.geometry_preview_active());
    assert!(panel.geometry_history.current().is_none());
    assert!(!panel.geometry_history.can_undo());
    assert!(!panel.geometry_history.can_redo());
    assert_eq!(
        panel.graph.as_ref().unwrap().revision(),
        StaticMeshGraph::from_bytes(&fs::read(&asset_path).unwrap())
            .unwrap()
            .revision()
    );
}
