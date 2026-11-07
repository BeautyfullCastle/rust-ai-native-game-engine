use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};

fn fixture() -> (StaticMeshGraph, Context) {
    let graph = crate::mesh_navigation::fixture_graph(FP::from_int(4)).unwrap();
    let context = Context {
        revision: graph.revision(),
        scene: Some("draft.scene.yaml".into()),
        epoch: 1,
        fields: ["1", "0", "1", "3", "2", "1", "4", "4"].map(str::to_owned),
    };
    (graph, context)
}

#[test]
fn actual_inspect_button_reports_both_endpoints_and_valid_route_without_writes() {
    let (graph, context) = fixture();
    let bytes = graph.to_bytes();
    let retained = std::sync::Arc::new(graph);
    let graph = retained.clone();
    let mut harness = Harness::builder()
        .with_size([1000.0, 800.0])
        .build_ui_state(
            move |ui, state: &mut State| state.show(ui, Some(&graph), Ok(context.clone())),
            State::default(),
        );
    harness.run_steps(2);
    assert!(harness.state().observation.is_none());
    harness.get_by_label("Inspect draft route").click();
    harness.run_steps(2);
    let (_, report) = harness.state().observation.as_ref().unwrap();
    let report = report.as_ref().unwrap();
    assert!(matches!(
        report.route,
        route_diagnostics::RouteOutcome::Ready { .. }
    ));
    assert!(report.start.is_ok());
    assert!(report.goal.is_ok());
    assert!(harness
        .query_by_label(
            "Inspection does not place or move an agent, save a scene, or authorize placement."
        )
        .is_some());
    assert_eq!(retained.to_bytes(), bytes);
}

#[test]
fn stale_fields_graph_scene_and_epoch_clear_report_without_reinspection() {
    let (graph, context) = fixture();
    for change in 0..12 {
        let mut state = State {
            observation: Some((context.clone(), inspect_fields(&graph, &context))),
            ..Default::default()
        };
        let mut changed = context.clone();
        match change {
            0..=7 => changed.fields[change].push('0'),
            8 => changed.revision[0] ^= 1,
            9 => changed.scene = Some("another.scene.yaml".into()),
            10 => changed.epoch += 1,
            _ => {
                state.sync(None);
                assert!(state.observation.is_none());
                continue;
            }
        }
        state.sync(Some(&changed));
        assert!(state.observation.is_none());
        state.sync(Some(&context)); // A -> B -> A never revives an old report.
        assert!(state.observation.is_none());
    }
}

#[test]
fn endpoint_labels_and_malformed_field_diagnostics_are_specific() {
    let (graph, mut context) = fixture();
    context.fields[3] = "250".into();
    let mut harness = Harness::builder()
        .with_size([1000.0, 800.0])
        .build_ui_state(
            move |ui, state: &mut State| state.show(ui, Some(&graph), Ok(context.clone())),
            State::default(),
        );
    harness.run_steps(2);
    harness.get_by_label("Inspect draft route").click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Goal projection failed: outside the loaded walkable triangles or beyond maximum vertical projection").is_some());
    assert!(harness
        .query_by_label("Route not searched: fix endpoint projection first.")
        .is_some());
    let (graph, mut context) = fixture();
    context.fields[0] = "not-a-number".into();
    assert!(inspect_fields(&graph, &context)
        .unwrap_err()
        .starts_with("Start X:"));
    context.fields[0] = "1".into();
    context.fields[6] = "-1".into();
    assert!(inspect_fields(&graph, &context)
        .unwrap_err()
        .contains("nonnegative"));
}

#[test]
fn blocked_draft_disables_button_and_discards_prior_report() {
    let (graph, context) = fixture();
    let state = State {
        observation: Some((context.clone(), inspect_fields(&graph, &context))),
        ..Default::default()
    };
    let mut harness = Harness::builder().build_ui_state(
        move |ui, state: &mut State| {
            state.show(ui, Some(&graph), Err("Open selected graph".into()))
        },
        state,
    );
    harness.run_steps(2);
    assert!(harness
        .get_by_label("Inspect draft route")
        .accesskit_node()
        .is_disabled());
    assert!(harness.state().observation.is_none());
}

#[test]
fn actual_editor_inspection_preserves_selection_history_checksum_and_scene_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("route.scene.yaml");
    std::fs::write(&path, "schema: orr.scene/1\nsingletons: {}\nentities: {}\n").unwrap();
    let mut editor = Editor::open_game(&path, crate::game::EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.create_ramp_and_bridge().unwrap();
    panel.place_agent(&mut editor).unwrap();
    panel.sync(&editor);
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let selection = editor.selection().cloned();
    let bytes = std::fs::read(&path).unwrap();
    let graph_bytes = panel.graph.as_ref().unwrap().to_bytes();
    let mut harness = Harness::builder()
        .with_size([1000.0, 800.0])
        .build_ui_state(
            |ui, state: &mut (Panel, Editor)| state.0.show_route_diagnostics(ui, &state.1),
            (panel, editor),
        );
    harness.run_steps(2);
    harness.get_by_label("Inspect draft route").click();
    harness.run_steps(2);
    let (panel, editor) = harness.state();
    assert!(panel
        .route_diagnostics
        .observation
        .as_ref()
        .unwrap()
        .1
        .is_ok());
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.selection(), selection.as_ref());
    assert_eq!(panel.graph.as_ref().unwrap().to_bytes(), graph_bytes);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
