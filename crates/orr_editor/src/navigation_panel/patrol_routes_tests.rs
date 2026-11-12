use super::*;
use orr_navigation::AgentProfile;
fn p(x: i32, z: i32) -> [FP; 2] {
    [FP::from_int(x), FP::from_int(z)]
}
fn assets(ramp: bool, split: bool) -> (Terrain, TerrainGraph) {
    let heights = if ramp {
        (0..9).map(|i| FP::from_int((i % 3) as i32)).collect()
    } else {
        vec![FP::ZERO; 9]
    };
    let t = Terrain::new(
        "patrol-preview".into(),
        3,
        3,
        p(0, 0),
        FP::from_int(4),
        heights,
        if split {
            vec![false, true, true, false]
        } else {
            vec![false; 4]
        },
    )
    .unwrap();
    let g = TerrainGraph::build(
        &t,
        AgentProfile {
            max_slope: FP::ONE,
            ..Default::default()
        },
    )
    .unwrap();
    (t, g)
}
fn config(g: &TerrainGraph, points: &[[FP; 2]], looped: bool) -> TerrainPatrol {
    let mut c = TerrainPatrol {
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
fn actual_core_paths_costs_and_revisions_match_without_mutation() {
    let (t, g) = assets(false, false);
    let original = (t.clone(), g.clone());
    let start = p(1, 1);
    let c = config(&g, &[p(6, 6), p(2, 1)], false);
    let r = inspect(&t, &g, &c, start).unwrap();
    assert_eq!(r.legs.len(), 2);
    for (l, (a, b)) in r
        .legs
        .iter()
        .zip([(start, c.points[0]), (c.points[0], c.points[1])])
    {
        let path = g
            .find_path(
                &t,
                a,
                b,
                SearchBudget {
                    max_expansions: MAX_ROUTE_TRIANGLES as u32,
                },
            )
            .unwrap();
        assert_eq!(l.points, path.waypoints());
        assert_eq!(l.corridor_triangles, path.corridor().len());
        assert_eq!(l.graph_cost, path.cost());
    }
    assert_eq!(
        r.total_graph_cost.raw(),
        r.legs.iter().map(|l| l.graph_cost.raw()).sum::<i64>()
    );
    assert_eq!(r.terrain_revision, t.revision());
    assert_eq!(r.graph_revision, g.revision());
    assert_eq!((t.clone(), g.clone()), original);
    assert_eq!(inspect(&t, &g, &c, start).unwrap(), r);
}
#[test]
fn maximum_loop_has_nine_legs_and_exact_closing_path() {
    let (t, g) = assets(false, false);
    let points = std::array::from_fn::<_, 8, _>(|i| if i % 2 == 0 { p(1, 1) } else { p(6, 6) });
    let c = config(&g, &points, true);
    let r = inspect(&t, &g, &c, p(2, 1)).unwrap();
    assert_eq!(r.legs.len(), 9);
    assert_eq!(r.waypoints.len(), 8);
    let close = g
        .find_path(&t, points[7], points[0], SearchBudget::default())
        .unwrap();
    assert_eq!(r.legs[8].points, close.waypoints());
    assert!(r
        .legs
        .iter()
        .all(|l| l.corridor_triangles <= MAX_ROUTE_TRIANGLES && l.points.len() <= MAX_LEG_POINTS));
}
#[test]
fn ramp_markers_use_actual_projected_height_and_paths() {
    let (t, g) = assets(true, false);
    let start = p(1, 1);
    let c = config(&g, &[p(6, 6)], false);
    let r = inspect(&t, &g, &c, start).unwrap();
    assert_eq!(r.start, g.project(&t, start).unwrap().position);
    assert_eq!(r.waypoints[0], g.project(&t, c.points[0]).unwrap().position);
    assert!(r.waypoints[0][1] > FP::ZERO);
    assert_eq!(r.legs[0].points.first(), Some(&r.start));
    assert_eq!(r.legs[0].points.last(), Some(&r.waypoints[0]));
}
#[test]
fn unreachable_later_leg_and_holes_return_no_partial_report() {
    let (t, g) = assets(false, true);
    let c = config(&g, &[p(2, 1), p(7, 7)], true);
    let e = inspect(&t, &g, &c, p(1, 1)).unwrap_err();
    assert!(e.contains("leg 2"), "{e}");
    let hole = config(&g, &[p(6, 1)], false);
    assert!(inspect(&t, &g, &hole, p(1, 1)).is_err());
}
#[test]
fn off_surface_exact_xz_and_stale_terrain_are_rejected() {
    let (t, g) = assets(false, false);
    let c = config(&g, &[p(1, 1)], false);
    assert!(inspect(&t, &g, &c, p(-1, 1)).is_err());
    let mut bad = c;
    bad.points[0][0] = FP::from_raw(-1);
    assert!(inspect(&t, &g, &bad, p(1, 1)).is_err());
    let (changed, _) = assets(true, false);
    assert!(inspect(&changed, &g, &c, p(1, 1)).is_err());
}
#[test]
fn malformed_canonical_configs_disabled_and_zero_length_are_bounded() {
    let (t, g) = assets(false, false);
    let base = config(&g, &[p(1, 1)], false);
    for case in 0..7 {
        let mut c = base;
        match case {
            0 => c.graph_revision[0] ^= 1,
            1 => c.count = 0,
            2 => c.count = 9,
            3 => c.looped = 2,
            4 => c.points[7] = p(1, 1),
            5 => c.enabled = 2,
            _ => c.wait_ticks = 3601,
        };
        assert!(inspect(&t, &g, &c, p(1, 1)).is_err());
    }
    assert!(inspect(&t, &g, &TerrainPatrol::default(), p(1, 1)).is_err());
    let r = inspect(&t, &g, &base, p(1, 1)).unwrap();
    assert_eq!(r.legs.len(), 1);
    assert_eq!(r.legs[0].points.first(), r.legs[0].points.last());
}
