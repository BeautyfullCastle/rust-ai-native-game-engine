use super::*;
use crate::caps::{Cap, Caps};
use crate::mesh_navigation::{authoring::create_mesh_scene, AgentSpec, MESH_NAVIGATION_BUILD_ID};
use orr_bridge::FrameView;
use orr_edit::{EditorDoc, Op, Origin, PlayController};
use orr_fp::{fp, FP};
use orr_games::mesh_navigation_game::mission::MissionPoint;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile, SearchBudget};
use serde_json::{json, Map};

fn graph(diagonal: bool) -> StaticMeshGraph {
    let vertices = [
        [FP::ZERO, FP::ZERO, FP::ZERO],
        [fp!(4), FP::ZERO, FP::ZERO],
        [FP::ZERO, FP::ZERO, fp!(4)],
        [fp!(4), FP::ZERO, fp!(4)],
    ];
    let faces = if diagonal {
        [[0, 2, 1], [2, 3, 1]]
    } else {
        [[0, 2, 3], [0, 3, 1]]
    };
    StaticMeshGraph::bake(&vertices, &faces, AgentProfile::default()).unwrap()
}

fn spec() -> AgentSpec {
    AgentSpec {
        start: [fp!(0.5), FP::ZERO, fp!(0.5)],
        goal: [fp!(3.5), FP::ZERO, fp!(3.5)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.25),
    }
}

fn mission_for(graph: &StaticMeshGraph) -> MeshMission {
    let mut mission = MeshMission::zeroed();
    mission.enabled = 1;
    mission.count = 2;
    mission.points[0] = MissionPoint {
        position: [fp!(1), FP::ZERO, fp!(1)],
        radius: fp!(0.5),
    };
    mission.points[1] = MissionPoint {
        position: [fp!(2), FP::ZERO, fp!(2)],
        radius: fp!(0.5),
    };
    mission.exit = MissionPoint {
        position: [fp!(3), FP::ZERO, fp!(3)],
        radius: fp!(0.5),
    };
    mission.graph_revision = graph.revision();
    mission
}

fn document() -> EditorDoc {
    let old_graph = graph(false);
    let mut document = create_mesh_scene(&old_graph, spec()).unwrap();
    let mission = mission_for(&old_graph);
    let descriptor = document.types().get(MISSION_NAME).unwrap();
    document
        .apply(
            Op::SetSingletonField {
                singleton: MISSION_NAME.into(),
                path: String::new(),
                value: descriptor.read(bytemuck::bytes_of(&mission)),
            },
            Origin::User,
        )
        .unwrap();
    document
}

fn pin_json(document: &EditorDoc, pin: &MeshScenePin) -> J {
    let descriptor = document.types().get(PIN_NAME).unwrap();
    value_to_json(&descriptor.read(bytemuck::bytes_of(pin)))
}

fn parameters(document: &EditorDoc, pin: MeshScenePin) -> Map<String, J> {
    let mut parameters = Map::new();
    parameters.insert(
        "expected_checksum".into(),
        json!(checksum_text(document.checksum())),
    );
    parameters.insert(
        "expected_graph_revision".into(),
        json!(revision_text(&graph(false).revision())),
    );
    parameters.insert("pin".into(), pin_json(document, &pin));
    parameters
}

fn limits() -> HostLimits {
    HostLimits {
        build_id: MESH_NAVIGATION_BUILD_ID,
        game: crate::GameHooks::new("MeshNavigation"),
        ..HostLimits::default()
    }
}

fn invoke(document: &mut EditorDoc, parameters: &Map<String, J>) -> Result<J, RpcError> {
    invoke_with(document, parameters, Caps::ALL, None)
}

fn invoke_with(
    document: &mut EditorDoc,
    parameters: &Map<String, J>,
    caps: Caps,
    mut play: Option<PlayController<MeshNavigation>>,
) -> Result<J, RpcError> {
    let mut target = ErpTarget::<MeshNavigation> {
        doc: document,
        play: &mut play,
    };
    let context = CallCtx {
        client: "geometry-test",
        caps,
        last_play: None,
        tx_check: None,
    };
    super::call(
        &mut target,
        &limits(),
        &context,
        &P(parameters),
        Origin::Agent("geometry-test".into()),
    )
}

fn invoke_with_limits(
    document: &mut EditorDoc,
    parameters: &Map<String, J>,
    caps: Caps,
    host_limits: &HostLimits,
) -> Result<J, RpcError> {
    let mut play = None;
    let mut target = ErpTarget::<MeshNavigation> {
        doc: document,
        play: &mut play,
    };
    let context = CallCtx {
        client: "geometry-test",
        caps,
        last_play: None,
        tx_check: None,
    };
    super::call(
        &mut target,
        host_limits,
        &context,
        &P(parameters),
        Origin::Agent("geometry-test".into()),
    )
}

fn invoke_local(
    document: &mut EditorDoc,
    parameters: &Map<String, J>,
    caps: Caps,
) -> Result<J, RpcError> {
    let mut play = None;
    let mut target = ErpTarget::<MeshNavigation> {
        doc: document,
        play: &mut play,
    };
    crate::call_local(
        &mut target,
        &limits(),
        "geometry-test",
        caps,
        "mesh.geometry.reproject",
        &J::Object(parameters.clone()),
    )
}

fn proposed_pin() -> MeshScenePin {
    MeshScenePin::new(&graph(true), spec()).unwrap()
}

#[test]
fn reproject_updates_pin_and_mission_as_one_undoable_admitted_edit() {
    let mut document = document();
    let checksum_before = document.checksum();
    let history_before = document.history().len();
    let request = parameters(&document, proposed_pin());
    let result = invoke(&mut document, &request).unwrap();
    let new_graph = graph(true);
    let new_pin = *document.frame().singleton::<MeshScenePin>();
    let new_mission = *document.frame().singleton::<MeshMission>();

    assert_eq!(result["changed"], true);
    assert_eq!(result["checksum"], checksum_text(document.checksum()));
    assert_eq!(
        result["graph_revision"],
        revision_text(&new_graph.revision())
    );
    assert_eq!(
        result["mission"]["graph_revision"],
        json!(new_graph.revision())
    );
    assert_eq!(document.history().len(), history_before + 1);
    assert_ne!(document.checksum(), checksum_before);
    assert_eq!(new_pin.graph_revision, new_graph.revision());
    assert_eq!(new_pin.distance_per_tick, spec().distance_per_tick);
    assert_eq!(new_mission.graph_revision, new_graph.revision());
    assert_eq!(new_mission.count, 2);
    assert_eq!(new_mission.points[0].radius, fp!(0.5));
    assert_eq!(new_mission.exit.radius, fp!(0.5));

    let checksum_after = document.checksum();
    document.undo().unwrap();
    assert_eq!(document.checksum(), checksum_before);
    document.redo().unwrap();
    assert_eq!(document.checksum(), checksum_after);
    let saved = document.save_yaml();
    let reopened = crate::mesh_navigation::mesh_doc_from_yaml(&saved).unwrap();
    assert_eq!(reopened.checksum(), checksum_after);
    assert_eq!(
        crate::mesh_navigation::mission::from_view(FrameView::of(reopened.frame()))
            .unwrap()
            .unwrap()
            .config,
        new_mission
    );
}

#[test]
fn atomic_reprojection_rolls_back_when_final_mission_admission_fails() {
    let mut document = document();
    let checksum_before = document.checksum();
    let history_before = document.history().len();
    let old_pin = *document.frame().singleton::<MeshScenePin>();
    let old_mission = *document.frame().singleton::<MeshMission>();
    let candidate_graph = graph(true);
    let candidate_pin = MeshScenePin::new(&candidate_graph, spec()).unwrap();
    let mut invalid_mission = mission_for(&candidate_graph);
    invalid_mission.points[0].position = [fp!(20), FP::ZERO, fp!(20)];
    let types = document.types();
    let disabled_mission_value = types
        .get(MISSION_NAME)
        .unwrap()
        .read(bytemuck::bytes_of(&MeshMission::zeroed()));
    let pin_value = types
        .get(PIN_NAME)
        .unwrap()
        .read(bytemuck::bytes_of(&candidate_pin));
    let invalid_mission_value = types
        .get(MISSION_NAME)
        .unwrap()
        .read(bytemuck::bytes_of(&invalid_mission));

    assert!(document
        .apply_atomic_batch(
            "reproject mesh mission geometry",
            vec![
                Op::SetSingletonField {
                    singleton: MISSION_NAME.into(),
                    path: String::new(),
                    value: disabled_mission_value,
                },
                Op::SetSingletonField {
                    singleton: PIN_NAME.into(),
                    path: String::new(),
                    value: pin_value,
                },
                Op::SetSingletonField {
                    singleton: MISSION_NAME.into(),
                    path: String::new(),
                    value: invalid_mission_value,
                },
            ],
            Origin::Agent("geometry-test".into()),
        )
        .is_err());
    assert_eq!(document.checksum(), checksum_before);
    assert_eq!(document.history().len(), history_before);
    assert_eq!(*document.frame().singleton::<MeshScenePin>(), old_pin);
    assert_eq!(*document.frame().singleton::<MeshMission>(), old_mission);
}

#[test]
fn stale_checksum_and_revision_refuse_without_changing_frame_or_history() {
    let mut document = document();
    let proposed = proposed_pin();
    let mut request = parameters(&document, proposed);
    request.insert("expected_checksum".into(), json!("0x0000000000000000"));
    let checksum = document.checksum();
    let history = document.history().len();
    assert_eq!(
        invoke(&mut document, &request).unwrap_err().kind(),
        Some("stale_checksum")
    );
    assert_eq!(document.checksum(), checksum);
    assert_eq!(document.history().len(), history);

    let mut request = parameters(&document, proposed);
    request.insert("expected_graph_revision".into(), json!("00".repeat(32)));
    assert_eq!(
        invoke(&mut document, &request).unwrap_err().kind(),
        Some("stale_graph_revision")
    );
    assert_eq!(document.checksum(), checksum);
    assert_eq!(document.history().len(), history);
}

#[test]
fn reproject_rejects_unprojectable_mission_without_partial_pin_change() {
    let mut document = document();
    let old_checksum = document.checksum();
    let old_history = document.history().len();
    let translated_vertices = [
        [fp!(20), FP::ZERO, FP::ZERO],
        [fp!(24), FP::ZERO, FP::ZERO],
        [fp!(20), FP::ZERO, fp!(4)],
        [fp!(24), FP::ZERO, fp!(4)],
    ];
    let shifted_graph = StaticMeshGraph::bake(
        &translated_vertices,
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let shifted_spec = AgentSpec {
        start: [fp!(20.5), FP::ZERO, fp!(0.5)],
        goal: [fp!(23.5), FP::ZERO, fp!(3.5)],
        ..spec()
    };
    let pin = MeshScenePin::new(&shifted_graph, shifted_spec).unwrap();
    let request = parameters(&document, pin);

    assert_eq!(
        invoke(&mut document, &request).unwrap_err().kind(),
        Some("invalid_value")
    );
    assert_eq!(document.checksum(), old_checksum);
    assert_eq!(document.history().len(), old_history);
}

#[test]
fn reproject_rejects_a_projected_but_unreachable_objective_without_partial_edit() {
    let mut document = document();
    let checksum_before = document.checksum();
    let history_before = document.history().len();
    let vertices = [
        [FP::ZERO, FP::ONE, FP::ZERO],
        [fp!(4), FP::ONE, FP::ZERO],
        [FP::ZERO, FP::ONE, fp!(4)],
        [fp!(4), FP::ONE, fp!(4)],
        [fp!(1.9), FP::ZERO, fp!(1.9)],
        [fp!(2.1), FP::ZERO, fp!(1.9)],
        [fp!(2), FP::ZERO, fp!(2.1)],
    ];
    let candidate_graph = StaticMeshGraph::bake(
        &vertices,
        &[[0, 2, 3], [0, 3, 1], [4, 6, 5]],
        AgentProfile::default(),
    )
    .unwrap();
    let candidate_spec = AgentSpec {
        start: [fp!(0.5), FP::ONE, fp!(0.5)],
        goal: [fp!(3.5), FP::ONE, fp!(3.5)],
        max_vertical: FP::ONE,
        ..spec()
    };
    let candidate_pin = MeshScenePin::new(&candidate_graph, candidate_spec).unwrap();
    let mission_point = project_position_for_test(&candidate_graph, [fp!(2), FP::ZERO, fp!(2)]);
    assert!(candidate_graph
        .find_path(
            candidate_pin.start,
            candidate_pin.goal,
            candidate_pin.max_vertical,
            SearchBudget::default(),
        )
        .is_ok());
    assert!(candidate_graph
        .find_path(
            candidate_pin.start,
            mission_point,
            candidate_pin.max_vertical,
            SearchBudget::default(),
        )
        .is_err());
    let request = parameters(&document, candidate_pin);

    assert_eq!(
        invoke(&mut document, &request).unwrap_err().kind(),
        Some("invalid_value")
    );
    assert_eq!(document.checksum(), checksum_before);
    assert_eq!(document.history().len(), history_before);
}

fn project_position_for_test(graph: &StaticMeshGraph, position: [FP; 3]) -> [FP; 3] {
    graph.project(position, FP::ONE).unwrap().position
}

#[test]
fn oversized_and_open_transaction_requests_are_refused_without_edits() {
    let mut document = document();
    let pin = proposed_pin();
    let old_checksum = document.checksum();
    let old_history = document.history().len();
    let mut oversized = parameters(&document, pin);
    oversized.insert("pin".into(), json!("x".repeat((1 << 20) + 1)));
    assert_eq!(
        invoke(&mut document, &oversized).unwrap_err().kind(),
        Some("limit_exceeded")
    );
    assert_eq!(document.checksum(), old_checksum);
    assert_eq!(document.history().len(), old_history);

    document.begin_tx("existing", Origin::User).unwrap();
    let checksum_with_tx = document.checksum();
    let history_with_tx = document.history().len();
    let request = parameters(&document, pin);
    assert_eq!(
        invoke(&mut document, &request).unwrap_err().kind(),
        Some("tx_open")
    );
    assert_eq!(document.checksum(), checksum_with_tx);
    assert_eq!(document.history().len(), history_with_tx);
    assert!(document.in_tx());
    document.rollback_tx().unwrap();
}

#[test]
fn pin_decoder_rejects_unknown_fields_and_noncanonical_hex() {
    let mut document = document();
    let checksum = document.checksum();
    let history = document.history().len();
    let mut request = parameters(&document, proposed_pin());
    request["pin"]["unexpected"] = json!(true);
    assert!(invoke(&mut document, &request).is_err());
    assert_eq!(document.checksum(), checksum);
    assert_eq!(document.history().len(), history);

    let mut request = parameters(&document, proposed_pin());
    request["pin"].as_object_mut().unwrap().remove("start");
    assert!(invoke(&mut document, &request).is_err());
    assert_eq!(document.checksum(), checksum);
    assert_eq!(document.history().len(), history);

    let mut request = parameters(&document, proposed_pin());
    request.insert("expected_graph_revision".into(), json!("AA".repeat(32)));
    assert_eq!(
        invoke(&mut document, &request).unwrap_err().kind(),
        Some("invalid_params")
    );
    assert_eq!(document.checksum(), checksum);
    assert_eq!(document.history().len(), history);
}

#[test]
fn dispatcher_discovery_permission_and_local_atomic_apply_are_integrated() {
    let mut document = document();
    let request = parameters(&document, proposed_pin());
    let checksum_before = document.checksum();
    let history_before = document.history().len();
    let read_only = Caps::of(&[Cap::Read]);
    let discovery = {
        let mut play = None;
        let mut target = ErpTarget::<MeshNavigation> {
            doc: &mut document,
            play: &mut play,
        };
        crate::call_local::<MeshNavigation>(
            &mut target,
            &limits(),
            "geometry-test",
            read_only,
            "rpc.discover",
            &J::Null,
        )
        .unwrap()
    };
    let method = discovery["methods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|method| method["name"] == "mesh.geometry.reproject")
        .unwrap();
    assert_eq!(method["capability"], "scene_edit");
    assert_eq!(method["allowed"], false);
    assert_eq!(
        invoke_local(&mut document, &request, read_only)
            .unwrap_err()
            .kind(),
        Some("permission_denied")
    );
    assert_eq!(document.checksum(), checksum_before);
    assert_eq!(document.history().len(), history_before);

    let result = invoke_local(&mut document, &request, Caps::ALL).unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(document.history().len(), history_before + 1);
    let checksum_after = document.checksum();
    document.undo().unwrap();
    assert_eq!(document.checksum(), checksum_before);
    document.redo().unwrap();
    assert_eq!(document.checksum(), checksum_after);
}

#[test]
fn game_build_and_capability_guards_precede_typed_frame_access() {
    let mut document = document();
    let request = parameters(&document, proposed_pin());
    let checksum = document.checksum();
    let history = document.history().len();

    assert_eq!(
        invoke_with(&mut document, &request, Caps::NONE, None)
            .unwrap_err()
            .kind(),
        Some("permission_denied")
    );
    assert_eq!(document.checksum(), checksum);
    assert_eq!(document.history().len(), history);

    let mut mismatched_limits = limits();
    mismatched_limits.build_id ^= 1;
    assert_eq!(
        invoke_with_limits(&mut document, &request, Caps::ALL, &mismatched_limits)
            .unwrap_err()
            .kind(),
        Some("profile_mismatch")
    );
    mismatched_limits = limits();
    mismatched_limits.game.name = "PhysGame".into();
    assert_eq!(
        invoke_with_limits(&mut document, &request, Caps::ALL, &mismatched_limits)
            .unwrap_err()
            .kind(),
        Some("profile_mismatch")
    );

    let mut play = None;
    let mut target = ErpTarget::<orr_testgame::Arena> {
        doc: &mut document,
        play: &mut play,
    };
    let context = CallCtx {
        client: "geometry-test",
        caps: Caps::ALL,
        last_play: None,
        tx_check: None,
    };
    assert_eq!(
        super::call::<orr_testgame::Arena>(
            &mut target,
            &limits(),
            &context,
            &P(&request),
            Origin::Agent("geometry-test".into()),
        )
        .unwrap_err()
        .kind(),
        Some("profile_mismatch")
    );
    assert_eq!(document.checksum(), checksum);
    assert_eq!(document.history().len(), history);
}

#[test]
fn reproject_is_edit_only_and_preserves_document_when_playing() {
    let mut document = document();
    let request = parameters(&document, proposed_pin());
    let checksum = document.checksum();
    let history = document.history().len();
    let play_config = document.play_config(1, 60);
    let play = PlayController::<MeshNavigation>::start_play(&document, play_config).unwrap();

    assert_eq!(
        invoke_with(&mut document, &request, Caps::ALL, Some(play))
            .unwrap_err()
            .kind(),
        Some("sim_running")
    );
    assert_eq!(document.checksum(), checksum);
    assert_eq!(document.history().len(), history);
}
