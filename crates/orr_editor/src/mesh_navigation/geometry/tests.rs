use super::{append_quad, faces, move_vertex, remove_face, Quad};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};

fn profile(max_slope: FP) -> AgentProfile {
    AgentProfile {
        max_slope,
        ..AgentProfile::default()
    }
}

fn quad(origin: [FP; 3], rise: FP) -> Quad {
    Quad {
        origin,
        width: fp!(4),
        depth: fp!(2),
        rise,
    }
}

#[test]
fn adjacent_flat_and_sloped_quads_share_the_edge() {
    let first = append_quad(
        None,
        profile(fp!(4)),
        quad([fp!(-2), fp!(0), fp!(-2)], fp!(0)),
    )
    .unwrap();
    let combined = append_quad(
        Some(&first),
        profile(fp!(4)),
        quad([fp!(-2), fp!(0), fp!(0)], fp!(3)),
    )
    .unwrap();
    assert_eq!(combined.vertices().len(), 6);
    assert_eq!(faces(&combined).unwrap().len(), 4);
    assert_eq!(combined.triangles().len(), 4);
    assert_eq!(
        StaticMeshGraph::from_bytes(&combined.to_bytes()).unwrap(),
        combined
    );
}

#[test]
fn editing_preserves_slope_rejected_source_faces() {
    let rejected = append_quad(
        None,
        profile(FP::ZERO),
        quad([fp!(0), fp!(0), fp!(0)], fp!(2)),
    )
    .unwrap();
    assert!(rejected.triangles().is_empty());
    assert_eq!(faces(&rejected).unwrap().len(), 2);
    let remaining = remove_face(&rejected, 0).unwrap();
    assert_eq!(faces(&remaining).unwrap().len(), 1);
    assert!(remaining.triangles().is_empty());
    assert_eq!(faces(&rejected).unwrap().len(), 2);
}

#[test]
fn failed_moves_and_removals_leave_the_original_graph_unchanged() {
    let original = append_quad(
        None,
        profile(fp!(4)),
        quad([fp!(0), fp!(0), fp!(0)], fp!(0)),
    )
    .unwrap();
    let bytes = original.to_bytes();
    assert!(move_vertex(&original, 0, original.vertices()[0]).is_err());
    assert!(move_vertex(&original, 99, [fp!(1), fp!(0), fp!(1)]).is_err());
    assert!(move_vertex(&original, 0, [fp!(257), fp!(0), fp!(0)]).is_err());
    assert!(remove_face(&original, 9).is_err());
    assert_eq!(original.to_bytes(), bytes);
}

#[test]
fn malformed_or_incompatible_geometry_is_rejected_without_changing_inputs() {
    let graph = append_quad(
        None,
        profile(fp!(4)),
        quad([fp!(0), fp!(0), fp!(0)], fp!(0)),
    )
    .unwrap();
    let original = graph.to_bytes();
    assert!(append_quad(
        Some(&graph),
        profile(FP::ZERO),
        quad([fp!(5), fp!(0), fp!(0)], fp!(0))
    )
    .is_err());
    assert!(append_quad(
        None,
        profile(fp!(4)),
        Quad {
            width: FP::ZERO,
            ..quad([fp!(0), fp!(0), fp!(0)], fp!(0))
        }
    )
    .is_err());
    assert!(append_quad(
        None,
        profile(fp!(4)),
        quad([fp!(255), fp!(0), fp!(0)], fp!(0))
    )
    .is_err());
    assert_eq!(graph.to_bytes(), original);
}
