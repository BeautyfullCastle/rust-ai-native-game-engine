use orr_fp::{fp, FP};
use orr_navigation::*;
use orr_terrain::{Edit, Terrain, TerrainDocument};

fn terrain(width: u32, depth: u32, heights: Vec<FP>, holes: Vec<bool>) -> Terrain {
    Terrain::new(
        "terrain/contracts".to_owned(),
        width,
        depth,
        [fp!(-2), fp!(-2)],
        FP::ONE,
        heights,
        holes,
    )
    .unwrap()
}
fn flat(width: u32, depth: u32) -> Terrain {
    terrain(
        width,
        depth,
        vec![FP::ZERO; (width * depth) as usize],
        vec![false; ((width - 1) * (depth - 1)) as usize],
    )
}
fn graph(t: &Terrain) -> TerrainGraph {
    TerrainGraph::build(t, AgentProfile::default()).unwrap()
}
fn xz(x: FP, z: FP) -> [FP; 2] {
    [x, z]
}

#[test]
fn stable_keys_vertex_ids_and_reciprocal_full_edges() {
    let mut holes = vec![false; 6];
    holes[1] = true;
    let t = terrain(4, 3, vec![FP::ZERO; 12], holes);
    let g = graph(&t);
    assert_eq!(g.vertices().len(), 12);
    assert_eq!(
        g.triangles().iter().map(|t| t.key.0).collect::<Vec<_>>(),
        vec![0, 1, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    for (id, pos) in g.vertices().iter().enumerate() {
        assert_eq!(Some(*pos), t.vertex_position(id as u32));
    }
    for triangle in g.triangles() {
        let mut previous = None;
        for p in &triangle.portals {
            assert!(previous.is_none_or(|old| old < p.neighbor));
            previous = Some(p.neighbor);
            assert!(p.vertices[0] < p.vertices[1]);
            let neighbor = g.triangle(p.neighbor).unwrap();
            assert!(p
                .vertices
                .iter()
                .all(|id| triangle.vertices.contains(id) && neighbor.vertices.contains(id)));
            let reverse = neighbor
                .portals
                .iter()
                .find(|p| p.neighbor == triangle.key)
                .unwrap();
            assert_eq!(reverse.vertices, p.vertices);
            assert_eq!(reverse.midpoint, p.midpoint);
        }
    }
}
#[test]
fn corner_contact_is_disconnected() {
    let t = terrain(3, 3, vec![FP::ZERO; 9], vec![false, true, true, false]);
    let g = graph(&t);
    assert_eq!(
        g.find_path(
            &t,
            xz(fp!(-1.75), fp!(-1.75)),
            xz(fp!(-0.25), fp!(-0.25)),
            SearchBudget::default()
        ),
        Err(NavigationError::Unreachable)
    );
}
#[test]
fn exact_slope_rejects_quantized_sqrt_two_and_third() {
    let t = terrain(2, 2, vec![fp!(0), fp!(1), fp!(1), fp!(2)], vec![false]);
    let truncated = AgentProfile {
        max_slope: FP::from_raw(92681),
        ..Default::default()
    };
    assert!(TerrainGraph::build(&t, truncated)
        .unwrap()
        .triangles()
        .is_empty());
    let raised = AgentProfile {
        max_slope: FP::from_raw(92682),
        ..Default::default()
    };
    assert_eq!(
        TerrainGraph::build(&t, raised).unwrap().triangles().len(),
        2
    );
    let third = Terrain::new(
        "terrain/third".into(),
        2,
        2,
        [FP::ZERO; 2],
        fp!(3),
        vec![FP::ZERO, FP::ONE, FP::ZERO, FP::ONE],
        vec![false],
    )
    .unwrap();
    assert!(TerrainGraph::build(
        &third,
        AgentProfile {
            max_slope: FP::from_raw(21845),
            ..Default::default()
        }
    )
    .unwrap()
    .triangles()
    .is_empty());
}
#[test]
fn profile_unsupported_values_are_rejected() {
    let t = flat(2, 2);
    for p in [
        AgentProfile {
            radius: FP::from_raw(1),
            ..Default::default()
        },
        AgentProfile {
            headroom: FP::ONE,
            ..Default::default()
        },
        AgentProfile {
            max_step: FP::ONE,
            ..Default::default()
        },
    ] {
        assert_eq!(
            TerrainGraph::build(&t, p),
            Err(NavigationError::UnsupportedProfile)
        );
    }
    for v in [-1, MAX_SLOPE_RAW + 1] {
        assert_eq!(
            TerrainGraph::build(
                &t,
                AgentProfile {
                    max_slope: FP::from_raw(v),
                    ..Default::default()
                }
            ),
            Err(NavigationError::NumericLimit)
        );
    }
}
#[test]
fn numerical_limits_reject_explicitly_without_panicking() {
    for spacing in [1, MIN_SPACING_RAW - 1, MAX_SPACING_RAW + 1] {
        let t = Terrain::new(
            "terrain/tiny".into(),
            2,
            2,
            [FP::ZERO; 2],
            FP::from_raw(spacing),
            vec![FP::ZERO; 4],
            vec![false],
        )
        .unwrap();
        assert_eq!(
            TerrainGraph::build(&t, AgentProfile::default()),
            Err(NavigationError::NumericLimit)
        );
    }
    let t = terrain(
        2,
        2,
        vec![FP::from_raw(i64::MIN), FP::ZERO, FP::ZERO, FP::ZERO],
        vec![false],
    );
    assert_eq!(
        TerrainGraph::build(&t, AgentProfile::default()),
        Err(NavigationError::NumericLimit)
    );
    let t = Terrain::new(
        "terrain/far".into(),
        2,
        2,
        [FP::from_raw(MAX_COORDINATE_RAW + 1), FP::ZERO],
        FP::ONE,
        vec![FP::ZERO; 4],
        vec![false],
    )
    .unwrap();
    assert_eq!(
        TerrainGraph::build(&t, AgentProfile::default()),
        Err(NavigationError::NumericLimit)
    );
}
#[test]
fn minimum_spacing_odd_midpoint_is_in_its_owner() {
    let t = Terrain::new(
        "terrain/odd".into(),
        3,
        3,
        [FP::from_raw(-31), FP::from_raw(-47)],
        FP::from_raw(17),
        vec![FP::ZERO; 9],
        vec![false; 4],
    )
    .unwrap();
    let g = graph(&t);
    for tri in g.triangles() {
        for portal in &tri.portals {
            let p = g
                .project(&t, [portal.midpoint[0], portal.midpoint[2]])
                .unwrap();
            assert!(p.triangle == tri.key || p.triangle == portal.neighbor);
            assert_eq!(p.position, portal.midpoint);
        }
    }
}
#[test]
fn projection_uses_canonical_diagonal_seam_hole_and_outer_owner() {
    let t = flat(3, 3);
    let g = graph(&t);
    assert_eq!(
        g.project(&t, [fp!(-1.5), fp!(-1.5)]).unwrap().triangle,
        TriangleKey(0)
    );
    assert_eq!(
        g.project(&t, [fp!(-1), fp!(-1)]).unwrap().triangle,
        TriangleKey(6)
    );
    assert_eq!(
        g.project(&t, [FP::ZERO, FP::ZERO]).unwrap().triangle,
        TriangleKey(6)
    );
    assert_eq!(
        g.project(&t, [FP::from_raw(1), FP::ZERO]),
        Err(NavigationError::OutsideTerrain)
    );
    let t = terrain(3, 3, vec![FP::ZERO; 9], vec![false, true, false, false]);
    let g = graph(&t);
    assert_eq!(
        g.project(&t, [fp!(-1), fp!(-1.5)]),
        Err(NavigationError::Unwalkable)
    );
    assert!(g
        .project(&t, [FP::from_raw(fp!(-1).raw() - 1), fp!(-1.5)])
        .is_ok());
}
#[test]
fn dijkstra_is_complete_stable_and_budgeted() {
    let t = flat(5, 5);
    let g = graph(&t);
    let a = [fp!(-1.75), fp!(-1.75)];
    let b = [fp!(1.75), fp!(1.75)];
    let p = g.find_path(&t, a, b, SearchBudget::default()).unwrap();
    assert_eq!(p, g.find_path(&t, a, b, SearchBudget::default()).unwrap());
    assert_eq!(p.waypoints().len(), p.corridor().len() + 1);
    assert_eq!(p.portals().len() + 1, p.corridor().len());
    for (segment, key) in p.waypoints().windows(2).zip(p.corridor().iter().copied()) {
        g.validate_segment(&t, key, segment[0], segment[1]).unwrap();
    }
    assert_eq!(
        g.find_path(&t, a, b, SearchBudget { max_expansions: 0 }),
        Err(NavigationError::BudgetExceeded { expanded: 0 })
    );
    assert_eq!(
        g.find_path(&t, a, b, SearchBudget { max_expansions: 1 }),
        Err(NavigationError::BudgetExceeded { expanded: 1 })
    );
    let same = g
        .find_path(&t, a, a, SearchBudget { max_expansions: 1 })
        .unwrap();
    assert_eq!(same.corridor().len(), 1);
    assert_eq!(same.cost(), FP::ZERO);
}
#[test]
fn canonical_cook_load_all_truncations_mutations_and_dependency_binding() {
    let t = flat(2, 2);
    let g = graph(&t);
    let bytes = g.cook();
    assert_eq!(
        g,
        TerrainGraph::load(&bytes, &t, AgentProfile::default()).unwrap()
    );
    assert_eq!(bytes, g.cook());
    for n in 0..bytes.len() {
        assert!(
            TerrainGraph::load(&bytes[..n], &t, AgentProfile::default()).is_err(),
            "prefix {n}"
        );
    }
    for i in 0..bytes.len() {
        let mut changed = bytes.clone();
        changed[i] ^= 1;
        assert!(
            TerrainGraph::load(&changed, &t, AgentProfile::default()).is_err(),
            "byte {i}"
        );
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert_eq!(
        TerrainGraph::load(&extra, &t, AgentProfile::default()),
        Err(NavigationError::MalformedAsset)
    );
    assert_eq!(
        TerrainGraph::load(&vec![0; MAX_FILE_BYTES + 1], &t, AgentProfile::default()),
        Err(NavigationError::MalformedAsset)
    );
    assert_eq!(
        TerrainGraph::load(
            &bytes,
            &t,
            AgentProfile {
                max_slope: fp!(2),
                ..Default::default()
            }
        ),
        Err(NavigationError::ProfileMismatch)
    );
    let other = Terrain::new(
        "terrain/other".into(),
        2,
        2,
        t.origin(),
        t.spacing(),
        t.heights().to_vec(),
        t.holes().to_vec(),
    )
    .unwrap();
    assert_eq!(
        TerrainGraph::load(&bytes, &other, AgentProfile::default()),
        Err(NavigationError::StaleTerrain)
    );
}
#[test]
fn agent_movement_elevation_distance_replay_stop_and_arrival() {
    let t = terrain(
        5,
        5,
        (0..25)
            .map(|i| FP::from_raw(i64::from(i % 5) * 8192))
            .collect(),
        vec![false; 16],
    );
    let g = graph(&t);
    let mut a = Navigator::new(&g, &t, [fp!(-1.75), fp!(-1.75)]).unwrap();
    a.replan(&g, &t, [fp!(1.75), fp!(1.25)], SearchBudget::default())
        .unwrap();
    let mut replay = a.clone();
    for _ in 0..500 {
        let old = a.position();
        a.advance(&g, &t, fp!(0.0625)).unwrap();
        replay.advance(&g, &t, fp!(0.0625)).unwrap();
        assert_eq!(a, replay);
        assert_eq!(a.checksum(), replay.checksum());
        assert_eq!(
            a.position()[1],
            t.sample(a.position()[0], a.position()[2]).unwrap()
        );
        let squared: i128 = (0..3)
            .map(|axis| {
                let d = i128::from(a.position()[axis].raw()) - i128::from(old[axis].raw());
                d * d
            })
            .sum();
        assert!(squared <= i128::from(fp!(0.0625).raw()).pow(2));
        if a.status() == NavigationStatus::Arrived {
            break;
        }
    }
    assert_eq!(a.status(), NavigationStatus::Arrived);
    assert_eq!([a.position()[0], a.position()[2]], [fp!(1.75), fp!(1.25)]);
    a.advance(&g, &t, fp!(100)).unwrap();
    assert_eq!(a, replay);
    a.replan(&g, &t, [fp!(-1.75), fp!(-1.75)], SearchBudget::default())
        .unwrap();
    a.advance(&g, &t, fp!(0.1)).unwrap();
    a.stop();
    let stopped = a.clone();
    a.advance(&g, &t, fp!(100)).unwrap();
    assert_eq!(a, stopped);
}
#[test]
fn errors_and_explicit_replans_are_atomic_across_terrain_edits() {
    let t = flat(4, 4);
    let g = graph(&t);
    let mut a = Navigator::new(&g, &t, [fp!(-1.75), fp!(-1.75)]).unwrap();
    a.replan(&g, &t, [fp!(0.75), fp!(0.75)], SearchBudget::default())
        .unwrap();
    let old = a.clone();
    assert_eq!(
        a.advance(&g, &t, FP::from_raw(-1)),
        Err(NavigationError::InvalidDistance)
    );
    assert_eq!(a, old);
    assert_eq!(
        a.replan(&g, &t, [fp!(100), fp!(100)], SearchBudget::default()),
        Err(NavigationError::OutsideTerrain)
    );
    assert_eq!(a, old);
    assert_eq!(
        a.replan(
            &g,
            &t,
            [fp!(0.75), fp!(0.75)],
            SearchBudget { max_expansions: 0 }
        ),
        Err(NavigationError::BudgetExceeded { expanded: 0 })
    );
    assert_eq!(a, old);
    let mut doc = TerrainDocument::new(t.clone());
    doc.apply(&[Edit::SetHeight {
        x: 3,
        z: 3,
        height: fp!(0.125),
    }])
    .unwrap();
    let changed = doc.terrain();
    for budget in [FP::ZERO, fp!(0.125)] {
        assert_eq!(
            a.advance(&g, changed, budget),
            Err(NavigationError::StaleTerrain)
        );
        assert_eq!(a, old);
    }
    let new = graph(changed);
    assert_eq!(
        a.advance(&new, changed, FP::ZERO),
        Err(NavigationError::StaleTerrain)
    );
    assert_eq!(a, old);
    a.replan(
        &new,
        changed,
        [fp!(0.75), fp!(0.75)],
        SearchBudget::default(),
    )
    .unwrap();
    a.advance(&new, changed, fp!(1)).unwrap();
    let prior = a.clone();
    let steep = TerrainGraph::build(
        changed,
        AgentProfile {
            max_slope: FP::ZERO,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        a.advance(&steep, changed, FP::ZERO),
        Err(NavigationError::StaleGraph)
    );
    assert_eq!(a, prior);
}
#[test]
fn stopped_and_arrived_states_still_reject_stale_dependencies() {
    let t = flat(3, 3);
    let g = graph(&t);
    let mut doc = TerrainDocument::new(t.clone());
    doc.apply(&[Edit::SetHeight {
        x: 2,
        z: 2,
        height: FP::from_raw(1),
    }])
    .unwrap();
    let mut a = Navigator::new(&g, &t, [fp!(-1.5), fp!(-1.5)]).unwrap();
    let stopped = a.clone();
    assert_eq!(
        a.advance(&g, doc.terrain(), FP::ZERO),
        Err(NavigationError::StaleTerrain)
    );
    assert_eq!(a, stopped);
    a.replan(&g, &t, [fp!(-1.5), fp!(-1.5)], SearchBudget::default())
        .unwrap();
    let arrived = a.clone();
    assert_eq!(a.status(), NavigationStatus::Arrived);
    assert_eq!(
        a.advance(&g, doc.terrain(), FP::ZERO),
        Err(NavigationError::StaleTerrain)
    );
    assert_eq!(a, arrived);
}
#[test]
fn tiny_budget_is_safe_zero_progress_and_large_budget_never_overshoots() {
    let t = flat(2, 2);
    let g = graph(&t);
    let mut a = Navigator::new(&g, &t, [fp!(-1.75), fp!(-1.75)]).unwrap();
    a.replan(&g, &t, [fp!(-1.25), fp!(-1.25)], SearchBudget::default())
        .unwrap();
    let old = a.clone();
    a.advance(&g, &t, FP::from_raw(1)).unwrap();
    assert_eq!(a, old);
    a.advance(&g, &t, FP::from_raw(i64::MAX)).unwrap();
    assert_eq!(a.status(), NavigationStatus::Arrived);
    assert_eq!([a.position()[0], a.position()[2]], [fp!(-1.25), fp!(-1.25)]);
}
#[test]
fn a_hole_owned_seam_is_rejected() {
    let t = terrain(3, 3, vec![FP::ZERO; 9], vec![false, true, false, false]);
    let g = graph(&t);
    // Canonical +X ownership rejects the hole seam without a neighbor fallback.
    let a = [fp!(-1), FP::ZERO, fp!(-2)];
    let b = [fp!(-1), FP::ZERO, fp!(-1)];
    assert!(g.validate_segment(&t, TriangleKey(1), a, b).is_err());
}

#[test]
fn steep_owned_edge_interior_is_rejected_despite_walkable_endpoints() {
    let t = terrain(
        3,
        3,
        vec![
            FP::ZERO,
            FP::ZERO,
            FP::ZERO,
            FP::ZERO,
            FP::ONE,
            FP::ZERO,
            FP::ZERO,
            FP::ZERO,
            FP::ZERO,
        ],
        vec![false; 4],
    );
    let g = graph(&t);
    let a = [fp!(-2), FP::ZERO, fp!(-1)];
    let b = [fp!(-1), FP::ONE, fp!(-1)];
    assert!(g.project(&t, [a[0], a[2]]).is_ok());
    assert!(g.project(&t, [b[0], b[2]]).is_ok());
    assert!(g.triangle(TriangleKey(0)).is_some());
    assert!(g.validate_segment(&t, TriangleKey(0), a, b).is_err());
}
#[test]
fn inclusive_geometry_limit_and_maximum_slope_build_without_overflow() {
    let t = Terrain::new(
        "terrain/limits".into(),
        2,
        2,
        [FP::from_raw(-MAX_COORDINATE_RAW); 2],
        FP::from_raw(MAX_SPACING_RAW),
        vec![FP::from_raw(MAX_COORDINATE_RAW); 4],
        vec![false],
    )
    .unwrap();
    let p = AgentProfile {
        max_slope: FP::from_raw(MAX_SLOPE_RAW),
        ..Default::default()
    };
    let g = TerrainGraph::build(&t, p).unwrap();
    assert_eq!(g.triangles().len(), 2);
    assert_eq!(g, TerrainGraph::load(&g.cook(), &t, p).unwrap());
}

#[test]
fn canonical_stopped_state_checksum_golden() {
    let t = flat(2, 2);
    let g = graph(&t);
    let a = Navigator::new(&g, &t, [fp!(-1.75), fp!(-1.75)]).unwrap();
    assert_eq!(
        a.checksum(),
        [
            224, 143, 58, 19, 233, 137, 155, 189, 152, 205, 181, 243, 127, 204, 29, 98, 126, 213,
            57, 181, 35, 163, 121, 177, 35, 241, 123, 73, 75, 192, 99, 239
        ]
    );
    let mut b = a.clone();
    b.replan(&g, &t, [fp!(-1.25), fp!(-1.25)], SearchBudget::default())
        .unwrap();
    assert_ne!(a.checksum(), b.checksum());
    b.stop();
    assert_eq!(a.checksum(), b.checksum());
}
