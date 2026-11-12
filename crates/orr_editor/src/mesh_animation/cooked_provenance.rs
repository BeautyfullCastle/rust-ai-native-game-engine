//! Cooked dependencies describe history, never resolver inputs or verified historical files.
use super::{AnimatedModel, ModelError};
use std::collections::BTreeSet;
pub(crate) fn is_cooked(asset: &str) -> bool {
    let name = asset.to_ascii_lowercase();
    name.ends_with(".animated.json") || name.ends_with(".orrmodel.json")
}
pub(crate) fn decode(asset: &str, bytes: &[u8]) -> Result<AnimatedModel, String> {
    let model = if is_cooked(asset) {
        AnimatedModel::from_bytes(bytes).map_err(|e| format!("cooked mesh animation: {e}"))?
    } else {
        orr_model::animation_import::import_with_resolver(asset, bytes, |_| {
            Err(ModelError::message(
                "mesh animation snapshots do not read external resources",
            ))
        })
        .map_err(|e| format!("import animated asset: {e}"))?
    };
    super::validate_model(&model)?;
    Ok(model)
}
fn safe_logical(path: &str) -> bool {
    if path.is_empty()
        || path.len() > 240
        || !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
    {
        return false;
    }
    path.split('/').all(|part| {
        let device = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        !part.is_empty()
            && !part.starts_with('.')
            && !part.ends_with('.')
            && !["CON", "PRN", "AUX", "NUL"].contains(&device.as_str())
            && !(device.len() == 4
                && (device.starts_with("COM") || device.starts_with("LPT"))
                && device.as_bytes()[3].is_ascii_digit())
    })
}
pub(super) fn validate_metadata(model: &AnimatedModel) -> Result<(), String> {
    let deps = &model.source().dependencies;
    if !(1..=3).contains(&deps.len()) || deps[0].uri != "$source" {
        return Err("cooked animation provenance requires one leading $source and at most two direct input records".into());
    }
    let mut seen = BTreeSet::new();
    for (index, dep) in deps.iter().enumerate() {
        if dep.sha256.len() != 64
            || !dep
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !seen.insert(dep.uri.to_ascii_lowercase())
            || (index > 0 && !safe_logical(&dep.uri))
        {
            return Err("cooked animation provenance needs unique safe logical paths and lowercase SHA-256 hashes".into());
        }
    }
    if deps.len() > 1 && !deps[1].uri.ends_with(".animated.json") {
        return Err(
            "cooked animation producer provenance requires an animated cooked source".into(),
        );
    }
    if deps.len() == 3 && !deps[2].uri.ends_with(".png") {
        return Err("cooked animation texture provenance requires a PNG record".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests;
