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
fn explicit_checkbox_requires_success_and_fresh_inspection_resets_overlay_without_graph_mutation() {
    let (graph, context) = fixture();
    let bytes = graph.to_bytes();
    let graph = std::sync::Arc::new(graph);
    let retained = graph.clone();
    let mut h = Harness::builder()
        .with_size([1200.0, 1000.0])
        .build_ui_state(
            move |ui, state: &mut State| state.show(ui, Some(&graph), Ok(context.clone())),
            State::default(),
        );
    h.run_steps(2);
    assert!(h
        .get_by_label("Show inspected draft route")
        .accesskit_node()
        .is_disabled());
    assert!(!h.state().show_route);
    h.get_by_label("Inspect draft route").click_accesskit();
    h.run_steps(2);
    assert!(!h.state().show_route);
    assert!(!h
        .get_by_label("Show inspected draft route")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Show inspected draft route")
        .click_accesskit();
    h.run_steps(2);
    assert!(h.state().show_route);
    let (_, result) = h.state().observation.as_ref().unwrap();
    let report = result.as_ref().unwrap();
    let route_diagnostics::RouteOutcome::Ready { path, .. } = &report.route else {
        panic!("expected ready")
    };
    assert!(!path.points.is_empty());
    h.get_by_label("Inspect draft route").click_accesskit();
    h.run_steps(2);
    assert!(!h.state().show_route);
    assert_eq!(retained.to_bytes(), bytes);
}
#[test]
fn every_stale_context_class_clears_observation_and_overlay_without_reviving_old_route() {
    let (graph, context) = fixture();
    for change in 0..12 {
        let mut state = State {
            observation: Some((context.clone(), inspect_fields(&graph, &context))),
            show_route: true,
            ..Default::default()
        };
        let mut changed = context.clone();
        match change {
            0..=7 => changed.fields[change].push('0'),
            8 => changed.revision[0] ^= 1,
            9 => changed.scene = Some("other.scene.yaml".into()),
            10 => changed.epoch += 1,
            _ => {
                state.sync(None);
                assert!(!state.show_route);
                assert!(state.observation.is_none());
                continue;
            }
        }
        state.sync(Some(&changed));
        assert!(!state.show_route);
        assert!(state.observation.is_none());
        state.sync(Some(&context));
        assert!(!state.show_route);
        assert!(state.observation.is_none());
    }
}
#[test]
fn failed_endpoint_and_malformed_input_routes_never_enable_overlay() {
    for bad in ["250", "not-a-number"] {
        let (graph, mut context) = fixture();
        context.fields[3] = bad.into();
        let mut h = Harness::builder()
            .with_size([1200.0, 1000.0])
            .build_ui_state(
                move |ui, state: &mut State| state.show(ui, Some(&graph), Ok(context.clone())),
                State::default(),
            );
        h.run_steps(2);
        h.get_by_label("Inspect draft route").click_accesskit();
        h.run_steps(2);
        assert!(h
            .get_by_label("Show inspected draft route")
            .accesskit_node()
            .is_disabled());
        assert!(!h.state().show_route);
    }
}
#[cfg(target_os = "linux")]
#[test]
fn closed_panel_getter_checks_live_editor_and_draft_context_without_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("route.scene.yaml");
    let bytes = b"schema: orr.scene/1\nsingletons: {}\nentities: {}\n";
    std::fs::write(&scene, bytes).unwrap();
    let mut editor = Editor::open_game(&scene, crate::game::EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.create_ramp_and_bridge().unwrap();
    panel.place_agent(&mut editor).unwrap();
    panel.sync(&editor);
    let context = panel.route_context(&editor).unwrap();
    let report = inspect_fields(panel.graph.as_ref().unwrap(), &context).unwrap();
    panel.route_diagnostics = State {
        observation: Some((context, Ok(report))),
        show_route: true,
        ..Default::default()
    };
    let history = editor.history().clone();
    let checksum = editor.checksum();
    let selection = editor.selection().cloned();
    let graph = panel.graph.as_ref().unwrap().to_bytes();
    // No diagnostics show() call occurs: renderer fetches while authoring panel is closed.
    assert!(panel.inspected_route_points(&editor).is_some());
    panel.start[0].push('0');
    assert!(panel.inspected_route_points(&editor).is_none());
    panel.start[0].pop();
    assert!(panel.inspected_route_points(&editor).is_none());
    panel.asset_path.push_str("different");
    assert!(panel.inspected_route_points(&editor).is_none());
    panel.asset_path = panel.loaded_asset_source.clone().unwrap_or_default();
    panel.geometry_staged = true;
    assert!(panel.inspected_route_points(&editor).is_none());
    panel.geometry_staged = false;
    panel.max_slope = "3".into();
    assert!(panel.inspected_route_points(&editor).is_none());
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.selection(), selection.as_ref());
    assert_eq!(panel.graph.as_ref().unwrap().to_bytes(), graph);
    assert_eq!(std::fs::read(scene).unwrap(), bytes);
    panel.max_slope = "4".into();
    editor.play();
    editor.sync();
    assert!(panel.inspected_route_points(&editor).is_none());
}
