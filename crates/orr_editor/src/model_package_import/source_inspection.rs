//! Read-only cooked animated source catalog; no license or destination access.
use super::*;
#[derive(Clone, Debug)]
pub(crate) struct Catalog {
    pub source_sha256: String,
    pub materials: Vec<MaterialEntry>,
    pub clips: Vec<ClipEntry>,
}
#[derive(Clone, Debug)]
pub(crate) struct MaterialEntry {
    pub index: u32,
    pub used_primitive_count: u32,
    pub base_color: [f32; 4],
    pub image: u32,
    pub width: u32,
    pub height: u32,
    pub linear_filter: bool,
    pub wrap_s: orr_model::Wrap,
    pub wrap_t: orr_model::Wrap,
}
#[derive(Clone, Debug)]
pub(crate) struct ClipEntry {
    pub index: u32,
    pub name: String,
    pub duration: f32,
}
pub(crate) fn inspect_animated_source(path: &Path) -> Result<Catalog, String> {
    if !cfg!(feature = "room-animated-models") {
        return Err(
            "animated source inspection requires room-animated-models; no input opened".into(),
        );
    }
    if !path
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.to_ascii_lowercase().ends_with(".animated.json"))
    {
        return Err("animated inspection source must be a .animated.json file".into());
    }
    let path = checked_file_path(path, "animated inspection source")?;
    let input = read_bounded_file(&path, MAX_SOURCE_BYTES, "animated inspection source")?;
    catalog_from_bytes(&input.bytes)
}
pub(super) fn catalog_from_bytes(bytes: &[u8]) -> Result<Catalog, String> {
    #[cfg(feature = "room-animated-models")]
    {
        let model = orr_model::animation::AnimatedModel::from_bytes(bytes)
            .map_err(|e| format!("animated source inspection: {e}"))?;
        let source = model.source();
        if source.clips.is_empty() {
            return Err("animated inspection requires at least one animation clip".into());
        }
        let materials = source
            .materials
            .iter()
            .enumerate()
            .map(|(index, material)| {
                let image = &source.images[material.image as usize];
                MaterialEntry {
                    index: index as u32,
                    used_primitive_count: source
                        .primitives
                        .iter()
                        .filter(|p| p.material == index as u32)
                        .count() as u32,
                    base_color: material.base_color,
                    image: material.image,
                    width: image.width,
                    height: image.height,
                    linear_filter: material.linear_filter,
                    wrap_s: material.wrap_s,
                    wrap_t: material.wrap_t,
                }
            })
            .collect();
        let clips = source
            .clips
            .iter()
            .enumerate()
            .map(|(index, clip)| ClipEntry {
                index: index as u32,
                name: clip.name.clone(),
                duration: clip.duration(),
            })
            .collect();
        Ok(Catalog {
            source_sha256: digest(bytes),
            materials,
            clips,
        })
    }
    #[cfg(not(feature = "room-animated-models"))]
    {
        let _ = bytes;
        Err("animated source inspection requires room-animated-models".into())
    }
}
