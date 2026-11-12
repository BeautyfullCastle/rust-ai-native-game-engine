//! Freeze an explicit animated clip sample into a new immutable static package.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PoseBakeVariant {
    pub clip: u32,
    pub time_seconds: f64,
    pub asset_id: String,
}

pub(crate) fn import_pose_bake_variant(
    request: Request,
    variant: PoseBakeVariant,
) -> Result<Report, String> {
    import_pose_bake_variant_checked(request, variant, None)
}

pub(crate) fn import_pose_bake_variant_checked(
    request: Request,
    variant: PoseBakeVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedAnimated || request.kind != ModelKind::Animated
    {
        return Err("pose bake requires CookedAnimated source and Animated model mode".into());
    }
    if !cfg!(feature = "room-animated-models") {
        return Err(
            "pose bake requires room-animated-models; no input or destination opened".into(),
        );
    }
    validate_pose_bake_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::PoseBake(variant)),
        expected_source_sha256,
    )
}

pub(crate) fn validate_pose_bake_variant(variant: &PoseBakeVariant) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".pose.json")?;
    let time = variant.time_seconds;
    if !time.is_finite()
        || time < 0.0
        || !(time as f32).is_finite()
        || (time > 0.0 && time as f32 == 0.0)
    {
        return Err("pose bake time must be finite, nonnegative and narrow to a finite nonzero f32 sample when positive".into());
    }
    Ok(())
}

pub(super) fn pose_bake_variant_bytes(
    source: &[u8],
    variant: &PoseBakeVariant,
) -> Result<Vec<u8>, String> {
    validate_pose_bake_variant(variant)?;
    #[cfg(feature = "room-animated-models")]
    {
        let recipe = serde_json::to_vec(&serde_json::json!({
            "format": "orr_animation_pose_bake", "version": 1,
            "source": "input.animated.json", "clip": variant.clip,
            "time": variant.time_seconds,
        }))
        .map_err(|error| format!("pose bake recipe: {error}"))?;
        let model = orr_model::pose_bake::bake_pose(&recipe, &variant.asset_id, |path| {
            if path != "input.animated.json" {
                return Err("unexpected pose bake dependency".into());
            }
            Ok(source.to_vec())
        })?;
        model
            .to_bytes()
            .map_err(|error| format!("pose bake cook: {error}"))
    }
    #[cfg(not(feature = "room-animated-models"))]
    {
        let _ = source;
        Err("pose bake requires room-animated-models".into())
    }
}

#[cfg(test)]
#[path = "pose_bake_tests.rs"]
mod tests;
