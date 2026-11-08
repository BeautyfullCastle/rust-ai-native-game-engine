//! Immutable animated sampler authoring through the established import pipeline.
use super::*;
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SamplerVariant {
    pub material: u32,
    pub linear_filter: bool,
    pub wrap_s: orr_model::Wrap,
    pub wrap_t: orr_model::Wrap,
    pub asset_id: String,
}
pub(crate) fn import_sampler_variant(
    request: Request,
    variant: SamplerVariant,
) -> Result<Report, String> {
    import_sampler_variant_checked(request, variant, None)
}
pub(crate) fn import_sampler_variant_checked(
    request: Request,
    variant: SamplerVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedAnimated || request.kind != ModelKind::Animated
    {
        return Err(
            "sampler variant requires CookedAnimated source and Animated model mode".into(),
        );
    }
    if !cfg!(feature = "room-animated-models") {
        return Err(
            "sampler variant requires room-animated-models; no input or destination opened".into(),
        );
    }
    validate_sampler_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::Sampler(variant)),
        expected_source_sha256,
    )
}
pub(super) fn validate_sampler_variant(variant: &SamplerVariant) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".sampler.json")?;
    if variant.material as usize >= orr_model::MAX_MATERIALS {
        return Err("sampler material index exceeds the model material limit".into());
    }
    Ok(())
}
pub(super) fn sampler_variant_bytes(
    source: &[u8],
    variant: &SamplerVariant,
) -> Result<Vec<u8>, String> {
    validate_sampler_variant(variant)?;
    #[cfg(feature = "room-animated-models")]
    {
        fn wrap(value: orr_model::Wrap) -> &'static str {
            match value {
                orr_model::Wrap::Clamp => "clamp",
                orr_model::Wrap::Repeat => "repeat",
                orr_model::Wrap::Mirror => "mirror",
            }
        }
        let recipe=serde_json::to_vec(&serde_json::json!({"format":"orr_animated_material_sampler","version":1,"source":"input.animated.json","material":variant.material,"linear_filter":variant.linear_filter,"wrap_s":wrap(variant.wrap_s),"wrap_t":wrap(variant.wrap_t)})).map_err(|e|format!("sampler recipe: {e}"))?;
        let model =
            orr_model::animated_sampler::author_sampler(&recipe, &variant.asset_id, |path| {
                if path != "input.animated.json" {
                    return Err("unexpected sampler dependency".into());
                }
                Ok(source.to_vec())
            })?;
        model
            .to_bytes()
            .map_err(|e| format!("sampler variant cook: {e}"))
    }
    #[cfg(not(feature = "room-animated-models"))]
    {
        let _ = source;
        Err("sampler variant requires room-animated-models".into())
    }
}
#[cfg(test)]
#[path = "sampler_tests.rs"]
mod tests;
