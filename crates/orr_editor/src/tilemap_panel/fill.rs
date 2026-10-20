//! Pure, bounded four-connected fill. The panel owns budgets and history.
use orr_sprite::SpriteDocument;
use orr_tilemap::{CellCoord, ValidatedDocument};

/// Build one candidate without modifying the source, including on errors/no-ops.
pub(super) fn candidate(
    current: &ValidatedDocument,
    seed: CellCoord,
    replacement: Option<u32>,
    sprite: &SpriteDocument,
) -> Result<(ValidatedDocument, usize), String> {
    // Check coordinates before even considering a no-op.
    let target = current.cell(seed)?;
    let source = current.document();
    let mut next = source.clone();
    let mut changed = 0;
    if target != replacement {
        // ValidatedDocument guarantees at most MAX_CELLS (4096) entries. Mark
        // cells on enqueue, so each cell is queued at most once, without recursion.
        let width = source.width as usize;
        let count = source.cells.len();
        let start = seed.row as usize * width + seed.column as usize;
        let mut visited = vec![false; count];
        let mut queue = Vec::with_capacity(count);
        visited[start] = true;
        queue.push(start);
        let mut cursor = 0;
        while cursor < queue.len() {
            let index = queue[cursor];
            cursor += 1;
            next.cells[index] = replacement;
            changed += 1;
            let column = index % width;
            let mut enqueue = |neighbor: usize| {
                if !visited[neighbor] && source.cells[neighbor] == target {
                    visited[neighbor] = true;
                    queue.push(neighbor);
                }
            };
            if column > 0 {
                enqueue(index - 1);
            }
            if column + 1 < width {
                enqueue(index + 1);
            }
            if index >= width {
                enqueue(index - width);
            }
            if index + width < count {
                enqueue(index + width);
            }
        }
    }
    // One existing validation pass covers atlas existence/size and structure,
    // even for a no-op. validate owns its resulting clone; the editor makes just
    // the single candidate clone above, never a clone/validation per cell.
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
    fn literal_components_distinguish_empty_zero_diagonals_and_row_edges() {
        let sprite = atlas();
        let source = document(
            3,
            3,
            vec![
                None,
                None,
                Some(0),
                None,
                Some(0),
                None,
                Some(0),
                None,
                None,
            ],
        );
        let before = source.clone();
        let (filled, count) =
            candidate(&source, CellCoord { column: 0, row: 0 }, Some(1), &sprite).unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            filled.document().cells,
            vec![
                Some(1),
                Some(1),
                Some(0),
                Some(1),
                Some(0),
                None,
                Some(0),
                None,
                None
            ]
        );
        assert_eq!(source, before);
        // The three zero cells touch diagonally only.
        let (erased, count) =
            candidate(&source, CellCoord { column: 1, row: 1 }, None, &sprite).unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            erased.document().cells,
            vec![None, None, Some(0), None, None, None, Some(0), None, None]
        );
        let edge = document(
            3,
            2,
            vec![Some(1), Some(1), Some(0), Some(0), Some(1), Some(1)],
        );
        let (filled, count) =
            candidate(&edge, CellCoord { column: 2, row: 0 }, None, &sprite).unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            filled.document().cells,
            vec![Some(1), Some(1), None, Some(0), Some(1), Some(1)]
        );
    }

    #[test]
    fn maximum_component_fill_and_erase_are_bounded_and_immutable() {
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
