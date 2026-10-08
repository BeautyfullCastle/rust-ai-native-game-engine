//! Shared read-only authored sprite documents and verified package decoding.
//! Authoring history, save operations and GPU texture caches stay in the editor.
use orr_sprite::SpriteDocument;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
pub const MAX_BYTES: u64 = 1024 * 1024;
const MAX_BINDINGS: usize = 4096;
pub type AssetKey = (String, String);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Source {
    Region(u32),
    Clip(String),
    /// Presentation-only clip selection from caller-observed motion.
    Locomotion {
        idle: String,
        walk: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub package: String,
    pub document: String,
    pub source: Source,
    /// World units per sprite pixel.
    pub units_per_pixel: f32,
}
impl Binding {
    /// Existing callers preview the idle clip of a locomotion binding.
    pub fn region(&self, document: &SpriteDocument, elapsed_ms: u64) -> Result<u32, String> {
        self.region_for_motion(document, elapsed_ms, false)
    }
    pub fn region_for_motion(
        &self,
        document: &SpriteDocument,
        elapsed_ms: u64,
        moving: bool,
    ) -> Result<u32, String> {
        if !self.units_per_pixel.is_finite() || !(0.0001..=100.0).contains(&self.units_per_pixel) {
            return Err("units per pixel must be finite and between 0.0001 and 100".into());
        }
        match &self.source {
            Source::Region(id) => document
                .region(*id)
                .map(|_| *id)
                .ok_or_else(|| format!("missing region {id}")),
            Source::Clip(id) => document
                .clip(id)
                .map(|clip| clip.sample(elapsed_ms).region)
                .ok_or_else(|| format!("missing clip {id}")),
            Source::Locomotion { idle, walk } => {
                // Resolve both clips so a missing inactive clip cannot hide
                // until the entity starts or stops moving.
                let idle = document
                    .clip(idle)
                    .ok_or_else(|| format!("missing idle clip {idle}"))?;
                let walk = document
                    .clip(walk)
                    .ok_or_else(|| format!("missing walk clip {walk}"))?;
                let clip = if moving { walk } else { idle };
                Ok(clip.sample(elapsed_ms).region)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u32,
    /// Scene filename, relative to this sidecar's parent directory.
    pub scene: String,
    /// Package project directory, relative to this sidecar's parent directory.
    pub project: String,
    /// Persistent scene GUIDs, never recyclable frame handles.
    pub bindings: BTreeMap<String, Binding>,
    /// Optional persistent scene GUID. View-only, never sent to the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_follow: Option<String>,
}
pub fn relative(path: &str) -> bool {
    !path.is_empty()
        && !Path::new(path).has_root()
        && !path.contains('\\')
        && !path.contains(':')
}
impl Document {
    /// Parse one bounded read-only v1/v2 presentation document.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_BYTES {
            return Err("binding file exceeds byte limit".into());
        }
        let document: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        document.validate()?;
        Ok(document)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.version, 1 | 2) {
            return Err("unsupported sprite binding version".into());
        }
        if self.version == 1 && self.has_v2_features() {
            return Err(
                "version-1 sprite bindings cannot contain locomotion or camera follow".into(),
            );
        }
        if let Some(guid) = &self.camera_follow {
            orr_reflect::Guid::parse(guid)
                .map_err(|_| format!("invalid camera follow scene GUID: {guid}"))?;
        }
        if !relative(&self.scene) || !relative(&self.project) {
            return Err("scene and project must be explicit relative paths".into());
        }
        if self.bindings.len() > MAX_BINDINGS {
            return Err("too many sprite bindings".into());
        }
        for (guid, binding) in &self.bindings {
            orr_reflect::Guid::parse(guid).map_err(|_| format!("invalid scene GUID: {guid}"))?;
            if binding.package.is_empty() || !relative(&binding.document) {
                return Err(format!("invalid package/document binding for {guid}"));
            }
            if !binding.units_per_pixel.is_finite()
                || !(0.0001..=100.0).contains(&binding.units_per_pixel)
            {
                return Err(format!("invalid sprite scale for {guid}"));
            }
            if let Source::Locomotion { idle, walk } = &binding.source {
                for (role, clip) in [("idle", idle), ("walk", walk)] {
                    if clip.trim().is_empty() || clip.len() > orr_sprite::MAX_CLIP_ID_BYTES {
                        return Err(format!("invalid {role} clip name for {guid}"));
                    }
                }
            }
        }
        Ok(())
    }
    pub fn has_v2_features(&self) -> bool {
        self.camera_follow.is_some()
            || self
                .bindings
                .values()
                .any(|binding| matches!(binding.source, Source::Locomotion { .. }))
    }
}

#[derive(Clone)]
pub struct Asset {
    pub document: SpriteDocument,
    pub rgba: Vec<u8>,
}
pub fn load_project_asset(
    project: &orr_package::Project,
    package: &str,
    document: &str,
) -> Result<Asset, String> {
    let bytes = project
        .read_asset(package, document)
        .map_err(|e| format!("sprite document: {e}"))?;
    let sprite = SpriteDocument::from_json(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
        .map_err(|e| format!("sprite document: {e}"))?;
    let image = project
        .read_asset(package, &sprite.atlas().image)
        .map_err(|e| format!("atlas: {e}"))?;
    let rgba = decode_atlas(&image, sprite.atlas().width, sprite.atlas().height)?;
    Ok(Asset {
        document: sprite,
        rgba,
    })
}
pub fn decode_atlas(
    image: &[u8],
    expected_width: u32,
    expected_height: u32,
) -> Result<Vec<u8>, String> {
    if expected_width == 0
        || expected_height == 0
        || expected_width > orr_sprite::MAX_ATLAS_DIMENSION
        || expected_height > orr_sprite::MAX_ATLAS_DIMENSION
    {
        return Err("invalid atlas dimensions".into());
    }
    if image.len() < 24 || &image[..8] != b"\x89PNG\r\n\x1a\n" || &image[12..16] != b"IHDR" {
        return Err("atlas must be PNG".into());
    }
    let width = u32::from_be_bytes(image[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(image[20..24].try_into().unwrap());
    if width != expected_width || height != expected_height {
        return Err("atlas PNG dimensions differ from sprite document".into());
    }
    let mut decoder = png17::Decoder::new(std::io::Cursor::new(image));
    decoder.set_transformations(png17::Transformations::EXPAND | png17::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    if reader.info().animation_control.is_some() {
        return Err("animated PNG atlases are unsupported; use sprite clips".into());
    }
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).map_err(|e| e.to_string())?;
    if info.width != width || info.height != height {
        return Err("decoded PNG frame dimensions differ from atlas".into());
    }
    let expected_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("atlas byte size overflow")?;
    let mut rgba = Vec::with_capacity(expected_bytes);
    match info.color_type {
        png17::ColorType::Rgba => rgba.extend_from_slice(&pixels[..info.buffer_size()]),
        png17::ColorType::Rgb => {
            for p in pixels[..info.buffer_size()].chunks_exact(3) {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        png17::ColorType::Grayscale => {
            for p in &pixels[..info.buffer_size()] {
                rgba.extend_from_slice(&[*p, *p, *p, 255]);
            }
        }
        png17::ColorType::GrayscaleAlpha => {
            for p in pixels[..info.buffer_size()].chunks_exact(2) {
                rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
        }
        _ => return Err("unsupported PNG color type".into()),
    }
    if rgba.len() != expected_bytes {
        return Err("decoded PNG pixel count differs from atlas".into());
    }
    Ok(rgba)
}

/// Resolve the complete bounded asset set before any host or GPU is created.
pub fn load_project_assets(
    document: &Document,
    project: &orr_package::Project,
) -> Result<BTreeMap<AssetKey, Asset>, String> {
    let references: BTreeSet<_> = document
        .bindings
        .values()
        .map(|b| (b.package.clone(), b.document.clone()))
        .collect();
    if references.len() > 8 {
        return Err(
            "sidecar references more than 8 sprite documents; reduce references before reloading"
                .into(),
        );
    }
    let mut assets = BTreeMap::new();
    for (package, path) in references {
        let asset = load_project_asset(project, &package, &path)
            .map_err(|e| format!("{package}/{path}: {e}"))?;
        for binding in document
            .bindings
            .values()
            .filter(|b| b.package == package && b.document == path)
        {
            binding.region(&asset.document, 0)?;
        }
        assets.insert((package, path), asset);
    }
    Ok(assets)
}
