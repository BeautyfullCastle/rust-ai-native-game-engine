//! Display-only inspected route. Coordinates are all admitted before any draw command.
#![allow(clippy::float_arithmetic)]
use super::{point_to_f32, Panel};
use crate::editor::Editor;
use orr_fp::FP;
use orr_render::RenderList3D;

const MAX_POINTS: usize = orr_navigation::MAX_TRIANGLES as usize + 1;
const ROUTE: [f32; 4] = [0.0, 0.9, 1.0, 1.0];
const START: [f32; 4] = [0.1, 1.0, 0.25, 1.0];
const GOAL: [f32; 4] = [1.0, 0.15, 0.1, 1.0];

pub(super) fn append(list: &mut RenderList3D, points: &[[FP; 3]]) -> Result<(), String> {
    if points.is_empty() || points.len() > MAX_POINTS {
        return Err("Inspected route point count is outside the core path bound".into());
    }
    let converted = points
        .iter()
        .map(|point| point_to_f32(*point))
        .collect::<Result<Vec<_>, _>>()?;
    // No world offset: markers and line show the exact admitted route coordinates.
    for pair in converted.windows(2) {
        list.line(pair[0], pair[1], 3.0, ROUTE);
    }
    for (point, color) in [(converted[0], START), (*converted.last().unwrap(), GOAL)] {
        list.aabb(point.map(|v| v - 0.06), point.map(|v| v + 0.06), 2.0, color);
    }
    Ok(())
}
impl Panel {
    pub(crate) fn paint_draft_route(&self, editor: &Editor, list: &mut RenderList3D) {
        if !editor.game().has_mesh_view() {
            return;
        }
        #[cfg(feature = "mesh-patrol")]
        self.paint_patrol_route(editor, list);
        if let Some(points) = self.inspected_route_points(editor) {
            let _ = append(list, points);
        }
    }
}
#[cfg(test)]
#[path = "route_diagnostics_overlay_tests.rs"]
mod tests;
