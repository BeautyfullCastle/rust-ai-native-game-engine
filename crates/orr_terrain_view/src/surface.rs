//! Optional opaque, stored-sample terrain appearance sidecars. Pure byte APIs;
//! no I/O, simulation changes, color conversion, or implicit missing-file policy.
use crate::QueryMarker;
use orr_model::{Dependency, Image, StaticModel};
use orr_terrain::Terrain;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod png;
#[cfg(test)]
mod tests;

pub const MAX_SIDE: u32 = 512;
pub const MAX_PNG_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SIDECAR_BYTES: usize = 8 * 1024 * 1024;

/// Version 1 binds appearance to a confined terrain path and logical asset ID,
/// not a height/hole revision. Image rows are top-to-bottom with opaque RGBA8.
/// JSON is compact canonical serde field order, without a trailing newline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    version: u32,
    terrain_path: String,
    asset_id: String,
    image: Image,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Surface(Document, [u8; 32]);

fn identity(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > orr_terrain::MAX_ASSET_ID_BYTES
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-/ .".contains(&b))
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("surface requires a bounded normalized relative identity".into());
    }
    Ok(())
}

/// Derive one normalized relative sidecar name. Backslashes, drive prefixes,
/// roots, empty components and dot/parent components are rejected on every OS.
pub fn sidecar_relative(terrain_relative: &str) -> Result<String, String> {
    identity(terrain_relative)?;
    if !terrain_relative.ends_with(".orrt")
        || terrain_relative.ends_with("/.orrt")
        || terrain_relative == ".orrt"
    {
        return Err("surface terrain path must name a .orrt file".into());
    }
    Ok(format!("{terrain_relative}.surface.json"))
}

impl Surface {
    pub fn from_png(terrain_relative: &str, asset_id: &str, bytes: &[u8]) -> Result<Self, String> {
        sidecar_relative(terrain_relative)?;
        identity(asset_id)?;
        let mut surface = Self(
            Document {
                format: "orr_terrain_surface".into(),
                version: 1,
                terrain_path: terrain_relative.into(),
                asset_id: asset_id.into(),
                image: png::decode(bytes)?,
            },
            [0; 32],
        );
        surface.validate()?;
        surface.1 = Sha256::digest(surface.to_bytes()?).into();
        Ok(surface)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_SIDECAR_BYTES {
            return Err("surface sidecar exceeds 8 MiB".into());
        }
        let document: Document =
            serde_json::from_slice(bytes).map_err(|e| format!("surface JSON: {e}"))?;
        let mut surface = Self(document, [0; 32]);
        surface.validate()?;
        if surface.to_bytes()? != bytes {
            return Err("surface sidecar must use canonical JSON bytes".into());
        }
        surface.1 = Sha256::digest(bytes).into();
        Ok(surface)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(&self.0).map_err(|e| format!("surface JSON: {e}"))?;
        if bytes.len() > MAX_SIDECAR_BYTES {
            return Err("surface sidecar exceeds 8 MiB".into());
        }
        Ok(bytes)
    }

    /// SHA-256 of the canonical sidecar, including identity and decoded pixels.
    pub fn digest(&self) -> [u8; 32] {
        self.1
    }
    pub fn terrain_path(&self) -> &str {
        &self.0.terrain_path
    }
    pub fn asset_id(&self) -> &str {
        &self.0.asset_id
    }
    pub fn image(&self) -> &Image {
        &self.0.image
    }

    pub fn validate_terrain(&self, relative: &str, terrain: &Terrain) -> Result<(), String> {
        sidecar_relative(relative)?;
        if relative != self.terrain_path() || terrain.asset_id() != self.asset_id() {
            return Err("surface terrain path or asset identity mismatch".into());
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        if self.0.format != "orr_terrain_surface" || self.0.version != 1 {
            return Err("unsupported terrain surface format/version".into());
        }
        sidecar_relative(self.terrain_path())?;
        identity(self.asset_id())?;
        let image = self.image();
        if !(1..=MAX_SIDE).contains(&image.width)
            || !(1..=MAX_SIDE).contains(&image.height)
            || image.rgba8.len() != image.width as usize * image.height as usize * 4
            || image.rgba8.chunks_exact(4).any(|pixel| pixel[3] != 255)
        {
            return Err("surface image requires 1..=512 dimensions and exact opaque RGBA8".into());
        }
        Ok(())
    }
}

/// Keep geometry, UVs, nearest/repeat sampling and marker material/image intact.
/// The terrain's $source digest remains truthful; the sidecar gets its own entry.
pub fn to_static_model(
    terrain: &Terrain,
    markers: &[QueryMarker],
    surface: &Surface,
) -> Result<Option<StaticModel>, String> {
    surface.validate_terrain(surface.terrain_path(), terrain)?;
    let Some(model) = crate::to_static_model(terrain, markers).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let mut source = model.source().clone();
    source.images[0] = surface.image().clone();
    source.dependencies.push(Dependency {
        uri: sidecar_relative(surface.terrain_path())?,
        sha256: surface
            .digest()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    });
    StaticModel::new(source)
        .map(Some)
        .map_err(|e| e.to_string())
}
