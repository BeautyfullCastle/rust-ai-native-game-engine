use super::{fixed, Template, BODY, COLLIDER};
use orr_fp::FP;
use serde_json::Value;

impl Template {
    /// Place a copy in the X/Z plane without changing canonical static physics.
    /// A positive quarter-turn maps `(x, z)` to `(-z, x)` around `pivot`,
    /// followed by `offset`. Only 0, 1, 2, and 3 quarter-turns are accepted.
    pub(crate) fn placed(
        &self,
        quarter_turns: u8,
        pivot: [FP; 2],
        offset: [FP; 2],
    ) -> Result<Self, String> {
        if quarter_turns > 3 {
            return Err("wall stamp quarter-turns must be in 0..=3".into());
        }
        self.encode()?;
        let mut placed = self.clone();
        for wall in &mut placed.walls {
            // Validation above establishes the complete component structure.
            let position = wall.components[BODY]["pos"]
                .as_array_mut()
                .ok_or("wall Body.pos must be a vector")?;
            let center = [fixed(&position[0])?, fixed(&position[2])?];
            let transformed = place_center(center, quarter_turns, pivot, offset)?;
            for (index, before, after) in [
                (0, center[0], transformed[0]),
                (2, center[1], transformed[1]),
            ] {
                // Preserve exact original JSON tokens for unchanged coordinates.
                if before != after {
                    position[index] =
                        serde_json::from_str::<Value>(&orr_reflect::decimal::fp_to_decimal(after))
                            .map_err(|error| format!("wall stamp coordinate encode: {error}"))?;
                }
            }
            if quarter_turns % 2 == 1 {
                wall.components[COLLIDER]["shape"]["half_extents"]
                    .as_array_mut()
                    .ok_or("wall half extents must be a vector")?
                    .swap(0, 2);
            }
        }
        // Reject the entire placement if any wall or the serialized size is invalid.
        placed.encode()?;
        Ok(placed)
    }
}

fn place_center(
    center: [FP; 2],
    quarter_turns: u8,
    pivot: [FP; 2],
    offset: [FP; 2],
) -> Result<[FP; 2], String> {
    const OVERFLOW: &str = "wall stamp placement exceeds fixed-point bounds";
    // Intermediate differences may exceed i64 even when the final position fits.
    let dx = i128::from(center[0].raw())
        .checked_sub(i128::from(pivot[0].raw()))
        .ok_or(OVERFLOW)?;
    let dz = i128::from(center[1].raw())
        .checked_sub(i128::from(pivot[1].raw()))
        .ok_or(OVERFLOW)?;
    let rotated = match quarter_turns {
        0 => [dx, dz],
        1 => [dz.checked_neg().ok_or(OVERFLOW)?, dx],
        2 => [
            dx.checked_neg().ok_or(OVERFLOW)?,
            dz.checked_neg().ok_or(OVERFLOW)?,
        ],
        3 => [dz, dx.checked_neg().ok_or(OVERFLOW)?],
        _ => return Err("wall stamp quarter-turns must be in 0..=3".into()),
    };
    let mut result = [FP::ZERO; 2];
    for axis in 0..2 {
        let raw = i128::from(pivot[axis].raw())
            .checked_add(rotated[axis])
            .and_then(|value| value.checked_add(i128::from(offset[axis].raw())))
            .ok_or(OVERFLOW)?;
        result[axis] = FP(i64::try_from(raw).map_err(|_| OVERFLOW)?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
