//! Origin-anchored, lossless grid resizing. The panel owns budgets and history.
use orr_sprite::SpriteDocument;
use orr_tilemap::{ValidatedDocument, MAX_CELLS, MAX_DIMENSION};

/// Build one candidate without modifying the source, including on errors/no-ops.
pub(super) fn candidate(
    current: &ValidatedDocument,
    width: u32,
    height: u32,
    sprite: &SpriteDocument,
) -> Result<ValidatedDocument, String> {
    // Bound dimensions before allocation or index arithmetic, including no-ops.
    if !(1..=MAX_DIMENSION).contains(&width) || !(1..=MAX_DIMENSION).contains(&height) {
        return Err("width and height must be in 1..=64".into());
    }
    let count = width.checked_mul(height).ok_or("width * height overflow")? as usize;
    if count > MAX_CELLS {
        return Err("tilemap resize exceeds 4096 cells".into());
    }
    let source = current.document();
    for (index, cell) in source.cells.iter().enumerate() {
        let column = index as u32 % source.width;
        let row = index as u32 / source.width;
        if cell.is_some() && (column >= width || row >= height) {
            return Err(format!(
                "resize would crop occupied cell ({column},{row}); erase it first"
            ));
        }
    }
    let mut next = source.clone();
    if width != source.width || height != source.height {
        next.width = width;
        next.height = height;
        next.cells = vec![None; count];
        for row in 0..source.height.min(height) {
            for column in 0..source.width.min(width) {
                next.cells[(row * width + column) as usize] =
                    source.cells[(row * source.width + column) as usize];
            }
        }
    }
    // One existing structure/atlas admission pass, even for equal dimensions.
    // validate owns its resulting clone; the editor makes only one candidate
    // clone above, never a clone or validation per retained cell.
    next.validate(sprite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_tilemap::{CellCoord, Document, SpriteRef};

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
                document: "art/sprites.json".into(),
            },
            width,
            height,
            tile_size: [1, 1],
            origin: [-0.0, 7.25],
            units_per_pixel: 0.125,
            cells,
        }
        .validate(&atlas())
        .unwrap()
    }

    #[test]
    fn asymmetric_growth_trim_and_mixed_axes_preserve_coordinates_and_metadata_bits() {
        let sprite = atlas();
        let source = document(3, 2, vec![Some(0), None, Some(1), None, Some(1), Some(0)]);
        let before = source.clone();
        let grown = candidate(&source, 5, 3, &sprite).unwrap();
        // Independent literal oracle: neither flat-prefix copying nor a wrong
        // source/destination stride can produce this 3x2 -> 5x3 rectangle.
        assert_eq!(
            grown.document().cells,
            vec![
                Some(0),
                None,
                Some(1),
                None,
                None,
                None,
                Some(1),
                Some(0),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ]
        );
        let old = source.document();
        let new = grown.document();
        assert_eq!((new.width, new.height), (5, 3));
        assert_eq!(new.version, old.version);
        assert_eq!(new.sprite, old.sprite);
        assert_eq!(new.tile_size, old.tile_size);
        assert_eq!(new.origin.map(f32::to_bits), old.origin.map(f32::to_bits));
        assert_eq!(new.units_per_pixel.to_bits(), old.units_per_pixel.to_bits());
        assert_eq!(
            grown.placements().collect::<Vec<_>>(),
            source.placements().collect::<Vec<_>>()
        );
        let positions: Vec<_> = grown.placements().map(|p| (p.cell, p.position)).collect();
        assert_eq!(
            positions,
            vec![
                (CellCoord { column: 0, row: 0 }, [0.0625, 7.1875]),
                (CellCoord { column: 2, row: 0 }, [0.3125, 7.1875]),
                (CellCoord { column: 1, row: 1 }, [0.1875, 7.0625]),
                (CellCoord { column: 2, row: 1 }, [0.3125, 7.0625]),
            ]
        );
        assert_eq!(candidate(&grown, 3, 2, &sprite).unwrap(), source);
        assert_eq!(source, before);

        let sparse = document(3, 2, vec![Some(0), Some(1), None, Some(1), None, None]);
        let mixed = candidate(&sparse, 2, 3, &sprite).unwrap();
        assert_eq!(
            mixed.document().cells,
            vec![Some(0), Some(1), Some(1), None, None, None]
        );
        assert_eq!(candidate(&mixed, 3, 2, &sprite).unwrap(), sparse);
        let empty = document(3, 2, vec![None; 6]);
        assert_eq!(
            candidate(&empty, 1, 1, &sprite).unwrap().document().cells,
            vec![None]
        );
    }

    #[test]
    fn every_cropped_edge_rejects_even_region_zero_without_mutation() {
        let sprite = atlas();
        // Check each removed coordinate, not just the first occupied cell, with
        // width-only, height-only, both-axis, and opposite-axis growth crops.
        for (width, height) in [(2, 2), (3, 1), (2, 1), (2, 3), (4, 1)] {
            for row in 0..2 {
                for column in 0..3 {
                    if column < width && row < height {
                        continue;
                    }
                    for region in [0, 1] {
                        let mut cells = vec![None; 6];
                        cells[(row * 3 + column) as usize] = Some(region);
                        let source = document(3, 2, cells);
                        let before = source.clone();
                        let error = candidate(&source, width, height, &sprite).unwrap_err();
                        assert!(error.contains(&format!("({column},{row})")));
                        assert!(error.contains("erase it first"));
                        assert_eq!(source, before);
                    }
                }
            }
        }
    }

    #[test]
    fn bounds_maximum_noop_and_full_atlas_admission_are_atomic() {
        let sprite = atlas();
        let source = document(1, 1, vec![Some(0)]);
        let before = source.clone();
        for (width, height) in [
            (0, 1),
            (1, 0),
            (65, 1),
            (1, 65),
            (65, 65),
            (u32::MAX, u32::MAX),
        ] {
            assert!(candidate(&source, width, height, &sprite).is_err());
            assert_eq!(source, before);
        }
        assert_eq!(candidate(&source, 1, 1, &sprite).unwrap(), source);
        let maximum = candidate(&source, 64, 64, &sprite).unwrap();
        assert_eq!(maximum.document().cells.len(), MAX_CELLS);
        assert_eq!(maximum.document().cells[0], Some(0));
        assert!(maximum.document().cells[1..].iter().all(Option::is_none));
        assert_eq!(candidate(&maximum, 1, 1, &sprite).unwrap(), source);
        let occupied = document(64, 64, vec![Some(0); MAX_CELLS]);
        assert_eq!(candidate(&occupied, 64, 64, &sprite).unwrap(), occupied);
        assert!(candidate(&occupied, 63, 64, &sprite).is_err());
        assert!(candidate(&occupied, 64, 63, &sprite).is_err());
        // Supplied atlas must be rechecked even for a no-op, both for missing
        // IDs and mismatched region dimensions. The immutable input stays valid
        // against its original atlas, not necessarily a newly supplied one.
        for json in [
            r#"{"format":"orr_sprite","version":1,"atlas":{"image":"other.png","width":1,"height":1},"regions":[{"id":9,"x":0,"y":0,"width":1,"height":1}],"clips":[]}"#,
            r#"{"format":"orr_sprite","version":1,"atlas":{"image":"other.png","width":2,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":2,"height":1}],"clips":[]}"#,
        ] {
            let other = SpriteDocument::from_json(json).unwrap();
            for (width, height) in [(1, 1), (2, 3)] {
                assert!(candidate(&source, width, height, &other).is_err());
                assert_eq!(source, before);
            }
        }
    }
}
