use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile, NavigationError, SearchBudget};

fn whole_units(value: i64) -> FP {
    FP::from_raw(value * 65_536)
}

fn point(x_coordinate: i64, y_coordinate: i64, z_coordinate: i64) -> [FP; 3] {
    [
        whole_units(x_coordinate),
        whole_units(y_coordinate),
        whole_units(z_coordinate),
    ]
}

fn raw_point(x_coordinate_raw: i64, y_coordinate_raw: i64, z_coordinate_raw: i64) -> [FP; 3] {
    [
        FP::from_raw(x_coordinate_raw),
        FP::from_raw(y_coordinate_raw),
        FP::from_raw(z_coordinate_raw),
    ]
}

fn floor() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(1, 0, 0),
            point(0, 0, 1),
            point(1, 0, 1),
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap()
}

#[test]
fn canonical_welding_and_input_permutations_have_one_identity() {
    let canonical = floor();
    let permuted = StaticMeshGraph::bake(
        &[
            point(1, 0, 1),
            point(1, 0, 0),
            point(0, 0, 0),
            point(0, 0, 1),
        ],
        &[[2, 0, 1], [2, 3, 0]],
        AgentProfile::default(),
    )
    .unwrap();
    let duplicate_vertices = StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(0, 0, 0),
            point(0, 0, 1),
            point(1, 0, 0),
            point(1, 0, 1),
            point(0, 0, 1),
        ],
        &[[0, 2, 4], [1, 4, 3]],
        AgentProfile::default(),
    )
    .unwrap();

    assert_eq!(canonical.vertices(), permuted.vertices());
    assert_eq!(canonical.triangles(), permuted.triangles());
    assert_eq!(canonical.revision(), permuted.revision());
    assert_eq!(canonical.to_bytes(), permuted.to_bytes());
    assert_eq!(canonical.vertices(), duplicate_vertices.vertices());
    assert_eq!(canonical.triangles(), duplicate_vertices.triangles());
    assert_eq!(canonical.revision(), duplicate_vertices.revision());
}

#[test]
fn arbitrary_ramp_layers_and_vertical_projection_are_deterministic() {
    let ramp = StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(1, 1, 0),
            point(1, 1, 1),
            point(2, 1, 0),
        ],
        &[[0, 2, 1], [1, 2, 3]],
        AgentProfile::default(),
    )
    .unwrap();
    let projection = ramp.project(point(0, 0, 0), FP::ZERO).unwrap();
    assert_eq!(projection.position, point(0, 0, 0));
    assert_eq!(ramp.triangles().len(), 2);
    assert_eq!(
        ramp.find_path(
            point(0, 0, 0),
            point(2, 1, 0),
            FP::ZERO,
            SearchBudget::default()
        )
        .unwrap()
        .corridor
        .len(),
        2
    );

    let layered = StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(0, 0, 1),
            point(1, 0, 0),
            point(0, 2, 0),
            point(0, 2, 1),
            point(1, 2, 0),
        ],
        &[[0, 1, 2], [3, 4, 5]],
        AgentProfile {
            max_slope: FP::ZERO,
            ..AgentProfile::default()
        },
    )
    .unwrap();
    let query = point(0, 1, 0);
    let first = layered.project(query, whole_units(1)).unwrap();
    for _ in 0..8 {
        assert_eq!(layered.project(query, whole_units(1)).unwrap(), first);
    }
    assert!(layered.project(query, whole_units(0)).is_err());
    assert!(layered.project(query, whole_units(2)).is_ok());
}

#[test]
fn reversed_degenerate_duplicate_and_nonmanifold_faces_are_rejected() {
    let vertices = [
        point(0, 0, 0),
        point(1, 0, 0),
        point(0, 0, 1),
        point(1, 0, 1),
    ];
    assert!(StaticMeshGraph::bake(&vertices, &[[0, 3, 2]], AgentProfile::default()).is_err());
    assert!(StaticMeshGraph::bake(&vertices, &[[0, 0, 2]], AgentProfile::default()).is_err());
    assert!(
        StaticMeshGraph::bake(&vertices, &[[0, 2, 3], [0, 2, 3]], AgentProfile::default()).is_err()
    );
    assert!(StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(1, 0, 0),
            point(0, 0, 1),
            point(0, 0, -1),
            point(0, 0, 2),
        ],
        &[[0, 2, 1], [0, 1, 3], [0, 4, 1]],
        AgentProfile::default()
    )
    .is_err());
}

#[test]
fn full_edge_portals_connect_but_corner_contacts_do_not() {
    let connected = floor();
    assert_eq!(connected.triangles().len(), 2);
    assert!(connected
        .triangles()
        .iter()
        .all(|triangle| triangle.portals.len() == 1));
    let start = point(0, 0, 1);
    let goal = point(1, 0, 0);
    let path = connected
        .find_path(start, goal, FP::ZERO, SearchBudget::default())
        .unwrap();
    connected.validate_path(&path).unwrap();
    assert_eq!(path.corridor.len(), 2);

    assert_eq!(
        connected.find_path(start, goal, FP::ZERO, SearchBudget { max_expansions: 1 }),
        Err(NavigationError::BudgetExceeded { expanded: 1 })
    );

    let corner_only = StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(1, 0, 0),
            point(0, 0, 1),
            point(1, 0, 1),
            point(2, 0, 2),
        ],
        &[[0, 2, 1], [2, 4, 3]],
        AgentProfile::default(),
    )
    .unwrap();
    assert!(corner_only
        .triangles()
        .iter()
        .all(|triangle| triangle.portals.is_empty()));
    assert_eq!(
        corner_only.find_path(
            point(0, 0, 0),
            point(2, 0, 2),
            FP::ZERO,
            SearchBudget::default()
        ),
        Err(NavigationError::Unreachable)
    );
}

#[test]
fn primitive_raw_shared_edge_uses_an_endpoint_as_its_lattice_portal() {
    let graph = StaticMeshGraph::bake(
        &[
            [FP::ZERO; 3],
            [FP::from_raw(1), FP::ZERO, FP::from_raw(1)],
            [FP::ZERO, FP::ZERO, FP::from_raw(1)],
            [FP::from_raw(1), FP::ZERO, FP::ZERO],
        ],
        &[[0, 2, 1], [0, 1, 3]],
        AgentProfile::default(),
    )
    .unwrap();
    assert_eq!(graph.triangles().len(), 2);
    let portal = &graph.triangles()[0].portals[0];
    assert_eq!(portal.position, [FP::ZERO; 3]);
    assert_eq!(graph.triangles()[1].portals[0].position, portal.position);
}

#[test]
fn negative_coordinates_remain_projectable() {
    let graph = StaticMeshGraph::bake(
        &[point(-1, 0, -1), point(-1, 0, 0), point(0, 0, -1)],
        &[[0, 1, 2]],
        AgentProfile::default(),
    )
    .unwrap();
    assert_eq!(
        graph.project(point(-1, 0, -1), FP::ZERO).unwrap().position,
        point(-1, 0, -1)
    );
}

#[test]
fn adjacent_raw_triangles_with_coincident_centroids_remain_connected_and_budgeted() {
    let graph = StaticMeshGraph::bake(
        &[
            raw_point(0, 0, 0),
            raw_point(2, 0, 1),
            raw_point(1, 0, -1),
            raw_point(1, 0, 1),
        ],
        &[[0, 1, 2], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let centers: Vec<[i64; 3]> = graph
        .triangles()
        .iter()
        .map(|triangle| {
            std::array::from_fn(|axis| {
                triangle
                    .vertices
                    .iter()
                    .map(|index| graph.vertices()[*index as usize][axis].raw())
                    .sum::<i64>()
                    .div_euclid(3)
            })
        })
        .collect();
    assert_eq!(centers, vec![[1, 0, 0]; 2]);
    let start = raw_point(1, 0, -1);
    let goal = raw_point(1, 0, 1);
    assert_eq!(
        graph.find_path(start, goal, FP::ZERO, SearchBudget { max_expansions: 1 }),
        Err(NavigationError::BudgetExceeded { expanded: 1 })
    );
    let route = graph
        .find_path(start, goal, FP::ZERO, SearchBudget { max_expansions: 2 })
        .unwrap();
    assert_eq!(route.corridor.len(), 2);
    graph.validate_path(&route).unwrap();
}

#[test]
fn projection_search_and_path_distance_stay_on_mesh_and_reject_forgery() {
    let graph = floor();
    let origin = point(0, 0, 0);
    let start = point(0, 0, 1);
    let goal = point(1, 0, 0);
    let projected = graph.project(point(0, 2, 0), whole_units(2)).unwrap();
    assert_eq!(projected.position, origin);
    assert!(graph.project(point(0, 2, 0), whole_units(1)).is_err());
    assert_eq!(
        graph.find_path(start, goal, FP::ZERO, SearchBudget { max_expansions: 0 }),
        Err(NavigationError::BudgetExceeded { expanded: 0 })
    );

    let path = graph
        .find_path(start, goal, FP::ZERO, SearchBudget::default())
        .unwrap();
    let length = graph.path_length(&path).unwrap();
    assert!(length > FP::ZERO);
    assert_eq!(graph.point_at_distance(&path, FP::ZERO).unwrap(), start);
    assert_eq!(graph.point_at_distance(&path, length).unwrap(), goal);
    assert_eq!(
        graph.point_at_distance(&path, whole_units(100)).unwrap(),
        goal
    );
    assert!(graph.point_at_distance(&path, FP::from_raw(-1)).is_err());
    let mut distance = FP::ZERO;
    while distance < length {
        let point = graph.point_at_distance(&path, distance).unwrap();
        let projection = graph.project(point, FP::ZERO).unwrap();
        assert_eq!(projection.position, point);
        distance += FP::from_raw(4_096);
    }
    graph.validate_path(&path).unwrap();

    let mut forged = path.clone();
    forged.points[0] = point(3, 0, 3);
    assert!(graph.validate_path(&forged).is_err());
    let mut stale = path;
    stale.revision[0] ^= 1;
    assert!(graph.validate_path(&stale).is_err());
}

#[test]
fn same_point_paths_are_zero_length_and_clamp_distances_after_the_goal() {
    let graph = floor();
    let position = point(0, 0, 1);
    let path = graph
        .find_path(position, position, FP::ZERO, SearchBudget::default())
        .unwrap();
    assert_eq!(graph.path_length(&path).unwrap(), FP::ZERO);
    assert_eq!(graph.point_at_distance(&path, FP::ZERO).unwrap(), position);
    assert_eq!(
        graph.point_at_distance(&path, whole_units(1)).unwrap(),
        position
    );
}

#[test]
fn negative_fractional_plane_height_rounds_down_to_the_raw_lattice() {
    let graph = StaticMeshGraph::bake(
        &[
            raw_point(0, 0, 0),
            raw_point(0, -1, 65_536),
            raw_point(65_536, 0, 0),
        ],
        &[[0, 1, 2]],
        AgentProfile::default(),
    )
    .unwrap();
    let projection = graph
        .project(raw_point(32_768, 0, 32_768), FP::from_raw(1))
        .unwrap();
    assert_eq!(projection.position, raw_point(32_768, -1, 32_768));
}

#[test]
fn exact_slope_and_raw_coordinate_edge_limits_are_inclusive() {
    let exact_slope = StaticMeshGraph::bake(
        &[point(0, 0, 0), point(0, 0, 1), point(1, 4, 0)],
        &[[0, 1, 2]],
        AgentProfile {
            max_slope: whole_units(4),
            ..AgentProfile::default()
        },
    )
    .unwrap();
    assert_eq!(exact_slope.triangles().len(), 1);
    let above_slope = StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(0, 0, 1),
            raw_point(65_536, 4 * 65_536 + 1, 0),
        ],
        &[[0, 1, 2]],
        AgentProfile {
            max_slope: whole_units(4),
            ..AgentProfile::default()
        },
    )
    .unwrap();
    assert!(above_slope.triangles().is_empty());

    let coordinate_limit_raw = 1_i64 << 40;
    for boundary_vertices in [
        [
            raw_point(coordinate_limit_raw, 0, coordinate_limit_raw),
            raw_point(coordinate_limit_raw, 0, coordinate_limit_raw - 1),
            raw_point(coordinate_limit_raw - 1, 0, coordinate_limit_raw),
        ],
        [
            raw_point(-coordinate_limit_raw, 0, -coordinate_limit_raw),
            raw_point(-coordinate_limit_raw, 0, -coordinate_limit_raw + 1),
            raw_point(-coordinate_limit_raw + 1, 0, -coordinate_limit_raw),
        ],
    ] {
        let boundary =
            StaticMeshGraph::bake(&boundary_vertices, &[[0, 1, 2]], AgentProfile::default())
                .unwrap();
        assert_eq!(boundary.triangles().len(), 1);
    }

    let edge_limit_raw = 1_i64 << 22;
    let edge_limit = StaticMeshGraph::bake(
        &[
            raw_point(0, 0, 0),
            raw_point(0, 0, edge_limit_raw),
            raw_point(edge_limit_raw, 0, 0),
        ],
        &[[0, 1, 2]],
        AgentProfile::default(),
    )
    .unwrap();
    assert_eq!(edge_limit.triangles().len(), 1);
    let edge_over_limit = [
        raw_point(0, 0, 0),
        raw_point(0, 0, edge_limit_raw),
        raw_point(edge_limit_raw + 1, 0, 0),
    ];
    assert!(
        StaticMeshGraph::bake(&edge_over_limit, &[[0, 1, 2]], AgentProfile::default()).is_err()
    );
}

#[test]
fn filtered_slope_keeps_a_stable_key_gap() {
    let graph = StaticMeshGraph::bake(
        &[
            point(-3, 0, 0),
            point(-3, 0, 1),
            point(-2, 2, 0),
            point(3, 0, 0),
            point(3, 0, 1),
            point(4, 0, 0),
        ],
        &[[0, 1, 2], [3, 4, 5]],
        AgentProfile::default(),
    )
    .unwrap();
    assert_eq!(graph.triangles().len(), 1);
    assert_eq!(graph.triangles()[0].key, 1);
    assert_eq!(graph.project(point(3, 0, 0), FP::ZERO).unwrap().triangle, 1);
}

#[test]
fn skinny_oblique_triangle_distance_samples_remain_inside_its_hull() {
    let graph = StaticMeshGraph::bake(
        &[
            raw_point(0, 0, 0),
            raw_point(65_536, 0, 1),
            raw_point(65_536, 0, 0),
        ],
        &[[0, 1, 2]],
        AgentProfile::default(),
    )
    .unwrap();
    let start = raw_point(0, 0, 0);
    let goal = raw_point(65_536, 0, 1);
    let path = graph
        .find_path(start, goal, FP::ZERO, SearchBudget::default())
        .unwrap();
    let length = graph.path_length(&path).unwrap();
    let mut distance = FP::ZERO;
    while distance < length {
        let position = graph.point_at_distance(&path, distance).unwrap();
        assert_eq!(
            graph.project(position, FP::ZERO).unwrap().position,
            position
        );
        distance += FP::from_raw(1_024);
    }
    assert_eq!(graph.point_at_distance(&path, length).unwrap(), goal);
}

#[test]
fn canonical_asset_roundtrips_and_rejects_mutated_or_noncanonical_bytes() {
    let graph = floor();
    let bytes = graph.to_bytes();
    let loaded = StaticMeshGraph::from_bytes(&bytes).unwrap();
    assert_eq!(loaded.to_bytes(), bytes);
    assert_eq!(loaded.revision(), graph.revision());
    assert_eq!(loaded.vertices(), graph.vertices());
    assert_eq!(loaded.triangles(), graph.triangles());
    let replayed = loaded
        .find_path(
            point(0, 0, 1),
            point(1, 0, 0),
            FP::ZERO,
            SearchBudget::default(),
        )
        .unwrap();
    let original = graph
        .find_path(
            point(0, 0, 1),
            point(1, 0, 0),
            FP::ZERO,
            SearchBudget::default(),
        )
        .unwrap();
    assert_eq!(replayed, original);

    let mut noncanonical = StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(1, 0, 0),
            point(0, 0, 1),
            point(1, 0, 1),
            point(3, 0, 3),
            point(4, 0, 4),
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap()
    .to_bytes();
    let first_unused = 44 + 4 * 24;
    let second_unused = first_unused + 24;
    for byte_index in 0..24 {
        noncanonical.swap(first_unused + byte_index, second_unused + byte_index);
    }
    assert!(StaticMeshGraph::from_bytes(&noncanonical).is_err());

    let mut valid_geometry_change = graph.to_bytes();
    for vertex_index in 0..4 {
        let y_offset = 44 + vertex_index * 24 + 8;
        valid_geometry_change[y_offset..y_offset + 8].copy_from_slice(&65_536_i64.to_le_bytes());
    }
    let changed_graph = StaticMeshGraph::from_bytes(&valid_geometry_change).unwrap();
    assert_ne!(changed_graph.revision(), graph.revision());

    assert!(StaticMeshGraph::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    let mut bad_magic = bytes.clone();
    bad_magic[0] ^= 1;
    assert!(StaticMeshGraph::from_bytes(&bad_magic).is_err());
    let mut changed = bytes.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    assert!(StaticMeshGraph::from_bytes(&changed).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(StaticMeshGraph::from_bytes(&trailing).is_err());
    let mut forged_vertex_count = graph.to_bytes();
    forged_vertex_count[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(StaticMeshGraph::from_bytes(&forged_vertex_count).is_err());
    let mut forged_face_count = graph.to_bytes();
    forged_face_count[140..144].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(StaticMeshGraph::from_bytes(&forged_face_count).is_err());
    assert!(StaticMeshGraph::from_bytes(&vec![0; 2_200_001]).is_err());
}

#[test]
fn empty_mesh_has_a_canonical_roundtrip() {
    let graph = StaticMeshGraph::bake(&[], &[], AgentProfile::default()).unwrap();
    assert!(graph.vertices().is_empty());
    assert!(graph.triangles().is_empty());
    let loaded = StaticMeshGraph::from_bytes(&graph.to_bytes()).unwrap();
    assert_eq!(loaded, graph);
}

#[test]
fn paths_are_bound_to_geometry_and_profile_revisions() {
    let graph = floor();
    let path = graph
        .find_path(
            point(0, 0, 1),
            point(1, 0, 0),
            FP::ZERO,
            SearchBudget::default(),
        )
        .unwrap();
    let shifted_vertices = [
        point(0, 1, 0),
        point(1, 1, 0),
        point(0, 1, 1),
        point(1, 1, 1),
    ];
    let shifted = StaticMeshGraph::bake(
        &shifted_vertices,
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let different_profile = StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(1, 0, 0),
            point(0, 0, 1),
            point(1, 0, 1),
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile {
            max_slope: FP::from_raw(1),
            ..AgentProfile::default()
        },
    )
    .unwrap();
    assert!(shifted.validate_path(&path).is_err());
    assert!(different_profile.validate_path(&path).is_err());
}

#[test]
fn bounds_and_open_sky_profile_are_enforced() {
    let vertices = [point(0, 0, 0), point(0, 0, 1), point(1, 0, 0)];
    for unsupported in [
        AgentProfile {
            radius: FP::ONE,
            ..AgentProfile::default()
        },
        AgentProfile {
            headroom: FP::ONE,
            ..AgentProfile::default()
        },
        AgentProfile {
            max_step: FP::ONE,
            ..AgentProfile::default()
        },
        AgentProfile {
            max_slope: fp!(4.0001),
            ..AgentProfile::default()
        },
        AgentProfile {
            max_slope: FP::from_raw(-1),
            ..AgentProfile::default()
        },
    ] {
        assert!(StaticMeshGraph::bake(&vertices, &[[0, 1, 2]], unsupported).is_err());
    }
    let too_far = [
        point(0, 0, 0),
        point(0, 0, 1),
        [FP::from_raw((1_i64 << 40) + 1), FP::ZERO, FP::ZERO],
    ];
    assert!(StaticMeshGraph::bake(&too_far, &[[0, 1, 2]], AgentProfile::default()).is_err());
    let edge_too_long = [
        point(0, 0, 0),
        point(0, 0, 1),
        [FP::from_raw(1 << 23), FP::ZERO, FP::ZERO],
    ];
    assert!(StaticMeshGraph::bake(&edge_too_long, &[[0, 1, 2]], AgentProfile::default()).is_err());
}
