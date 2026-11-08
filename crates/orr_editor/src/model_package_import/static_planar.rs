//! Immutable cooked-static planar UV authoring through the import pipeline.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StaticPlanarUvVariant {
    pub material: u32,
    pub plane: String,
    pub origin: [f64; 3],
    pub units_per_repeat: [f64; 2],
    pub asset_id: String,
}

pub(crate) fn import_static_planar_uv_variant(
    request: Request,
    variant: StaticPlanarUvVariant,
) -> Result<Report, String> {
    import_static_planar_uv_variant_checked(request, variant, None)
}

pub(crate) fn import_static_planar_uv_variant_checked(
    request: Request,
    variant: StaticPlanarUvVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedStatic || request.kind != ModelKind::Static {
        return Err(
            "static planar UV variant requires CookedStatic source and Static model mode".into(),
        );
    }
    validate_static_planar_uv_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::StaticPlanarUv(variant)),
        expected_source_sha256,
    )
}

pub(crate) fn validate_static_planar_uv_variant(
    variant: &StaticPlanarUvVariant,
) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".planar-uv.json")?;
    if variant.material as usize >= orr_model::MAX_MATERIALS {
        return Err("static planar UV material index exceeds the model material limit".into());
    }
    if !["xy", "xz", "yz"].contains(&variant.plane.as_str()) {
        return Err("static planar UV plane must be xy, xz or yz".into());
    }
    if !variant
        .origin
        .iter()
        .all(|v| v.is_finite() && (*v as f32).is_finite())
    {
        return Err("static planar UV origin requires three finite f32 values".into());
    }
    if !variant
        .units_per_repeat
        .iter()
        .all(|v| v.is_finite() && *v > 0.0 && (*v as f32).is_finite() && (*v as f32) > 0.0)
    {
        return Err(
            "static planar UV units_per_repeat requires two positive finite nonzero f32 values"
                .into(),
        );
    }
    Ok(())
}

pub(super) fn static_planar_uv_variant_bytes(
    source: &[u8],
    variant: &StaticPlanarUvVariant,
) -> Result<Vec<u8>, String> {
    validate_static_planar_uv_variant(variant)?;
    // Retain admitted original f64 recipe values. The unchanged producer owns
    // f32 decode/rounding and projection against model-space vertex positions.
    let recipe = serde_json::to_vec(&serde_json::json!({
        "format": "orr_static_planar_uv", "version": 1,
        "source": "input.model.json", "material": variant.material,
        "origin": variant.origin, "units_per_repeat": variant.units_per_repeat,
        "plane": variant.plane,
    }))
    .map_err(|error| format!("static planar UV recipe: {error}"))?;
    let model =
        orr_model::static_planar_uv::project_model_uv(&recipe, &variant.asset_id, |path| {
            if path != "input.model.json" {
                return Err("unexpected static planar UV dependency".into());
            }
            Ok(source.to_vec())
        })?;
    model
        .to_bytes()
        .map_err(|error| format!("static planar UV variant cook: {error}"))
}

#[cfg(test)]
#[path = "static_planar_tests.rs"]
mod tests;
