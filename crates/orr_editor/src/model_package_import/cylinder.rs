//! Source-less geometry creation with explicit package rights and immutable output.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CylinderSpec {
    pub radius: f64,
    pub height: f64,
    pub segments: u32,
    pub asset_id: String,
}

pub(crate) struct CylinderRequest {
    pub license: PathBuf,
    pub destination: PathBuf,
    pub name: String,
    pub version: String,
    pub license_acknowledged: bool,
    pub spec: CylinderSpec,
}

pub(crate) fn validate_cylinder_spec(spec: &CylinderSpec) -> Result<(), String> {
    validate_variant_identity(&spec.asset_id, ".cylinder.json")?;
    if ![spec.radius, spec.height]
        .iter()
        .all(|v| v.is_finite() && (0.001..=1_000_000.0).contains(v))
    {
        return Err(
            "cylinder radius and height must be finite f64 values within [0.001,1000000]".into(),
        );
    }
    if !matches!(spec.segments, 8 | 16 | 32 | 64 | 128) {
        return Err("cylinder segments must be 8, 16, 32, 64 or 128".into());
    }
    Ok(())
}

pub(crate) fn create_cylinder_package(request: CylinderRequest) -> Result<Report, String> {
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    validate_cylinder_spec(&request.spec)?;
    validate_path_shape(&request.license, "license source")?;
    validate_destination_policy(&request.destination)?;
    import_package_inner(
        Request {
            source: PathBuf::new(),
            license: request.license,
            destination: request.destination,
            name: request.name,
            version: request.version,
            kind: ModelKind::Static,
            source_format: SourceFormat::CookedStatic,
            license_acknowledged: request.license_acknowledged,
        },
        Some(AuthoringVariant::Cylinder(request.spec)),
        None,
    )
}

pub(super) fn cylinder_recipe_bytes(spec: &CylinderSpec) -> Result<Vec<u8>, String> {
    validate_cylinder_spec(spec)?;
    serde_json::to_vec(&serde_json::json!({
        "format": "orr_static_model_cylinder", "version": 1,
        "radius": spec.radius, "height": spec.height, "segments": spec.segments,
    }))
    .map_err(|error| format!("cylinder recipe: {error}"))
}

pub(super) fn cylinder_model_bytes(recipe: &[u8], asset_id: &str) -> Result<Vec<u8>, String> {
    orr_model::static_cylinder::author_cylinder(recipe, asset_id)?
        .to_bytes()
        .map_err(|error| format!("cylinder cook: {error}"))
}

#[cfg(test)]
#[path = "cylinder_tests.rs"]
mod tests;
