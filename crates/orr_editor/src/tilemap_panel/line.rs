//! One-cell-wide, endpoint-inclusive integer lines. No scene/schema changes.
use orr_sprite::SpriteDocument;
use orr_tilemap::{CellCoord, ValidatedDocument};

/// Canonicalize endpoint order so reversing a drag cannot change tie pixels.
/// Callers provide admitted grid coordinates (both axes are at most 63).
pub(super) fn cells(start: CellCoord, end: CellCoord) -> Vec<CellCoord> {
    let (start, end) = if (start.column, start.row) <= (end.column, end.row) {
        (start, end)
    } else {
        (end, start)
    };
    let (mut x, mut y) = (i64::from(start.column), i64::from(start.row));
    let (end_x, end_y) = (i64::from(end.column), i64::from(end.row));
    let dx = (end_x - x).abs();
    let dy = -(end_y - y).abs();
    let sx = if x < end_x { 1 } else { -1 };
    let sy = if y < end_y { 1 } else { -1 };
    let mut error = dx + dy;
    let mut cells = Vec::new();
    loop {
        cells.push(CellCoord {
            column: x as u32,
            row: y as u32,
        });
        if x == end_x && y == end_y {
            break;
        }
        let twice = 2 * error;
        if twice >= dy {
            error += dy;
            x += sx;
        }
        if twice <= dx {
            error += dx;
            y += sy;
        }
    }
    cells
}

pub(super) fn candidate(
    current: &ValidatedDocument,
    start: CellCoord,
    end: CellCoord,
    region: Option<u32>,
    atlas: &SpriteDocument,
) -> Result<ValidatedDocument, String> {
    // Reject original endpoints before allocation or no-op shortcuts.
    current.cell(start)?;
    current.cell(end)?;
    let mut next = current.document().clone();
    for cell in cells(start, end) {
        next.cells[(cell.row * next.width + cell.column) as usize] = region;
    }
    next.validate(atlas)
}

#[cfg(test)]
mod tests;
