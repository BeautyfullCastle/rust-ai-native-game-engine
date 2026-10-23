use super::*;
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_render::OrbitCamera;

fn graph_with_unreferenced_vertex() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            [FP::from_int(-1), FP::ZERO, FP::from_int(-1)],
            [FP::from_int(1), FP::ZERO, FP::from_int(-1)],
            [FP::from_int(-1), FP::ZERO, FP::from_int(1)],
            [FP::from_int(1), FP::ZERO, FP::from_int(1)],
            [FP::from_int(7), FP::ZERO, FP::from_int(7)],
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
fn pick_uses_only_referenced_vertices_and_stable_nearest_pixel() {
    let graph = graph_with_unreferenced_vertex();
    let camera = OrbitCamera::new([3.0, 0.0, 3.0], 0.0, 0.8, 24.0).camera();
    let viewport = (800, 600);
    let pixel = camera.world_to_screen([-1.0, 0.0, -1.0], viewport).unwrap();
    assert_eq!(
        pick_vertex(&graph, &camera, pixel, viewport).unwrap(),
        Some(0)
    );
    let unreferenced = camera
        .world_to_screen([7.0, 0.0, 7.0], viewport)
        .expect("unreferenced fixture vertex projects in front of the camera");
    assert!((0.0..=viewport.0 as f32).contains(&unreferenced[0]));
    assert!((0.0..=viewport.1 as f32).contains(&unreferenced[1]));
    assert_eq!(
        pick_vertex(&graph, &camera, unreferenced, viewport).unwrap(),
        None
    );
}

#[test]
fn picker_rejects_invalid_viewport_and_far_pixels() {
    let graph = graph_with_unreferenced_vertex();
    let camera = OrbitCamera::new([0.0, 0.0, 0.0], 0.0, 0.8, 7.0).camera();
    assert!(pick_vertex(&graph, &camera, [f32::NAN, 1.0], (800, 600)).is_err());
    assert!(pick_vertex(&graph, &camera, [1.0, 1.0], (0, 600)).is_err());
    assert_eq!(
        pick_vertex(&graph, &camera, [799.0, 599.0], (800, 600)).unwrap(),
        None
    );
}

#[test]
fn snapped_drag_position_converts_to_exact_fixed_point() {
    let point = quantize_snapped([1.25, 0.0, -2.5]).unwrap();
    assert_eq!(
        point,
        [FP::from_raw(81_920), FP::ZERO, FP::from_raw(-163_840)]
    );
    assert!(quantize_snapped([257.0, 0.0, 0.0]).is_err());
}
