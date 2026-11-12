use super::*;
#[test]
fn extraction_keeps_initial_markers_all_bends_and_closing_leg_in_order() {
    use super::super::super::patrol_routes::{Leg, Report};
    let p = |x| [FP::from_int(x), FP::ZERO, FP::ZERO];
    let report = Report {
        revision: [1; 32],
        start: p(0),
        waypoints: vec![p(2), p(4)],
        legs: vec![
            Leg {
                points: vec![p(0), p(1), p(2)],
                length: FP::ZERO,
                corridor_triangles: 2,
            },
            Leg {
                points: vec![p(2), p(3), p(4)],
                length: FP::ZERO,
                corridor_triangles: 2,
            },
            Leg {
                points: vec![p(4), p(5), p(2)],
                length: FP::ZERO,
                corridor_triangles: 2,
            },
        ],
        total_length: FP::ZERO,
    };
    let actual = points(&report).unwrap();
    assert_eq!(actual.len(), 12);
    assert_eq!(actual[4], [1.0, 0.0, 0.0]);
    assert_eq!(actual[10], [5.0, 0.0, 0.0]);
    let mut bad = report.clone();
    bad.legs.clear();
    assert!(points(&bad).is_err());
}
