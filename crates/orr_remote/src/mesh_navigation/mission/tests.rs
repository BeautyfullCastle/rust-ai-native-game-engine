use super::*;
use crate::mesh_navigation::{authoring::create_mesh_scene, AgentSpec};
use bytemuck::Zeroable;
use orr_bridge::FrameView;
use orr_edit::{EditorDoc, Op, Origin};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use serde_json::json;

fn graph() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [fp!(4), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(4)],
            [fp!(4), FP::ZERO, fp!(4)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap()
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
    let mut config = MeshMission::zeroed();
    config.enabled = 1;
    config.count = 1;
    config.points[0] = MissionPoint {
        position: [fp!(1), FP::ZERO, fp!(1)],
        radius: fp!(0.5),
    };
    config.exit = MissionPoint {
        position: [fp!(3), FP::ZERO, fp!(3)],
        radius: fp!(0.5),
    };
    config.graph_revision = graph.revision();
    config
}

fn document_with_mission(config: &MeshMission) -> EditorDoc {
    let mut document = create_mesh_scene(&graph(), spec()).unwrap();
    let descriptor = document.types().get(MISSION_NAME).unwrap();
    let reflected = descriptor.read(bytemuck::bytes_of(config));
    document
        .apply(
            Op::SetSingletonField {
                singleton: MISSION_NAME.into(),
                path: String::new(),
                value: reflected,
            },
            Origin::User,
        )
        .unwrap();
    document
}

#[test]
fn disabled_mission_is_absent_only_when_both_singletons_are_canonical_zero() {
    let document = crate::mesh_navigation::mesh_doc_from_yaml(
        "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
    )
    .unwrap();
    assert!(from_view(FrameView::of(document.frame()))
        .unwrap()
        .is_none());

    let mut invalid = document.frame().clone();
    let mut run = *invalid.singleton::<MeshMissionRun>();
    run.collected = 1;
    invalid.set_singleton(run);
    assert!(from_view(FrameView::of(&invalid)).is_err());
}

#[test]
fn mission_view_and_patch_request_use_the_admitted_graph_and_reflected_fixed_values() {
    let graph = graph();
    let config = mission_for(&graph);
    let document = document_with_mission(&config);
    let frame_view = FrameView::of(document.frame());
    let mission_view = from_view(frame_view).unwrap().unwrap();
    assert_eq!(mission_view.config, config);
    assert_eq!(mission_view.run, MeshMissionRun::zeroed());
    let view = mesh_from_view(frame_view).unwrap();
    let request = patch_request(&config, &view).unwrap();
    assert_eq!(request["name"], MISSION_NAME);
    assert_eq!(request["path"], "");
    let expected = crate::json::value_to_json(
        &mesh_navigation_types()
            .get(MISSION_NAME)
            .unwrap()
            .read(bytemuck::bytes_of(&config)),
    );
    assert_eq!(request["value"], expected);
    assert_eq!(request["value"]["points"][0]["radius"], json!(0.5));
    assert_eq!(request["value"]["graph_revision"], json!(graph.revision()));

    let mut progressed = document.frame().clone();
    progressed.set_singleton(MeshMissionRun {
        collected: 1,
        won: 0,
    });
    let progressed_view = from_view(FrameView::of(&progressed)).unwrap().unwrap();
    assert_eq!(progressed_view.run.collected, 1);

    let mut stale = config;
    stale.graph_revision[0] ^= 1;
    assert!(patch_request(&stale, &view).is_err());
}

#[test]
fn mission_points_must_be_canonical_and_run_must_fit_the_configured_count() {
    let graph = graph();
    let mut config = mission_for(&graph);
    config.points[0].position[1] = fp!(0.25);
    assert!(mission::validate_config(&config, &graph, FP::ZERO).is_err());

    let config = mission_for(&graph);
    let run = MeshMissionRun {
        collected: 2,
        won: 0,
    };
    assert!(mission::validate_run(&config, &run).is_err());
}
