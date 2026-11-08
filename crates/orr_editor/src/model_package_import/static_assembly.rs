//! Exactly two placements of one immutable cooked source and its supplied license.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StaticAssemblyPart {
    pub offset: [f64; 3],
    pub scale: [f64; 3],
    pub quarter_turns: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StaticAssemblyVariant {
    pub parts: [StaticAssemblyPart; 2],
    pub asset_id: String,
}

pub(crate) fn import_static_assembly_variant(
    request: Request,
    variant: StaticAssemblyVariant,
) -> Result<Report, String> {
    import_static_assembly_variant_checked(request, variant, None)
}

pub(crate) fn import_static_assembly_variant_checked(
    request: Request,
    variant: StaticAssemblyVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedStatic || request.kind != ModelKind::Static {
        return Err("static assembly requires CookedStatic source and Static model mode".into());
    }
    validate_static_assembly_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::StaticAssembly(variant)),
        expected_source_sha256,
    )
}

pub(crate) fn validate_static_assembly_variant(
    variant: &StaticAssemblyVariant,
) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".assemble.json")?;
    for part in &variant.parts {
        part_placement(part)?;
    }
    Ok(())
}

fn part_placement(part: &StaticAssemblyPart) -> Result<[[f32; 4]; 4], String> {
    let bound = f64::from(f32::MAX);
    if part.quarter_turns > 3
        || !part
            .offset
            .iter()
            .all(|v| v.is_finite() && v.abs() <= bound)
        || !part
            .scale
            .iter()
            .all(|v| v.is_finite() && *v > 0.0 && *v <= bound && (*v as f32) > 0.0)
    {
        return Err("static assembly requires finite f32 offset, positive f32 scale and Y quarter turns 0..3".into());
    }
    let [sx, sy, sz] = part.scale.map(|v| v as f32);
    let [x, y, z] = part.offset.map(|v| v as f32);
    // Column-major T * Ry * S. Positive right-handed Y yaw maps +X to -Z.
    let placement = match part.quarter_turns {
        0 => [
            [sx, 0.0, 0.0, 0.0],
            [0.0, sy, 0.0, 0.0],
            [0.0, 0.0, sz, 0.0],
            [x, y, z, 1.0],
        ],
        1 => [
            [0.0, 0.0, -sx, 0.0],
            [0.0, sy, 0.0, 0.0],
            [sz, 0.0, 0.0, 0.0],
            [x, y, z, 1.0],
        ],
        2 => [
            [-sx, 0.0, 0.0, 0.0],
            [0.0, sy, 0.0, 0.0],
            [0.0, 0.0, -sz, 0.0],
            [x, y, z, 1.0],
        ],
        3 => [
            [0.0, 0.0, sx, 0.0],
            [0.0, sy, 0.0, 0.0],
            [-sz, 0.0, 0.0, 0.0],
            [x, y, z, 1.0],
        ],
        _ => unreachable!("admitted quarter turns"),
    };
    orr_model::normal_matrix(placement)
        .map_err(|error| format!("static assembly placement: {error}"))?;
    Ok(placement)
}

pub(super) fn static_assembly_variant_bytes(
    source: &[u8],
    variant: &StaticAssemblyVariant,
) -> Result<Vec<u8>, String> {
    validate_static_assembly_variant(variant)?;
    let first = part_placement(&variant.parts[0])?;
    let second = part_placement(&variant.parts[1])?;
    let recipe = serde_json::to_vec(&serde_json::json!({
        "format": "orr_static_model_assemble", "version": 1,
        "parts": [
            {"key": "first", "source": "input.model.json", "placement": first},
            {"key": "second", "source": "input.model.json", "placement": second},
        ],
    }))
    .map_err(|error| format!("static assembly recipe: {error}"))?;
    let mut captured = Some(source.to_vec());
    let model = orr_model::static_assemble::assemble_model(&recipe, &variant.asset_id, |path| {
        if path != "input.model.json" {
            return Err("unexpected static assembly dependency".into());
        }
        captured
            .take()
            .ok_or_else(|| "static assembly source resolved more than once".into())
    })?;
    model
        .to_bytes()
        .map_err(|error| format!("static assembly variant cook: {error}"))
}

#[cfg(test)]
#[path = "static_assembly_tests.rs"]
mod tests;
