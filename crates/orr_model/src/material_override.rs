//! Per-instance static material authoring. The immutable cooked model is never
//! changed: a selected slot replaces only its imported linear RGB factor.
use crate::{Error, MAX_MATERIALS, StaticModel, invalid};
use serde::{Deserialize, Serialize};

/// One bounded material-slot override. Factors multiply decoded texture RGB,
/// exactly once, in place of the imported factor. Alpha and sampling stay intact.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct MaterialOverride {
    pub material_slot: u32,
    pub base_color_factor: [f32; 3],
}

impl<'de> Deserialize<'de> for MaterialOverride {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // A derived struct decoder also accepts positional sequences. This new
        // sidecar payload is an object, never a [slot, rgb] alternate spelling.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            material_slot: u32,
            base_color_factor: [f32; 3],
        }
        struct ObjectVisitor;
        impl<'de> serde::de::Visitor<'de> for ObjectVisitor {
            type Value = MaterialOverride;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(
                    "a material override object with material_slot and base_color_factor",
                )
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                map: M,
            ) -> Result<Self::Value, M::Error> {
                let fields =
                    Fields::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(MaterialOverride {
                    material_slot: fields.material_slot,
                    base_color_factor: fields.base_color_factor,
                })
            }
        }
        deserializer.deserialize_map(ObjectVisitor)
    }
}

impl MaterialOverride {
    pub fn validate(self) -> Result<(), Error> {
        if self.material_slot as usize >= MAX_MATERIALS {
            return Err(invalid(
                "material override slot exceeds the supported material limit",
            ));
        }
        if !self
            .base_color_factor
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        {
            return Err(invalid(
                "material override RGB factor must be finite and within [0,1]",
            ));
        }
        Ok(())
    }

    /// Unused slots are not authorable, even when present in the source array.
    pub fn validate_for(self, model: &StaticModel) -> Result<(), Error> {
        self.validate()?;
        if self.material_slot as usize >= model.source().materials.len()
            || !model
                .source()
                .primitives
                .iter()
                .any(|primitive| primitive.material == self.material_slot)
        {
            return Err(invalid(
                "material override must select a used static material slot",
            ));
        }
        Ok(())
    }

    /// Resolve a previously validated override for one primitive. Imported alpha
    /// is preserved, including the existing opaque renderer's treatment of it.
    pub fn effective_base_color(self, material_slot: u32, imported: [f32; 4]) -> [f32; 4] {
        if material_slot == self.material_slot {
            [
                self.base_color_factor[0],
                self.base_color_factor[1],
                self.base_color_factor[2],
                imported[3],
            ]
        } else {
            imported
        }
    }
}
