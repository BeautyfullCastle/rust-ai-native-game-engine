use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};

const MAX_COORDINATE_RAW: i64 = 256 * FP::SCALE;
const MAX_GRID_CELLS: u32 = 49;
const MAX_GRAPH_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Grid {
    pub origin: [FP; 3],
    pub width: FP,
    pub depth: FP,
    pub columns: u32,
    pub rows: u32,
    pub excluded_cells: u64,
}

pub(crate) fn create(grid: Grid, profile: AgentProfile) -> Result<StaticMeshGraph, String> {
    validate_grid(grid)?;
    let cell_count = grid
        .columns
        .checked_mul(grid.rows)
        .ok_or_else(|| "grid cell count overflow".to_owned())?;
    let valid_mask = (1_u64 << cell_count) - 1;
    if grid.excluded_cells & !valid_mask != 0 {
        return Err("excluded-cell mask contains bits outside the grid".into());
    }
    let active_cells = cell_count - (grid.excluded_cells & valid_mask).count_ones();
    if active_cells == 0 {
        return Err("grid must contain at least one walkable cell".into());
    }

    let vertex_columns = grid.columns + 1;
    let vertex_rows = grid.rows + 1;
    let vertex_count = vertex_columns
        .checked_mul(vertex_rows)
        .ok_or_else(|| "grid vertex count overflow".to_owned())? as usize;
    let mut source_vertices = Vec::with_capacity(vertex_count);
    for row in 0..=grid.rows {
        let coordinate_z = interpolate(grid.origin[2], grid.depth, row, grid.rows)?;
        for column in 0..=grid.columns {
            let coordinate_x = interpolate(grid.origin[0], grid.width, column, grid.columns)?;
            source_vertices.push([coordinate_x, grid.origin[1], coordinate_z]);
        }
    }

    let mut used_vertex_ids = vec![None; vertex_count];
    let mut vertices = Vec::with_capacity(vertex_count);
    let mut faces = Vec::with_capacity(active_cells as usize * 2);
    for row in 0..grid.rows {
        for column in 0..grid.columns {
            let cell = row * grid.columns + column;
            if grid.excluded_cells & (1_u64 << cell) != 0 {
                continue;
            }
            let near_x = row * vertex_columns + column;
            let far_x = near_x + 1;
            let near_z = near_x + vertex_columns;
            let far_corner = near_z + 1;
            let near_x = map_vertex(
                near_x,
                &source_vertices,
                &mut used_vertex_ids,
                &mut vertices,
            )?;
            let far_x = map_vertex(far_x, &source_vertices, &mut used_vertex_ids, &mut vertices)?;
            let near_z = map_vertex(
                near_z,
                &source_vertices,
                &mut used_vertex_ids,
                &mut vertices,
            )?;
            let far_corner = map_vertex(
                far_corner,
                &source_vertices,
                &mut used_vertex_ids,
                &mut vertices,
            )?;
            faces.push([near_x, near_z, far_corner]);
            faces.push([near_x, far_corner, far_x]);
        }
    }

    let graph = StaticMeshGraph::bake(&vertices, &faces, profile)
        .map_err(|error| format!("grid mesh admission failed: {error}"))?;
    if graph.to_bytes().len() > MAX_GRAPH_BYTES {
        return Err("generated grid exceeds the 8 KiB mesh asset limit".into());
    }
    Ok(graph)
}

fn validate_grid(grid: Grid) -> Result<(), String> {
    if !(1..=7).contains(&grid.columns) || !(1..=7).contains(&grid.rows) {
        return Err("grid columns and rows must be between 1 and 7".into());
    }
    if grid.width.raw() <= 0 || grid.depth.raw() <= 0 {
        return Err("grid width and depth must be positive".into());
    }
    if grid.width.raw() < i64::from(grid.columns) || grid.depth.raw() < i64::from(grid.rows) {
        return Err("grid dimensions are too small for distinct fixed-point cells".into());
    }
    for coordinate in grid.origin {
        validate_coordinate(coordinate)?;
    }
    validate_coordinate(interpolate(
        grid.origin[0],
        grid.width,
        grid.columns,
        grid.columns,
    )?)?;
    validate_coordinate(interpolate(
        grid.origin[2],
        grid.depth,
        grid.rows,
        grid.rows,
    )?)?;
    let cell_count = grid
        .columns
        .checked_mul(grid.rows)
        .ok_or_else(|| "grid cell count overflow".to_owned())?;
    if cell_count > MAX_GRID_CELLS {
        return Err("grid cell limit exceeded".into());
    }
    Ok(())
}

fn interpolate(origin: FP, length: FP, step: u32, steps: u32) -> Result<FP, String> {
    if step > steps || steps == 0 {
        return Err("invalid grid interpolation index".into());
    }
    let length_raw = i128::from(length.raw());
    let offset = length_raw
        .checked_mul(i128::from(step))
        .ok_or_else(|| "grid coordinate multiplication overflow".to_owned())?
        / i128::from(steps);
    let raw = i128::from(origin.raw())
        .checked_add(offset)
        .ok_or_else(|| "grid coordinate addition overflow".to_owned())?;
    let raw = i64::try_from(raw).map_err(|_| "grid coordinate exceeds fixed-point range")?;
    Ok(FP::from_raw(raw))
}

fn map_vertex(
    source_index: u32,
    source_vertices: &[[FP; 3]],
    mapped_indices: &mut [Option<u32>],
    vertices: &mut Vec<[FP; 3]>,
) -> Result<u32, String> {
    let index = source_index as usize;
    if let Some(mapped) = mapped_indices
        .get(index)
        .ok_or_else(|| "grid vertex index is invalid".to_owned())?
    {
        return Ok(*mapped);
    }
    let position = *source_vertices
        .get(index)
        .ok_or_else(|| "grid vertex index is invalid".to_owned())?;
    for coordinate in position {
        validate_coordinate(coordinate)?;
    }
    let mapped = u32::try_from(vertices.len()).map_err(|_| "grid vertex limit exceeded")?;
    vertices.push(position);
    mapped_indices[index] = Some(mapped);
    Ok(mapped)
}

fn validate_coordinate(coordinate: FP) -> Result<(), String> {
    if i128::from(coordinate.raw()).abs() > i128::from(MAX_COORDINATE_RAW) {
        return Err("grid coordinates must remain within ±256 units".into());
    }
    Ok(())
}

#[path = "geometry_grid/tests.rs"]
#[cfg(test)]
mod tests;
