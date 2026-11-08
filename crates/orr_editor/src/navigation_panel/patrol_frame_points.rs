//! Bounded conversion of the complete admitted inspection, including all bends.
#![allow(clippy::float_arithmetic)]
use super::patrol_routes::{Report, MAX_LEG_POINTS};
use orr_fp::FP;
pub(super) fn points(report: &Report) -> Result<Vec<[f32; 3]>, String> {
    let count = report.waypoints.len();
    if !(1..=8).contains(&count) || report.legs.len() < count || report.legs.len() > count + 1 {
        return Err("Invalid inspected route shape".into());
    }
    let total = report.legs.iter().try_fold(count + 1, |n, l| {
        if l.points.is_empty() || l.points.len() > MAX_LEG_POINTS {
            return Err("Inspected leg exceeds route bound".to_owned());
        }
        n.checked_add(l.points.len())
            .ok_or("Route point overflow".to_owned())
    })?;
    let mut out = Vec::with_capacity(total);
    for point in std::iter::once(&report.start)
        .chain(report.waypoints.iter())
        .chain(report.legs.iter().flat_map(|l| l.points.iter()))
    {
        for v in point {
            let f = v.to_f32();
            if v.raw().unsigned_abs() > 256 * 65536
                || !f.is_finite()
                || f64::from(f) * 65536.0 != v.raw() as f64
            {
                return Err(
                    "Terrain route framing requires exact coordinates within +/-256".into(),
                );
            }
        }
        out.push(point.map(FP::to_f32));
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation_panel::patrol_routes::Leg;
    #[test]
    fn extraction_includes_elevated_start_markers_bends_and_closure_and_rejects_bounds() {
        let p = |x| [FP::from_int(x), FP::from_int(x), FP::ZERO];
        let report = Report {
            terrain_revision: [1; 32],
            graph_revision: [2; 32],
            start: p(0),
            waypoints: vec![p(2), p(4)],
            legs: vec![
                Leg {
                    points: vec![p(0), p(1), p(2)],
                    corridor_triangles: 2,
                    graph_cost: FP::ZERO,
                },
                Leg {
                    points: vec![p(2), p(3), p(4)],
                    corridor_triangles: 2,
                    graph_cost: FP::ZERO,
                },
                Leg {
                    points: vec![p(4), p(5), p(2)],
                    corridor_triangles: 2,
                    graph_cost: FP::ZERO,
                },
            ],
            total_graph_cost: FP::ZERO,
        };
        let values = points(&report).unwrap();
        assert_eq!(values.len(), 12);
        assert_eq!(values[4], [1.0, 1.0, 0.0]);
        assert_eq!(values[10], [5.0, 5.0, 0.0]);
        let mut bad = report.clone();
        bad.legs[2].points[1] = p(257);
        assert!(points(&bad).is_err());
        bad = report.clone();
        bad.legs[0].points.clear();
        assert!(points(&bad).is_err());
        bad = report;
        bad.waypoints.clear();
        assert!(points(&bad).is_err());
    }
}
