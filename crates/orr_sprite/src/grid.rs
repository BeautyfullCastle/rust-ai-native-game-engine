//! Bounded regular-grid authoring for a **new** sprite document. This module
//! never opens the atlas image or changes existing assets or bindings.
use crate::{
    invalid, Atlas, ClipSource, Error, Frame, PlaybackMode, Region, SpriteDocument, SpriteSource,
    FORMAT, MAX_ATLAS_DIMENSION, MAX_CLIP_ID_BYTES, MAX_FRAMES_PER_CLIP, MAX_IMAGE_NAME_BYTES,
    MAX_REGIONS, MAX_TOTAL_FRAMES, VERSION,
};

/// Explicit top-left pixel grid. Unused border pixels are allowed; requested
/// cells are never clipped or silently discarded. Spacing is between cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSpec {
    pub columns: u32,
    pub rows: u32,
    pub cell_width: u32,
    pub cell_height: u32,
    pub offset_x: u32,
    pub offset_y: u32,
    pub spacing_x: u32,
    pub spacing_y: u32,
    /// Consecutive IDs in this new document, left-to-right then top-to-bottom.
    pub first_region_id: u32,
}

/// One clip over a contiguous range of zero-based row-major cell indices.
/// `first_cell` is an index, not a region ID. Every frame has the same duration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridClip {
    pub name: String,
    pub first_cell: u32,
    pub frame_count: u32,
    pub duration_ms: u64,
    pub mode: PlaybackMode,
}

/// Generate all grid regions and one clip using the existing version-1 schema.
/// Checks names, dimensions, counts, rectangle extents, IDs, clip range and
/// duration arithmetic before allocating generated region or frame arrays.
/// The atlas image remains inert metadata. This does not preserve or reuse IDs
/// from an existing document and must not be used to re-slice bound assets.
pub fn generate(atlas: Atlas, grid: GridSpec, clip: GridClip) -> Result<SpriteDocument, Error> {
    if atlas.image.trim().is_empty() || atlas.image.len() > MAX_IMAGE_NAME_BYTES {
        return Err(invalid(
            "grid atlas image name is empty or exceeds byte limit",
        ));
    }
    if clip.name.trim().is_empty() || clip.name.len() > MAX_CLIP_ID_BYTES {
        return Err(invalid("grid clip name is empty or exceeds byte limit"));
    }
    if atlas.width == 0
        || atlas.height == 0
        || atlas.width > MAX_ATLAS_DIMENSION
        || atlas.height > MAX_ATLAS_DIMENSION
    {
        return Err(invalid("grid atlas dimensions outside schema limits"));
    }
    if grid.columns == 0 || grid.rows == 0 || grid.cell_width == 0 || grid.cell_height == 0 {
        return Err(invalid("grid counts and cell dimensions must be positive"));
    }
    let count = grid
        .columns
        .checked_mul(grid.rows)
        .filter(|&n| u64::from(n) <= MAX_REGIONS as u64)
        .ok_or_else(|| invalid("grid region count exceeds schema limit or overflows"))?;
    grid.first_region_id
        .checked_add(count - 1)
        .ok_or_else(|| invalid("grid region ID overflow"))?;
    fn edge(offset: u32, cells: u32, size: u32, spacing: u32) -> Option<u32> {
        cells
            .checked_mul(size)?
            .checked_add((cells - 1).checked_mul(spacing)?)?
            .checked_add(offset)
    }
    if edge(grid.offset_x, grid.columns, grid.cell_width, grid.spacing_x)
        .is_none_or(|x| x > atlas.width)
        || edge(grid.offset_y, grid.rows, grid.cell_height, grid.spacing_y)
            .is_none_or(|y| y > atlas.height)
    {
        return Err(invalid("grid cells exceed atlas bounds or overflow"));
    }
    if clip.frame_count == 0
        || u64::from(clip.frame_count) > MAX_FRAMES_PER_CLIP as u64
        || u64::from(clip.frame_count) > MAX_TOTAL_FRAMES as u64
        || clip
            .first_cell
            .checked_add(clip.frame_count)
            .is_none_or(|end| end > count)
    {
        return Err(invalid(
            "grid clip range is empty, outside grid or exceeds schema limit",
        ));
    }
    if clip.duration_ms == 0
        || clip
            .duration_ms
            .checked_mul(u64::from(clip.frame_count))
            .is_none()
    {
        return Err(invalid("grid clip duration is zero or overflows"));
    }

    // These bounded operations cannot overflow after the checks above. Keep
    // spacing separate from size so an unused gap on a singleton axis is valid.
    let regions = (0..count)
        .map(|i| {
            let column = i % grid.columns;
            let row = i / grid.columns;
            Region {
                id: grid.first_region_id + i,
                x: grid.offset_x + column * grid.cell_width + column * grid.spacing_x,
                y: grid.offset_y + row * grid.cell_height + row * grid.spacing_y,
                width: grid.cell_width,
                height: grid.cell_height,
            }
        })
        .collect();
    let frames = (clip.first_cell..clip.first_cell + clip.frame_count)
        .map(|i| Frame {
            region: grid.first_region_id + i,
            duration_ms: clip.duration_ms,
        })
        .collect();
    SpriteDocument::new(SpriteSource {
        format: FORMAT.into(),
        version: VERSION,
        atlas,
        regions,
        clips: vec![ClipSource {
            id: clip.name,
            mode: clip.mode,
            frames,
        }],
    })
}
