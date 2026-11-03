//! Authored RGB gain across the complete signed SH9 field; not a new bake.
use orr_render::irradiance::{IrradianceGrid, IrradianceProvenance, MAX_COEFFICIENT};

/// A bitwise no-op returns None so callers retain provenance, receipts and history.
/// No coefficient is clamped, and the supplied grid is never modified.
pub(super) fn apply_gain(
    source: &IrradianceGrid,
    gain: [f32; 3],
) -> Result<Option<IrradianceGrid>, String> {
    source.validate()?;
    if !gain
        .iter()
        .all(|value| value.is_finite() && (0.0..=16.0).contains(value))
    {
        return Err("Probe RGB gains must be finite and within 0..=16".into());
    }
    let mut result = source.clone();
    let mut changed = false;
    for (node, original_node) in result.coefficients.iter_mut().zip(&source.coefficients) {
        for (coefficient, original) in node.iter_mut().zip(original_node) {
            for channel in 0..3 {
                let value = f64::from(original[channel]) * f64::from(gain[channel]);
                if !value.is_finite() || value.abs() > f64::from(MAX_COEFFICIENT) {
                    return Err(
                        "Probe RGB gain exceeds the signed SH coefficient limit; nothing changed"
                            .into(),
                    );
                }
                coefficient[channel] = value as f32;
                changed |= coefficient[channel].to_bits() != original[channel].to_bits();
            }
        }
    }
    result.validate()?;
    if !changed {
        return Ok(None);
    }
    result.provenance = IrradianceProvenance::Authored;
    Ok(Some(result))
}

#[cfg(test)]
#[path = "color_gain/tests.rs"]
mod tests;
