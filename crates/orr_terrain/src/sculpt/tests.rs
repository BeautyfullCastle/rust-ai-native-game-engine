use super::*;
use crate::TerrainDocument;

fn terrain(side: u32, height: FP) -> Terrain {
    Terrain::new(
        "sculpt".into(),
        side,
        side,
        [FP::from_int(-7), FP::from_int(3)],
        FP::from_int(2),
        vec![height; (side * side) as usize],
        vec![false; ((side - 1) * (side - 1)) as usize],
    )
    .unwrap()
}

#[test]
fn parabolic_disk_has_literal_heights_and_preserves_metadata_and_holes() {
    let mut doc = TerrainDocument::new(terrain(5, FP::ZERO));
    doc.apply(&[Edit::SetHole {
        x: 1,
        z: 1,
        hole: true,
    }])
    .unwrap();
    let before = doc.terrain().clone();
    let edits = radial_edits(&before, [2, 2], 2, FP::from_int(4)).unwrap();
    assert_eq!(edits.len(), 9);
    doc.apply(&edits).unwrap();
    assert_eq!(
        doc.terrain()
            .heights()
            .iter()
            .map(|h| h.raw())
            .collect::<Vec<_>>(),
        [
            0, 0, 0, 0, 0, 0, 131072, 196608, 131072, 0, 0, 196608, 262144, 196608, 0, 0, 131072,
            196608, 131072, 0, 0, 0, 0, 0, 0
        ]
    );
    assert_eq!(doc.terrain().holes(), before.holes());
    assert_eq!(doc.terrain().origin(), before.origin());
    assert_eq!(doc.terrain().spacing(), before.spacing());
    assert_eq!(doc.terrain().asset_id(), before.asset_id());
    let saved = doc.terrain().cook();
    let loaded = Terrain::load(&saved).unwrap();
    assert_eq!(
        loaded.sample(FP::from_int(-3), FP::from_int(7)),
        Some(FP::from_int(4))
    );
    assert_eq!(loaded.sample(FP::from_int(-5), FP::from_int(5)), None);
    assert!(doc.undo());
    assert_eq!(doc.terrain(), &before);
    assert!(doc.redo());
    assert_eq!(doc.terrain().cook(), saved);
}

#[test]
fn clipped_corner_radius_one_and_signed_quantization_are_exact() {
    let source = terrain(5, FP::from_int(10));
    let edits = radial_edits(&source, [0, 0], 2, FP::from_int(-4)).unwrap();
    assert_eq!(
        edits,
        vec![
            Edit::SetHeight {
                x: 0,
                z: 0,
                height: FP::from_int(6)
            },
            Edit::SetHeight {
                x: 1,
                z: 0,
                height: FP::from_int(7)
            },
            Edit::SetHeight {
                x: 0,
                z: 1,
                height: FP::from_int(7)
            },
            Edit::SetHeight {
                x: 1,
                z: 1,
                height: FP::from_int(8)
            },
        ]
    );
    assert_eq!(
        radial_edits(&source, [4, 4], 1, FP::ONE).unwrap(),
        vec![Edit::SetHeight {
            x: 4,
            z: 4,
            height: FP::from_int(11)
        }]
    );
    // A raw-unit delta truncates every partial weight to zero for both signs.
    for sign in [-1, 1] {
        assert_eq!(
            radial_edits(&source, [2, 2], 2, FP::from_raw(sign)).unwrap(),
            vec![Edit::SetHeight {
                x: 2,
                z: 2,
                height: FP::from_raw(655360 + sign)
            }]
        );
    }
    for delta in [i64::MIN, -234567, 234567, i64::MAX] {
        let mut doc = TerrainDocument::new(terrain(5, FP::ZERO));
        let edits = radial_edits(doc.terrain(), [2, 2], 3, FP::from_raw(delta)).unwrap();
        doc.apply(&edits).unwrap();
        // Check negative/positive symmetry except MIN, whose inverse is not i64.
        if let Some(inverse) = delta.checked_neg() {
            let inverse = radial_edits(doc.terrain(), [2, 2], 3, FP::from_raw(inverse)).unwrap();
            doc.apply(&inverse).unwrap();
            assert!(doc.terrain().heights().iter().all(|h| *h == FP::ZERO));
        }
    }
}

#[test]
fn invalid_bounds_budget_and_late_overflow_reject_without_source_mutation() {
    let source = terrain(129, FP::ZERO);
    let bytes = source.cook();
    for (center, radius) in [
        ([129, 0], 2),
        ([0, u32::MAX], 2),
        ([0, 0], 0),
        ([0, 0], 129),
        ([0, 0], u32::MAX),
    ] {
        assert!(radial_edits(&source, center, radius, FP::ZERO).is_err());
    }
    for delta in [FP::ZERO, FP::ONE] {
        assert!(radial_edits(&source, [64, 64], 64, delta).is_err());
    }
    // A clipped disk below budget stays legal; the next larger disk exceeds it.
    assert!(radial_edits(&source, [0, 0], 71, FP::ZERO)
        .unwrap()
        .is_empty());
    assert!(radial_edits(&source, [0, 0], 73, FP::ZERO).is_err());
    assert_eq!(source.cook(), bytes);
    for (extreme, delta) in [(i64::MAX, 4), (i64::MIN, -4)] {
        let mut doc = TerrainDocument::new(terrain(3, FP::ZERO));
        doc.apply(&[Edit::SetHeight {
            x: 2,
            z: 2,
            height: FP::from_raw(extreme),
        }])
        .unwrap();
        let before = doc.terrain().cook();
        assert!(radial_edits(doc.terrain(), [1, 1], 2, FP::from_raw(delta)).is_err());
        assert_eq!(doc.terrain().cook(), before);
    }
}

#[test]
fn noops_preserve_redo_dirty_region_and_revision() {
    let mut doc = TerrainDocument::new(terrain(5, FP::ZERO));
    doc.apply(&radial_edits(doc.terrain(), [2, 2], 2, FP::ONE).unwrap())
        .unwrap();
    let changed = doc.terrain().cook();
    assert!(doc.undo());
    let revision = doc.terrain().revision();
    let dirty = doc.dirty_region();
    doc.apply(&radial_edits(doc.terrain(), [2, 2], 2, FP::ZERO).unwrap())
        .unwrap();
    assert!(doc.can_redo());
    assert_eq!(doc.terrain().revision(), revision);
    assert_eq!(doc.dirty_region(), dirty);
    assert!(doc.redo());
    assert_eq!(doc.terrain().cook(), changed);
}

#[test]
fn rectangular_grid_uses_its_row_stride_and_maximum_radius_clips_safely() {
    let source = Terrain::new(
        "rect".into(),
        5,
        2,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 10],
        vec![false; 4],
    )
    .unwrap();
    let mut doc = TerrainDocument::new(source);
    doc.apply(&radial_edits(doc.terrain(), [3, 1], 2, FP::from_int(4)).unwrap())
        .unwrap();
    assert_eq!(
        doc.terrain()
            .heights()
            .iter()
            .map(|h| h.raw())
            .collect::<Vec<_>>(),
        [0, 0, 131072, 196608, 131072, 0, 0, 196608, 262144, 196608]
    );
    let small = terrain(2, FP::ZERO);
    assert_eq!(
        radial_edits(&small, [0, 0], 128, FP::from_raw(16384)).unwrap(),
        vec![
            Edit::SetHeight {
                x: 0,
                z: 0,
                height: FP::from_raw(16384)
            },
            Edit::SetHeight {
                x: 1,
                z: 0,
                height: FP::from_raw(16383)
            },
            Edit::SetHeight {
                x: 0,
                z: 1,
                height: FP::from_raw(16383)
            },
            Edit::SetHeight {
                x: 1,
                z: 1,
                height: FP::from_raw(16382)
            },
        ]
    );
}
