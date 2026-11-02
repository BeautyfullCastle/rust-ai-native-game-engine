use serde::{Deserialize, Serialize};

pub(crate) const MAX_BYTES: usize = 16 * 1024 * 1024;
const SCHEMA: u32 = 1;
const PREFAB_SCENE_FILENAME: &str = "mesh-prop-prefab.scene.yaml";

#[derive(Clone)]
pub(crate) struct Template {
    name: String,
    art: crate::mesh_art::Snapshot,
    #[cfg(feature = "mesh-prop-shading")]
    shading: Option<crate::mesh_prop_shading::Settings>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    schema: u32,
    name: String,
    graph_revision: [u8; 32],
    art_json: String,
    #[cfg(feature = "mesh-prop-shading")]
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::mesh_prop_shading::deserialize_present"
    )]
    shading: Option<crate::mesh_prop_shading::Settings>,
}

#[cfg(test)]
#[path = "mesh_prop_prefab/tests.rs"]
mod tests;

impl Template {
    pub(crate) fn capture(name: &str, art: &crate::mesh_art::Snapshot) -> Result<Self, String> {
        validate_name(name)?;
        let template = Self {
            name: name.to_owned(),
            art: art.clone(),
            #[cfg(feature = "mesh-prop-shading")]
            shading: None,
        };
        template.encode()?;
        Ok(template)
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, String> {
        validate_name(&self.name)?;
        #[cfg(feature = "mesh-prop-shading")]
        if let Some(shading) = &self.shading {
            shading.validate(self.art.source_model_arc())?;
        }
        let art = self.art.encode_embedded(PREFAB_SCENE_FILENAME)?;
        let wire = Wire {
            #[cfg(feature = "mesh-prop-shading")]
            shading: self.shading.clone(),
            schema: SCHEMA,
            name: self.name.clone(),
            graph_revision: self.art.graph_revision(),
            art_json: String::from_utf8(art)
                .map_err(|_| "embedded prop prefab art was not UTF-8")?,
        };
        let bytes = serde_json::to_vec(&wire)
            .map_err(|error| format!("serialize mesh prop prefab: {error}"))?;
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err("mesh prop prefab must fit within 16 MiB".into());
        }
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8], target_graph: [u8; 32]) -> Result<Self, String> {
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err("mesh prop prefab must be 1..=16 MiB".into());
        }
        let wire: Wire = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid mesh prop prefab: {error}"))?;
        if wire.schema != SCHEMA {
            return Err("unsupported mesh prop prefab schema".into());
        }
        validate_name(&wire.name)?;
        if wire.art_json.is_empty() || wire.art_json.len() > MAX_BYTES {
            return Err("mesh prop prefab art must fit within 16 MiB".into());
        }
        let art = crate::mesh_art::Snapshot::decode_embedded(
            wire.art_json.as_bytes(),
            PREFAB_SCENE_FILENAME,
            wire.graph_revision,
        )?
        .with_graph_revision(target_graph);
        let template = Self {
            name: wire.name,
            art,
            #[cfg(feature = "mesh-prop-shading")]
            shading: wire.shading,
        };
        // Validate the target-bound art and full record, including scalar slots.
        template.encode()?;
        Ok(template)
    }

    /// Return a new template with explicit optional prop shading. The embedded
    /// art and its source provenance remain unchanged; None clears only this copy.
    #[cfg(feature = "mesh-prop-shading")]
    pub(crate) fn with_shading(
        mut self,
        shading: Option<crate::mesh_prop_shading::Settings>,
    ) -> Result<Self, String> {
        self.shading = shading;
        self.encode()?;
        Ok(self)
    }

    #[cfg(feature = "mesh-prop-shading")]
    pub(crate) fn shading(&self) -> Option<&crate::mesh_prop_shading::Settings> {
        self.shading.as_ref()
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn art(&self) -> &crate::mesh_art::Snapshot {
        &self.art
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty()
        || name.trim() != name
        || name.len() > 64
        || name.chars().any(char::is_control)
    {
        return Err(
            "mesh prop prefab name must be trimmed, nonempty, at most 64 bytes, and contain no controls"
                .into(),
        );
    }
    Ok(())
}
