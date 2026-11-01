use super::*;
use orr_navigation::{AgentProfile, MAX_COORDINATE_RAW};

fn point(x: i32, y: i32, z: i32) -> [FP; 3] {
    [FP::from_int(x), FP::from_int(y), FP::from_int(z)]
}

fn graph(disconnected: bool) -> StaticMeshGraph {
    let vertices = [
        point(0, 0, 0),
        point(0, 0, 4),
        point(4, 0, 0),
        point(4, 0, 4),
        point(8, 0, 0),
        point(8, 0, 4),
        point(12, 0, 0),
    ];
    let faces = if disconnected {
        vec![[0, 1, 2], [4, 5, 6]]
    } else {
        vec![[0, 1, 2], [2, 1, 3]]
    };
    StaticMeshGraph::bake(&vertices, &faces, AgentProfile::default()).unwrap()
}

#[test]
fn endpoint_failures_are_independent_and_skip_search() {
    let graph = graph(false);
    let inside = point(1, 0, 1);
    let outside = point(-1, 0, -1);
    let budget = SearchBudget { max_expansions: 0 };
    for (start, goal) in [(outside, inside), (inside, outside), (outside, outside)] {
        let report = inspect_route(&graph, start, goal, FP::ZERO, budget);
        assert_eq!(report.start.is_ok(), start == inside);
        assert_eq!(report.goal.is_ok(), goal == inside);
        if start == outside {
            assert_eq!(report.start, Err(NavigationError::OutsideTerrain));
        }
        if goal == outside {
            assert_eq!(report.goal, Err(NavigationError::OutsideTerrain));
        }
        assert_eq!(report.route, RouteOutcome::EndpointsInvalid);
    }
}

#[test]
fn disconnected_valid_endpoints_report_unreachable() {
    let graph = graph(true);
    let report = inspect_route(
        &graph,
        point(1, 0, 1),
        point(9, 0, 1),
        FP::ZERO,
        SearchBudget::default(),
    );
    assert!(report.start.is_ok() && report.goal.is_ok());
    assert_eq!(
        report.route,
        RouteOutcome::Failed(NavigationError::Unreachable)
    );
}

#[test]
fn same_triangle_length_and_zero_length_need_no_search_expansions() {
    let graph = graph(false);
    for (goal, length) in [(point(2, 0, 1), FP::ONE), (point(1, 0, 1), FP::ZERO)] {
        let report = inspect_route(
            &graph,
            point(1, 0, 1),
            goal,
            FP::ZERO,
            SearchBudget { max_expansions: 0 },
        );
        assert_eq!(
            report.route,
            RouteOutcome::Ready {
                corridor_triangles: 1,
                length
            }
        );
    }
}

#[test]
fn valid_corridor_uses_validated_core_length_and_is_deterministic_read_only() {
    let graph = graph(false);
    let bytes = graph.to_bytes();
    let start = point(1, 0, 1);
    let goal = point(3, 0, 3);
    let path = graph
        .find_path(start, goal, FP::ZERO, SearchBudget::default())
        .unwrap();
    graph.validate_path(&path).unwrap();
    assert_eq!(path.corridor.len(), 2);
    let expected = inspect_route(&graph, start, goal, FP::ZERO, SearchBudget::default());
    assert_eq!(expected.revision, graph.revision());
    assert_eq!(
        expected.route,
        RouteOutcome::Ready {
            corridor_triangles: 2,
            length: graph.path_length(&path).unwrap()
        }
    );
    for _ in 0..4 {
        assert_eq!(
            inspect_route(&graph, start, goal, FP::ZERO, SearchBudget::default()),
            expected
        );
        assert_eq!(graph.to_bytes(), bytes);
    }
}

#[test]
fn budget_error_retains_successful_projections_without_mutation() {
    let graph = graph(false);
    let bytes = graph.to_bytes();
    let report = inspect_route(
        &graph,
        point(1, 0, 1),
        point(3, 0, 3),
        FP::ZERO,
        SearchBudget { max_expansions: 0 },
    );
    assert!(report.start.is_ok() && report.goal.is_ok());
    assert_eq!(
        report.route,
        RouteOutcome::Failed(NavigationError::BudgetExceeded { expanded: 0 })
    );
    assert_eq!(graph.to_bytes(), bytes);
}

#[test]
fn numeric_limits_are_endpoint_errors_and_do_not_hide_other_endpoint() {
    let graph = graph(false);
    let bytes = graph.to_bytes();
    let invalid = [FP::from_raw(MAX_COORDINATE_RAW + 1), FP::ZERO, FP::ZERO];
    let valid = point(1, 0, 1);
    let report = inspect_route(&graph, invalid, valid, FP::ZERO, SearchBudget::default());
    assert_eq!(report.start, Err(NavigationError::NumericLimit));
    assert!(report.goal.is_ok());
    assert_eq!(report.route, RouteOutcome::EndpointsInvalid);
    let negative = inspect_route(
        &graph,
        valid,
        valid,
        FP::from_raw(-1),
        SearchBudget::default(),
    );
    assert_eq!(negative.start, Err(NavigationError::NumericLimit));
    assert_eq!(negative.goal, Err(NavigationError::NumericLimit));
    assert_eq!(negative.route, RouteOutcome::EndpointsInvalid);
    assert_eq!(graph.to_bytes(), bytes);
}

#[test]
fn vertical_tolerance_projects_but_does_not_claim_clearance() {
    let graph = graph(false);
    let start = point(1, 1, 1);
    let goal = point(2, 0, 1);
    let rejected = inspect_route(&graph, start, goal, FP::ZERO, SearchBudget::default());
    assert_eq!(rejected.start, Err(NavigationError::OutsideTerrain));
    assert!(rejected.goal.is_ok());
    let admitted = inspect_route(&graph, start, goal, FP::ONE, SearchBudget::default());
    assert_eq!(admitted.start.unwrap().position, point(1, 0, 1));
    assert_eq!(
        admitted.route,
        RouteOutcome::Ready {
            corridor_triangles: 1,
            length: FP::ONE
        }
    );
}
