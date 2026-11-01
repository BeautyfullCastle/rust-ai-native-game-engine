//! Read-only, bounded point-agent route diagnostics; not clearance validation.
use orr_fp::FP;
use orr_navigation::{
    mesh::{MeshProjection, StaticMeshGraph},
    NavigationError, SearchBudget,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RouteReport {
    pub start: Result<MeshProjection, NavigationError>,
    pub goal: Result<MeshProjection, NavigationError>,
    pub route: RouteOutcome,
    pub revision: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RouteOutcome {
    EndpointsInvalid,
    Ready {
        corridor_triangles: usize,
        length: FP,
    },
    Failed(NavigationError),
}

/// Inspect both projections independently, then search only if both are valid.
/// Length is the core's validated portal-polyline length, not a shortest-path or
/// radius/headroom guarantee. Neither the graph nor any scene state is changed.
pub(super) fn inspect_route(
    graph: &StaticMeshGraph,
    start: [FP; 3],
    goal: [FP; 3],
    max_vertical: FP,
    budget: SearchBudget,
) -> RouteReport {
    let start_projection = graph.project(start, max_vertical);
    let goal_projection = graph.project(goal, max_vertical);
    let route = if start_projection.is_ok() && goal_projection.is_ok() {
        match graph.find_path(start, goal, max_vertical, budget) {
            Ok(path) => match graph.path_length(&path) {
                Ok(length) => RouteOutcome::Ready {
                    corridor_triangles: path.corridor.len(),
                    length,
                },
                Err(error) => RouteOutcome::Failed(error),
            },
            Err(error) => RouteOutcome::Failed(error),
        }
    } else {
        RouteOutcome::EndpointsInvalid
    };
    RouteReport {
        start: start_projection,
        goal: goal_projection,
        route,
        revision: graph.revision(),
    }
}

#[cfg(test)]
#[path = "route_diagnostics/tests.rs"]
mod tests;
