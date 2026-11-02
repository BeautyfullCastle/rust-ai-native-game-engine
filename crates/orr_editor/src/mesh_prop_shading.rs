//! Optional static-prop presentation overrides; no cooked-model mutation.
use orr_model::StaticModel;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
const MAX_SLOTS: usize = 256;
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Slot {
    pub(crate) material: u32,
    pub(crate) metallic: f32,
    pub(crate) roughness: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Settings {
    version: u32,
    slots: Vec<Slot>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotWire {
    material: u32,
    metallic: f32,
    roughness: f32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsWire {
    version: u32,
    #[serde(deserialize_with = "bounded_slots")]
    slots: Vec<Slot>,
}
fn map<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    struct Visitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
        type Value = T;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an object")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, m: A) -> Result<T, A::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(m))
        }
    }
    d.deserialize_map(Visitor(std::marker::PhantomData))
}
impl<'de> Deserialize<'de> for Slot {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire: SlotWire = map(d)?;
        Ok(Self {
            material: wire.material,
            metallic: wire.metallic,
            roughness: wire.roughness,
        })
    }
}
impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire: SettingsWire = map(d)?;
        if wire.version != 1 {
            return Err(serde::de::Error::custom("unsupported prop shading version"));
        }
        Self::canonical(wire.slots).map_err(serde::de::Error::custom)
    }
}
fn bounded_slots<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Slot>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Vec<Slot>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("1..256 shading slots")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            if seq.size_hint().is_some_and(|n| n > MAX_SLOTS) {
                return Err(serde::de::Error::custom("too many shading slots"));
            }
            let mut slots = Vec::new();
            while slots.len() < MAX_SLOTS {
                match seq.next_element()? {
                    Some(slot) => slots.push(slot),
                    None => return Ok(slots),
                }
            }
            if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom("too many shading slots"));
            }
            Ok(slots)
        }
    }
    d.deserialize_seq(Visitor)
}
pub(crate) fn deserialize_present<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Settings>, D::Error> {
    Settings::deserialize(d).map(Some)
}
impl Settings {
    fn canonical(mut slots: Vec<Slot>) -> Result<Self, String> {
        if slots.is_empty() || slots.len() > MAX_SLOTS {
            return Err("prop shading requires 1..256 slots".into());
        }
        slots.sort_by_key(|s| s.material);
        for i in 0..slots.len() {
            if i > 0 && slots[i - 1].material == slots[i].material {
                return Err("duplicate prop shading material".into());
            }
            orr_render::ScalarMaterial::new(slots[i].metallic, slots[i].roughness)
                .map_err(|e| e.to_string())?;
            if slots[i].metallic == 0.0 {
                slots[i].metallic = 0.0;
            }
            if slots[i].roughness == 0.0 {
                slots[i].roughness = 0.0;
            }
        }
        Ok(Self { version: 1, slots })
    }
    pub(crate) fn new(slots: Vec<Slot>, model: &StaticModel) -> Result<Self, String> {
        let settings = Self::canonical(slots)?;
        settings.validate(model)?;
        Ok(settings)
    }
    pub(crate) fn slots(&self) -> &[Slot] {
        &self.slots
    }
    pub(crate) fn validate(&self, model: &StaticModel) -> Result<(), String> {
        for slot in &self.slots {
            if slot.material as usize >= model.source().materials.len()
                || !model
                    .source()
                    .primitives
                    .iter()
                    .any(|p| p.material == slot.material)
            {
                return Err("prop shading slot must refer to a used model material".into());
            }
        }
        Ok(())
    }
    pub(crate) fn materials(
        &self,
        model: &StaticModel,
    ) -> Result<Vec<orr_render::MaterialShading>, String> {
        self.validate(model)?;
        let mut table =
            vec![orr_render::MaterialShading::LegacyDiffuse; model.source().materials.len()];
        for slot in &self.slots {
            table[slot.material as usize] = orr_render::MaterialShading::MetallicRoughness(
                orr_render::ScalarMaterial::new(slot.metallic, slot.roughness)
                    .map_err(|e| e.to_string())?,
            );
        }
        Ok(table)
    }
    pub(crate) fn identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"orr.mesh-prop-shading.v1\0");
        hash.update((self.slots.len() as u32).to_le_bytes());
        for slot in &self.slots {
            hash.update(slot.material.to_le_bytes());
            hash.update(slot.metallic.to_bits().to_le_bytes());
            hash.update(slot.roughness.to_bits().to_le_bytes());
        }
        hash.finalize().into()
    }
}
#[cfg(test)]
#[path = "mesh_prop_shading/tests.rs"]
mod tests;
