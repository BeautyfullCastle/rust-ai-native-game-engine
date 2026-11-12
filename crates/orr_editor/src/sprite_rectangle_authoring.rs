//! Explicit non-grid PNG atlas rectangle authoring; no project mutation or automatic install.
pub(crate) use crate::sprite_grid_authoring::PngInspection;
use orr_package::sprite_package::authoring;
use orr_sprite::{Atlas, SpriteDocument, SpriteSource};
use std::path::{Component, Path, PathBuf};

const MAX_RECTANGLES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RectangleSpec {
    pub regions: Vec<orr_sprite::Region>,
}

pub(crate) struct Request {
    pub png: PathBuf,
    pub license: PathBuf,
    pub destination: PathBuf,
    pub name: String,
    pub version: String,
    pub license_acknowledged: bool,
    pub inspection: PngInspection,
    pub spec: RectangleSpec,
}

#[derive(Clone, Debug)]
pub(crate) struct Report {
    pub root: PathBuf,
    pub package: String,
    pub document: String,
    pub width: u32,
    pub height: u32,
    pub region_count: u32,
    pub source_sha256: String,
}

pub(crate) fn inspect_png(path: &Path) -> Result<PngInspection, String> {
    validate_path_shape(path, "PNG")?;
    authoring::inspect_png(path)
}

pub(crate) fn validate_rectangle_spec(
    spec: &RectangleSpec,
    image: &PngInspection,
) -> Result<SpriteDocument, String> {
    validate_hash(&image.sha256)?;
    if image.width == 0
        || image.height == 0
        || image.width > orr_sprite::MAX_ATLAS_DIMENSION
        || image.height > orr_sprite::MAX_ATLAS_DIMENSION
    {
        return Err("inspected PNG dimensions must be within 1..=2048".into());
    }
    if !(1..=MAX_RECTANGLES).contains(&spec.regions.len()) {
        return Err("rectangle atlas must contain 1..=256 regions".into());
    }

    SpriteDocument::new(SpriteSource {
        format: orr_sprite::FORMAT.into(),
        version: orr_sprite::VERSION,
        atlas: Atlas {
            image: "atlas.png".into(),
            width: image.width,
            height: image.height,
        },
        regions: spec.regions.clone(),
        clips: vec![],
    })
    .map_err(|error| format!("sprite rectangle document: {error}"))
}

pub(crate) fn create_package(request: Request) -> Result<Report, String> {
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    let document = validate_rectangle_spec(&request.spec, &request.inspection)?;
    validate_path_shape(&request.png, "PNG")?;
    validate_path_shape(&request.license, "license")?;
    validate_path_shape(&request.destination, "package destination")?;
    if request.destination.components().any(|component| {
        matches!(component, Component::Normal(name) if is_package_store_component(name))
    }) {
        return Err("sprite package destination cannot be inside the .orr store".into());
    }

    let mut json = document
        .to_json()
        .map_err(|error| error.to_string())?
        .into_bytes();
    json.push(b'\n');
    let source_sha256 = request.inspection.sha256.to_ascii_lowercase();
    let core = authoring::Request {
        png: request.png,
        json: PathBuf::new(),
        license: request.license,
        name: request.name.clone(),
        version: request.version,
        out: request.destination.clone(),
    };
    // The checked packer captures PNG/license bytes and preserves its existing
    // visible-partial publication contract. Never delete or retry on failure.
    let published = authoring::import_package_bytes_checked(&core, &json, &source_sha256)
        .map_err(|error| format!("sprite package destination {}: {error}. If partial output exists, retain it for inspection; choose a new destination before retrying.", request.destination.display()))?;

    Ok(Report {
        root: request.destination,
        package: request.name,
        document: "sprites.json".into(),
        width: published.width,
        height: published.height,
        // validate_rectangle_spec caps this at 256, so this conversion cannot fail.
        region_count: document.regions().len() as u32,
        source_sha256,
    })
}

fn is_package_store_component(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    // Win32 path resolution ignores trailing dots and spaces in components,
    // so these spellings can otherwise alias the protected `.orr` store.
    #[cfg(windows)]
    let name = name.trim_end_matches(['.', ' ']);
    name.eq_ignore_ascii_case(".orr")
}

fn validate_hash(hash: &str) -> Result<(), String> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("inspected PNG hash must contain 64 hexadecimal characters".into());
    }
    Ok(())
}

fn validate_path_shape(path: &Path, label: &str) -> Result<(), String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("{label} path must be Unicode"))?;
    if !path.is_absolute()
        || text.len() > 4096
        || path.components().count() > 64
        || text.chars().any(char::is_control)
        || text.split(['/', '\\']).any(|p| p == "." || p == "..")
        || path
            .components()
            .any(|p| matches!(p, Component::ParentDir | Component::CurDir))
    {
        return Err(format!(
            "{label} path must be bounded, absolute and normalized"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "sprite_rectangle_authoring/tests.rs"]
mod tests;
