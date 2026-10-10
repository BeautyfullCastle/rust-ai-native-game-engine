//! Discrete, integer-only sculpt stamps over an admitted terrain snapshot.
//! The circular footprint includes its boundary and clips at grid edges. Holes
//! keep their flags; hidden vertices remain editable just like individual ones.
use super::TerrainSession;
use orr_fp::FP;
use orr_terrain::{Edit, Terrain, MAX_EDITS};

/// A full radius-32 disc contains 3,209 vertices, below the core batch limit.
pub const MAX_BRUSH_RADIUS: u32 = 32;
pub const MAX_BRUSH_VERTICES: usize = 3209;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushOperation {
    /// Uniform signed height change. Negative values lower the footprint.
    RaiseLower { delta: FP },
    /// Set every footprint vertex to this absolute world height.
    Flatten { height: FP },
    /// One clipped 3×3 box-filter pass, including the vertex itself. Every mean
    /// reads the original snapshot and rounds toward negative infinity in Q16.
    /// Grid vertices beside or inside holes participate; hole flags never change.
    Smooth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainBrush {
    pub center: [u32; 2],
    /// Radius in grid steps, not world units. Zero edits only the center.
    pub radius: u32,
    pub operation: BrushOperation,
}

/// X-fast row order, no duplicates. Shared by the preview and edit path so the
/// displayed footprint cannot disagree with the set of authored vertices.
pub fn footprint(
    terrain: &Terrain,
    center: [u32; 2],
    radius: u32,
) -> Result<Vec<[u32; 2]>, String> {
    if radius > MAX_BRUSH_RADIUS {
        return Err(format!(
            "terrain brush radius must be 0–{MAX_BRUSH_RADIUS} grid steps"
        ));
    }
    let [cx, cz] = center;
    if cx >= terrain.width() || cz >= terrain.depth() {
        return Err("terrain brush center is outside the vertex grid".into());
    }
    // Validated terrain dimensions are at most 129, so index/radius additions
    // and squared distances below are bounded well inside u32.
    let mut vertices = Vec::new();
    for z in cz.saturating_sub(radius)..=(cz + radius).min(terrain.depth() - 1) {
        for x in cx.saturating_sub(radius)..=(cx + radius).min(terrain.width() - 1) {
            let dx = x.abs_diff(cx);
            let dz = z.abs_diff(cz);
            if dx * dx + dz * dz <= radius * radius {
                if vertices.len() >= MAX_BRUSH_VERTICES || vertices.len() >= MAX_EDITS {
                    return Err("terrain brush vertex limit".into());
                }
                vertices.push([x, z]);
            }
        }
    }
    Ok(vertices)
}

impl TerrainBrush {
    pub(super) fn edits(self, terrain: &Terrain) -> Result<Vec<Edit>, String> {
        footprint(terrain, self.center, self.radius)?
            .into_iter()
            .map(|[x, z]| {
                let height = match self.operation {
                    BrushOperation::RaiseLower { delta } => {
                        let raw = terrain.heights()[(z * terrain.width() + x) as usize]
                            .raw()
                            .checked_add(delta.raw())
                            .ok_or("terrain brush height overflow")?;
                        FP::from_raw(raw)
                    }
                    BrushOperation::Flatten { height } => height,
                    BrushOperation::Smooth => {
                        let mut sum = 0_i128;
                        let mut count = 0_i128;
                        for nz in z.saturating_sub(1)..=(z + 1).min(terrain.depth() - 1) {
                            for nx in x.saturating_sub(1)..=(x + 1).min(terrain.width() - 1) {
                                sum += i128::from(
                                    terrain.heights()[(nz * terrain.width() + nx) as usize].raw(),
                                );
                                count += 1;
                            }
                        }
                        // At most nine i64 heights; their i128 mean fits i64.
                        let raw = i64::try_from(sum.div_euclid(count))
                            .map_err(|_| "terrain brush mean overflow")?;
                        FP::from_raw(raw)
                    }
                };
                Ok(Edit::SetHeight { x, z, height })
            })
            .collect()
    }
}

impl TerrainSession {
    /// Validate and stage the whole stamp before publishing one core transaction.
    /// Mesh/query admission and history use the existing atomic edit path.
    pub fn apply_brush(&mut self, brush: TerrainBrush) -> Result<(), String> {
        self.require_editable()?;
        let edits = brush.edits(self.terrain().ok_or("open a terrain first")?)?;
        self.apply(&edits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terrain(side: u32, heights: Vec<FP>) -> Terrain {
        Terrain::new(
            "brush.orrt".into(),
            side,
            side,
            [FP::ZERO; 2],
            FP::ONE,
            heights,
            vec![false; ((side - 1) * (side - 1)) as usize],
        )
        .unwrap()
    }

    #[test]
    fn radial_footprint_clips_boundaries_and_has_an_explicit_work_cap() {
        let grid = terrain(129, vec![FP::ZERO; 129 * 129]);
        assert_eq!(
            footprint(&grid, [64, 64], 32).unwrap().len(),
            MAX_BRUSH_VERTICES
        );
        assert!(footprint(&grid, [64, 64], 32).unwrap().len() <= MAX_EDITS);
        assert_eq!(footprint(&grid, [0, 0], 0).unwrap(), [[0, 0]]);
        assert_eq!(
            footprint(&grid, [0, 0], 1).unwrap(),
            [[0, 0], [1, 0], [0, 1]]
        );
        assert_eq!(
            footprint(&grid, [64, 64], 1).unwrap(),
            [[64, 63], [63, 64], [64, 64], [65, 64], [64, 65]]
        );
        assert!(footprint(&grid, [0, 0], 33).is_err());
        assert!(footprint(&grid, [0, 0], u32::MAX).is_err());
        assert!(footprint(&grid, [u32::MAX, 0], 1).is_err());
        assert!(footprint(&grid, [0, 129], 1).is_err());
    }

    #[test]
    fn smoothing_uses_original_neighbors_clips_edges_and_floors_negative_means() {
        let grid = terrain(
            3,
            vec![
                FP::from_raw(-1),
                FP::ZERO,
                FP::ZERO,
                FP::ZERO,
                FP::from_raw(9),
                FP::ZERO,
                FP::ZERO,
                FP::ZERO,
                FP::ZERO,
            ],
        );
        let edits = TerrainBrush {
            center: [1, 1],
            radius: 2,
            operation: BrushOperation::Smooth,
        }
        .edits(&grid)
        .unwrap();
        let heights: Vec<i64> = edits
            .iter()
            .map(|edit| match edit {
                Edit::SetHeight { height, .. } => height.raw(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(heights, [2, 1, 2, 1, 0, 1, 2, 1, 2]);
        let negative = terrain(2, vec![FP::from_raw(-1), FP::ZERO, FP::ZERO, FP::ZERO]);
        assert_eq!(
            TerrainBrush {
                center: [0, 0],
                radius: 0,
                operation: BrushOperation::Smooth
            }
            .edits(&negative)
            .unwrap(),
            [Edit::SetHeight {
                x: 0,
                z: 0,
                height: FP::from_raw(-1)
            }]
        );
    }

    #[test]
    fn checked_delta_rejects_late_overflow_and_smoothing_handles_extreme_heights() {
        let high = terrain(
            2,
            vec![FP::ZERO, FP::from_raw(i64::MAX), FP::ZERO, FP::ZERO],
        );
        assert!(TerrainBrush {
            center: [0, 0],
            radius: 1,
            operation: BrushOperation::RaiseLower { delta: FP::ONE }
        }
        .edits(&high)
        .is_err());
        let low = terrain(2, vec![FP::from_raw(i64::MIN); 4]);
        assert!(TerrainBrush {
            center: [0, 0],
            radius: 1,
            operation: BrushOperation::RaiseLower {
                delta: FP::from_raw(-1)
            }
        }
        .edits(&low)
        .is_err());
        assert_eq!(
            TerrainBrush {
                center: [0, 0],
                radius: 0,
                operation: BrushOperation::Smooth
            }
            .edits(&low)
            .unwrap(),
            [Edit::SetHeight {
                x: 0,
                z: 0,
                height: FP::from_raw(i64::MIN)
            }]
        );
    }
}
