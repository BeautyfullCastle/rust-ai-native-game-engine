use super::*;
use crate::{DirtyRegion, Edit, MAX_EDITS, MAX_FILE_BYTES, MAX_HISTORY};

fn f(value: i64) -> FP {
    FP::from_raw(value * 65536)
}
fn fixture() -> Terrain {
    Terrain::new(
        "fixtures/expanded.orrt".into(),
        3,
        4,
        [f(-3), f(5)],
        f(2),
        (1..=12).map(f).collect(),
        vec![false, true, false, false, true, false],
    )
    .unwrap()
}
fn unchanged(doc: &TerrainDocument, before: &TerrainDocument) {
    assert_eq!(doc.terrain.cook(), before.terrain.cook());
    assert_eq!(doc.terrain.revision(), before.terrain.revision());
    assert_eq!(doc.dirty, before.dirty);
    assert_eq!(doc.undo, before.undo);
    assert_eq!(doc.redo, before.redo);
}

#[test]
fn expansion_preserves_old_grid_coordinates_metadata_and_hole_strides_on_each_axis() {
    let original = fixture();
    let bytes = original.cook();
    let revision = original.revision();
    for (width, depth) in [(5, 4), (3, 6), (5, 6)] {
        for fill in [
            FP::from_raw(i64::MIN),
            FP::from_raw(i64::MAX),
            FP::from_raw(-17),
        ] {
            let expanded = original.expanded_grid(width, depth, fill).unwrap();
            assert_eq!(expanded.asset_id(), original.asset_id());
            assert_eq!(expanded.origin(), original.origin());
            assert_eq!(expanded.spacing(), original.spacing());
            assert_eq!((expanded.width(), expanded.depth()), (width, depth));
            for z in 0..depth {
                for x in 0..width {
                    let index = (z * width + x) as usize;
                    if x < original.width() && z < original.depth() {
                        assert_eq!(
                            expanded.heights()[index],
                            original.heights()[(z * original.width() + x) as usize]
                        );
                        assert_eq!(
                            expanded.vertex_position(index as u32),
                            original.vertex_position(z * original.width() + x)
                        );
                    } else {
                        assert_eq!(expanded.heights()[index], fill);
                    }
                }
            }
            for z in 0..depth - 1 {
                for x in 0..width - 1 {
                    let expected = if x < original.width() - 1 && z < original.depth() - 1 {
                        original.holes()[(z * (original.width() - 1) + x) as usize]
                    } else {
                        false
                    };
                    assert_eq!(expanded.holes()[(z * (width - 1) + x) as usize], expected);
                }
            }
            assert_eq!(Terrain::load(&expanded.cook()).unwrap(), expanded);
        }
    }
    assert_eq!(original.cook(), bytes);
    assert_eq!(original.revision(), revision);
}

#[test]
fn expansion_reaches_max_grid_without_using_or_weakening_the_sample_edit_budget() {
    let terrain = Terrain::new(
        "max-growth".into(),
        2,
        2,
        [FP::ZERO; 2],
        f(1),
        vec![f(1), f(2), f(3), f(4)],
        vec![true],
    )
    .unwrap();
    let mut doc = TerrainDocument::new(terrain.clone());
    doc.expand_grid(MAX_SIDE, MAX_SIDE, FP::from_raw(i64::MIN))
        .unwrap();
    let expanded = doc.terrain();
    assert_eq!(expanded.heights().len(), (MAX_SIDE * MAX_SIDE) as usize);
    assert!(expanded.heights().len() - terrain.heights().len() > MAX_EDITS);
    assert_eq!(
        expanded.holes().len(),
        ((MAX_SIDE - 1) * (MAX_SIDE - 1)) as usize
    );
    assert_eq!(expanded.holes().iter().filter(|&&hole| hole).count(), 1);
    assert_eq!(
        expanded.triangles().len(),
        (((MAX_SIDE - 1) * (MAX_SIDE - 1) - 1) * 2) as usize
    );
    assert!(expanded.cook().len() <= MAX_FILE_BYTES);
    assert_eq!(Terrain::load(&expanded.cook()).unwrap(), *expanded);
    assert_eq!(
        doc.dirty_region(),
        Some(DirtyRegion {
            min: [0, 0],
            max: [MAX_SIDE - 2; 2]
        })
    );
    let before = doc.clone();
    assert!(doc
        .apply(&vec![
            Edit::SetHeight {
                x: 0,
                z: 0,
                height: f(1)
            };
            MAX_EDITS + 1
        ])
        .is_err());
    unchanged(&doc, &before);
    assert!(doc.undo());
    assert_eq!(doc.terrain(), &terrain);
    assert!(!doc.undo());
}

#[test]
fn expansion_validation_rejects_shrink_invalid_dimensions_and_coordinate_overflow() {
    let original = fixture();
    for (width, depth) in [
        (0, 4),
        (1, 4),
        (3, 0),
        (3, 1),
        (130, 4),
        (3, 130),
        (u32::MAX, 4),
        (3, u32::MAX),
        (2, 4),
        (3, 3),
        (2, 6),
        (5, 3),
    ] {
        assert!(original.validate_expansion(width, depth).is_err());
        assert!(original.expanded_grid(width, depth, f(9)).is_err());
    }
    original.validate_expansion(3, 4).unwrap();
    original.validate_expansion(129, 129).unwrap();
    for axis in 0..2 {
        let mut origin = [FP::ZERO; 2];
        origin[axis] = FP::from_raw(i64::MAX - 1);
        let edge = Terrain::new(
            "edge".into(),
            2,
            2,
            origin,
            FP::from_raw(1),
            vec![FP::ZERO; 4],
            vec![false],
        )
        .unwrap();
        let (width, depth) = if axis == 0 { (3, 2) } else { (2, 3) };
        assert!(edge
            .validate_expansion(width, depth)
            .unwrap_err()
            .to_string()
            .contains("coordinate overflow"));
    }
    // Wide coordinate math admits a legal endpoint near both i64 extremes.
    let wide = Terrain::new(
        "wide".into(),
        2,
        2,
        [FP::from_raw(i64::MIN); 2],
        FP::from_raw(i64::MAX),
        vec![FP::ZERO; 4],
        vec![false],
    )
    .unwrap();
    let expanded = wide.expanded_grid(3, 3, f(1)).unwrap();
    assert_eq!(
        expanded.vertex_position(8).unwrap(),
        [FP::from_raw(i64::MAX - 1), f(1), FP::from_raw(i64::MAX - 1)]
    );
    assert!(wide.validate_expansion(4, 3).is_err());
    assert!(wide.validate_expansion(3, 4).is_err());
}

#[test]
fn equal_dimensions_are_true_noops_preserving_redo_dirty_and_snapshot_history() {
    let original = fixture();
    assert_eq!(
        original
            .expanded_grid(3, 4, FP::from_raw(i64::MAX))
            .unwrap(),
        original
    );
    let mut doc = TerrainDocument::new(original);
    doc.expand_grid(5, 6, f(20)).unwrap();
    assert!(doc.undo());
    let before = doc.clone();
    doc.expand_grid(3, 4, FP::from_raw(i64::MIN)).unwrap();
    unchanged(&doc, &before);
    doc.clear_dirty();
    let before = doc.clone();
    doc.expand_grid(3, 4, f(99)).unwrap();
    unchanged(&doc, &before);
    assert!(doc.redo());
    let before = doc.clone();
    doc.expand_grid(5, 6, f(-99)).unwrap();
    unchanged(&doc, &before);
    assert!(doc.undo());
    assert!(!doc.undo());
}

#[test]
fn expansion_undo_redo_restores_exact_revisions_shapes_dirty_rectangles_and_bounded_history() {
    let original = fixture();
    let mut doc = TerrainDocument::new(original.clone());
    doc.expand_grid(5, 6, f(20)).unwrap();
    let expanded = doc.terrain().clone();
    assert_ne!(expanded.revision(), original.revision());
    assert_eq!(
        doc.dirty_region(),
        Some(DirtyRegion {
            min: [0, 0],
            max: [3, 4]
        })
    );
    assert!(doc.undo());
    assert_eq!(doc.terrain(), &original);
    assert_eq!(
        doc.dirty_region(),
        Some(DirtyRegion {
            min: [0, 0],
            max: [1, 2]
        })
    );
    assert!(doc.redo());
    assert_eq!(doc.terrain().cook(), expanded.cook());
    assert_eq!(doc.terrain().revision(), expanded.revision());
    assert_eq!(
        doc.dirty_region(),
        Some(DirtyRegion {
            min: [0, 0],
            max: [3, 4]
        })
    );
    let mut bounded = TerrainDocument::new(original);
    for step in 1..=MAX_HISTORY + 3 {
        bounded
            .expand_grid(3 + step as u32, 4 + step as u32, f(step as i64))
            .unwrap();
    }
    assert_eq!(bounded.undo.len(), MAX_HISTORY);
    for _ in 0..MAX_HISTORY {
        assert!(bounded.undo());
    }
    assert!(!bounded.undo());
    assert_eq!(
        (bounded.terrain().width(), bounded.terrain().depth()),
        (6, 7)
    );
    for _ in 0..MAX_HISTORY {
        assert!(bounded.redo());
    }
    assert!(!bounded.redo());
}

#[test]
fn ordinary_edits_after_growth_keep_precise_current_stride_dirty_regions_and_atomic_limits() {
    let mut doc = TerrainDocument::new(fixture());
    doc.expand_grid(5, 6, f(20)).unwrap();
    doc.clear_dirty();
    doc.apply(&[Edit::SetHeight {
        x: 3,
        z: 4,
        height: f(99),
    }])
    .unwrap();
    assert_eq!(
        doc.dirty_region(),
        Some(DirtyRegion {
            min: [2, 3],
            max: [3, 4]
        })
    );
    assert_eq!(doc.terrain().heights()[23], f(99)); // Row 4, column 3 at width 5.
    assert_eq!(doc.terrain().heights()[7], f(6)); // Preserved row 1, column 2.
    assert!(doc.undo());
    let before = doc.clone();
    doc.apply(&[]).unwrap();
    unchanged(&doc, &before);
    assert!(doc
        .apply(&[
            Edit::SetHeight {
                x: 0,
                z: 0,
                height: f(-99)
            },
            Edit::SetHole {
                x: 4,
                z: 4,
                hole: true
            }
        ])
        .is_err());
    unchanged(&doc, &before);
    assert!(doc.redo());
    doc.clear_dirty();
    doc.apply(&[Edit::SetHole {
        x: 3,
        z: 4,
        hole: true,
    }])
    .unwrap();
    assert_eq!(
        doc.dirty_region(),
        Some(DirtyRegion {
            min: [3, 4],
            max: [3, 4]
        })
    );
    assert!(doc.undo());
    doc.expand_grid(6, 6, f(-7)).unwrap();
    assert!(!doc.can_redo());
    assert_eq!(
        doc.dirty_region(),
        Some(DirtyRegion {
            min: [0, 0],
            max: [4, 4]
        })
    );
}

#[test]
fn expanded_sampling_preserves_old_interiors_holes_and_follows_new_positive_seam_owners() {
    let original = fixture();
    let expanded = original.expanded_grid(5, 6, f(20)).unwrap();
    assert_eq!(original.sample(f(-2), f(6)), Some(f(3)));
    assert_eq!(expanded.surface(f(-2), f(6)), original.surface(f(-2), f(6)));
    assert!(original.sample(f(0), f(6)).is_none());
    assert!(expanded.sample(f(0), f(6)).is_none());
    // The former +X outer boundary now belongs to the added solid cell.
    assert!(original.sample(f(1), f(6)).is_none());
    assert_eq!(expanded.sample(f(1), f(6)), Some(FP::from_raw(9 * 32768)));
    assert_eq!(expanded.sample(f(2), f(6)), Some(FP::from_raw(23 * 32768)));
    assert_eq!(expanded.sample(f(-2), f(12)), Some(f(15)));
    assert_eq!(expanded.sample(f(5), f(15)), Some(f(20)));
    assert!(expanded
        .sample(FP::from_raw(f(5).raw() + 1), f(15))
        .is_none());
    assert!(expanded
        .sample(f(-3), FP::from_raw(f(15).raw() + 1))
        .is_none());
}

#[test]
fn rejected_expansions_leave_content_revision_dirty_and_complete_undo_redo_unchanged() {
    let mut doc = TerrainDocument::new(fixture());
    doc.apply(&[Edit::SetHeight {
        x: 0,
        z: 0,
        height: f(-7),
    }])
    .unwrap();
    doc.expand_grid(5, 6, f(20)).unwrap();
    assert!(doc.undo());
    let before = doc.clone();
    for (width, depth) in [(2, 4), (3, 3), (130, 4), (3, 130), (u32::MAX, u32::MAX)] {
        assert!(doc.expand_grid(width, depth, f(123)).is_err());
        unchanged(&doc, &before);
    }
    assert!(doc.redo());
    assert_eq!((doc.terrain().width(), doc.terrain().depth()), (5, 6));
    let edge = Terrain::new(
        "edge".into(),
        2,
        2,
        [FP::from_raw(i64::MAX - 1); 2],
        FP::from_raw(1),
        vec![FP::ZERO; 4],
        vec![false],
    )
    .unwrap();
    let mut edge = TerrainDocument::new(edge);
    edge.apply(&[Edit::SetHole {
        x: 0,
        z: 0,
        hole: true,
    }])
    .unwrap();
    assert!(edge.undo());
    let before = edge.clone();
    assert!(edge.expand_grid(3, 3, f(1)).is_err());
    unchanged(&edge, &before);
}
