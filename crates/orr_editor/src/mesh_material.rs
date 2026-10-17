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
    check_derived_work(source_model)?;
    let mut source = model.source().clone();
    for material in &mut source.materials {
        for (component, multiplier) in material.base_color.iter_mut().zip(tint) {
            *component *= multiplier;
        }
    }
    StaticModel::new(source).map_err(|error| format!("validate tinted model: {error}"))
}

pub(crate) fn check_derived_work(source: &orr_model::ModelSource) -> Result<(), String> {
    let mut work_bytes = std::mem::size_of::<orr_model::ModelSource>();
    add_work_bytes(&mut work_bytes, source.format.len())?;
    add_work_bytes(&mut work_bytes, source.asset_id.len())?;
    add_repeated_work(
        &mut work_bytes,
        source.dependencies.len(),
        std::mem::size_of::<orr_model::Dependency>(),
    )?;
    add_repeated_work(
        &mut work_bytes,
        source.primitives.len(),
        std::mem::size_of::<orr_model::Primitive>(),
    )?;
    add_repeated_work(
        &mut work_bytes,
        source.materials.len(),
        std::mem::size_of::<orr_model::Material>(),
    )?;
    add_repeated_work(
        &mut work_bytes,
        source.images.len(),
        std::mem::size_of::<orr_model::Image>(),
    )?;
    for dependency in &source.dependencies {
        add_work_bytes(&mut work_bytes, dependency.uri.len())?;
        add_work_bytes(&mut work_bytes, dependency.sha256.len())?;
    }
    for primitive in &source.primitives {
        add_work_bytes(&mut work_bytes, primitive.id.len())?;
        let vertex_bytes = primitive
            .vertices
            .len()
            .checked_mul(std::mem::size_of::<orr_model::Vertex>())
            .ok_or_else(|| "derived mesh work estimate overflowed".to_owned())?;
        let index_bytes = primitive
            .indices
            .len()
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or_else(|| "derived mesh work estimate overflowed".to_owned())?;
        add_work_bytes(&mut work_bytes, vertex_bytes)?;
        add_work_bytes(&mut work_bytes, index_bytes)?;
    }
    for image in &source.images {
        add_work_bytes(&mut work_bytes, image.rgba8.len())?;
    }
    if work_bytes > MAX_DERIVED_WORK_BYTES {
        return Err("derived mesh presentation exceeds the 8 MiB work limit".into());
    }
    Ok(())
}

fn add_work_bytes(total: &mut usize, bytes: usize) -> Result<(), String> {
    *total = total
        .checked_add(bytes)
        .ok_or_else(|| "derived mesh work estimate overflowed".to_owned())?;
    Ok(())
}

fn add_repeated_work(total: &mut usize, count: usize, size: usize) -> Result<(), String> {
    let bytes = count
        .checked_mul(size)
        .ok_or_else(|| "derived mesh work estimate overflowed".to_owned())?;
    add_work_bytes(total, bytes)
}

#[cfg(test)]
#[path = "mesh_material/tests.rs"]
mod tests;
