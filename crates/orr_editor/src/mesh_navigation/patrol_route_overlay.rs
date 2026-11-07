//! Ephemeral all-leg patrol overlay; admission finishes before any draw command.
#![allow(clippy::float_arithmetic)]
use super::{Panel, patrol_routes::Report, point_to_f32};
use crate::editor::Editor;
use orr_render::RenderList3D;

const ROUTE: [f32; 4] = [0.0, 0.8, 1.0, 1.0];
const CLOSURE: [f32; 4] = [0.85, 0.35, 1.0, 1.0];
const START: [f32; 4] = [0.1, 1.0, 0.25, 1.0];
const WAYPOINT: [f32; 4] = [1.0, 0.65, 0.05, 1.0];

fn append(list: &mut RenderList3D, report: &Report) -> Result<(), String> {
    let count = report.waypoints.len();
    if !(1..=8).contains(&count) || report.legs.len() < count || report.legs.len() > count + 1 {
        return Err("Patrol overlay leg/waypoint count is invalid".into());
    }
    let start = point_to_f32(report.start)?;
    let waypoints = report
        .waypoints
        .iter()
        .copied()
        .map(point_to_f32)
        .collect::<Result<Vec<_>, _>>()?;
    let paths = report
        .legs
        .iter()
        .enumerate()
        .map(|(index, leg)| {
            if leg.points.is_empty()
                || leg.points.len() > orr_navigation::MAX_TRIANGLES as usize + 1
            {
                return Err("Patrol overlay path exceeds the core bound".into());
            }
            let expected_start = if index == 0 {
                report.start
            } else {
                report.waypoints[index - 1]
            };
            let expected_goal = report.waypoints[if index == count { 0 } else { index }];
            if leg.points.first() != Some(&expected_start)
                || leg.points.last() != Some(&expected_goal)
            {
                return Err("Patrol overlay leg endpoints differ from inspected waypoints".into());
            }
            leg.points
                .iter()
                .copied()
                .map(point_to_f32)
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, String>>()?;
    // Everything above is fallible. Never draw a misleading partial patrol.
    for (index, path) in paths.iter().enumerate() {
        let color = if index == count { CLOSURE } else { ROUTE };
        for pair in path.windows(2) {
            list.line(pair[0], pair[1], 3.0, color);
        }
    }
    list.aabb(start.map(|v| v - 0.07), start.map(|v| v + 0.07), 2.0, START);
    for point in waypoints {
        list.aabb(
            point.map(|v| v - 0.05),
            point.map(|v| v + 0.05),
            2.0,
            WAYPOINT,
        );
    }
    Ok(())
}
impl Panel {
    pub(crate) fn sync_patrol_inspection_focus(&self, editor: &Editor, ctx: &egui::Context) {
        if ctx.input(|input| {
            !input.focused
                || input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::WindowFocused(false)))
        }) {
            self.retire_patrol_route();
        }
        // Observe context changes even when the inspector and viewport are hidden.
        let _ = self.inspected_patrol_route(editor);
    }
    pub(super) fn paint_patrol_route(&self, editor: &Editor, list: &mut RenderList3D) {
        if let Some(report) = self.inspected_patrol_route(editor) {
            let _ = append(list, report);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::patrol_routes::Leg;
    use super::*;
    use orr_fp::FP;
    fn p(x: i32) -> [FP; 3] {
        [FP::from_int(x), FP::ZERO, FP::ZERO]
    }
    fn report() -> Report {
        Report {
            revision: [1; 32],
            start: p(0),
            waypoints: vec![p(1), p(2)],
            legs: vec![
                Leg {
                    points: vec![p(0), p(1)],
                    length: FP::ONE,
                    corridor_triangles: 1,
                },
                Leg {
                    points: vec![p(1), p(2)],
                    length: FP::ONE,
                    corridor_triangles: 1,
                },
                Leg {
                    points: vec![p(2), p(1)],
                    length: FP::ONE,
                    corridor_triangles: 1,
                },
            ],
            total_length: FP::from_int(3),
        }
    }
    #[test]
    fn all_legs_and_closure_have_exact_separate_lines_and_waypoint_markers() {
        let mut list = RenderList3D::default();
        append(&mut list, &report()).unwrap();
        assert_eq!(list.lines.len(), 3 + 36);
        for (i, (a, b, color)) in [(0., 1., ROUTE), (1., 2., ROUTE), (2., 1., CLOSURE)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(list.lines[i].a, [a, 0., 0.]);
            assert_eq!(list.lines[i].b, [b, 0., 0.]);
            assert_eq!(list.lines[i].color, color);
        }
        assert!(list.lines[3..15].iter().all(|l| l.color == START));
        assert!(list.lines[15..].iter().all(|l| l.color == WAYPOINT));
        assert_eq!(list.instance_count(), 0);
    }
    #[test]
    fn invalid_last_leg_and_waypoint_never_append_partial_commands() {
        let mut list = RenderList3D::default();
        list.line([0.; 3], [1.; 3], 1., [1.; 4]);
        let baseline = list.lines.clone();
        let mut bad = report();
        bad.legs[2].points.insert(1, p(257));
        assert!(append(&mut list, &bad).is_err());
        assert_eq!(list.lines, baseline);
        let mut bad = report();
        bad.waypoints[1] = p(257);
        assert!(append(&mut list, &bad).is_err());
        assert_eq!(list.lines, baseline);
        let mut bad = report();
        bad.legs[2].points.clear();
        assert!(append(&mut list, &bad).is_err());
        assert_eq!(list.lines, baseline);
    }
    #[test]
    fn missing_or_excess_legs_and_wrong_endpoint_never_draw() {
        for kind in 0..3 {
            let mut bad = report();
            match kind {
                0 => bad.legs.truncate(1),
                1 => bad.legs.push(bad.legs[2].clone()),
                _ => bad.legs[2].points[0] = p(0),
            }
            let mut list = RenderList3D::default();
            assert!(append(&mut list, &bad).is_err());
            assert!(list.is_empty());
        }
    }
}
