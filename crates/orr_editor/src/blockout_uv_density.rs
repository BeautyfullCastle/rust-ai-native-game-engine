//! Recipe-space texel-repeat density for the editor's fixed-topology boxes.
//!
//! These calculations use model/recipe units, before any model binding or scene
//! transform. They only propose scale values; callers explicitly apply those
//! values through the existing recipe history.
#![allow(clippy::float_arithmetic)]

use crate::blockout::{Face, Recipe};

/// Return the selected face's planar U/V scales for one texture repeat per
/// `units_per_repeat` recipe units. The source recipe is never changed.
///
/// The baked box's U/V edges are Y/Z on ±X, X/Z on ±Y, and X/Y on ±Z. Recipe UVs
/// are scaled before rotation, so quarter-turns must not swap these scales.
/// Values outside the existing recipe's 0.001..=16 scale range are rejected,
/// rather than clamped, since clamping would silently change the requested
/// density.
pub fn face_scale(recipe: &Recipe, face: Face, units_per_repeat: f32) -> Result<[f32; 2], String> {
    if !units_per_repeat.is_finite() || units_per_repeat <= 0.0 {
        return Err("Recipe units per repeat must be finite and greater than zero".into());
    }
    recipe.validate()?;

    let size =
        std::array::from_fn::<_, 3, _>(|axis| recipe.bounds.max[axis] - recipe.bounds.min[axis]);
    let [u, v] = match face {
        Face::PosX | Face::NegX => [size[1], size[2]],
        Face::PosY | Face::NegY => [size[0], size[2]],
        Face::PosZ | Face::NegZ => [size[0], size[1]],
    };
    let scale = [u / units_per_repeat, v / units_per_repeat];
    if !scale
        .iter()
        .all(|value| value.is_finite() && (0.001..=16.0).contains(value))
    {
        return Err(
            "Requested density requires UV scales outside 0.001..16; change recipe units per repeat"
                .into(),
        );
    }
    Ok(scale)
}
