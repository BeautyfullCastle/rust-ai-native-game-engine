use super::*;
use crate::mesh_navigation::build_view_model;
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::hazards::{HazardSphere, MeshHazardRun, MeshHazards};
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

fn hazards(view: &MeshView) -> HazardsView {
    let mut config = MeshHazards {
        enabled: 1,
        count: 1,
        graph_revision: view.graph.revision(),
        ..Default::default()
    };
    config.spheres[0] = HazardSphere {
        center: [FP::ONE; 3],
        radius: FP::HALF,
    };
    HazardsView {
        config,
        run: MeshHazardRun::default(),
    }
}
fn base(view: &MeshView) -> StaticModel {
    build_view_model(&view.graph, &view.agent.path, view.agent.position).unwrap()
}
#[test]
fn disabled_hazards_leave_legacy_model_bytes_unchanged() {
    let view = view();
    assert_eq!(
        build(base(&view), &view, None).unwrap().to_bytes().unwrap(),
        base(&view).to_bytes().unwrap()
    );
}
#[test]
fn warning_bands_follow_authored_radius_with_bounded_geometry() {
    let view = view();
    let hazards = hazards(&view);
    let model = build(base(&view), &view, Some(&hazards)).unwrap();
    let primitive = model.source().primitives.last().unwrap();
    assert_eq!(primitive.vertices.len(), 3 * SEGMENTS * 2 * 12);
    for vertex in &primitive.vertices {
        let delta = vertex.position.map(|v| v - 1.0);
        let radius = delta.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((radius - 0.5).abs() < 0.000001);
        assert!(vertex.position.iter().all(|v| (0.5..=1.5).contains(v)));
    }
    for axis in 0..3 {
        assert!(primitive.vertices.iter().any(|v| v.position[axis] == 0.5));
        assert!(primitive.vertices.iter().any(|v| v.position[axis] == 1.5));
    }
}
#[test]
fn loss_highlights_only_hit_sphere_and_preserves_base() {
    let view = view();
    let mut hazards = hazards(&view);
    hazards.config.count = 2;
    hazards.config.spheres[1] = hazards.config.spheres[0];
    let active = build(base(&view), &view, Some(&hazards)).unwrap();
    hazards.run = MeshHazardRun {
        lost: 1,
        hit_index: 1,
        lost_tick: 5,
    };
    let lost = build(base(&view), &view, Some(&hazards)).unwrap();
    assert_eq!(active.source().primitives, lost.source().primitives);
    assert_eq!(
        &active.source().materials[..4],
        &lost.source().materials[..4]
    );
    assert_ne!(
        active.source().materials.last(),
        lost.source().materials.last()
    );
}
#[test]
fn malformed_bounds_revision_and_loss_are_rejected() {
    let view = view();
    let valid = hazards(&view);
    for invalid in 0..5 {
        let mut hazards = HazardsView {
            config: valid.config,
            run: valid.run,
        };
        match invalid {
            0 => hazards.config.count = 9,
            1 => hazards.config.graph_revision[0] ^= 1,
            2 => hazards.config.spheres[0].radius = FP::ZERO,
            3 => hazards.config.spheres[0].center[0] = FP::from_int(257),
            _ => hazards.run.hit_index = 1,
        }
        assert!(build(base(&view), &view, Some(&hazards)).is_err());
    }
}
#[test]
fn legacy_identity_returns_before_missing_hazard_singleton_read() {
    let mesh = view();
    let document = create_mesh_scene(&mesh.graph, mesh.agent.spec).unwrap();
    let frame = orr_bridge::FrameView::of(document.frame());
    assert!(read(crate::game::EditorGame::MeshNavigation, frame)
        .unwrap()
        .is_none());
    assert!(read(crate::game::EditorGame::PhysGame, frame)
        .unwrap()
        .is_none());
}
#[test]
fn maximum_hazards_and_radius_remain_finite_and_bounded() {
    let view = view();
    let mut hazards = hazards(&view);
    hazards.config.count = 8;
    hazards.config.spheres.fill(HazardSphere {
        center: [FP::from_int(256); 3],
        radius: FP::from_int(256),
    });
    let model = build(base(&view), &view, Some(&hazards)).unwrap();
    assert_eq!(model.source().primitives.len(), 11);
    for primitive in &model.source().primitives[3..] {
        assert_eq!(primitive.vertices.len(), 2304);
        assert!(primitive.vertices.iter().all(|v| v
            .position
            .iter()
            .all(|p| p.is_finite() && (0.0..=512.0).contains(p))));
    }
    hazards.config.spheres[0].radius = FP::from_raw(256 * FP::SCALE + 1);
    assert!(build(base(&view), &view, Some(&hazards)).is_err());
}
