//! Immutable cooked-static pivot authoring through the established import pipeline.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StaticPivotVariant {
    pub pivot: [f64; 3],
    pub asset_id: String,
}

pub(crate) fn import_static_pivot_variant(
    request: Request,
    variant: StaticPivotVariant,
) -> Result<Report, String> {
    import_static_pivot_variant_checked(request, variant, None)
}

pub(crate) fn import_static_pivot_variant_checked(
    request: Request,
    variant: StaticPivotVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedStatic || request.kind != ModelKind::Static {
        return Err(
            "static pivot variant requires CookedStatic source and Static model mode".into(),
        );
    }
    validate_static_pivot_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::StaticPivot(variant)),
        expected_source_sha256,
    )
}

pub(crate) fn validate_static_pivot_variant(variant: &StaticPivotVariant) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".pivot.json")?;
    // The existing recipe parses f32 before checking its bound. Retain that
    // rounding contract, while rejecting nonfinite inputs before serialization.
    if !variant.pivot.iter().all(|value| {
        value.is_finite() && (*value as f32).is_finite() && (*value as f32).abs() <= 1.0e6
    }) {
        return Err("static pivot requires three finite f32 values within +/-1000000".into());
    }
    Ok(())
}

pub(super) fn static_pivot_variant_bytes(
    source: &[u8],
    variant: &StaticPivotVariant,
) -> Result<Vec<u8>, String> {
    validate_static_pivot_variant(variant)?;
    let pivot = variant.pivot.map(|value| value as f32);
    let recipe = serde_json::to_vec(&serde_json::json!({
        "format": "orr_static_model_pivot", "version": 1,
        "source": "input.model.json", "pivot": pivot,
    }))
    .map_err(|error| format!("static pivot recipe: {error}"))?;
    let model = orr_model::static_pivot::rebase_model_pivot(&recipe, &variant.asset_id, |path| {
        if path != "input.model.json" {
            return Err("unexpected static pivot dependency".into());
        }
        Ok(source.to_vec())
    })?;
    model
        .to_bytes()
        .map_err(|error| format!("static pivot variant cook: {error}"))
}

#[cfg(test)]
#[path = "static_pivot_tests.rs"]
mod tests;
