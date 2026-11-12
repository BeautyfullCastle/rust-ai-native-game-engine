//! Immutable animated UV authoring through the established import pipeline.
use super::*;
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UvVariant {
    pub material: u32,
    pub offset: [f64; 2],
    pub scale: [f64; 2],
    pub quarter_turns: u32,
    pub asset_id: String,
}
pub(crate) fn import_uv_variant(request: Request, variant: UvVariant) -> Result<Report, String> {
    import_uv_variant_checked(request, variant, None)
}
pub(crate) fn import_uv_variant_checked(
    request: Request,
    variant: UvVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedAnimated || request.kind != ModelKind::Animated
    {
        return Err("UV variant requires CookedAnimated source and Animated model mode".into());
    }
    if !cfg!(feature = "room-animated-models") {
        return Err(
            "UV variant requires room-animated-models; no input or destination opened".into(),
        );
    }
    validate_uv_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::Uv(variant)),
        expected_source_sha256,
    )
}
pub(super) fn validate_uv_variant(variant: &UvVariant) -> Result<(), String> {
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
            .all(|v| v.is_finite() && *v > 0. && *v <= bound && (*v as f32) > 0.)
    {
        return Err("UV variant requires an admitted material, finite f32 offset, positive f32 scale and quarter turns 0..3".into());
    }
    Ok(())
}
pub(super) fn uv_variant_bytes(source: &[u8], variant: &UvVariant) -> Result<Vec<u8>, String> {
    validate_uv_variant(variant)?;
    #[cfg(feature = "room-animated-models")]
    {
        let recipe=serde_json::to_vec(&serde_json::json!({"format":"orr_animated_material_uv","version":1,"source":"input.animated.json","material":variant.material,"offset":variant.offset,"scale":variant.scale,"quarter_turns":variant.quarter_turns})).map_err(|e|format!("UV recipe: {e}"))?;
        let model =
            orr_model::animated_uv::transform_model_uv(&recipe, &variant.asset_id, |path| {
                if path != "input.animated.json" {
                    return Err("unexpected UV dependency".into());
                }
                Ok(source.to_vec())
            })?;
        model
            .to_bytes()
            .map_err(|e| format!("UV variant cook: {e}"))
    }
    #[cfg(not(feature = "room-animated-models"))]
    {
        let _ = source;
        Err("UV variant requires room-animated-models".into())
    }
}
#[cfg(test)]
#[path = "uv_tests.rs"]
mod tests;
