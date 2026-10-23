use super::*;
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::mission::{MeshMission, MeshMissionRun, MissionPoint};
use orr_remote::mesh_navigation::{authoring::create_mesh_scene, mesh_from_view, AgentSpec};

fn view() -> MeshView {
    let graph = StaticMeshGraph::bake(
        &[
            [FP::ZERO; 3],
            [FP::ZERO, FP::ZERO, FP::from_int(2)],
            [FP::from_int(2), FP::ZERO, FP::ZERO],
            [FP::from_int(2), FP::ZERO, FP::from_int(2)],
        ],
        &[[0, 1, 3], [0, 3, 2]],
        AgentProfile::default(),
    )
    .unwrap();
    let document = create_mesh_scene(
        &graph,
        AgentSpec {
            start: [FP::ZERO; 3],
            goal: [FP::ONE, FP::ZERO, FP::ONE],
            max_vertical: FP::ZERO,
            distance_per_tick: FP::HALF,
        },
    )
    .unwrap();
    mesh_from_view(orr_bridge::FrameView::of(document.frame())).unwrap()
}

fn mission(view: &MeshView) -> MissionView {
    let mut config = MeshMission {
        enabled: 1,
        count: 1,
        exit: MissionPoint {
            position: view.agent.spec.goal,
            radius: FP::HALF,
        },
        graph_revision: view.graph.revision(),
        ..Default::default()
    };
    config.points[0] = MissionPoint {
        position: view.agent.spec.start,
        radius: FP::HALF,
    };
    MissionView {
        config,
        run: MeshMissionRun::default(),
    }
}

#[test]
fn absent_mission_preserves_legacy_model_bytes() {
    let view = view();
    let baseline = build_view_model(&view.graph, &view.agent.path, view.agent.position).unwrap();
    assert_eq!(
        build(&view, None).unwrap().to_bytes().unwrap(),
        baseline.to_bytes().unwrap()
    );
}

#[test]
fn collection_removes_diamonds_and_unlocks_exit_without_changing_agent() {
    let view = view();
    let mut mission = mission(&view);
    let waiting = build(&view, Some(&mission)).unwrap();
    assert_eq!(waiting.source().primitives.len(), 5);
    assert_eq!(waiting.source().primitives[3].indices.len(), 24);
    let agent = waiting.source().primitives[2].clone();
    mission.run.collected = 1;
    let unlocked = build(&view, Some(&mission)).unwrap();
    assert_eq!(unlocked.source().primitives.len(), 4);
    assert_eq!(unlocked.source().primitives[2], agent);
    assert_ne!(
        waiting.source().materials.last(),
        unlocked.source().materials.last()
    );
    mission.run.won = 1;
    let won = build(&view, Some(&mission)).unwrap();
    assert_ne!(
        unlocked.source().materials.last(),
        won.source().materials.last()
    );
    assert_eq!(won.source().primitives[2], agent);
}

#[test]
fn stale_and_invalid_marker_state_fails_without_touching_input() {
    let view = view();
    let mut mission = mission(&view);
    mission.config.graph_revision[0] ^= 1;
    assert!(build(&view, Some(&mission)).is_err());
    mission.config.graph_revision = view.graph.revision();
    mission.run.collected = 2;
    assert!(build(&view, Some(&mission)).is_err());
}
