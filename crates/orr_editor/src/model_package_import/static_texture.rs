//! Immutable cooked-static PNG skin authoring through the established import pipeline.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StaticTextureVariant {
    pub material: u32,
    pub texture: PathBuf,
    pub asset_id: String,
}

pub(crate) fn import_static_texture_variant(
    request: Request,
    variant: StaticTextureVariant,
) -> Result<Report, String> {
    import_static_texture_variant_checked(request, variant, None)
}

pub(crate) fn import_static_texture_variant_checked(
    request: Request,
    variant: StaticTextureVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedStatic || request.kind != ModelKind::Static {
        return Err(
            "static texture variant requires CookedStatic source and Static model mode".into(),
        );
    }
    validate_static_texture_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::StaticTexture(variant)),
        expected_source_sha256,
    )
}

pub(crate) fn validate_static_texture_variant(
    variant: &StaticTextureVariant,
) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".retexture.json")?;
    if variant.material as usize >= orr_model::MAX_MATERIALS {
        return Err("static texture material index exceeds the model material limit".into());
    }
    validate_path_shape(&variant.texture, "static PNG texture")?;
    if !variant
        .texture
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|v| v.eq_ignore_ascii_case("png"))
    {
        return Err("static texture variant requires a .png file".into());
    }
    Ok(())
}

pub(super) fn static_texture_variant_bytes(
    source: &[u8],
    png: &[u8],
    variant: &StaticTextureVariant,
) -> Result<Vec<u8>, String> {
    validate_static_texture_variant(variant)?;
    let recipe = serde_json::to_vec(&serde_json::json!({
        "format": "orr_static_material_texture", "version": 1,
        "source": "input.model.json", "texture": "skin.png", "material": variant.material,
    }))
    .map_err(|error| format!("static texture recipe: {error}"))?;
    let model =
        orr_model::static_retexture::retexture_model(
            &recipe,
            &variant.asset_id,
            |path| match path {
                "input.model.json" => Ok(source.to_vec()),
                "skin.png" => Ok(png.to_vec()),
                _ => Err("unexpected static texture dependency".into()),
            },
        )?;
    model
        .to_bytes()
        .map_err(|error| format!("static texture variant cook: {error}"))
}

#[cfg(test)]
#[path = "static_texture_tests.rs"]
mod tests;
