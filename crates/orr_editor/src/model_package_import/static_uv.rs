//! Immutable cooked-static UV authoring through the established import pipeline.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StaticUvVariant {
    pub material: u32,
    pub offset: [f64; 2],
    pub scale: [f64; 2],
    pub quarter_turns: u32,
    pub asset_id: String,
}

pub(crate) fn import_static_uv_variant(
    request: Request,
    variant: StaticUvVariant,
) -> Result<Report, String> {
    import_static_uv_variant_checked(request, variant, None)
}

pub(crate) fn import_static_uv_variant_checked(
    request: Request,
    variant: StaticUvVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    // Admit the complete authoring request before opening any input or destination.
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedStatic || request.kind != ModelKind::Static {
        return Err("static UV variant requires CookedStatic source and Static model mode".into());
    }
    validate_static_uv_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::StaticUv(variant)),
        expected_source_sha256,
    )
}

fn validate_static_uv_variant(variant: &StaticUvVariant) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".uv.json")?;
    let bound = f64::from(f32::MAX);
    if variant.material as usize >= orr_model::MAX_MATERIALS
        || variant.quarter_turns > 3
        || !variant
            .offset
            .iter()
            .all(|v| v.is_finite() && v.abs() <= bound)
        || !variant
            .scale
            .iter()
            .all(|v| v.is_finite() && *v > 0.0 && *v <= bound && (*v as f32) > 0.0)
    {
        return Err("static UV variant requires an admitted material, finite f32 offset, positive f32 scale and quarter turns 0..3".into());
    }
    Ok(())
}

pub(super) fn static_uv_variant_bytes(
    source: &[u8],
    variant: &StaticUvVariant,
) -> Result<Vec<u8>, String> {
    validate_static_uv_variant(variant)?;
    let recipe = serde_json::to_vec(&serde_json::json!({
        "format": "orr_static_material_uv", "version": 1,
        "source": "input.model.json", "material": variant.material,
        "offset": variant.offset, "scale": variant.scale,
        "quarter_turns": variant.quarter_turns,
    }))
    .map_err(|error| format!("static UV recipe: {error}"))?;
    let model = orr_model::static_uv::transform_model_uv(&recipe, &variant.asset_id, |path| {
        if path != "input.model.json" {
            return Err("unexpected static UV dependency".into());
        }
        Ok(source.to_vec())
    })?;
    model
        .to_bytes()
        .map_err(|error| format!("static UV variant cook: {error}"))
}

#[cfg(test)]
#[path = "static_uv_tests.rs"]
mod tests;
