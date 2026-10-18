use super::{create, Grid};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile, SearchBudget};

fn profile() -> AgentProfile {
    AgentProfile {
        max_slope: fp!(4),
        ..AgentProfile::default()
    }
}

fn grid(columns: u32, rows: u32) -> Grid {
    Grid {
        origin: [fp!(-3), fp!(0), fp!(-3)],
        width: fp!(6),
        depth: fp!(6),
        columns,
        rows,
        excluded_cells: 0,
    }
}

#[test]
fn connected_tiles_share_edges_and_paths_can_route_around_a_hole() {
    let adjacent = create(grid(2, 1), profile()).unwrap();
    assert_eq!(adjacent.vertices().len(), 6);
    assert_eq!(adjacent.triangles().len(), 4);
    assert!(adjacent
        .find_path(
            [fp!(-2), FP::ZERO, FP::ZERO],
            [fp!(2), FP::ZERO, FP::ZERO],
            FP::ZERO,
            SearchBudget::default(),
        )
        .is_ok());

    let mut with_hole = grid(3, 3);
    with_hole.excluded_cells = 1 << 4;
    let graph = create(with_hole, profile()).unwrap();
    assert_eq!(graph.triangles().len(), 16);
    assert!(graph.project([FP::ZERO; 3], FP::ZERO).is_err());
    let route = graph
        .find_path(
            [fp!(-2), FP::ZERO, FP::ZERO],
            [fp!(2), FP::ZERO, FP::ZERO],
            FP::ZERO,
            SearchBudget::default(),
        )
        .unwrap();
    assert!(route
        .points
        .iter()
        .skip(1)
        .take(route.points.len().saturating_sub(2))
        .any(|position| position[2] != FP::ZERO));
}

#[test]
fn generation_is_deterministic_and_round_trips_canonically() {
    let mut layout = grid(4, 3);
    layout.excluded_cells = (1 << 3) | (1 << 8);
    let first = create(layout, profile()).unwrap();
    let second = create(layout, profile()).unwrap();
    assert_eq!(first.to_bytes(), second.to_bytes());
    assert_eq!(first.revision(), second.revision());
    assert_eq!(
        StaticMeshGraph::from_bytes(&first.to_bytes()).unwrap(),
        first
    );
}

#[test]
fn one_cell_and_fractional_dimensions_preserve_exact_outer_endpoints() {
    let one_cell = create(grid(1, 1), profile()).unwrap();
    assert_eq!(one_cell.triangles().len(), 2);

    let fractional = Grid {
        origin: [FP::ZERO; 3],
        width: fp!(1),
        depth: fp!(1),
        columns: 3,
        rows: 2,
        excluded_cells: 0,
    };
    let graph = create(fractional, profile()).unwrap();
    assert!(graph.vertices().iter().any(|point| point[0] == fp!(1)));
    assert!(graph.vertices().iter().any(|point| point[2] == fp!(1)));
    assert!(graph
        .vertices()
        .iter()
        .all(|point| (0..=FP::SCALE).contains(&point[0].raw())
            && (0..=FP::SCALE).contains(&point[2].raw())));
}

#[test]
fn maximum_grid_stays_within_asset_bounds_and_preserves_plane_height() {
    let maximum = Grid {
        origin: [fp!(-7), fp!(2), fp!(-7)],
        width: fp!(14),
        depth: fp!(14),
        columns: 7,
        rows: 7,
        excluded_cells: 0,
    };
    let graph = create(maximum, profile()).unwrap();
    assert_eq!(graph.triangles().len(), 98);
    assert!(graph.to_bytes().len() <= 8192);
    assert!(graph
        .vertices()
        .iter()
        .all(|position| position[1] == fp!(2)));
}

#[test]
fn invalid_dimensions_masks_profiles_and_coordinates_fail_closed() {
    for invalid in [
        Grid {
            columns: 0,
            ..grid(1, 1)
        },
        Grid {
            rows: 8,
            ..grid(1, 1)
        },
        Grid {
            width: FP::ZERO,
            ..grid(1, 1)
        },
        Grid {
            width: FP::from_raw(1),
            columns: 2,
            ..grid(2, 1)
        },
        Grid {
            origin: [fp!(256), FP::ZERO, FP::ZERO],
            width: fp!(1),
            ..grid(1, 1)
        },
        Grid {
            excluded_cells: 1 << 1,
            ..grid(1, 1)
        },
        Grid {
            excluded_cells: 1,
            ..grid(1, 1)
        },
    ] {
        assert!(create(invalid, profile()).is_err());
    }

    assert!(create(
        grid(1, 1),
        AgentProfile {
            radius: fp!(1),
            ..profile()
        }
    )
    .is_err());
}
