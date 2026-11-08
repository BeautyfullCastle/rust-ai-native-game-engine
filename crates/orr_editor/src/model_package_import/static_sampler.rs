//! Immutable cooked-static sampler authoring through the established import pipeline.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StaticSamplerVariant {
    pub material: u32,
    pub linear_filter: bool,
    pub wrap_s: orr_model::Wrap,
    pub wrap_t: orr_model::Wrap,
    pub asset_id: String,
}

pub(crate) fn import_static_sampler_variant(
    request: Request,
    variant: StaticSamplerVariant,
) -> Result<Report, String> {
    import_static_sampler_variant_checked(request, variant, None)
}

pub(crate) fn import_static_sampler_variant_checked(
    request: Request,
    variant: StaticSamplerVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedStatic || request.kind != ModelKind::Static {
        return Err(
            "static sampler variant requires CookedStatic source and Static model mode".into(),
        );
    }
    validate_static_sampler_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::StaticSampler(variant)),
        expected_source_sha256,
    )
}

pub(crate) fn validate_static_sampler_variant(
    variant: &StaticSamplerVariant,
) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".sampler.json")?;
    if variant.material as usize >= orr_model::MAX_MATERIALS {
        return Err("static sampler material index exceeds the model material limit".into());
    }
    Ok(())
}

pub(super) fn static_sampler_variant_bytes(
    source: &[u8],
    variant: &StaticSamplerVariant,
) -> Result<Vec<u8>, String> {
    validate_static_sampler_variant(variant)?;
    fn wrap(value: orr_model::Wrap) -> &'static str {
        match value {
            orr_model::Wrap::Clamp => "clamp",
            orr_model::Wrap::Repeat => "repeat",
            orr_model::Wrap::Mirror => "mirror",
        }
    }
    let recipe = serde_json::to_vec(&serde_json::json!({
        "format": "orr_static_material_sampler", "version": 1,
        "source": "input.model.json", "material": variant.material,
        "linear_filter": variant.linear_filter,
        "wrap_s": wrap(variant.wrap_s), "wrap_t": wrap(variant.wrap_t),
    }))
    .map_err(|error| format!("static sampler recipe: {error}"))?;
    let model = orr_model::static_sampler::author_sampler(&recipe, &variant.asset_id, |path| {
        if path != "input.model.json" {
            return Err("unexpected static sampler dependency".into());
        }
        Ok(source.to_vec())
    })?;
    model
        .to_bytes()
        .map_err(|error| format!("static sampler variant cook: {error}"))
}

#[cfg(test)]
#[path = "static_sampler_tests.rs"]
mod tests;
