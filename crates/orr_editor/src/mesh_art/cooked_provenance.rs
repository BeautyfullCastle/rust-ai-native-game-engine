//! Cooked dependencies describe history, never resolver inputs or verified historical files.
use super::{ModelSource, StaticModel};
use std::collections::BTreeSet;
pub(crate) fn is_cooked(asset: &str) -> bool {
    let name = asset.to_ascii_lowercase();
    name.ends_with(".model.json") || name.ends_with(".orrmodel.json") || name.ends_with(".orrmodel")
}
pub(crate) fn decode(asset: &str, bytes: &[u8]) -> Result<StaticModel, String> {
    let model = if is_cooked(asset) {
        StaticModel::from_bytes(bytes).map_err(|e| format!("cooked mesh static model: {e}"))?
    } else if asset.to_ascii_lowercase().ends_with(".glb") {
        orr_model::import::import_with_resolver(asset, bytes, |_| {
            Err(orr_model::Error::message(
                "mesh static model snapshots do not read external resources",
            ))
        })
        .map_err(|e| format!("import static asset: {e}"))?
    } else {
        return Err(
            "static model asset must be .glb, .model.json, .orrmodel.json or .orrmodel".into(),
        );
    };
    super::validate_source_budget(model.source())?;
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
pub(super) fn validate_metadata(source: &ModelSource) -> Result<(), String> {
    let deps = &source.dependencies;
    if !(1..=256).contains(&deps.len()) || deps[0].uri != "$source" {
        return Err("cooked static model provenance requires one leading $source and at most 255 direct input records".into());
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
            return Err("cooked static model provenance needs unique safe logical paths and lowercase SHA-256 hashes".into());
        }
    }
    Ok(())
}
#[cfg(test)]
pub(crate) mod tests;
