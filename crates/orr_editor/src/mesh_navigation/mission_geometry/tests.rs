use super::super::geometry_panel;
use super::*;
use orr_fp::{fp, FP};
use orr_navigation::AgentProfile;
use orr_remote::mesh_navigation::{AgentSpec, MissionPoint};
use serde_json::json;

fn flat_graph(height: FP) -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            [FP::ZERO, height, FP::ZERO],
            [fp!(4), height, FP::ZERO],
            [FP::ZERO, height, fp!(4)],
            [fp!(4), height, fp!(4)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap()
}

fn disconnected_graph(height: FP) -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            [FP::ZERO, height, FP::ZERO],
            [fp!(4), height, FP::ZERO],
            [FP::ZERO, height, fp!(4)],
            [fp!(4), height, fp!(4)],
            [fp!(10), height, FP::ZERO],
            [fp!(14), height, FP::ZERO],
            [fp!(10), height, fp!(4)],
            [fp!(14), height, fp!(4)],
        ],
        &[[0, 2, 3], [0, 3, 1], [4, 6, 7], [4, 7, 5]],
        AgentProfile::default(),
    )
    .unwrap()
}

fn active_mission(graph: &StaticMeshGraph) -> MeshMission {
    let mut config = MeshMission::zeroed();
    config.enabled = 1;
    config.count = 2;
    config.points[0] = MissionPoint {
        position: [fp!(1), FP::ZERO, fp!(1)],
        radius: fp!(0.25),
    };
    config.points[1] = MissionPoint {
        position: [fp!(2), FP::ZERO, fp!(2)],
        radius: fp!(0.75),
    };
    config.exit = MissionPoint {
        position: [fp!(3), FP::ZERO, fp!(3)],
        radius: fp!(1),
    };
    config.graph_revision = graph.revision();
    config
}

#[test]
fn preview_reprojects_agent_and_active_mission_preserving_route_settings_and_radii() {
    let old_graph = flat_graph(FP::ZERO);
    let candidate_graph = flat_graph(fp!(0.25));
    let source_mission = active_mission(&old_graph);
    let old_spec = AgentSpec {
        start: [fp!(0.5), FP::ZERO, fp!(0.5)],
        goal: [fp!(3.5), FP::ZERO, fp!(3.5)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.125),
    };
    let context = Context {
        identity: Identity {
            scene: PathBuf::from("scene.scene.yaml"),
            epoch: 4,
            tick: 0,
            checksum: 0x1234,
            old_revision: old_graph.revision(),
            candidate_revision: candidate_graph.revision(),
            projection_max_vertical: fp!(0.5),
        },
        source_pin: MeshScenePin::new(&old_graph, old_spec).unwrap(),
        source_mission,
        source_run: MeshMissionRun::zeroed(),
        candidate_graph: candidate_graph.clone(),
    };
    let preview = build_preview(&context).unwrap();
    assert_eq!(preview.pin.graph_revision, candidate_graph.revision());
    assert_eq!(
        preview.pin.spec().distance_per_tick,
        old_spec.distance_per_tick
    );
    assert_eq!(preview.pin.spec().max_vertical, fp!(0.5));
    assert_eq!(preview.new_start[1], fp!(0.25));
    assert_eq!(preview.new_goal[1], fp!(0.25));
    assert_eq!(preview.mission.count, source_mission.count);
    assert_eq!(
        preview.mission.points[0].radius,
        source_mission.points[0].radius
    );
    assert_eq!(
        preview.mission.points[1].radius,
        source_mission.points[1].radius
    );
    assert_eq!(preview.mission.exit.radius, source_mission.exit.radius);
    assert_eq!(preview.mission.points[0].position[1], fp!(0.25));
    assert_eq!(preview.mission.points[1].position[1], fp!(0.25));
    assert_eq!(preview.mission.exit.position[1], fp!(0.25));
    assert_eq!(preview.mission.points[2], MissionPoint::zeroed());
    assert_eq!(preview.mission.graph_revision, candidate_graph.revision());
    mission::validate_config(
        &preview.mission,
        &candidate_graph,
        preview.pin.spec().max_vertical,
    )
    .unwrap();
}

#[test]
fn preview_rejects_existing_progress_instead_of_rebinding_the_run() {
    let old_graph = flat_graph(FP::ZERO);
    let candidate_graph = flat_graph(fp!(0.25));
    let source_mission = active_mission(&old_graph);
    let old_spec = AgentSpec {
        start: [fp!(0.5), FP::ZERO, fp!(0.5)],
        goal: [fp!(3.5), FP::ZERO, fp!(3.5)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.125),
    };
    let mut context = Context {
        identity: Identity {
            scene: PathBuf::from("scene.scene.yaml"),
            epoch: 4,
            tick: 0,
            checksum: 0x1234,
            old_revision: old_graph.revision(),
            candidate_revision: candidate_graph.revision(),
            projection_max_vertical: fp!(0.5),
        },
        source_pin: MeshScenePin::new(&old_graph, old_spec).unwrap(),
        source_mission,
        source_run: MeshMissionRun::zeroed(),
        candidate_graph,
    };
    context.source_run.collected = 1;
    assert!(build_preview(&context)
        .unwrap_err()
        .contains("Reset mission progress"));
}

#[test]
fn pending_route_edits_are_rejected_but_vertical_tolerance_may_change() {
    let source = AgentSpec {
        start: [fp!(0.5), FP::ZERO, fp!(0.5)],
        goal: [fp!(3.5), FP::ZERO, fp!(3.5)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.125),
    };
    let mut authored = source;
    authored.max_vertical = fp!(0.5);
    assert!(validate_pending_route(&authored, &source).is_ok());

    authored.start[0] = fp!(0.75);
    assert!(validate_pending_route(&authored, &source)
        .unwrap_err()
        .contains("pending agent start"));
    authored = source;
    authored.goal[2] = fp!(3.25);
    assert!(validate_pending_route(&authored, &source)
        .unwrap_err()
        .contains("pending agent start"));
    authored = source;
    authored.distance_per_tick = fp!(0.25);
    assert!(validate_pending_route(&authored, &source)
        .unwrap_err()
        .contains("pending agent start"));
}

#[test]
fn preview_refuses_a_point_that_cannot_be_projected_and_keeps_the_source_values() {
    let old_graph = flat_graph(FP::ZERO);
    let candidate_graph = StaticMeshGraph::bake(
        &[
            [fp!(10), FP::ZERO, fp!(10)],
            [fp!(14), FP::ZERO, fp!(10)],
            [fp!(10), FP::ZERO, fp!(14)],
            [fp!(14), FP::ZERO, fp!(14)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let old_spec = AgentSpec {
        start: [fp!(0.5), FP::ZERO, fp!(0.5)],
        goal: [fp!(3.5), FP::ZERO, fp!(3.5)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.125),
    };
    let source_mission = active_mission(&old_graph);
    let context = Context {
        identity: Identity {
            scene: PathBuf::from("scene.scene.yaml"),
            epoch: 4,
            tick: 0,
            checksum: 0x1234,
            old_revision: old_graph.revision(),
            candidate_revision: candidate_graph.revision(),
            projection_max_vertical: FP::ZERO,
        },
        source_pin: MeshScenePin::new(&old_graph, old_spec).unwrap(),
        source_mission,
        source_run: MeshMissionRun::zeroed(),
        candidate_graph,
    };
    assert!(build_preview(&context).is_err());
    assert_eq!(context.source_mission, source_mission);
    assert_eq!(context.source_pin.spec(), old_spec);
}

#[test]
fn preview_rejects_disconnected_collectible_or_exit_even_when_both_project() {
    let old_graph = disconnected_graph(FP::ZERO);
    let candidate_graph = disconnected_graph(fp!(0.25));
    let old_spec = AgentSpec {
        start: [fp!(0.5), FP::ZERO, fp!(0.5)],
        goal: [fp!(3.5), FP::ZERO, fp!(3.5)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.125),
    };

    let mut source_mission = active_mission(&old_graph);
    source_mission.points[1].position = [fp!(11), FP::ZERO, fp!(1)];
    let context = Context {
        identity: Identity {
            scene: PathBuf::from("scene.scene.yaml"),
            epoch: 4,
            tick: 0,
            checksum: 0x1234,
            old_revision: old_graph.revision(),
            candidate_revision: candidate_graph.revision(),
            projection_max_vertical: fp!(0.5),
        },
        source_pin: MeshScenePin::new(&old_graph, old_spec).unwrap(),
        source_mission,
        source_run: MeshMissionRun::zeroed(),
        candidate_graph: candidate_graph.clone(),
    };
    let error = build_preview(&context).unwrap_err();
    assert!(error.contains("collectible 2"));
    assert_eq!(
        context.source_mission.points[1].position,
        [fp!(11), FP::ZERO, fp!(1)]
    );

    let mut source_mission = active_mission(&old_graph);
    source_mission.exit.position = [fp!(11), FP::ZERO, fp!(1)];
    let context = Context {
        source_mission,
        ..context
    };
    let error = build_preview(&context).unwrap_err();
    assert!(error.contains("mission exit"));
    assert_eq!(
        context.source_mission.exit.position,
        [fp!(11), FP::ZERO, fp!(1)]
    );
}

#[test]
fn mission_opt_in_does_not_allow_geometry_to_orphan_any_presentation_sidecar() {
    let directory = tempfile::tempdir().unwrap();
    let scene_path = directory.path().join("mission-geometry-sidecar.scene.yaml");
    let mut editor = crate::mesh_workspace::mission::prepare(
        &scene_path,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    editor.sync();
    let mut navigation = NavigationPanel::default();
    navigation.sync(&editor);
    assert!(navigation.mission_bound);
    navigation.mission_geometry.opted_in = true;

    let original_checksum = editor.checksum();
    let original_history_len = editor.history().entries.len();
    let original_pin = *editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<MeshScenePin>();
    let original_graph = navigation.graph.as_ref().unwrap().to_bytes();
    for suffix in [
        ".mesh-art.json",
        ".mesh-props.json",
        ".mesh-camera.json",
        ".mesh-animation.json",
        ".mesh-lighting.json",
        ".mesh-hud.json",
    ] {
        let sidecar_path = PathBuf::from(format!("{}{}", scene_path.display(), suffix));
        let sidecar_contents = format!("opaque sidecar {suffix}").into_bytes();
        std::fs::write(&sidecar_path, &sidecar_contents).unwrap();

        let error = navigation
            .stage_geometry(
                geometry_panel::Action::MoveVertex {
                    index: 3,
                    position: [FP::from_int(8), fp!(0.5), FP::from_int(6)],
                },
                &editor,
            )
            .unwrap_err();
        assert!(
            error.contains("Remove and save existing scene-bound"),
            "{suffix}: {error}"
        );
        assert_eq!(std::fs::read(&sidecar_path).unwrap(), sidecar_contents);
        assert_eq!(editor.checksum(), original_checksum);
        assert_eq!(editor.history().entries.len(), original_history_len);
        assert_eq!(
            *editor
                .snapshot()
                .unwrap()
                .predicted()
                .singleton::<MeshScenePin>(),
            original_pin
        );
        assert_eq!(
            navigation.graph.as_ref().unwrap().to_bytes(),
            original_graph
        );
        assert!(!navigation.geometry_staged);
        std::fs::remove_file(sidecar_path).unwrap();
    }
}

#[test]
fn request_uses_canonical_expected_identity_and_the_full_reflected_pin_object() {
    let graph = flat_graph(FP::ZERO);
    let spec = AgentSpec {
        start: [fp!(0.5), FP::ZERO, fp!(0.5)],
        goal: [fp!(3.5), FP::ZERO, fp!(3.5)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.125),
    };
    let pin = MeshScenePin::new(&graph, spec).unwrap();
    let mission_config = active_mission(&graph);
    let context = Context {
        identity: Identity {
            scene: PathBuf::from("scene.scene.yaml"),
            epoch: 9,
            tick: 0,
            checksum: 0xfeed,
            old_revision: graph.revision(),
            candidate_revision: graph.revision(),
            projection_max_vertical: FP::ZERO,
        },
        source_pin: pin,
        source_mission: mission_config,
        source_run: MeshMissionRun::zeroed(),
        candidate_graph: graph,
    };
    let mut preview = build_preview(&context).unwrap();
    preview.identity.candidate_revision[0] ^= 1;
    preview.pin.graph_revision = preview.identity.candidate_revision;
    preview.mission.graph_revision = preview.identity.candidate_revision;
    let request = reproject_request(&context, &preview).unwrap_err();
    assert!(request.contains("does not match"));

    let mut valid_preview = build_preview(&context).unwrap();
    valid_preview.identity.candidate_revision = context.identity.candidate_revision;
    let request = reproject_request(&context, &valid_preview).unwrap();
    assert_eq!(request["expected_checksum"], json!("0x000000000000feed"));
    assert_eq!(
        request["expected_graph_revision"],
        json!(hex_revision(context.identity.old_revision))
    );
    assert_eq!(
        request["pin"]["graph_len"],
        json!(valid_preview.pin.graph_len)
    );
    assert_eq!(
        request["pin"]["graph_revision"],
        json!(valid_preview.pin.graph_revision)
    );
    assert_eq!(
        request["pin"]["graph_bytes"].as_array().unwrap().len(),
        8192
    );
    assert!(request.get("name").is_none());
    assert!(request.get("value").is_none());
}

#[test]
fn response_checksum_requires_exact_lowercase_sixteen_digit_hex() {
    for value in [
        json!(0),
        json!("feed"),
        json!("0xfeed"),
        json!("0x000000000000FEED"),
        json!("0x000000000000feed "),
    ] {
        assert!(parse_checksum(&value).is_err());
    }
    assert_eq!(
        parse_checksum(&json!("0x000000000000feed")).unwrap(),
        0xfeed
    );
}

#[test]
fn response_requires_the_reviewed_graph_and_full_projected_mission() {
    let source_graph = flat_graph(FP::ZERO);
    let candidate_graph = flat_graph(fp!(0.25));
    let source_mission = active_mission(&source_graph);
    let source_pin = MeshScenePin::new(
        &source_graph,
        AgentSpec {
            start: [fp!(0.5), FP::ZERO, fp!(0.5)],
            goal: [fp!(3.5), FP::ZERO, fp!(3.5)],
            max_vertical: fp!(0.5),
            distance_per_tick: fp!(0.125),
        },
    )
    .unwrap();
    let context = Context {
        identity: Identity {
            scene: PathBuf::from("mission.scene.yaml"),
            epoch: 2,
            tick: 0,
            checksum: 0x44,
            old_revision: source_graph.revision(),
            candidate_revision: candidate_graph.revision(),
            projection_max_vertical: fp!(0.5),
        },
        source_pin,
        source_mission,
        source_run: MeshMissionRun::zeroed(),
        candidate_graph,
    };
    let preview = build_preview(&context).unwrap();
    let response = json!({
        "changed": true,
        "checksum": "0x0000000000000045",
        "graph_revision": hex_revision(preview.identity.candidate_revision),
        "mission": reflected_mission(&preview.mission).unwrap(),
    });
    verify_response(&response, &preview).unwrap();

    let mut wrong_revision = response.clone();
    wrong_revision["graph_revision"] = json!(hex_revision(context.identity.old_revision));
    assert!(verify_response(&wrong_revision, &preview).is_err());

    let mut changed_mission = response;
    changed_mission["mission"]["points"][0]["radius"] = json!(0.5);
    assert!(verify_response(&changed_mission, &preview).is_err());
}
