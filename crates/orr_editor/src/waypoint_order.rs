//! Structural draft edits preserve authored text and report accepted intent.
const MAX_POINTS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edit {
    InsertBefore(usize),
    MoveUp(usize),
    MoveDown(usize),
}

/// `true` means a structural edit was accepted, even when equal row values
/// make the resulting vector compare equal. Callers must still retire row
/// identities, pending picks and inspections for every accepted operation.
pub(crate) fn apply<T: Clone>(rows: &mut Vec<T>, edit: Edit, max: usize) -> bool {
    let limit = max.min(MAX_POINTS);
    if rows.is_empty() || rows.len() > limit {
        return false;
    }
    match edit {
        Edit::InsertBefore(index) if index < rows.len() && rows.len() < limit => {
            let row = rows[index].clone();
            rows.insert(index, row);
        }
        Edit::MoveUp(index) if index > 0 && index < rows.len() => rows.swap(index, index - 1),
        Edit::MoveDown(index) if index < rows.len() - 1 => rows.swap(index, index + 1),
        _ => return false,
    }
    true
}

#[cfg(test)]
#[path = "waypoint_order_tests.rs"]
mod tests;
