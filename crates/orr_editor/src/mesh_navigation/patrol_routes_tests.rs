use super::*;
use orr_navigation::AgentProfile;
fn point(x: i32, z: i32) -> [FP; 3] {
    [FP::from_int(x), FP::ZERO, FP::from_int(z)]
}
fn graph(disconnected: bool) -> StaticMeshGraph {
    let vertices = [
        point(0, 0),
        point(0, 4),
        point(4, 0),
        point(4, 4),
        point(8, 0),
        point(8, 4),
        point(12, 0),
    ];
    let faces = if disconnected {
        vec![[0, 1, 2], [4, 5, 6]]
    } else {
        vec![[0, 1, 2], [2, 1, 3]]
    };
    StaticMeshGraph::bake(&vertices, &faces, AgentProfile::default()).unwrap()
}
fn config(g: &StaticMeshGraph, points: &[[FP; 3]], looped: bool) -> MeshPatrol {
    let mut c = MeshPatrol {
        enabled: 1,
        count: points.len() as u32,
        looped: u32::from(looped),
        graph_revision: g.revision(),
        ..Default::default()
    };
    c.points[..points.len()].copy_from_slice(points);
    c
}
#[test]
fn exact_portal_paths_lengths_and_total_match_actual_core_without_mutation() {
    let g = graph(false);
    let before = g.to_bytes();
    let start = point(1, 1);
    let c = config(&g, &[point(3, 3), point(2, 1)], false);
    let r = inspect(&g, &c, start, FP::ZERO).unwrap();
    assert_eq!(r.start, start);
    assert_eq!(r.revision, g.revision());
    assert_eq!(r.waypoints, c.points[..2]);
    assert_eq!(r.legs.len(), 2);
    for (leg, (a, b)) in r
        .legs
        .iter()
        .zip([(start, c.points[0]), (c.points[0], c.points[1])])
    {
        let p = g
            .find_path(
                a,
                b,
                FP::ZERO,
                SearchBudget {
                    max_expansions: g.triangles().len() as u32,
                },
            )
            .unwrap();
        assert_eq!(leg.points, p.points);
        assert_eq!(leg.corridor_triangles, p.corridor.len());
        assert_eq!(leg.length, g.path_length(&p).unwrap());
    }
    assert_eq!(
        r.total_length.raw(),
        r.legs.iter().map(|l| l.length.raw()).sum::<i64>()
    );
    assert_eq!(g.to_bytes(), before);
    assert_eq!(inspect(&g, &c, start, FP::ZERO).unwrap(), r);
}
#[test]
fn eight_looped_waypoints_have_nine_legs_including_actual_closing_path() {
    let g = graph(false);
    let points =
        std::array::from_fn::<_, 8, _>(|i| if i % 2 == 0 { point(1, 1) } else { point(3, 3) });
    let c = config(&g, &points, true);
    let r = inspect(&g, &c, point(2, 1), FP::ZERO).unwrap();
    assert_eq!(r.legs.len(), 9);
    assert_eq!(r.waypoints.len(), 8);
    let close = g
        .find_path(points[7], points[0], FP::ZERO, SearchBudget::default())
        .unwrap();
    assert_eq!(r.legs[8].points, close.points);
    assert_eq!(r.legs[8].length, g.path_length(&close).unwrap());
}
#[test]
fn unreachable_later_leg_returns_indexed_error_not_partial_report() {
    let g = graph(true);
    let c = config(&g, &[point(2, 1), point(9, 1)], true);
    let before = g.to_bytes();
    let e = inspect(&g, &c, point(1, 1), FP::ZERO).unwrap_err();
    assert!(e.contains("leg 2"), "{e}");
    assert_eq!(g.to_bytes(), before);
}
#[test]
fn invalid_first_leg_is_identified_and_negative_vertical_rejected() {
    let g = graph(false);
    let c = config(&g, &[point(1, 1)], false);
    assert!(inspect(&g, &c, point(-1, -1), FP::ZERO)
        .unwrap_err()
        .contains("leg 1"));
    assert!(inspect(&g, &c, point(1, 1), -FP::ONE).is_err());
}
#[test]
fn malformed_revision_count_flags_tail_bounds_and_disabled_fail_closed() {
    let g = graph(false);
    let base = config(&g, &[point(1, 1)], false);
    for case in 0..7 {
        let mut c = base;
        match case {
            0 => c.graph_revision[0] ^= 1,
            1 => c.count = 0,
            2 => c.count = 9,
            3 => c.looped = 2,
            4 => c.points[7] = point(1, 1),
            5 => c.points[0] = point(257, 0),
            _ => c.wait_ticks = 3601,
        };
        assert!(inspect(&g, &c, point(1, 1), FP::ZERO).is_err());
    }
    assert!(inspect(&g, &MeshPatrol::default(), point(1, 1), FP::ZERO).is_err());
}
#[test]
fn zero_length_and_single_point_loop_keep_complete_bounded_reports() {
    let g = graph(false);
    let mut raised = point(2, 1);
    raised[1] = FP::ONE;
    let mut start = point(1, 1);
    start[1] = FP::ONE;
    let c = config(&g, &[raised], true);
    let projected = inspect(&g, &c, start, FP::ONE).unwrap();
    assert_eq!(projected.start, point(1, 1));
    assert_eq!(projected.waypoints, vec![point(2, 1)]);
    assert_eq!(projected.legs[0].points.first(), Some(&projected.start));
    assert_eq!(
        projected.legs[0].points.last(),
        Some(&projected.waypoints[0])
    );

    let g = graph(false);
    let p = point(1, 1);
    for looped in [false, true] {
        let c = config(&g, &[p], looped);
        let r = inspect(&g, &c, p, FP::ZERO).unwrap();
        assert_eq!(r.legs.len(), if looped { 2 } else { 1 });
        assert_eq!(r.total_length, FP::ZERO);
        for l in r.legs {
            assert_eq!(l.length, FP::ZERO);
            assert_eq!(l.points.first(), Some(&p));
            assert_eq!(l.points.last(), Some(&p));
            assert_eq!(l.corridor_triangles, 1);
        }
    }
}
