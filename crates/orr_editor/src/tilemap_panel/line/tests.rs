use super::*;
use crate::tilemap_panel::{History, TilemapPanel};
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

fn cell(column: u32, row: u32) -> CellCoord {
    CellCoord { column, row }
}

#[test]
fn literal_all_octants_and_reversed_ties() {
    for (start, end, expected) in [
        ((0, 0), (4, 2), vec![(0, 0), (1, 1), (2, 1), (3, 2), (4, 2)]),
        ((0, 2), (4, 0), vec![(0, 2), (1, 1), (2, 1), (3, 0), (4, 0)]),
        ((0, 0), (2, 4), vec![(0, 0), (1, 1), (1, 2), (2, 3), (2, 4)]),
        ((0, 4), (2, 0), vec![(0, 4), (1, 3), (1, 2), (2, 1), (2, 0)]),
        ((1, 2), (4, 2), vec![(1, 2), (2, 2), (3, 2), (4, 2)]),
        ((2, 1), (2, 4), vec![(2, 1), (2, 2), (2, 3), (2, 4)]),
        ((3, 2), (3, 2), vec![(3, 2)]),
    ] {
        let start = cell(start.0, start.1);
        let end = cell(end.0, end.1);
        let expected: Vec<_> = expected.into_iter().map(|(x, y)| cell(x, y)).collect();
        assert_eq!(cells(start, end), expected);
        assert_eq!(cells(end, start), expected);
    }
}

#[test]
fn asymmetric_candidate_preserves_metadata_and_roundtrips_to_consumer() {
    let atlas = atlas();
    let source = document(5, 3, vec![None; 15]);
    let before = source.clone();
    let next = candidate(&source, cell(0, 0), cell(4, 2), Some(0), &atlas).unwrap();
    let mut expected = source.document().clone();
    expected.cells = vec![
        Some(0),
        None,
        None,
        None,
        None,
        None,
        Some(0),
        Some(0),
        None,
        None,
        None,
        None,
        None,
        Some(0),
        Some(0),
    ];
    assert_eq!(next.document(), &expected);
    assert_eq!(source, before);
    let bytes = crate::tilemap_panel::encode(&next).unwrap();
    let consumed = Document::from_bytes(&bytes)
        .unwrap()
        .validate(&atlas)
        .unwrap();
    assert_eq!(consumed, next);
    assert_eq!(
        candidate(&next, cell(4, 2), cell(0, 0), None, &atlas).unwrap(),
        source
    );
}

#[test]
fn maximum_diagonal_is_bounded_and_invalid_inputs_leave_source_unchanged() {
    let atlas = atlas();
    let source = document(64, 64, vec![None; 4096]);
    let next = candidate(&source, cell(0, 63), cell(63, 0), Some(1), &atlas).unwrap();
    assert_eq!(next.occupied_count(), 64);
    for row in 0..64 {
        assert_eq!(next.cell(cell(63 - row, row)).unwrap(), Some(1));
    }
    for bad in [cell(64, 0), cell(0, 64), cell(u32::MAX, u32::MAX)] {
        assert!(candidate(&source, bad, cell(0, 0), None, &atlas).is_err());
        assert!(candidate(&source, cell(0, 0), bad, None, &atlas).is_err());
    }
    for region in [2, 99, u32::MAX] {
        assert!(candidate(&source, cell(0, 0), cell(63, 63), Some(region), &atlas).is_err());
    }
    assert_eq!(source.occupied_count(), 0);
}

#[test]
fn single_history_transaction_noop_and_budget_failure_preserve_redo() {
    let atlas = atlas();
    let source = document(5, 3, vec![None; 15]);
    let mut history = History::new(source.clone()).unwrap();
    let next = candidate(&source, cell(0, 0), cell(4, 2), Some(1), &atlas).unwrap();
    TilemapPanel::budget(&next, 4091).unwrap();
    assert!(history.commit(next.clone()));
    assert_eq!(history.undo, vec![source.clone()]);
    history.undo();
    let saved = history.saved.clone();
    assert!(!history.commit(candidate(&source, cell(0, 0), cell(4, 2), None, &atlas).unwrap()));
    assert!(TilemapPanel::budget(&next, 4092).is_err());
    assert!(TilemapPanel::budget(&next, usize::MAX).is_err());
    assert_eq!(history.redo, vec![next.clone()]);
    assert_eq!(history.saved, saved);
    history.redo();
    assert_eq!(history.current, next);
}
