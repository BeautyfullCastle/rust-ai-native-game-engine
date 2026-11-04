//! Bounded whole-map replacement. The panel owns budgets and history.
use orr_sprite::SpriteDocument;
use orr_tilemap::{CellCoord, ValidatedDocument};

/// Match the seed's exact optional region, including disconnected cells.
/// Source and history remain untouched on errors and no-ops.
pub(super) fn candidate(
    current: &ValidatedDocument,
    seed: CellCoord,
    replacement: Option<u32>,
    sprite: &SpriteDocument,
) -> Result<(ValidatedDocument, usize), String> {
    let target = current.cell(seed)?;
    // Validation bounds this single linear scan to MAX_CELLS (4096).
    let mut next = current.document().clone();
    let mut changed = 0;
    if target != replacement {
        for cell in &mut next.cells {
            if *cell == target {
                *cell = replacement;
                changed += 1;
            }
        }
    }
    // Revalidate even no-ops against the supplied atlas.
    Ok((next.validate(sprite)?, changed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_tilemap::{Document, SpriteRef};

    fn atlas() -> SpriteDocument {
        SpriteDocument::from_json(
            r#"{"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":4,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":1,"height":1},{"id":1,"x":1,"y":0,"width":1,"height":1},{"id":2,"x":2,"y":0,"width":2,"height":1}],"clips":[]}"#,
        )
        .unwrap()
    }

    fn document(width: u32, height: u32, cells: Vec<Option<u32>>) -> ValidatedDocument {
        Document {
            version: 1,
            sprite: SpriteRef {
                package: "fixture".into(),
                document: "sprites.json".into(),
            },
            width,
            height,
            tile_size: [1, 1],
            origin: [0., 0.],
            units_per_pixel: 1.,
            cells,
        }
        .validate(&atlas())
        .unwrap()
    }

    #[test]
    fn literal_disconnected_cells_match_exact_optional_region() {
        let sprite = atlas();
        let source = document(
            3,
            3,
            vec![
                None,
                Some(0),
                Some(1),
                Some(0),
                None,
                Some(1),
                Some(1),
                Some(0),
                None,
            ],
        );
        let before = source.clone();
        let (replaced, count) =
            candidate(&source, CellCoord { column: 0, row: 0 }, Some(1), &sprite).unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            replaced.document().cells,
            vec![
                Some(1),
                Some(0),
                Some(1),
                Some(0),
                Some(1),
                Some(1),
                Some(1),
                Some(0),
                Some(1)
            ]
        );
        let (erased, count) =
            candidate(&source, CellCoord { column: 1, row: 0 }, None, &sprite).unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            erased.document().cells,
            vec![
                None,
                None,
                Some(1),
                None,
                None,
                Some(1),
                Some(1),
                None,
                None
            ]
        );
        let (zero, count) =
            candidate(&source, CellCoord { column: 2, row: 0 }, Some(0), &sprite).unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            zero.document().cells,
            vec![
                None,
                Some(0),
                Some(0),
                Some(0),
                None,
                Some(0),
                Some(0),
                Some(0),
                None
            ]
        );
        assert_eq!(source, before);
    }

    #[test]
    fn maximum_map_replace_and_erase_are_bounded_and_immutable() {
        let sprite = atlas();
        let source = document(64, 64, vec![None; orr_tilemap::MAX_CELLS]);
        let before = source.clone();
        let (filled, count) = candidate(
            &source,
            CellCoord {
                column: 63,
                row: 63,
            },
            Some(0),
            &sprite,
        )
        .unwrap();
        assert_eq!(count, 4096);
        assert_eq!(filled.document().cells, vec![Some(0); 4096]);
        assert_eq!(source, before);
        let (erased, count) =
            candidate(&filled, CellCoord { column: 0, row: 0 }, None, &sprite).unwrap();
        assert_eq!(count, 4096);
        assert_eq!(erased, source);
        assert_eq!(filled.document().cells, vec![Some(0); 4096]);
    }

    #[test]
    fn noops_and_invalid_bounds_or_regions_preserve_source() {
        let sprite = atlas();
        let source = document(2, 1, vec![None, Some(0)]);
        let before = source.clone();
        for (seed, replacement) in [
            (CellCoord { column: 0, row: 0 }, None),
            (CellCoord { column: 1, row: 0 }, Some(0)),
        ] {
            let (next, count) = candidate(&source, seed, replacement, &sprite).unwrap();
            assert_eq!(count, 0);
            assert_eq!(next, before);
        }
        for (seed, replacement) in [
            (CellCoord { column: 2, row: 0 }, None),
            (CellCoord { column: 0, row: 1 }, None),
            (
                CellCoord {
                    column: u32::MAX,
                    row: u32::MAX,
                },
                None,
            ),
            (CellCoord { column: 0, row: 0 }, Some(99)),
            (CellCoord { column: 0, row: 0 }, Some(2)),
        ] {
            assert!(candidate(&source, seed, replacement, &sprite).is_err());
            assert_eq!(source, before);
        }
        // A no-op must not bypass revalidation against the supplied atlas.
        let other = SpriteDocument::from_json(
            r#"{"format":"orr_sprite","version":1,"atlas":{"image":"other.png","width":1,"height":1},"regions":[{"id":9,"x":0,"y":0,"width":1,"height":1}],"clips":[]}"#,
        )
        .unwrap();
        assert!(candidate(&source, CellCoord { column: 1, row: 0 }, Some(0), &other).is_err());
        assert_eq!(source, before);
    }
}
