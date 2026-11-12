use super::*;

#[test]
fn insertion_duplicates_selected_raw_row_before_it_without_normalizing_text() {
    let mut rows = vec![
        [" 01.000 ".to_owned(), "-0".into()],
        ["invalid".into(), "1/2".into()],
    ];
    let original = rows.clone();
    assert!(apply(&mut rows, Edit::InsertBefore(1), 8));
    assert_eq!(
        rows,
        vec![
            original[0].clone(),
            original[1].clone(),
            original[1].clone()
        ]
    );
}

#[test]
fn moving_rows_preserves_complete_xyz_and_reverses_with_opposite_move() {
    let original = vec![["a", "b", "c"], ["d", "e", "f"], ["g", "h", "i"]];
    let mut rows = original.clone();
    assert!(apply(&mut rows, Edit::MoveUp(2), 8));
    assert_eq!(rows, vec![original[0], original[2], original[1]]);
    assert!(apply(&mut rows, Edit::MoveDown(1), 8));
    assert_eq!(rows, original);
}

#[test]
fn invalid_indices_and_boundary_moves_are_atomic_noops() {
    let original = vec![1, 2, 3];
    for edit in [
        Edit::InsertBefore(3),
        Edit::InsertBefore(usize::MAX),
        Edit::MoveUp(0),
        Edit::MoveUp(3),
        Edit::MoveDown(2),
        Edit::MoveDown(usize::MAX),
    ] {
        let mut rows = original.clone();
        assert!(!apply(&mut rows, edit, 8));
        assert_eq!(rows, original);
    }
    let mut empty: Vec<u8> = Vec::new();
    assert!(!apply(&mut empty, Edit::InsertBefore(0), 8));
    assert!(empty.is_empty());
}

#[test]
fn hard_eight_point_limit_and_caller_limit_do_not_allow_oversized_edits() {
    for (count, max) in [
        (8, 8),
        (8, usize::MAX),
        (3, 3),
        (3, 2),
        (1, 0),
        (9, usize::MAX),
    ] {
        let mut rows = vec![0; count];
        let original = rows.clone();
        assert!(!apply(&mut rows, Edit::InsertBefore(0), max));
        assert_eq!(rows, original);
    }
    let mut rows = vec![0; 7];
    assert!(apply(&mut rows, Edit::InsertBefore(0), usize::MAX));
    assert_eq!(rows.len(), 8);
    assert!(apply(&mut rows, Edit::MoveDown(0), 8));
}

#[test]
fn identical_row_moves_are_accepted_structural_intent() {
    let mut rows = vec![["same"; 3]; 2];
    let before = rows.clone();
    assert!(apply(&mut rows, Edit::MoveUp(1), 8));
    assert_eq!(rows, before);
    assert!(apply(&mut rows, Edit::MoveDown(0), 8));
    assert_eq!(rows, before);
    assert!(apply(&mut rows, Edit::InsertBefore(0), 8));
    assert_eq!(rows.len(), 3);
}
