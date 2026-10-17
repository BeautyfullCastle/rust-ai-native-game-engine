use orr_model::StaticModel;

const MAX_DERIVED_WORK_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn identity() -> [f32; 4] {
    [1.0; 4]
}

pub(crate) fn is_identity(tint: &[f32; 4]) -> bool {
    *tint == identity()
}

pub(crate) fn validate(tint: [f32; 4]) -> Result<[f32; 4], String> {
    if tint
        .iter()
        .all(|component| component.is_finite() && (0.0..=1.0).contains(component))
    {
        Ok(tint)
    } else {
        Err("material tint must contain finite RGBA multipliers in 0..=1".into())
    }
}

pub(crate) fn tinted_model(model: &StaticModel, tint: [f32; 4]) -> Result<StaticModel, String> {
    let tint = validate(tint)?;
    let source_model = model.source();
    let mut work_bytes = source_model
        .images
        .iter()
        .try_fold(0usize, |total, image| total.checked_add(image.rgba8.len()))
        .ok_or_else(|| "material tint work estimate overflowed".to_owned())?;
    for primitive in &source_model.primitives {
        let vertex_bytes = primitive
            .vertices
            .len()
            .checked_mul(std::mem::size_of::<orr_model::Vertex>())
            .ok_or_else(|| "material tint work estimate overflowed".to_owned())?;
        let index_bytes = primitive
            .indices
            .len()
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or_else(|| "material tint work estimate overflowed".to_owned())?;
        work_bytes = work_bytes
            .checked_add(vertex_bytes)
            .and_then(|value| value.checked_add(index_bytes))
            .ok_or_else(|| "material tint work estimate overflowed".to_owned())?;
    }
    if work_bytes > MAX_DERIVED_WORK_BYTES {
        return Err("material tint derived model exceeds the 8 MiB work limit".into());
    }
    let mut source = model.source().clone();
    for material in &mut source.materials {
        for (component, multiplier) in material.base_color.iter_mut().zip(tint) {
            *component *= multiplier;
        }
    }
    StaticModel::new(source).map_err(|error| format!("validate tinted model: {error}"))
}

#[cfg(test)]
#[path = "mesh_material/tests.rs"]
mod tests;
