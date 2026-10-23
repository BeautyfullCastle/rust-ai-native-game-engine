use orr_model::{StaticModel, Wrap};
use serde::{Deserialize, Serialize};

const MAX_SETTINGS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub(crate) material: u32,
    pub(crate) tint: [f32; 4],
    pub(crate) linear_filter: bool,
    pub(crate) wrap_s: Wrap,
    pub(crate) wrap_t: Wrap,
}

impl Settings {
    pub(crate) fn identity(material: u32, source: &orr_model::Material) -> Self {
        Self {
            material,
            tint: [1.0; 4],
            linear_filter: source.linear_filter,
            wrap_s: source.wrap_s,
            wrap_t: source.wrap_t,
        }
    }

    pub(crate) fn is_identity_for(&self, source: &orr_model::Material) -> bool {
        self.tint == [1.0; 4]
            && self.linear_filter == source.linear_filter
            && self.wrap_s == source.wrap_s
            && self.wrap_t == source.wrap_t
    }
}

pub(crate) fn validate(settings: &[Settings], model: &StaticModel) -> Result<(), String> {
    if settings.len() > MAX_SETTINGS {
        return Err("material slot settings exceed the 256-slot limit".into());
    }

    let mut seen = vec![false; model.source().materials.len()];
    let mut used = vec![false; model.source().materials.len()];
    for primitive in &model.source().primitives {
        if let Ok(material_index) = usize::try_from(primitive.material) {
            if let Some(is_used) = used.get_mut(material_index) {
                *is_used = true;
            }
        }
    }

    for slot in settings {
        if !slot
            .tint
            .iter()
            .all(|component| component.is_finite() && (0.0..=1.0).contains(component))
        {
            return Err("material slot tint must be finite RGBA values in 0..=1".into());
        }
        let material_index = usize::try_from(slot.material)
            .map_err(|_| "material slot index exceeds this platform")?;
        let Some(is_seen) = seen.get_mut(material_index) else {
            return Err("material slot index is outside the model".into());
        };
        if *is_seen {
            return Err("material slot settings contain a duplicate material index".into());
        }
        *is_seen = true;
        if !used[material_index] {
            return Err("material slot must be used by a model primitive".into());
        }
    }
    Ok(())
}

pub(crate) fn apply(model: &StaticModel, settings: &[Settings]) -> Result<StaticModel, String> {
    validate(settings, model)?;
    crate::mesh_material::check_derived_work(model.source())?;

    let mut source = model.source().clone();
    for slot in settings {
        let material_index = usize::try_from(slot.material)
            .map_err(|_| "material slot index exceeds this platform")?;
        let material = &mut source.materials[material_index];
        if slot.tint != [1.0; 4] {
            for (component, multiplier) in material.base_color.iter_mut().zip(slot.tint) {
                *component *= multiplier;
            }
        }
        material.linear_filter = slot.linear_filter;
        material.wrap_s = slot.wrap_s;
        material.wrap_t = slot.wrap_t;
    }
    StaticModel::new(source).map_err(|error| format!("validate material-slot model: {error}"))
}

#[cfg(test)]
#[path = "mesh_material_slots/tests.rs"]
mod tests;
