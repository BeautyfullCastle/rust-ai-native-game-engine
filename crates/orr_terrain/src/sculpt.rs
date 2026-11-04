//! Bounded radial height stamps for deterministic terrain authoring.
use crate::{Edit, Error, Terrain, MAX_EDITS, MAX_SIDE};
use orr_fp::FP;

/// Produce one reversible edit batch without changing the source.
///
/// The center is a vertex index and radius is in grid intervals (world radius
/// is `radius * spacing`). The disk is clipped to the terrain boundary. Its
/// parabolic weight is `1 - distance_squared / radius_squared`, zero at the
/// edge. Each signed raw delta rounds once toward zero, making raise/lower
/// exact inverses in the absence of overflow. Holes remain holes.
///
/// The footprint budget counts every vertex strictly inside the disk, even
/// zero-delta vertices. Invalid input, budget excess or any height overflow
/// rejects the entire batch. The returned edits are row-major and changed-only.
pub fn radial_edits(
    terrain: &Terrain,
    center: [u32; 2],
    radius: u32,
    delta: FP,
) -> Result<Vec<Edit>, Error> {
    if center[0] >= terrain.width() || center[1] >= terrain.depth() {
        return Err(Error("sculpt center outside terrain"));
    }
    if radius == 0 || radius >= MAX_SIDE {
        return Err(Error(
            "sculpt radius must be between 1 and 128 grid intervals",
        ));
    }
    let min = [
        center[0].saturating_sub(radius),
        center[1].saturating_sub(radius),
    ];
    let max = [
        (center[0] + radius).min(terrain.width() - 1),
        (center[1] + radius).min(terrain.depth() - 1),
    ];
    let squared_radius = i128::from(radius) * i128::from(radius);
    let mut footprint = 0;
    let mut result = Vec::new();
    for z in min[1]..=max[1] {
        for x in min[0]..=max[0] {
            let dx = i128::from(x) - i128::from(center[0]);
            let dz = i128::from(z) - i128::from(center[1]);
            let weight = squared_radius - dx * dx - dz * dz;
            if weight <= 0 {
                continue;
            }
            footprint += 1;
            if footprint > MAX_EDITS {
                return Err(Error("sculpt footprint exceeds edit batch limit"));
            }
            // Radius is at most 128; i64 raw delta times weight fits i128.
            let offset = i128::from(delta.raw()) * weight / squared_radius;
            let old = terrain.heights()[(z * terrain.width() + x) as usize];
            let raw = i64::try_from(i128::from(old.raw()) + offset)
                .map_err(|_| Error("sculpt height overflow"))?;
            if raw != old.raw() {
                result.push(Edit::SetHeight {
                    x,
                    z,
                    height: FP::from_raw(raw),
                });
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
