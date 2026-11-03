//! Panel-local immutable cell snapshots; never an operating-system clipboard.
use orr_sprite::SpriteDocument;
use orr_tilemap::{CellCoord, ValidatedDocument};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Geometry {
    sprite: (String, String),
    dimensions: [u32; 2],
    tile_size: [u32; 2],
    origin: [u32; 2],
    units: u32,
}
impl Geometry {
    fn of(source: &ValidatedDocument) -> Self {
        let d = source.document();
        Self {
            sprite: (d.sprite.package.clone(), d.sprite.document.clone()),
            dimensions: [d.width, d.height],
            tile_size: d.tile_size,
            origin: d.origin.map(f32::to_bits),
            units: d.units_per_pixel.to_bits(),
        }
    }
}
/// Rearranges cells only: tile image orientation and source geometry are unchanged.
#[derive(Clone, Copy, Debug)]
pub(super) enum LayoutTransform {
    Clockwise,
    Horizontal,
    Vertical,
}
#[derive(Clone, Debug)]
pub(super) struct Pattern {
    pub(super) width: u32,
    pub(super) height: u32,
    cells: Vec<Option<u32>>,
    geometry: Geometry,
    epoch: u64,
    atlas: String,
}
impl Pattern {
    pub(super) fn capture(
        source: &ValidatedDocument,
        start: CellCoord,
        end: CellCoord,
        epoch: u64,
        atlas: &SpriteDocument,
    ) -> Result<Self, String> {
        source.cell(start)?;
        source.cell(end)?;
        let width = start.column.abs_diff(end.column) + 1;
        let height = start.row.abs_diff(end.row) + 1;
        let count = width.checked_mul(height).ok_or("pattern size overflow")? as usize;
        if count > orr_tilemap::MAX_CELLS {
            return Err("pattern exceeds 4096 cells".into());
        }
        let mut cells = Vec::with_capacity(count);
        for row in start.row.min(end.row)..=start.row.max(end.row) {
            for column in start.column.min(end.column)..=start.column.max(end.column) {
                cells.push(source.cell(CellCoord { column, row })?);
            }
        }
        // Validate even an all-empty capture against the current atlas.
        source.document().validate(atlas)?;
        Ok(Self {
            width,
            height,
            cells,
            geometry: Geometry::of(source),
            epoch,
            atlas: atlas.to_json().map_err(|e| e.to_string())?,
        })
    }
    pub(super) fn transform(&mut self, transform: LayoutTransform) {
        // Captured dimensions are nonzero and their product is at most MAX_CELLS.
        // Always read the immutable old layout, including empty erasing cells.
        let (width, height) = match transform {
            LayoutTransform::Clockwise => (self.height, self.width),
            _ => (self.width, self.height),
        };
        let mut cells = vec![None; self.cells.len()];
        for row in 0..self.height {
            for column in 0..self.width {
                let (x, y) = match transform {
                    LayoutTransform::Clockwise => (self.height - 1 - row, column),
                    LayoutTransform::Horizontal => (self.width - 1 - column, row),
                    LayoutTransform::Vertical => (column, self.height - 1 - row),
                };
                cells[(y * width + x) as usize] = self.cells[(row * self.width + column) as usize];
            }
        }
        self.width = width;
        self.height = height;
        self.cells = cells;
    }
    pub(super) fn geometry_matches(&self, source: &ValidatedDocument) -> bool {
        self.geometry == Geometry::of(source)
    }
    pub(super) fn context_matches(&self, source: &ValidatedDocument, epoch: u64) -> bool {
        self.epoch == epoch && self.geometry_matches(source)
    }
    pub(super) fn matches(
        &self,
        source: &ValidatedDocument,
        epoch: u64,
        atlas: &SpriteDocument,
    ) -> bool {
        self.context_matches(source, epoch) && atlas.to_json().is_ok_and(|json| json == self.atlas)
    }
    pub(super) fn end(
        &self,
        source: &ValidatedDocument,
        anchor: CellCoord,
    ) -> Result<CellCoord, String> {
        source.cell(anchor)?;
        let end = CellCoord {
            column: anchor
                .column
                .checked_add(self.width - 1)
                .ok_or("stamp column overflow")?,
            row: anchor
                .row
                .checked_add(self.height - 1)
                .ok_or("stamp row overflow")?,
        };
        source
            .cell(end)
            .map_err(|_| "whole stamp must fit inside the map".to_string())?;
        Ok(end)
    }
    pub(super) fn candidate(
        &self,
        source: &ValidatedDocument,
        anchor: CellCoord,
        epoch: u64,
        atlas: &SpriteDocument,
    ) -> Result<ValidatedDocument, String> {
        if !self.matches(source, epoch, atlas) {
            return Err("copied pattern expired; Copy again".into());
        }
        self.end(source, anchor)?;
        let mut next = source.document().clone();
        for row in 0..self.height {
            for column in 0..self.width {
                next.cells[((anchor.row + row) * next.width + anchor.column + column) as usize] =
                    self.cells[(row * self.width + column) as usize];
            }
        }
        next.validate(atlas)
    }
}
#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;
