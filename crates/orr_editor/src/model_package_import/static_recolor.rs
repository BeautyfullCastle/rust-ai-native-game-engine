//! Immutable cooked-static RGB factor authoring through the import pipeline.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StaticColorVariant {
    pub material: u32,
    pub linear_rgb: [f64; 3],
    pub asset_id: String,
}

pub(crate) fn import_static_color_variant(
    request: Request,
    variant: StaticColorVariant,
) -> Result<Report, String> {
    import_static_color_variant_checked(request, variant, None)
}

pub(crate) fn import_static_color_variant_checked(
    request: Request,
    variant: StaticColorVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedStatic || request.kind != ModelKind::Static {
        return Err(
            "static color variant requires CookedStatic source and Static model mode".into(),
        );
    }
    validate_static_color_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::StaticColor(variant)),
        expected_source_sha256,
    )
}

pub(crate) fn validate_static_color_variant(variant: &StaticColorVariant) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".recolor.json")?;
    if variant.material as usize >= orr_model::MAX_MATERIALS {
        return Err("static color material index exceeds the model material limit".into());
    }
    // Validate the original f64 values: f32 rounding must not rescue an input
    // below zero or above one. Preserve those admitted values in recipe bytes.
    if !variant
        .linear_rgb
        .iter()
        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    {
        return Err("linear_rgb must contain three finite linear-light values in 0..=1".into());
    }
    Ok(())
}

pub(super) fn static_color_variant_bytes(
    source: &[u8],
    variant: &StaticColorVariant,
) -> Result<Vec<u8>, String> {
    validate_static_color_variant(variant)?;
    let recipe = serde_json::to_vec(&serde_json::json!({
        "format": "orr_static_material_color", "version": 1,
        "source": "input.model.json", "material": variant.material,
        "linear_rgb": variant.linear_rgb,
    }))
    .map_err(|error| format!("static color recipe: {error}"))?;
    let model = orr_model::static_recolor::recolor_model(&recipe, &variant.asset_id, |path| {
        if path != "input.model.json" {
            return Err("unexpected static color dependency".into());
        }
        Ok(source.to_vec())
    })?;
    model
        .to_bytes()
        .map_err(|error| format!("static color variant cook: {error}"))
}

#[cfg(test)]
#[path = "static_recolor_tests.rs"]
mod tests;
