//! Immutable two-clip sequence authoring through the established import pipeline.
use super::*;
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SequenceVariant {
    pub first_clip: u32,
    pub second_clip: u32,
    pub transition_seconds: f64,
    pub name: String,
    pub asset_id: String,
}
pub(crate) fn import_sequence_variant(
    request: Request,
    variant: SequenceVariant,
) -> Result<Report, String> {
    import_sequence_variant_checked(request, variant, None)
}
pub(crate) fn import_sequence_variant_checked(
    request: Request,
    variant: SequenceVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedAnimated || request.kind != ModelKind::Animated
    {
        return Err(
            "sequence variant requires CookedAnimated source and Animated model mode".into(),
        );
    }
    if !cfg!(feature = "room-animated-models") {
        return Err(
            "sequence variant requires room-animated-models; no input or destination opened".into(),
        );
    }
    validate_sequence_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::Sequence(variant)),
        expected_source_sha256,
    )
}
pub(super) fn validate_sequence_variant(variant: &SequenceVariant) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".sequence.json")?;
    if variant.name.trim().is_empty()
        || variant.name.len() > 128
        || variant.name.chars().any(char::is_control)
    {
        return Err("sequence clip name must have 1..=128 UTF-8 bytes and no controls".into());
    }
    if !variant.transition_seconds.is_finite()
        || variant.transition_seconds <= 0.
        || variant.transition_seconds > 60.
        || variant.transition_seconds as f32 == 0.
    {
        return Err("sequence transition must be finite in (0,60] and positive as f32".into());
    }
    #[cfg(feature = "room-animated-models")]
    if variant.first_clip as usize >= orr_model::animation::MAX_CLIPS
        || variant.second_clip as usize >= orr_model::animation::MAX_CLIPS
    {
        return Err("sequence clip index exceeds the model clip limit".into());
    }
    Ok(())
}
pub(super) fn sequence_variant_bytes(
    source: &[u8],
    variant: &SequenceVariant,
) -> Result<Vec<u8>, String> {
    validate_sequence_variant(variant)?;
    #[cfg(feature = "room-animated-models")]
    {
        let recipe=serde_json::to_vec(&serde_json::json!({"format":"orr_animation_clip_sequence","version":1,"source":"input.animated.json","first_clip":variant.first_clip,"second_clip":variant.second_clip,"transition_seconds":variant.transition_seconds,"name":variant.name})).map_err(|e|format!("sequence recipe: {e}"))?;
        let model =
            orr_model::animation_sequence::sequence_clips(&recipe, &variant.asset_id, |path| {
                if path != "input.animated.json" {
                    return Err("unexpected sequence dependency".into());
                }
                Ok(source.to_vec())
            })?;
        model
            .to_bytes()
            .map_err(|e| format!("sequence variant cook: {e}"))
    }
    #[cfg(not(feature = "room-animated-models"))]
    {
        let _ = source;
        Err("sequence variant requires room-animated-models".into())
    }
}
#[cfg(test)]
#[path = "sequence_tests.rs"]
mod tests;
