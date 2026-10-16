//! Independent integer geometry and graph oracles. These tests deliberately
//! derive topology from vertex intersections and slope from oriented plane
//! coefficients, rather than calling the mesh's internal geometry helpers.
use orr_fp::FP;
use orr_navigation::{
    AgentProfile, NavigationError, NavigationStatus, Navigator, SearchBudget, TerrainGraph,
    TriangleKey, MAX_COORDINATE_RAW, MAX_FILE_BYTES, MAX_SLOPE_RAW, MAX_SPACING_RAW,
    MIN_SPACING_RAW,
};
use orr_terrain::Terrain;
use sha2::{Digest, Sha256};

fn raw(value: i64) -> FP {
    FP::from_raw(value)
}

fn terrain(width: u32, depth: u32, spacing: i64, heights: &[i64], holes: &[bool]) -> Terrain {
    Terrain::new(
        "oracle/terrain".into(),
        width,
        depth,
        [FP::ZERO; 2],
        raw(spacing),
        heights.iter().copied().map(raw).collect(),
        holes.to_vec(),
    )
    .unwrap()
}

#[derive(Clone, Debug)]
struct OracleTriangle {
    id: u32,
    vertices: [u32; 3],
    points: [[i128; 3]; 3],
}

// Enumerate cells independently. IDs retain two slots for each source cell,
// including holes and rejected triangles.
fn triangles(source: &Terrain) -> Vec<OracleTriangle> {
    let mut result = Vec::new();
    for cell in 0..(source.width() - 1) * (source.depth() - 1) {
        if source.holes()[cell as usize] {
            continue;
        }
        let x = cell % (source.width() - 1);
        let z = cell / (source.width() - 1);
        let a = z * source.width() + x;
        for (local, vertices) in [
            [a, a + source.width(), a + source.width() + 1],
            [a, a + source.width() + 1, a + 1],
        ]
        .into_iter()
        .enumerate()
        {
            result.push(OracleTriangle {
                id: cell * 2 + local as u32,
                vertices,
                points: vertices.map(|i| {
                    let x = i % source.width();
                    let z = i / source.width();
                    [
                        i128::from(source.origin()[0].raw())
                            + i128::from(x) * i128::from(source.spacing().raw()),
                        i128::from(source.heights()[i as usize].raw()),
                        i128::from(source.origin()[1].raw())
                            + i128::from(z) * i128::from(source.spacing().raw()),
                    ]
                }),
            });
        }
    }
    result
}

fn plane_normal(triangle: &OracleTriangle) -> [i128; 3] {
    let [a, b, c] = triangle.points;
    let u = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
    let v = std::array::from_fn::<_, 3, _>(|i| c[i] - a[i]);
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

// Fixtures passed here have deliberately small coefficients, so all products
// fit i128. Compare the exact rational gradient without sqrt or FP rounding.
fn slope_accepts(triangle: &OracleTriangle, max_slope: i64) -> bool {
    let [x, y, z] = plane_normal(triangle);
    (x * x + z * z) * 65536 * 65536 <= i128::from(max_slope) * i128::from(max_slope) * y * y
}

fn shared_edge(a: &OracleTriangle, b: &OracleTriangle) -> bool {
    a.vertices
        .iter()
        .filter(|vertex| b.vertices.contains(vertex))
        .count()
        == 2
}

fn area(a: [i128; 2], b: [i128; 2], c: [i128; 2]) -> i128 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn triangle_height(triangle: &OracleTriangle, point: [FP; 2]) -> Option<FP> {
    let flat = triangle.points.map(|v| [v[0], v[2]]);
    let q = point.map(|v| i128::from(v.raw()));
    let total = area(flat[0], flat[1], flat[2]);
    let weights = [
        area(q, flat[1], flat[2]),
        area(flat[0], q, flat[2]),
        area(flat[0], flat[1], q),
    ];
    if weights
        .iter()
        .any(|w| *w != 0 && w.signum() != total.signum())
    {
        return None;
    }
    let numerator: i128 = triangle
        .points
        .iter()
        .zip(weights)
        .map(|(p, w)| p[1] * w)
        .sum();
    let (numerator, total) = if total < 0 {
        (-numerator, -total)
    } else {
        (numerator, total)
    };
    Some(raw(numerator.div_euclid(total) as i64))
}

// Locate the owning cell with half-open boxes, then use oriented areas to
// locate its triangle. Never fall back to a neighbour across a hole or slope
// rejection. The source's canonical first triangle owns its diagonal.
fn canonical_projection(source: &Terrain, max_slope: i64, point: [FP; 2]) -> Option<(u32, FP)> {
    let all = triangles(source);
    for z in 0..source.depth() - 1 {
        for x in 0..source.width() - 1 {
            let min = [
                i128::from(source.origin()[0].raw())
                    + i128::from(x) * i128::from(source.spacing().raw()),
                i128::from(source.origin()[1].raw())
                    + i128::from(z) * i128::from(source.spacing().raw()),
            ];
            let max = min.map(|v| v + i128::from(source.spacing().raw()));
            let q = point.map(|v| i128::from(v.raw()));
            let last = [x == source.width() - 2, z == source.depth() - 2];
            if (0..2).any(|i| q[i] < min[i] || q[i] > max[i] || (!last[i] && q[i] == max[i])) {
                continue;
            }
            let first = 2 * (z * (source.width() - 1) + x);
            for id in first..first + 2 {
                let triangle = all.iter().find(|triangle| triangle.id == id)?;
                if let Some(height) = triangle_height(triangle, point) {
                    return slope_accepts(triangle, max_slope).then_some((id, height));
                }
            }
            return None;
        }
    }
    None
}

fn profile(max_slope: i64) -> AgentProfile {
    AgentProfile {
        max_slope: raw(max_slope),
        ..AgentProfile::default()
    }
}

fn centroid(triangle: &OracleTriangle) -> [FP; 3] {
    std::array::from_fn(|axis| {
        raw(triangle
            .points
            .iter()
            .map(|p| p[axis])
            .sum::<i128>()
            .div_euclid(3) as i64)
    })
}

// Bisection over the integer distance, independent of the production Newton
// square-root implementation. The L1 norm is an exact upper bound.
fn distance(a: [FP; 3], b: [FP; 3]) -> u64 {
    let delta = std::array::from_fn::<_, 3, _>(|axis| {
        i128::from(a[axis].raw()) - i128::from(b[axis].raw())
    });
    let squared: u128 = delta.iter().map(|v| (v * v) as u128).sum();
    let mut low = 0u128;
    let mut high: u128 = delta.iter().map(|v| v.unsigned_abs()).sum();
    while low < high {
        let middle = low + (high - low) / 2;
        if middle * middle >= squared {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    u64::try_from(low).unwrap()
}

fn endpoints(triangle: &OracleTriangle) -> [FP; 2] {
    let center = centroid(triangle);
    [center[0], center[2]]
}

fn floyd_warshall(nodes: &[OracleTriangle]) -> Vec<Vec<Option<u64>>> {
    let n = nodes.len();
    let mut costs = vec![vec![None; n]; n];
    for i in 0..n {
        costs[i][i] = Some(0);
        for j in 0..n {
            if shared_edge(&nodes[i], &nodes[j]) {
                costs[i][j] = Some(distance(centroid(&nodes[i]), centroid(&nodes[j])));
            }
        }
    }
    for intermediate in 0..n {
        for from in 0..n {
            for to in 0..n {
                if let (Some(a), Some(b)) = (costs[from][intermediate], costs[intermediate][to]) {
                    let candidate = a + b;
                    if costs[from][to].is_none_or(|old| candidate < old) {
                        costs[from][to] = Some(candidate);
                    }
                }
            }
        }
    }
    costs
}

#[test]
fn exact_plane_slope_admission_exhaustive_small_heights() {
    // Normal coefficients stay small enough for the independent i128 oracle.
    for spacing in [17, 65536, 196608] {
        for code in 0..81 {
            let mut digits = code;
            let heights: Vec<i64> = (0..4)
                .map(|_| {
                    let height = (digits % 3 - 1) * spacing;
                    digits /= 3;
                    height
                })
                .collect();
            let source = terrain(2, 2, spacing, &heights, &[false]);
            let nodes = triangles(&source);
            for limit in [0, 21845, 65535, 65536, 92681, 92682, 131072] {
                let graph = TerrainGraph::build(&source, profile(limit)).unwrap();
                for node in &nodes {
                    assert_eq!(
                        graph.triangle(TriangleKey(node.id)).is_some(),
                        slope_accepts(node, limit),
                        "spacing={spacing}, heights={heights:?}, slope={limit}, key={}",
                        node.id
                    );
                }
            }
        }
    }
    // Diagnostic surface slope truncation must never admit an actually steeper
    // plane. This fixture also exposes exact sqrt(2) vs its rounded-down FP value.
    let square_root_boundary = terrain(2, 2, 65536, &[0, 65536, 65536, 131072], &[false]);
    assert_eq!(
        square_root_boundary.surface(raw(0), raw(0)).unwrap().slope,
        Some(raw(92681))
    );
    assert!(TerrainGraph::build(&square_root_boundary, profile(92681))
        .unwrap()
        .triangles()
        .is_empty());
    assert_eq!(
        TerrainGraph::build(&square_root_boundary, profile(92682))
            .unwrap()
            .triangles()
            .len(),
        2
    );
    let rational_boundary = terrain(2, 2, 196608, &[0, 65536, 0, 65536], &[false]);
    assert_eq!(
        rational_boundary.surface(raw(0), raw(0)).unwrap().slope,
        Some(raw(21845))
    );
    assert!(TerrainGraph::build(&rational_boundary, profile(21845))
        .unwrap()
        .triangles()
        .is_empty());
    assert_eq!(
        TerrainGraph::build(&rational_boundary, profile(21846))
            .unwrap()
            .triangles()
            .len(),
        2
    );
    let tiny_boundary = terrain(2, 2, 17, &[0, 1, 0, 1], &[false]);
    assert_eq!(
        tiny_boundary.surface(raw(0), raw(0)).unwrap().slope,
        Some(raw(3855))
    );
    assert!(TerrainGraph::build(&tiny_boundary, profile(3855))
        .unwrap()
        .triangles()
        .is_empty());
    assert_eq!(
        TerrainGraph::build(&tiny_boundary, profile(3856))
            .unwrap()
            .triangles()
            .len(),
        2
    );
}

#[test]
fn all_hole_masks_preserve_source_keys_and_only_reciprocal_shared_edges() {
    for mask in 0u32..16 {
        let holes: Vec<bool> = (0..4).map(|cell| mask & (1 << cell) != 0).collect();
        let source = terrain(3, 3, 65536, &[0; 9], &holes);
        let graph = TerrainGraph::build(&source, profile(0)).unwrap();
        let expected = triangles(&source);
        assert_eq!(graph.triangles().len(), expected.len(), "hole mask {mask}");
        for node in &expected {
            let actual = graph.triangle(TriangleKey(node.id)).unwrap();
            assert_eq!(
                actual.vertices, node.vertices,
                "hole mask {mask}, key {}",
                node.id
            );
            let expected_neighbors: Vec<_> = expected
                .iter()
                .filter(|other| shared_edge(node, other))
                .map(|other| TriangleKey(other.id))
                .collect();
            assert_eq!(
                actual
                    .portals
                    .iter()
                    .map(|p| p.neighbor)
                    .collect::<Vec<_>>(),
                expected_neighbors,
                "hole mask {mask}, key {}",
                node.id
            );
            for portal in &actual.portals {
                let other = graph.triangle(portal.neighbor).unwrap();
                let mut shared: Vec<_> = node
                    .vertices
                    .iter()
                    .copied()
                    .filter(|v| other.vertices.contains(v))
                    .collect();
                shared.sort();
                assert_eq!(shared, portal.vertices);
                let reciprocal = other
                    .portals
                    .iter()
                    .find(|p| p.neighbor == actual.key)
                    .unwrap();
                assert_eq!(reciprocal.vertices, portal.vertices);
                assert_eq!(reciprocal.midpoint, portal.midpoint);
                let a = source.vertex_position(portal.vertices[0]).unwrap();
                let b = source.vertex_position(portal.vertices[1]).unwrap();
                for axis in [0, 2] {
                    assert_eq!(
                        portal.midpoint[axis].raw(),
                        (i128::from(a[axis].raw()) + i128::from(b[axis].raw())).div_euclid(2)
                            as i64
                    );
                }
                let projected =
                    canonical_projection(&source, 0, [portal.midpoint[0], portal.midpoint[2]])
                        .unwrap();
                assert_eq!(portal.midpoint[1], projected.1);
            }
        }
        for key in 0..8 {
            assert_eq!(
                graph.triangle(TriangleKey(key)).is_some(),
                !holes[key as usize / 2]
            );
        }
        assert!(graph.triangle(TriangleKey(u32::MAX)).is_none());
    }
}

#[test]
fn projection_matches_independent_owner_and_area_oracle_including_holes_and_slopes() {
    for mask in 0u32..16 {
        let holes: Vec<bool> = (0..4).map(|cell| mask & (1 << cell) != 0).collect();
        let source = terrain(
            3,
            3,
            65536,
            &[0, 65536, 0, 0, 65536, 131072, 0, 0, 0],
            &holes,
        );
        for slope in [0, 65536, 92682, 196608] {
            let graph = TerrainGraph::build(&source, profile(slope)).unwrap();
            for x in -1..=17 {
                for z in -1..=17 {
                    let point = [raw(x * 8192), raw(z * 8192)];
                    let expected = canonical_projection(&source, slope, point);
                    let actual = graph.project(&source, point);
                    match expected {
                        Some((id, height)) => {
                            let p = actual.unwrap();
                            assert_eq!(
                                p.triangle,
                                TriangleKey(id),
                                "hole mask {mask}, slope {slope}, point {point:?}"
                            );
                            assert_eq!(p.position, [point[0], height, point[1]]);
                        }
                        None => assert!(
                            matches!(
                                actual,
                                Err(NavigationError::OutsideTerrain | NavigationError::Unwalkable)
                            ),
                            "hole mask {mask}, slope {slope}, point {point:?}: {actual:?}"
                        ),
                    }
                }
            }
        }
    }
}

#[test]
fn all_pairs_corridor_costs_match_floyd_warshall_on_every_small_hole_graph() {
    for mask in 0u32..16 {
        let holes: Vec<bool> = (0..4).map(|cell| mask & (1 << cell) != 0).collect();
        // A shallow nonflat plane makes the independent cost oracle exercise Y.
        let heights: Vec<_> = (0..3)
            .flat_map(|z| (0..3).map(move |x| x * 8192 - z * 4096))
            .collect();
        let source = terrain(3, 3, 65536, &heights, &holes);
        let graph = TerrainGraph::build(&source, profile(65536)).unwrap();
        let nodes = triangles(&source);
        let costs = floyd_warshall(&nodes);
        for (from, start) in nodes.iter().enumerate() {
            for (to, goal) in nodes.iter().enumerate() {
                let result = graph.find_path(
                    &source,
                    endpoints(start),
                    endpoints(goal),
                    SearchBudget::default(),
                );
                let Some(expected_cost) = costs[from][to] else {
                    assert_eq!(
                        result,
                        Err(NavigationError::Unreachable),
                        "hole mask {mask}, {} -> {}",
                        start.id,
                        goal.id
                    );
                    continue;
                };
                let path = result.unwrap();
                assert_eq!(
                    path.cost().raw() as u64,
                    expected_cost,
                    "hole mask {mask}, {} -> {}",
                    start.id,
                    goal.id
                );
                assert_eq!(path.corridor().first(), Some(&TriangleKey(start.id)));
                assert_eq!(path.corridor().last(), Some(&TriangleKey(goal.id)));
                assert_eq!(path.portals().len() + 1, path.corridor().len());
                assert_eq!(path.waypoints().len(), path.corridor().len() + 1);
                assert_eq!(path.graph_revision(), graph.revision());
                assert_eq!(path.terrain_revision(), source.revision());
                let mut corridor_cost = 0;
                for pair in path.corridor().windows(2) {
                    let a = nodes.iter().find(|p| p.id == pair[0].0).unwrap();
                    let b = nodes.iter().find(|p| p.id == pair[1].0).unwrap();
                    assert!(shared_edge(a, b));
                    corridor_cost += distance(centroid(a), centroid(b));
                }
                assert_eq!(corridor_cost, expected_cost);
                // Endpoints in a closed convex source triangle prove geometric
                // containment. Boundary owner admission is checked independently.
                for (segment, key) in path.waypoints().windows(2).zip(path.corridor()) {
                    let owner = nodes.iter().find(|p| p.id == key.0).unwrap();
                    for point in segment {
                        let xz = [point[0], point[2]];
                        assert_eq!(triangle_height(owner, xz), Some(point[1]));
                        assert_eq!(
                            canonical_projection(&source, 65536, xz).unwrap().1,
                            point[1]
                        );
                    }
                    for fraction in 1..16 {
                        let point = std::array::from_fn(|axis| {
                            raw((i128::from(segment[0][axis * 2].raw()) * (16 - fraction)
                                + i128::from(segment[1][axis * 2].raw()) * fraction)
                                .div_euclid(16) as i64)
                        });
                        assert!(canonical_projection(&source, 65536, point).is_some());
                    }
                }
                assert_eq!(
                    graph
                        .find_path(
                            &source,
                            endpoints(start),
                            endpoints(goal),
                            SearchBudget::default()
                        )
                        .unwrap(),
                    path
                );
                let reopened = TerrainGraph::load(&graph.cook(), &source, graph.profile()).unwrap();
                assert_eq!(
                    reopened
                        .find_path(
                            &source,
                            endpoints(start),
                            endpoints(goal),
                            SearchBudget::default()
                        )
                        .unwrap(),
                    path
                );
            }
        }
    }
}

#[test]
fn stable_equal_cost_ties_complete_paths_and_budget_exhaustion_are_explicit() {
    let source = terrain(3, 3, 196608, &[0; 9], &[false; 4]);
    let graph = TerrainGraph::build(&source, profile(0)).unwrap();
    let nodes = triangles(&source);
    let start = endpoints(&nodes[0]);
    let goal = endpoints(&nodes[7]);
    let path = graph
        .find_path(&source, start, goal, SearchBudget { max_expansions: 8 })
        .unwrap();
    // The two three-edge arcs have exactly equal cost. Stable (cost,key)
    // settlement keeps the first predecessor discovered on each direction.
    assert_eq!(
        path.corridor(),
        &[
            TriangleKey(0),
            TriangleKey(1),
            TriangleKey(2),
            TriangleKey(7)
        ]
    );
    let reverse = graph
        .find_path(&source, goal, start, SearchBudget { max_expansions: 8 })
        .unwrap();
    assert_eq!(
        reverse.corridor(),
        &[
            TriangleKey(7),
            TriangleKey(6),
            TriangleKey(5),
            TriangleKey(0)
        ]
    );
    assert_eq!(path.cost(), reverse.cost());
    for max_expansions in 0..8 {
        assert_eq!(
            graph.find_path(&source, start, goal, SearchBudget { max_expansions }),
            Err(NavigationError::BudgetExceeded {
                expanded: max_expansions
            })
        );
    }
    assert_eq!(
        graph.find_path(&source, start, start, SearchBudget { max_expansions: 0 }),
        Err(NavigationError::BudgetExceeded { expanded: 0 })
    );
    let same = graph
        .find_path(&source, start, start, SearchBudget { max_expansions: 1 })
        .unwrap();
    assert_eq!(same.corridor(), &[TriangleKey(0)]);
    assert_eq!(same.cost(), FP::ZERO);
    assert_eq!(same.waypoints()[0], same.waypoints()[1]);

    let islands = terrain(3, 3, 65536, &[0; 9], &[false, true, true, false]);
    let island_graph = TerrainGraph::build(&islands, profile(0)).unwrap();
    let island_nodes = triangles(&islands);
    assert_eq!(
        island_nodes
            .iter()
            .find(|t| t.id == 1)
            .unwrap()
            .vertices
            .iter()
            .filter(|v| island_nodes
                .iter()
                .find(|t| t.id == 6)
                .unwrap()
                .vertices
                .contains(v))
            .count(),
        1
    );
    let a = endpoints(&island_nodes[0]);
    let b = endpoints(island_nodes.last().unwrap());
    assert_eq!(
        island_graph.find_path(&islands, a, b, SearchBudget { max_expansions: 1 }),
        Err(NavigationError::BudgetExceeded { expanded: 1 })
    );
    assert_eq!(
        island_graph.find_path(&islands, a, b, SearchBudget { max_expansions: 2 }),
        Err(NavigationError::Unreachable)
    );
    assert_eq!(
        island_graph.find_path(&islands, a, b, SearchBudget::default()),
        Err(NavigationError::Unreachable)
    );
}

#[test]
fn movement_charges_full_3d_budget_across_waypoints_and_preserves_surface_height() {
    let heights: Vec<_> = (0..3)
        .flat_map(|z| (0..3).map(move |x| x * 8192 + z * 4096))
        .collect();
    let source = terrain(3, 3, 65536, &heights, &[false; 4]);
    let graph = TerrainGraph::build(&source, profile(65536)).unwrap();
    let nodes = triangles(&source);
    let start = endpoints(&nodes[0]);
    let goal = endpoints(&nodes[7]);
    let path = graph
        .find_path(&source, start, goal, SearchBudget::default())
        .unwrap();
    let mut navigator = Navigator::new(&graph, &source, start).unwrap();
    navigator
        .replan(&graph, &source, goal, SearchBudget::default())
        .unwrap();
    let initial = navigator.clone();
    let total: u64 = path
        .waypoints()
        .windows(2)
        .map(|p| distance(p[0], p[1]))
        .sum();
    let mut exact_budget = initial.clone();
    exact_budget
        .advance(&graph, &source, raw(total as i64))
        .unwrap();
    assert_eq!(exact_budget.status(), NavigationStatus::Arrived);
    assert_eq!(exact_budget.position(), *path.waypoints().last().unwrap());
    let mut insufficient = initial.clone();
    insufficient
        .advance(&graph, &source, raw(total as i64 - 1))
        .unwrap();
    assert_eq!(insufficient.status(), NavigationStatus::Moving);
    assert_ne!(insufficient.position(), exact_budget.position());

    let budgets = [0, 1, 2, 7, 1024, 4096, 16384, 65536];
    for tick in 0..128 {
        let before = navigator.clone();
        let budget = budgets[tick % budgets.len()];
        navigator.advance(&graph, &source, raw(budget)).unwrap();
        let mut duplicate = before.clone();
        duplicate.advance(&graph, &source, raw(budget)).unwrap();
        assert_eq!(navigator, duplicate, "tick {tick}");
        // Charge every waypoint reached during this call, not merely the direct
        // start/end chord which would hide overspending around a corner.
        let mut cursor = before.position();
        let mut spent = 0;
        for index in before.next_waypoint()..navigator.next_waypoint() {
            let waypoint = path.waypoints()[index as usize];
            spent += distance(cursor, waypoint);
            cursor = waypoint;
        }
        spent += distance(cursor, navigator.position());
        assert!(
            spent <= budget as u64,
            "tick {tick}, spent={spent}, budget={budget}"
        );
        let point = navigator.position();
        let expected = canonical_projection(&source, 65536, [point[0], point[2]]).unwrap();
        assert_eq!(point[1], expected.1, "tick {tick}");
        let segment = (navigator.next_waypoint() as usize)
            .saturating_sub(1)
            .min(path.corridor().len() - 1);
        let owner = nodes
            .iter()
            .find(|t| t.id == path.corridor()[segment].0)
            .unwrap();
        assert_eq!(triangle_height(owner, [point[0], point[2]]), Some(point[1]));
        if navigator.status() == NavigationStatus::Arrived {
            break;
        }
    }
    assert_eq!(navigator.status(), NavigationStatus::Arrived);
    assert_eq!(navigator.position(), exact_budget.position());
    let arrived = navigator.clone();
    navigator.advance(&graph, &source, raw(i64::MAX)).unwrap();
    assert_eq!(navigator, arrived);
}

#[test]
fn tiny_movement_budget_never_rounds_up_the_3d_ramp_distance() {
    let source = terrain(2, 2, 64, &[0, 64, 0, 64], &[false]);
    let graph = TerrainGraph::build(&source, profile(65536)).unwrap();
    let mut navigator = Navigator::new(&graph, &source, [raw(48), raw(16)]).unwrap();
    navigator
        .replan(&graph, &source, [raw(56), raw(16)], SearchBudget::default())
        .unwrap();
    let before = navigator.clone();
    navigator.advance(&graph, &source, raw(1)).unwrap();
    assert_eq!(navigator, before); // One lattice X step also rises one Y step.
    navigator.advance(&graph, &source, raw(2)).unwrap();
    assert_eq!(navigator.position(), [raw(49), raw(49), raw(16)]);
    assert_eq!(distance(before.position(), navigator.position()), 2);
    assert_eq!(navigator.status(), NavigationStatus::Moving);
    let before = navigator.clone();
    assert_eq!(
        navigator.advance(&graph, &source, raw(-1)),
        Err(NavigationError::InvalidDistance)
    );
    assert_eq!(navigator, before);
}

#[test]
fn failed_replans_stale_dependencies_and_stop_are_atomic() {
    let source = terrain(3, 3, 65536, &[0; 9], &[false; 4]);
    let graph = TerrainGraph::build(&source, profile(65536)).unwrap();
    let start = [raw(8192), raw(16384)];
    let goal = [raw(114688), raw(98304)];
    let mut navigator = Navigator::new(&graph, &source, start).unwrap();
    assert_eq!(navigator.status(), NavigationStatus::Stopped);
    navigator
        .replan(&graph, &source, goal, SearchBudget::default())
        .unwrap();
    navigator.advance(&graph, &source, raw(8192)).unwrap();
    let moving = navigator.clone();
    assert_eq!(
        navigator.replan(&graph, &source, goal, SearchBudget { max_expansions: 0 }),
        Err(NavigationError::BudgetExceeded { expanded: 0 })
    );
    assert_eq!(navigator, moving);
    assert_eq!(
        navigator.replan(&graph, &source, [raw(-1), raw(0)], SearchBudget::default()),
        Err(NavigationError::OutsideTerrain)
    );
    assert_eq!(navigator, moving);
    let changed = terrain(3, 3, 65536, &[65536; 9], &[false; 4]);
    assert_eq!(
        navigator.advance(&graph, &changed, raw(65536)),
        Err(NavigationError::StaleTerrain)
    );
    assert_eq!(navigator, moving);
    let different_profile = TerrainGraph::build(&source, profile(131072)).unwrap();
    assert_eq!(
        navigator.advance(&different_profile, &source, raw(65536)),
        Err(NavigationError::StaleGraph)
    );
    assert_eq!(navigator, moving);
    let rebuilt = TerrainGraph::build(&changed, profile(65536)).unwrap();
    navigator
        .replan(&rebuilt, &changed, goal, SearchBudget::default())
        .unwrap();
    assert_eq!(navigator.position()[1], raw(65536));
    let position = navigator.position();
    navigator.stop();
    assert_eq!(navigator.status(), NavigationStatus::Stopped);
    assert!(navigator.path().is_none());
    assert_eq!(navigator.next_waypoint(), 0);
    assert_eq!(navigator.position(), position);
    let stopped = navigator.clone();
    navigator
        .advance(&rebuilt, &changed, raw(i64::MAX))
        .unwrap();
    assert_eq!(navigator, stopped);
    assert_eq!(
        navigator.advance(&rebuilt, &source, FP::ZERO),
        Err(NavigationError::StaleTerrain)
    );
    assert_eq!(navigator, stopped);
}

#[test]
fn cooked_graph_rejects_every_truncation_byte_mutation_and_forged_counts() {
    let source = terrain(3, 3, 65536, &[0; 9], &[false, false, true, false]);
    let profile = profile(65536);
    let graph = TerrainGraph::build(&source, profile).unwrap();
    let bytes = graph.cook();
    let loaded = TerrainGraph::load(&bytes, &source, profile).unwrap();
    assert_eq!(loaded, graph);
    assert_eq!(loaded.cook(), bytes);
    assert_eq!(loaded.revision(), graph.revision());
    for length in 0..bytes.len() {
        assert!(
            TerrainGraph::load(&bytes[..length], &source, profile).is_err(),
            "prefix {length}"
        );
    }
    for index in 0..bytes.len() {
        let mut corrupt = bytes.clone();
        corrupt[index] ^= 1;
        assert!(
            TerrainGraph::load(&corrupt, &source, profile).is_err(),
            "modified byte {index}"
        );
    }
    let mut appended = bytes.clone();
    appended.push(0);
    assert_eq!(
        TerrainGraph::load(&appended, &source, profile),
        Err(NavigationError::MalformedAsset)
    );
    assert_eq!(
        TerrainGraph::load(&vec![0; MAX_FILE_BYTES + 1], &source, profile),
        Err(NavigationError::MalformedAsset)
    );
    let id_length = usize::from(u16::from_le_bytes([bytes[40], bytes[41]]));
    let vertex_count = 42 + id_length + 32;
    let triangle_count = vertex_count + 4 + source.heights().len() * 24;
    for offset in [
        vertex_count,
        triangle_count,
        triangle_count + 4,
        triangle_count + 8,
    ] {
        let mut corrupt = bytes.clone();
        corrupt[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            TerrainGraph::load(&corrupt, &source, profile),
            Err(NavigationError::MalformedAsset)
        );
    }
    let mut oversized_id = bytes.clone();
    oversized_id[40..42].copy_from_slice(&u16::MAX.to_le_bytes());
    assert_eq!(
        TerrainGraph::load(&oversized_id, &source, profile),
        Err(NavigationError::MalformedAsset)
    );
    let altered_terrain = terrain(3, 3, 65536, &[1; 9], &[false, false, true, false]);
    assert_eq!(
        TerrainGraph::load(&bytes, &altered_terrain, profile),
        Err(NavigationError::StaleTerrain)
    );
    assert_eq!(
        TerrainGraph::load(&bytes, &source, self::profile(131072)),
        Err(NavigationError::ProfileMismatch)
    );
    assert_eq!(graph.cook(), bytes);
}

#[test]
fn declared_numeric_bounds_and_unsupported_agent_dimensions_fail_closed() {
    for spacing in [1, MIN_SPACING_RAW - 1, MAX_SPACING_RAW + 1] {
        let source = terrain(2, 2, spacing, &[0; 4], &[false]);
        assert_eq!(
            TerrainGraph::build(&source, profile(0)),
            Err(NavigationError::NumericLimit)
        );
    }
    for spacing in [MIN_SPACING_RAW, MAX_SPACING_RAW] {
        let source = terrain(2, 2, spacing, &[0; 4], &[false]);
        let graph = TerrainGraph::build(&source, profile(0)).unwrap();
        assert_eq!(graph.triangles().len(), 2);
        assert_eq!(
            TerrainGraph::load(&graph.cook(), &source, profile(0)).unwrap(),
            graph
        );
    }
    for height in [
        i64::MIN,
        i64::MAX,
        -MAX_COORDINATE_RAW - 1,
        MAX_COORDINATE_RAW + 1,
    ] {
        let source = terrain(2, 2, MIN_SPACING_RAW, &[height; 4], &[false]);
        assert_eq!(
            TerrainGraph::build(&source, profile(0)),
            Err(NavigationError::NumericLimit)
        );
    }
    for coordinate in [-MAX_COORDINATE_RAW, MAX_COORDINATE_RAW - MIN_SPACING_RAW] {
        let source = Terrain::new(
            "oracle/terrain".into(),
            2,
            2,
            [raw(coordinate); 2],
            raw(MIN_SPACING_RAW),
            vec![raw(coordinate); 4],
            vec![false],
        )
        .unwrap();
        let graph = TerrainGraph::build(&source, profile(0)).unwrap();
        assert_eq!(
            graph
                .project(&source, [raw(coordinate); 2])
                .unwrap()
                .position,
            [raw(coordinate); 3]
        );
    }
    for coordinate in [-MAX_COORDINATE_RAW - 1, MAX_COORDINATE_RAW] {
        let source = Terrain::new(
            "oracle/terrain".into(),
            2,
            2,
            [raw(coordinate); 2],
            raw(MIN_SPACING_RAW),
            vec![FP::ZERO; 4],
            vec![false],
        )
        .unwrap();
        assert_eq!(
            TerrainGraph::build(&source, profile(0)),
            Err(NavigationError::NumericLimit)
        );
    }
    let extreme = Terrain::new(
        "oracle/extreme".into(),
        2,
        2,
        [raw(i64::MIN); 2],
        raw(i64::MAX),
        vec![raw(i64::MIN), raw(i64::MAX), raw(i64::MAX), raw(i64::MIN)],
        vec![false],
    )
    .unwrap();
    assert_eq!(
        TerrainGraph::build(&extreme, profile(0)),
        Err(NavigationError::NumericLimit)
    );
    let source = terrain(2, 2, 65536, &[0; 4], &[false]);
    for slope in [-1, MAX_SLOPE_RAW + 1] {
        assert_eq!(
            TerrainGraph::build(&source, profile(slope)),
            Err(NavigationError::NumericLimit)
        );
    }
    assert!(TerrainGraph::build(&source, profile(MAX_SLOPE_RAW)).is_ok());
    for dimension in 0..3 {
        for value in [-1, 1, 65536] {
            let mut unsupported = profile(65536);
            match dimension {
                0 => unsupported.radius = raw(value),
                1 => unsupported.headroom = raw(value),
                _ => unsupported.max_step = raw(value),
            }
            assert_eq!(
                TerrainGraph::build(&source, unsupported),
                Err(NavigationError::UnsupportedProfile)
            );
        }
    }
}

fn record_trace(bytes: &mut Vec<u8>, navigator: &Navigator) {
    for coordinate in navigator.position() {
        bytes.extend(coordinate.raw().to_le_bytes());
    }
    bytes.push(match navigator.status() {
        NavigationStatus::Stopped => 0,
        NavigationStatus::Moving => 1,
        NavigationStatus::Arrived => 2,
    });
    bytes.extend(navigator.next_waypoint().to_le_bytes());
    if let Some(path) = navigator.path() {
        bytes.push(1);
        bytes.extend(path.graph_revision());
        bytes.extend(path.terrain_revision());
        bytes.extend(path.cost().raw().to_le_bytes());
        bytes.extend((path.corridor().len() as u32).to_le_bytes());
        for key in path.corridor() {
            bytes.extend(key.0.to_le_bytes());
        }
        bytes.extend((path.waypoints().len() as u32).to_le_bytes());
        for point in path.waypoints() {
            for coordinate in point {
                bytes.extend(coordinate.raw().to_le_bytes());
            }
        }
    } else {
        bytes.push(0);
    }
}

fn replay_trace(source: &Terrain, graph: &TerrainGraph) -> (Vec<u8>, Vec<Navigator>, [u8; 32]) {
    let start = [raw(8192), raw(16384)];
    let goal = [raw(114688), raw(98304)];
    let mut navigator = Navigator::new(graph, source, start).unwrap();
    let mut bytes = b"orr_navigation independent oracle trace v1\0".to_vec();
    bytes.extend(graph.revision());
    bytes.extend(source.revision());
    let mut snapshots = vec![navigator.clone()];
    record_trace(&mut bytes, &navigator);
    navigator
        .replan(graph, source, goal, SearchBudget::default())
        .unwrap();
    snapshots.push(navigator.clone());
    record_trace(&mut bytes, &navigator);
    for budget in [0, 1, 17, 8192, 65536, 131072, i64::MAX] {
        navigator.advance(graph, source, raw(budget)).unwrap();
        snapshots.push(navigator.clone());
        record_trace(&mut bytes, &navigator);
    }
    assert_eq!(navigator.status(), NavigationStatus::Arrived);
    navigator.stop();
    snapshots.push(navigator.clone());
    record_trace(&mut bytes, &navigator);
    navigator
        .replan(graph, source, start, SearchBudget::default())
        .unwrap();
    snapshots.push(navigator.clone());
    record_trace(&mut bytes, &navigator);
    for budget in [4096, 16384, 65536, i64::MAX] {
        navigator.advance(graph, source, raw(budget)).unwrap();
        snapshots.push(navigator.clone());
        record_trace(&mut bytes, &navigator);
    }
    assert_eq!(navigator.status(), NavigationStatus::Arrived);
    let digest = Sha256::digest(&bytes).into();
    (bytes, snapshots, digest)
}

#[test]
fn replay_trace_sha256_and_full_state_match_after_terrain_and_graph_reload() {
    let source = terrain(
        3,
        3,
        65536,
        &[0, 8192, 16384, 4096, 12288, 20480, 8192, 16384, 24576],
        &[false; 4],
    );
    let graph = TerrainGraph::build(&source, profile(65536)).unwrap();
    let reopened_source = Terrain::load(&source.cook()).unwrap();
    let reopened_graph =
        TerrainGraph::load(&graph.cook(), &reopened_source, profile(65536)).unwrap();
    let first = replay_trace(&source, &graph);
    let repeated = replay_trace(&source, &graph);
    let reopened = replay_trace(&reopened_source, &reopened_graph);
    assert_eq!(first, repeated);
    assert_eq!(first, reopened);
    for (original, restored) in first.1.iter().zip(&reopened.1) {
        assert_eq!(original.checksum(), restored.checksum());
    }
    assert_ne!(first.1[0].checksum(), first.1[1].checksum());
    // Ensure the trace is exercising state transitions, not hashing a stationary
    // agent or an empty record. Check equality of full private state via Eq too.
    assert_eq!(first.1[0].status(), NavigationStatus::Stopped);
    assert_eq!(first.1[1].status(), NavigationStatus::Moving);
    assert_eq!(first.1[1], first.1[2]); // Explicit zero-distance tick.
    assert_eq!(first.1.last().unwrap().status(), NavigationStatus::Arrived);
    assert_ne!(first.1[0].position(), first.1[8].position());
    assert_eq!(first.1[0].position(), first.1.last().unwrap().position());
    assert_ne!(first.2, [0; 32]);
}
