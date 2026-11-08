//! Explicit regular PNG grid authoring; no project mutation or automatic install.
pub(crate) use authoring::PngInspection;
use orr_package::sprite_package::authoring;
use orr_sprite::{Atlas, ClipSource, Frame, PlaybackMode, Region, SpriteDocument, SpriteSource};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GridSpec {
    pub cell_width: u32,
    pub cell_height: u32,
    pub clip: String,
    pub mode: PlaybackMode,
    pub duration_ms: u64,
}

pub(crate) struct Request {
    pub png: PathBuf,
    pub license: PathBuf,
    pub destination: PathBuf,
    pub name: String,
    pub version: String,
    pub license_acknowledged: bool,
    pub inspection: PngInspection,
    pub spec: GridSpec,
}

#[derive(Clone, Debug)]
pub(crate) struct Report {
    pub root: PathBuf,
    pub package: String,
    pub document: String,
    pub width: u32,
    pub height: u32,
    pub cells: u32,
    pub source_sha256: String,
}

pub(crate) fn inspect_png(path: &Path) -> Result<PngInspection, String> {
    validate_path_shape(path, "PNG")?;
    authoring::inspect_png(path)
}

pub(crate) fn validate_grid_spec(
    spec: &GridSpec,
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
    if spec.cell_width == 0
        || spec.cell_height == 0
        || !image.width.is_multiple_of(spec.cell_width)
        || !image.height.is_multiple_of(spec.cell_height)
    {
        return Err("nonzero cell dimensions must divide the inspected PNG exactly".into());
    }
    let columns = image.width / spec.cell_width;
    let rows = image.height / spec.cell_height;
    let cells = columns
        .checked_mul(rows)
        .filter(|n| (1..=256).contains(n))
        .ok_or("sprite grid must contain 1..=256 cells")?;
    if spec.clip.trim().is_empty()
        || spec.clip.len() > orr_sprite::MAX_CLIP_ID_BYTES
        || spec.clip.chars().any(char::is_control)
    {
        return Err(
            "clip name must be nonblank, at most 128 UTF-8 bytes and contain no control characters"
                .into(),
        );
    }
    if spec.duration_ms == 0 || spec.duration_ms.checked_mul(u64::from(cells)).is_none() {
        return Err(
            "frame duration must be positive and the total clip duration must fit u64".into(),
        );
    }
    let regions = (0..cells)
        .map(|id| Region {
            id,
            x: (id % columns) * spec.cell_width,
            y: (id / columns) * spec.cell_height,
            width: spec.cell_width,
            height: spec.cell_height,
        })
        .collect();
    let frames = (0..cells)
        .map(|region| Frame {
            region,
            duration_ms: spec.duration_ms,
        })
        .collect();
    SpriteDocument::new(SpriteSource {
        format: orr_sprite::FORMAT.into(),
        version: orr_sprite::VERSION,
        atlas: Atlas {
            image: "atlas.png".into(),
            width: image.width,
            height: image.height,
        },
        regions,
        clips: vec![ClipSource {
            id: spec.clip.clone(),
            mode: spec.mode,
            frames,
        }],
    })
    .map_err(|error| format!("sprite grid document: {error}"))
}

pub(crate) fn create_package(request: Request) -> Result<Report, String> {
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    let document = validate_grid_spec(&request.spec, &request.inspection)?;
    validate_path_shape(&request.png, "PNG")?;
    validate_path_shape(&request.license, "license")?;
    validate_path_shape(&request.destination, "package destination")?;
    if request.destination.components().any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy().eq_ignore_ascii_case(".orr"))
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
    // The checked packer captures PNG/license bytes and keeps its existing
    // visible-partial publication contract. Never delete or retry on failure.
    let report = authoring::import_package_bytes_checked(&core, &json, &source_sha256)
        .map_err(|error| format!("sprite package destination {}: {error}. If partial output exists, retain it for inspection; choose a new destination before retrying.", request.destination.display()))?;
    Ok(Report {
        root: request.destination,
        package: request.name,
        document: "sprites.json".into(),
        width: report.width,
        height: report.height,
        cells: document.regions().len() as u32,
        source_sha256,
    })
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
#[path = "sprite_grid_authoring/tests.rs"]
mod tests;
