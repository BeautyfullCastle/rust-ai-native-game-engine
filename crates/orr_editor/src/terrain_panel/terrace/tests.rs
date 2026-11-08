use super::*;
fn terrain(width: u32, depth: u32, heights: Vec<i64>) -> Terrain {
    Terrain::new(
        "terrace-test".into(),
        width,
        depth,
        [FP::ZERO; 2],
        FP::ONE,
        heights.into_iter().map(FP::from_raw).collect(),
        vec![false; ((width - 1) * (depth - 1)) as usize],
    )
    .unwrap()
}
fn apply(
    t: Terrain,
    min: [u32; 2],
    max: [u32; 2],
    base: i64,
    step: i64,
) -> orr_terrain::TerrainDocument {
    let changes = edits(&t, min, max, FP::from_raw(base), FP::from_raw(step)).unwrap();
    let mut d = orr_terrain::TerrainDocument::new(t);
    d.apply(&changes).unwrap();
    d
}
#[test]
fn negative_values_floor_euclidean_instead_of_truncating() {
    let d = apply(
        terrain(2, 2, vec![-1, -65537, 65535, 65536]),
        [0, 0],
        [1, 1],
        0,
        65536,
    );
    assert_eq!(
        d.terrain()
            .heights()
            .iter()
            .map(|x| x.raw())
            .collect::<Vec<_>>(),
        vec![-65536, -131072, 0, 65536]
    );
}
#[test]
fn fractional_steps_and_nonzero_base_are_exact() {
    let d = apply(
        terrain(2, 2, vec![0, 8192, 24576, -8193]),
        [0, 0],
        [1, 1],
        8192,
        16384,
    );
    assert_eq!(
        d.terrain()
            .heights()
            .iter()
            .map(|x| x.raw())
            .collect::<Vec<_>>(),
        vec![-8192, 8192, 24576, -24576]
    );
}
#[test]
fn ordered_inclusive_rectangle_emits_changed_only_in_z_x_order_and_preserves_holes() {
    let t = terrain(3, 3, vec![9, 9, 9, 9, 8, 9, 9, 9, 9]);
    let mut d = orr_terrain::TerrainDocument::new(t);
    d.apply(&[Edit::SetHole {
        x: 0,
        z: 0,
        hole: true,
    }])
    .unwrap();
    let holes = d.terrain().holes().to_vec();
    let changes = edits(d.terrain(), [1, 1], [2, 2], FP::ZERO, FP::from_raw(4)).unwrap();
    assert!(matches!(
        changes.as_slice(),
        [
            Edit::SetHeight { x: 2, z: 1, .. },
            Edit::SetHeight { x: 1, z: 2, .. },
            Edit::SetHeight { x: 2, z: 2, .. }
        ]
    ));
    d.apply(&changes).unwrap();
    assert_eq!(d.terrain().holes(), holes);
    assert_eq!(d.terrain().heights()[0].raw(), 9);
    assert_eq!(d.terrain().heights()[4].raw(), 8);
}
#[test]
fn invalid_step_and_rectangles_reject_without_source_changes() {
    let t = terrain(2, 2, vec![1; 4]);
    let before = t.cook();
    for step in [0, -1] {
        assert!(edits(&t, [0, 0], [1, 1], FP::ZERO, FP::from_raw(step)).is_err());
    }
    for (min, max) in [([1, 0], [0, 1]), ([0, 0], [2, 1])] {
        assert!(edits(&t, min, max, FP::ZERO, FP::ONE).is_err());
    }
    assert_eq!(t.cook(), before);
}
#[test]
fn overflow_at_late_vertex_rejects_entire_batch_without_saturation() {
    let t = terrain(2, 2, vec![3, 3, 3, i64::MIN]);
    let before = t.cook();
    assert!(edits(&t, [0, 0], [1, 1], FP::from_raw(i64::MAX), FP::from_raw(2)).is_err());
    assert_eq!(t.cook(), before);
    let safe = terrain(2, 2, vec![i64::MAX; 4]);
    assert!(edits(
        &safe,
        [0, 0],
        [1, 1],
        FP::from_raw(i64::MIN),
        FP::from_raw(1)
    )
    .unwrap()
    .is_empty());
}
#[test]
fn footprint_budget_includes_noops_and_noop_has_no_undo() {
    let large = terrain(65, 65, vec![0; 65 * 65]);
    assert!(edits(&large, [0, 0], [64, 64], FP::ZERO, FP::ONE).is_err());
    assert!(edits(&large, [0, 0], [63, 63], FP::ZERO, FP::ONE)
        .unwrap()
        .is_empty());
    let d = apply(terrain(2, 2, vec![0; 4]), [0, 0], [1, 1], 0, 65536);
    assert!(!d.can_undo());
}
