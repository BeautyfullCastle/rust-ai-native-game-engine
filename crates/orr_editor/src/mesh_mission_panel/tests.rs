use super::*;

#[test]
fn default_mission_points_are_fixed_and_route_reachable_candidates() {
    let points = default_points();
    assert_eq!(points.len(), MAX_MISSION_POINTS);
    assert_eq!(points[0].join(","), "1.5,0,1,0.4");
    assert_eq!(points[1].join(","), "2,0,1,0.4");
    assert_eq!(points[7].join(","), "2,0,1,0.4");
    assert_eq!(
        parse_point(&points[0], "collectible").unwrap().position[0].raw(),
        98_304
    );
}

#[test]
fn point_parser_preserves_exact_fixed_point_and_limits() {
    let parsed = parse_point(
        &["-0.1".into(), "0.25".into(), "1.5".into(), "0.4".into()],
        "collectible",
    )
    .unwrap();
    assert_eq!(parsed.position[0], parse_fixed("-0.1", "x").unwrap());
    assert_eq!(parsed.position[1], parse_fixed("0.25", "y").unwrap());
    assert_eq!(
        format_point(parsed),
        ["-0.1", "0.25", "1.5", "0.4"].map(str::to_owned)
    );
    assert_eq!(
        parse_point(&format_point(parsed), "collectible").unwrap(),
        parsed
    );

    assert!(parse_point(
        &["256.0000153".into(), "0".into(), "0".into(), "0.5".into()],
        "collectible",
    )
    .is_err());
    assert!(parse_point(
        &["0".into(), "0".into(), "0".into(), "0".into()],
        "collectible",
    )
    .is_err());
    assert!(parse_point(
        &["0".into(), "0".into(), "0".into(), "4.0000153".into()],
        "collectible",
    )
    .is_err());
    assert!(parse_point(
        &["0".into(), "0".into(), "0".into(), "not-a-number".into()],
        "collectible",
    )
    .is_err());
}

#[test]
fn disabled_mission_is_canonical_zero_and_count_is_bounded() {
    let mission = disabled_mission();
    assert_eq!(mission.enabled, 0);
    assert_eq!(mission.count, 0);
    assert!(mission
        .points
        .iter()
        .all(|point| *point == MissionPoint::default()));
    assert_eq!(mission.exit, MissionPoint::default());
    assert_eq!(mission.graph_revision, [0; 32]);

    assert_eq!(parse_count("1").unwrap(), 1);
    assert_eq!(parse_count("8").unwrap(), 8);
    for count in ["", "0", "9", "2.0", "-1", "123"] {
        assert!(parse_count(count).is_err());
    }
}

#[test]
fn authoritative_apply_readback_refreshes_mission_presence_for_route_staging() {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("mission.scene.yaml");
    let settings = crate::mesh_workspace::mission::Settings {
        collectible_count: 1,
        ..Default::default()
    };
    let mut editor = crate::mesh_workspace::mission::prepare(&scene, &settings).unwrap();
    let original_mission = mission::from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .config;
    let mut panel = Panel::default();
    panel.sync(&editor);
    assert!(panel.current_mission_enabled());

    panel.confirm_remove = true;
    panel.remove(&mut editor);
    assert!(panel.error.is_none());
    assert!(!panel.current_mission_enabled());
    assert!(!panel.can_stage_route_checkpoints(&editor));

    panel.enabled = true;
    panel.count = original_mission.count.to_string();
    panel.points = original_mission.points.map(format_point);
    panel.exit = format_point(original_mission.exit);
    panel.form_dirty = true;
    panel.apply(&mut editor);

    let applied = mission::from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap();
    assert_eq!(applied.config, original_mission);
    assert!(panel.error.is_none());
    assert!(panel.current_mission_enabled());
    assert!(!panel.dirty());
    assert!(panel.can_stage_route_checkpoints(&editor));

    panel.enabled = false;
    panel.form_dirty = true;
    panel.apply(&mut editor);
    assert!(panel.error.is_none());
    assert!(!panel.current_mission_enabled());
    assert!(!panel.can_stage_route_checkpoints(&editor));
    assert!(mission::from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .is_none());

    panel.enabled = true;
    panel.count = original_mission.count.to_string();
    panel.points = original_mission.points.map(format_point);
    panel.exit = format_point(original_mission.exit);
    panel.form_dirty = true;
    panel.apply(&mut editor);
    assert!(panel.error.is_none());
    assert!(panel.current_mission_enabled());
    assert!(panel.can_stage_route_checkpoints(&editor));

    let checksum_before_draft = editor.checksum();
    let history_before_draft = editor.history().entries.len();
    panel.points[0][0] = "3.25".into();
    panel.form_dirty = true;
    assert_eq!(editor.checksum(), checksum_before_draft);
    assert_eq!(editor.history().entries.len(), history_before_draft);
    assert!(panel.current_mission_enabled());
    assert!(!panel.can_stage_route_checkpoints(&editor));
}

#[test]
fn route_checkpoint_widget_is_enabled_after_reenable_apply() {
    use egui_kittest::{
        kittest::{NodeT, Queryable},
        Harness,
    };

    struct UiState {
        _directory: tempfile::TempDir,
        editor: crate::editor::Editor,
        panel: Panel,
        checksum: u64,
        history_len: usize,
        mission: MeshMission,
    }

    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("mission.scene.yaml");
    let settings = crate::mesh_workspace::mission::Settings {
        collectible_count: 1,
        ..Default::default()
    };
    let mut editor = crate::mesh_workspace::mission::prepare(&scene, &settings).unwrap();
    let mut original_mission = mission::from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .config;
    let admitted_view = mesh_from_view(editor.snapshot().unwrap().predicted()).unwrap();
    original_mission.points[0].position = admitted_view.pin.spec().start;
    let mut panel = Panel::default();
    panel.sync(&editor);
    panel.confirm_remove = true;
    panel.remove(&mut editor);
    panel.enabled = true;
    panel.count = original_mission.count.to_string();
    panel.points = original_mission.points.map(format_point);
    panel.exit = format_point(original_mission.exit);
    panel.form_dirty = true;
    panel.apply(&mut editor);
    assert!(panel.error.is_none());
    assert!(panel.current_mission_enabled());

    let checksum = editor.checksum();
    let history_len = editor.history().entries.len();
    let mut harness = Harness::builder()
        .with_size([1100.0, 1200.0])
        .build_ui_state(
            |ui, state: &mut UiState| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |scroll_ui| {
                        state.panel.show(scroll_ui, &mut state.editor)
                    });
            },
            UiState {
                _directory: directory,
                editor,
                panel,
                checksum,
                history_len,
                mission: original_mission,
            },
        );
    harness.run_steps(3);
    harness.get_by_label("Mesh mission").click();
    harness.run_steps(3);

    assert!(!harness
        .get_by_label("Stage mission route checkpoints")
        .accesskit_node()
        .is_disabled());
    harness
        .get_by_label("Stage mission route checkpoints")
        .click_accesskit();
    harness.run_steps(3);

    let state = harness.state();
    assert!(state.panel.error.is_none());
    assert!(state.panel.dirty());
    assert_eq!(state.editor.checksum(), state.checksum);
    assert_eq!(state.editor.history().entries.len(), state.history_len);
    let authoritative = mission::from_view(state.editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap();
    assert_eq!(authoritative.config, state.mission);
}

#[cfg(feature = "mesh-hazards")]
#[path = "hazards_tests.rs"]
mod hazards;
