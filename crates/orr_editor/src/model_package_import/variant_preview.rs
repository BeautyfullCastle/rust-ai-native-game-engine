//! Read-only authoring preview from captured direct inputs. Never publishes or installs.
use super::*;
use orr_model::animation::AnimatedModel;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Spec {
    Color(ColorVariant),
    Texture(TextureVariant),
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Request {
    pub source: PathBuf,
    pub spec: Spec,
    pub expected_source_sha256: Option<String>,
}
#[derive(Clone, Debug)]
pub(crate) struct Prepared {
    pub model: Arc<AnimatedModel>,
    pub asset_sha256: String,
    pub source_sha256: String,
    pub png_sha256: Option<String>,
}
fn validate(request: &Request) -> Result<(), String> {
    validate_expected_source_hash(request.expected_source_sha256.as_deref())?;
    match &request.spec {
        Spec::Color(spec) => validate_color_variant(spec)?,
        Spec::Texture(spec) => validate_texture_variant(spec)?,
    }
    validate_path_shape(&request.source, "animated preview source")?;
    if !request
        .source
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.to_ascii_lowercase().ends_with(".animated.json"))
    {
        return Err("animated preview source must be a .animated.json file".into());
    }
    Ok(())
}
pub(crate) fn prepare(request: Request) -> Result<Prepared, String> {
    validate(&request)?;
    let path = checked_file_path(&request.source, "animated preview source")?;
    let source = read_bounded_file(&path, MAX_SOURCE_BYTES, "animated preview source")?;
    // Reject stale inspected sources before reading even a requested PNG.
    check_source_hash(&request, &source.digest)?;
    let png = match &request.spec {
        Spec::Texture(spec) => {
            let path = checked_file_path(&spec.texture, "preview PNG texture")?;
            Some(read_bounded_file(
                &path,
                MAX_SOURCE_BYTES,
                "preview PNG texture",
            )?)
        }
        Spec::Color(_) => None,
    };
    from_captured(
        &request,
        &source.bytes,
        png.as_ref().map(|p| p.bytes.as_slice()),
    )
}
fn check_source_hash(request: &Request, actual: &str) -> Result<(), String> {
    if request
        .expected_source_sha256
        .as_deref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(actual))
    {
        return Err("inspected animated source changed; inspect again before previewing".into());
    }
    Ok(())
}
fn from_captured(request: &Request, source: &[u8], png: Option<&[u8]>) -> Result<Prepared, String> {
    validate(request)?;
    let source_sha256 = digest(source);
    check_source_hash(request, &source_sha256)?;
    let bytes = match &request.spec {
        Spec::Color(spec) => color_variant_bytes(source, spec)?,
        Spec::Texture(spec) => {
            texture_variant_bytes(source, png.ok_or("preview PNG snapshot missing")?, spec)?
        }
    };
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err("preview cooked output exceeds 64 MiB limit".into());
    }
    let model =
        AnimatedModel::from_bytes(&bytes).map_err(|e| format!("preview cooked model: {e}"))?;
    Ok(Prepared {
        model: Arc::new(model),
        asset_sha256: digest(&bytes),
        source_sha256,
        png_sha256: match &request.spec {
            Spec::Color(_) => None,
            Spec::Texture(_) => png.map(digest),
        },
    })
}
#[cfg(test)]
#[path = "variant_preview_tests.rs"]
mod tests;
