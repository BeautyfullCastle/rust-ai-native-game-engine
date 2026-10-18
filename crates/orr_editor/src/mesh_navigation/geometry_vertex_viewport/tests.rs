use super::*;
use crate::{editor::Editor, game::EditorGame};
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_render::OrbitCamera;
use std::fs;

fn floor_graph() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            [FP::from_int(-2), FP::ZERO, FP::from_int(-2)],
            [FP::from_int(2), FP::ZERO, FP::from_int(-2)],
            [FP::from_int(-2), FP::ZERO, FP::from_int(2)],
            [FP::from_int(2), FP::ZERO, FP::from_int(2)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile {
            max_slope: FP::from_int(4),
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        },
    )
    .unwrap()
}

#[test]
fn drag_preview_uses_base_graph_and_does_not_mutate_it() {
    let graph = floor_graph();
    let original_bytes = graph.to_bytes();
    let camera = OrbitCamera::new([0.0, 0.0, 0.0], 0.0, 0.8, 8.0).camera();
    let viewport = (800, 600);
    let initial = [-2.0, 0.0, -2.0];
    let pixel = camera.world_to_screen(initial, viewport).unwrap();
    let drag = crate::mesh_prop_view::Drag::new(&camera, pixel, viewport, initial).unwrap();
    let target = drag
        .position(&camera, [pixel[0] + 18.0, pixel[1]], viewport, Some(0.25))
        .unwrap();
    let position = super::super::geometry_vertex_view::quantize_snapped(target).unwrap();
    let candidate = geometry::move_vertex(&graph, 0, position).unwrap();
    let candidate_preview = geometry_preview::build(&candidate).unwrap();
    assert_ne!(candidate.revision(), graph.revision());
    assert!(!candidate_preview.source().primitives.is_empty());
    assert_eq!(graph.to_bytes(), original_bytes);
}

#[test]
fn cancel_discards_transient_preview_without_changing_the_source_graph() {
    let graph = floor_graph();
    let original_bytes = graph.to_bytes();
    let candidate =
        geometry::move_vertex(&graph, 0, [FP::from_int(-1), FP::ZERO, FP::from_int(-2)]).unwrap();
    let preview = geometry_preview::build(&candidate).unwrap();
    let mut state = State {
        armed: true,
        selected: Some(0),
        context: None,
        status: String::new(),
        snap: "0.25".into(),
        drag: Some(DragState {
            graph: graph.clone(),
            index: 0,
            drag: crate::mesh_prop_view::Drag::new(
                &OrbitCamera::new([0.0, 0.0, 0.0], 0.0, 0.8, 8.0).camera(),
                [400.0, 300.0],
                (800, 600),
                [-2.0, 0.0, -2.0],
            )
            .unwrap(),
            camera: OrbitCamera::new([0.0, 0.0, 0.0], 0.0, 0.8, 8.0).camera(),
            viewport: (800, 600),
            candidate_model: Some(preview),
            final_position: Some(candidate.vertices()[0]),
            final_valid: true,
        }),
    };
    assert!(state.preview_active());
    state.cancel("canceled");
    assert!(!state.preview_active());
    assert!(!state.busy());
    assert_eq!(graph.to_bytes(), original_bytes);
}

#[test]
fn valid_release_stages_one_local_entry_without_writing_the_host() {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("vertex-view.scene.yaml");
    fs::write(
        &scene,
        "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut panel = crate::mesh_navigation::Panel::default();
    panel.create_ramp_and_bridge().unwrap();
    panel.place_agent(&mut editor).unwrap();
    panel.sync(&editor);
    let graph = panel.graph.as_ref().unwrap().clone();
    let base_bytes = graph.to_bytes();
    let host_checksum = editor.checksum();
    let host_history = editor.history().entries.len();
    let camera = OrbitCamera::new([2.0, 1.0, 1.0], 0.18, 0.82, 13.0).camera();
    let viewport = (800, 600);
    let index = graph
        .vertices()
        .iter()
        .enumerate()
        .find_map(|(index, vertex)| {
            let point: [f32; 3] = vertex
                .map(super::super::geometry_vertex_view::exact_display)
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .ok()?
                .try_into()
                .ok()?;
            let pixel = camera.world_to_screen([point[0], point[1], point[2]], viewport)?;
            if pixel[0] < 0.0
                || pixel[1] < 0.0
                || pixel[0] > viewport.0 as f32
                || pixel[1] > viewport.1 as f32
                || super::super::geometry_vertex_view::pick_vertex(&graph, &camera, pixel, viewport)
                    .ok()?
                    != Some(index as u32)
            {
                return None;
            }
            Some((index as u32, point, pixel))
        })
        .unwrap();
    let (vertex_index, point, press_pixel) = index;
    let mut target_pixel = None;
    let mut expected_position = None;
    for delta in [4.0, 8.0, 12.0, 16.0, 20.0, 24.0] {
        let drag = crate::mesh_prop_view::Drag::new(&camera, press_pixel, viewport, point).unwrap();
        let Some(target) = drag.position(
            &camera,
            [press_pixel[0] + delta, press_pixel[1]],
            viewport,
            Some(0.5),
        ) else {
            continue;
        };
        let Ok(position) = super::super::geometry_vertex_view::quantize_snapped(target) else {
            continue;
        };
        if geometry::move_vertex(&graph, vertex_index, position)
            .is_ok_and(|candidate| candidate.revision() != graph.revision())
        {
            target_pixel = Some([press_pixel[0] + delta, press_pixel[1]]);
            expected_position = Some(position);
            break;
        }
    }
    let target_pixel = target_pixel.expect("fixture should admit at least one snapped vertex move");
    let expected_position = expected_position.unwrap();
    let mut state = State::default();
    state.snap = "0.5".into();
    state.arm(&panel, &editor);
    assert!(state.armed);
    state.input(
        &mut panel,
        &editor,
        &camera,
        viewport,
        ViewInput {
            pixel: Some(press_pixel),
            press_pixel: Some(press_pixel),
            started: true,
            ..ViewInput::default()
        },
    );
    state.input(
        &mut panel,
        &editor,
        &camera,
        viewport,
        ViewInput {
            pixel: Some(target_pixel),
            dragging: true,
            ..ViewInput::default()
        },
    );
    assert!(state.preview_active());
    state.input(
        &mut panel,
        &editor,
        &camera,
        viewport,
        ViewInput {
            pixel: Some(target_pixel),
            released: true,
            ..ViewInput::default()
        },
    );
    assert!(state.busy());
    assert!(panel.geometry_history.can_undo());
    assert_eq!(editor.checksum(), host_checksum);
    assert_eq!(editor.history().entries.len(), host_history);
    assert_eq!(graph.to_bytes(), base_bytes);
    assert_ne!(panel.graph.as_ref().unwrap().revision(), graph.revision());
    let staged_graph = panel.graph.as_ref().unwrap().clone();
    let selected = state.selected.unwrap() as usize;
    assert_eq!(staged_graph.vertices()[selected], expected_position,);
    state.finish();
    assert!(!state.busy());
    assert_eq!(panel.geometry_history.undo(), Some(Some(graph.clone())));
    assert!(!panel.geometry_history.can_undo());
    assert_eq!(panel.geometry_history.redo(), Some(Some(staged_graph)));
}

#[test]
fn staged_unplaced_geometry_can_be_armed_without_changing_the_host() {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("blank-origin.scene.yaml");
    fs::write(
        &scene,
        "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut panel = crate::mesh_navigation::Panel::default();
    panel.sync(&editor);
    let host_checksum = editor.checksum();
    let host_history = editor.history().entries.len();
    panel
        .stage_geometry(
            geometry_panel::Action::AppendQuad(geometry::Quad {
                origin: [FP::from_int(-2), FP::ZERO, FP::from_int(-2)],
                width: FP::from_int(4),
                depth: FP::from_int(4),
                rise: FP::ZERO,
            }),
            &editor,
        )
        .unwrap();
    assert!(panel.geometry_staged);
    assert_eq!(editor.checksum(), host_checksum);
    assert_eq!(editor.history().entries.len(), host_history);
    let mut state = State::default();
    state.arm(&panel, &editor);
    assert!(state.armed);
    assert!(state.context.is_some());
    assert_eq!(editor.checksum(), host_checksum);
    assert_eq!(editor.history().entries.len(), host_history);
}

#[test]
fn invalid_final_pointer_keeps_the_staged_graph_and_history_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("invalid-release.scene.yaml");
    fs::write(
        &scene,
        "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut panel = crate::mesh_navigation::Panel::default();
    panel.sync(&editor);
    panel
        .stage_geometry(
            geometry_panel::Action::AppendQuad(geometry::Quad {
                origin: [FP::from_int(-2), FP::ZERO, FP::from_int(-2)],
                width: FP::from_int(4),
                depth: FP::from_int(4),
                rise: FP::ZERO,
            }),
            &editor,
        )
        .unwrap();
    let graph = panel.graph.as_ref().unwrap().clone();
    let graph_revision = graph.revision();
    let local_history = panel.geometry_history.clone();
    let host_checksum = editor.checksum();
    let host_history_len = editor.history().entries.len();
    let camera = OrbitCamera::new([0.0, 1.0, 1.0], 0.15, 0.8, 8.0).camera();
    let viewport = (800, 600);
    let pixel = graph
        .vertices()
        .iter()
        .enumerate()
        .find_map(|(index, vertex)| {
            let point: [f32; 3] = vertex
                .map(super::super::geometry_vertex_view::exact_display)
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .ok()?
                .try_into()
                .ok()?;
            let pixel = camera.world_to_screen(point, viewport)?;
            (pixel[0] >= 0.0
                && pixel[1] >= 0.0
                && pixel[0] <= viewport.0 as f32
                && pixel[1] <= viewport.1 as f32
                && super::super::geometry_vertex_view::pick_vertex(
                    &graph, &camera, pixel, viewport,
                )
                .ok()?
                    == Some(index as u32))
            .then_some(pixel)
        })
        .unwrap();
    let mut state = State::default();
    state.arm(&panel, &editor);
    state.input(
        &mut panel,
        &editor,
        &camera,
        viewport,
        ViewInput {
            pixel: Some(pixel),
            press_pixel: Some(pixel),
            started: true,
            ..ViewInput::default()
        },
    );
    assert!(state.dragging());
    state.input(
        &mut panel,
        &editor,
        &camera,
        viewport,
        ViewInput {
            pixel: Some([pixel[0] + 24.0, pixel[1]]),
            dragging: true,
            ..ViewInput::default()
        },
    );
    assert!(state.preview_active(), "{}", state.status);
    state.input(
        &mut panel,
        &editor,
        &camera,
        viewport,
        ViewInput {
            pixel: Some([10_000.0, 10_000.0]),
            dragging: true,
            ..ViewInput::default()
        },
    );
    assert!(!state.preview_active());
    assert!(state.preview_model().is_none());
    assert!(state.drag.as_ref().unwrap().final_position.is_none());
    state.input(
        &mut panel,
        &editor,
        &camera,
        viewport,
        ViewInput {
            pixel: Some([10_000.0, 10_000.0]),
            released: true,
            ..ViewInput::default()
        },
    );
    assert!(state.armed);
    assert!(!state.dragging());
    assert_eq!(panel.graph.as_ref().unwrap().revision(), graph_revision);
    assert_eq!(panel.geometry_history, local_history);
    assert_eq!(editor.checksum(), host_checksum);
    assert_eq!(editor.history().entries.len(), host_history_len);
    state.finish();
}

#[test]
fn camera_or_viewport_change_cancels_drag_without_host_or_graph_changes() {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("vertex-cancel.scene.yaml");
    fs::write(
        &scene,
        "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut panel = crate::mesh_navigation::Panel::default();
    panel.create_ramp_and_bridge().unwrap();
    panel.place_agent(&mut editor).unwrap();
    panel.sync(&editor);
    let graph = panel.graph.as_ref().unwrap().clone();
    let original_revision = graph.revision();
    let host_checksum = editor.checksum();
    let host_history = editor.history().entries.len();
    let camera = OrbitCamera::new([2.0, 1.0, 1.0], 0.18, 0.82, 13.0).camera();
    let viewport = (800, 600);
    let pixel = graph
        .vertices()
        .iter()
        .enumerate()
        .find_map(|(index, vertex)| {
            let point: [f32; 3] = vertex
                .map(super::super::geometry_vertex_view::exact_display)
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .ok()?
                .try_into()
                .ok()?;
            let pixel = camera.world_to_screen([point[0], point[1], point[2]], viewport)?;
            (pixel[0] >= 0.0
                && pixel[1] >= 0.0
                && pixel[0] <= viewport.0 as f32
                && pixel[1] <= viewport.1 as f32
                && super::super::geometry_vertex_view::pick_vertex(
                    &graph, &camera, pixel, viewport,
                )
                .ok()?
                    == Some(index as u32))
            .then_some(pixel)
        })
        .unwrap();
    let mut state = State::default();
    state.arm(&panel, &editor);
    state.input(
        &mut panel,
        &editor,
        &camera,
        viewport,
        ViewInput {
            pixel: Some(pixel),
            press_pixel: Some(pixel),
            started: true,
            ..ViewInput::default()
        },
    );
    assert!(state.dragging());
    state.input(
        &mut panel,
        &editor,
        &camera,
        (viewport.0 + 1, viewport.1),
        ViewInput {
            pixel: Some([pixel[0] + 8.0, pixel[1]]),
            dragging: true,
            ..ViewInput::default()
        },
    );
    assert!(!state.busy());
    assert_eq!(panel.graph.as_ref().unwrap().revision(), original_revision);
    assert_eq!(editor.checksum(), host_checksum);
    assert_eq!(editor.history().entries.len(), host_history);
}
