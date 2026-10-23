use super::*;
use crate::mesh_navigation::geometry_grid::{self, Grid};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::{mission::MissionPoint, AgentSpec, MeshScenePin};

fn editor_fixture() -> (tempfile::TempDir, crate::editor::Editor) {
    let directory = tempfile::tempdir().unwrap();
    let scene_path = directory.path().join("mission.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(
        &scene_path,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    (directory, editor)
}

fn mission_fixture() -> (tempfile::TempDir, MeshView, MeshMission) {
    let (directory, editor) = editor_fixture();
    let frame = editor.snapshot().unwrap().predicted();
    let view = mesh_from_view(frame).unwrap();
    let config = mission::from_view(frame).unwrap().unwrap().config;
    (directory, view, config)
}

fn multi_portal_view(mut view: MeshView) -> (MeshView, MeshMission) {
    let graph = geometry_grid::create(
        Grid {
            origin: [FP::ZERO; 3],
            width: fp!(8),
            depth: fp!(6),
            columns: 6,
            rows: 4,
            excluded_cells: 0,
        },
        AgentProfile::default(),
    )
    .unwrap();
    let spec = AgentSpec {
        start: [FP::ONE, FP::ZERO, fp!(3)],
        goal: [fp!(7), FP::ZERO, fp!(3)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.25),
    };
    view.pin = MeshScenePin::new(&graph, spec).unwrap();
    view.graph = graph;
    let mut config = MeshMission {
        enabled: 1,
        count: 3,
        graph_revision: view.graph.revision(),
        ..Default::default()
    };
    for (index, point_x) in [fp!(2), fp!(4), fp!(6)].into_iter().enumerate() {
        config.points[index] = MissionPoint {
            position: [point_x, FP::ZERO, fp!(3)],
            radius: FP::from_raw(FP::SCALE / 2),
        };
    }
    config.exit = MissionPoint {
        position: spec.goal,
        radius: fp!(0.75),
    };
    (view, config)
}

#[test]
fn checkpoints_are_distinct_deterministic_and_preserve_authored_radii() {
    let (_directory, source_view, _source_config) = mission_fixture();
    let (view, config) = multi_portal_view(source_view);
    let candidate_config = candidate(&view, &config).unwrap();
    let repeated = candidate(&view, &config).unwrap();

    assert_eq!(candidate_config, repeated);
    assert_eq!(candidate_config.enabled, config.enabled);
    assert_eq!(candidate_config.count, config.count);
    assert_eq!(candidate_config.exit.radius, config.exit.radius);
    assert_eq!(candidate_config.exit.position, view.pin.spec().goal);
    assert_eq!(candidate_config.graph_revision, view.graph.revision());
    for index in 0..config.count as usize {
        assert_eq!(
            candidate_config.points[index].radius,
            config.points[index].radius
        );
        view.graph
            .project(
                candidate_config.points[index].position,
                view.pin.max_vertical,
            )
            .unwrap();
    }
    assert_ne!(
        candidate_config.points[0].position,
        candidate_config.points[1].position
    );
    assert_ne!(
        candidate_config.points[1].position,
        candidate_config.points[2].position
    );
    assert_ne!(candidate_config, config);
    mission::validate_config(&candidate_config, &view.graph, view.pin.max_vertical).unwrap();
}

#[test]
fn route_with_too_few_interior_waypoints_is_rejected_without_changing_config() {
    let (_directory, source_view, _source_config) = mission_fixture();
    let graph = StaticMeshGraph::bake(
        &[
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [fp!(4), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(4)],
            [fp!(4), FP::ZERO, fp!(4)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let spec = AgentSpec {
        start: [fp!(0.5), FP::ZERO, fp!(0.5)],
        goal: [fp!(3.5), FP::ZERO, fp!(3.5)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.25),
    };
    let mut view = source_view;
    view.pin = MeshScenePin::new(&graph, spec).unwrap();
    view.graph = graph;
    let config = MeshMission {
        enabled: 1,
        count: 2,
        points: [
            MissionPoint {
                position: [fp!(1), FP::ZERO, fp!(1)],
                radius: fp!(0.5),
            },
            MissionPoint {
                position: [fp!(2), FP::ZERO, fp!(2)],
                radius: fp!(0.5),
            },
            MissionPoint::default(),
            MissionPoint::default(),
            MissionPoint::default(),
            MissionPoint::default(),
            MissionPoint::default(),
            MissionPoint::default(),
        ],
        exit: MissionPoint {
            position: [fp!(3), FP::ZERO, fp!(3)],
            radius: fp!(0.5),
        },
        graph_revision: view.graph.revision(),
    };
    let original = config;

    let error = candidate(&view, &config).unwrap_err();

    assert!(error.contains("distinct interior checkpoints"));
    assert_eq!(config, original);
}

#[test]
fn disabled_or_stale_missions_are_rejected() {
    let (_directory, source_view, _source_config) = mission_fixture();
    let (view, config) = multi_portal_view(source_view);
    let mut disabled = config;
    disabled.enabled = 0;
    assert!(candidate(&view, &disabled).is_err());

    let mut stale = config;
    stale.graph_revision[0] ^= 1;
    assert!(candidate(&view, &stale).is_err());
}

#[test]
fn staging_refuses_dirty_stale_and_play_contexts_without_host_or_history_changes() {
    let (_directory, mut editor) = editor_fixture();
    let mut panel = Panel::default();
    panel.sync(&editor);
    panel.count = "7".into();
    panel.form_dirty = true;
    let fields_before = (
        panel.count.clone(),
        panel.points.clone(),
        panel.exit.clone(),
    );
    let checksum_before = editor.checksum();
    let history_before = editor.history().entries.len();

    panel.stage_route_checkpoints(&editor);

    assert_eq!(
        (
            panel.count.clone(),
            panel.points.clone(),
            panel.exit.clone()
        ),
        fields_before
    );
    assert_eq!(editor.checksum(), checksum_before);
    assert_eq!(editor.history().entries.len(), history_before);
    assert!(panel.error.as_deref().unwrap().contains("clean matching"));

    panel.form_dirty = false;
    panel.context_stale = true;
    panel.error = None;
    panel.stage_route_checkpoints(&editor);
    assert_eq!(editor.checksum(), checksum_before);
    assert_eq!(editor.history().entries.len(), history_before);
    assert_eq!(panel.count, fields_before.0);
    assert!(panel.error.as_deref().unwrap().contains("clean matching"));

    panel.context_stale = false;
    panel.error = None;
    assert!(editor.start_play());
    assert_eq!(editor.mode(), Mode::Play);
    let play_checksum = editor.checksum();
    let play_history = editor.history().entries.len();
    panel.stage_route_checkpoints(&editor);
    assert_eq!(editor.checksum(), play_checksum);
    assert_eq!(editor.history().entries.len(), play_history);
    assert_eq!(panel.count, fields_before.0);
    assert!(panel.error.is_some());
}

#[test]
fn mission_removal_controls_keep_the_fixed_inspector_inside_the_screen() {
    use egui_kittest::{
        kittest::{NodeT, Queryable},
        Harness,
    };

    struct LayoutState {
        editor: crate::editor::Editor,
        panel: Panel,
        checksum: u64,
        history_len: usize,
    }

    for size in [[700.0, 900.0], [1000.0, 900.0]] {
        let (_directory, editor) = editor_fixture();
        let checksum = editor.checksum();
        let history_len = editor.history().entries.len();
        let mut harness = Harness::builder().with_size(size).build_ui_state(
            |ui, state: &mut LayoutState| {
                egui::Panel::right("inspector")
                    .resizable(false)
                    .exact_size(340.0)
                    .show(ui, |panel_ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("inspector_scroll")
                            .auto_shrink([false, false])
                            .show(panel_ui, |scroll_ui| {
                                scroll_ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                state.panel.show(scroll_ui, &mut state.editor);
                            });
                    });
            },
            LayoutState {
                editor,
                panel: Panel::default(),
                checksum,
                history_len,
            },
        );
        harness.run_steps(5);

        if harness.query_by_label("Remove mesh mission").is_some() {
            harness.get_by_label("Mesh mission").click();
            harness.run_steps(3);
        }
        assert!(harness.query_by_label("Remove mesh mission").is_none());

        let collapsed_state =
            egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("inspector"))
                .expect("collapsed mission inspector panel state");
        let screen_rect = harness.ctx.input(|input| input.viewport_rect());
        assert!(
            collapsed_state.outer_rect.max.x <= screen_rect.max.x + 0.5,
            "collapsed inspector {:?} exceeds screen {:?} at size {size:?}",
            collapsed_state.outer_rect,
            screen_rect
        );
        assert!((collapsed_state.outer_rect.width() - 340.0).abs() < 0.5);

        harness.get_by_label("Mesh mission").click();
        harness.run_steps(4);
        assert!(harness
            .query_by_label("I confirm removing the mesh mission")
            .is_some());
        harness.get_by_label("Remove mesh mission").scroll_to_me();
        harness.run_steps(2);
        assert!(harness
            .get_by_label("Remove mesh mission")
            .accesskit_node()
            .is_disabled());
        harness
            .get_by_label("I confirm removing the mesh mission")
            .scroll_to_me();
        harness.run_steps(3);
        harness
            .get_by_label("I confirm removing the mesh mission")
            .click_accesskit();
        harness.run_steps(3);
        assert!(!harness
            .get_by_label("Remove mesh mission")
            .accesskit_node()
            .is_disabled());

        let expanded_state =
            egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("inspector"))
                .expect("expanded mission inspector panel state");
        let screen_rect = harness.ctx.input(|input| input.viewport_rect());
        assert!(
            expanded_state.outer_rect.max.x <= screen_rect.max.x + 0.5,
            "expanded inspector {:?} exceeds screen {:?} at size {size:?}",
            expanded_state.outer_rect,
            screen_rect
        );
        assert!((expanded_state.outer_rect.width() - 340.0).abs() < 0.5);
        assert_eq!(harness.state().editor.checksum(), harness.state().checksum);
        assert_eq!(
            harness.state().editor.history().entries.len(),
            harness.state().history_len
        );
    }
}
