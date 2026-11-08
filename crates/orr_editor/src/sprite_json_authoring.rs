//! Explicit sprite package authoring from an existing document and PNG.
//! Inspection captures only bounded source metadata; package creation rechecks
//! the JSON bytes and delegates publication to the checked core writer.
use orr_package::sprite_package::authoring;
use orr_sprite::SpriteDocument;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Summary {
    image_filename: String,
    width: u32,
    height: u32,
    region_count: usize,
    clip_count: usize,
}

/// Immutable approval data for one exact JSON path and one inspected PNG.
/// Fields stay private so callers cannot forge hashes or typed document data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Inspection {
    json_path: PathBuf,
    png_path: PathBuf,
    json_sha256: String,
    png: authoring::PngInspection,
    summary: Summary,
}

impl Inspection {
    pub(crate) fn image_filename(&self) -> &str {
        &self.summary.image_filename
    }

    pub(crate) fn width(&self) -> u32 {
        self.summary.width
    }

    pub(crate) fn height(&self) -> u32 {
        self.summary.height
    }

    pub(crate) fn region_count(&self) -> usize {
        self.summary.region_count
    }

    pub(crate) fn clip_count(&self) -> usize {
        self.summary.clip_count
    }

    pub(crate) fn json_sha256(&self) -> &str {
        &self.json_sha256
    }

    pub(crate) fn png_sha256(&self) -> &str {
        &self.png.sha256
    }
}

pub(crate) struct Request {
    pub inspection: Inspection,
    pub license_path: PathBuf,
    pub destination: PathBuf,
    pub name: String,
    pub version: String,
    pub license_acknowledged: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Report {
    pub root: PathBuf,
    pub package: String,
    pub document: String,
    pub image_filename: String,
    pub width: u32,
    pub height: u32,
    pub region_count: usize,
    pub clip_count: usize,
    pub json_sha256: String,
    pub png_sha256: String,
}

/// Read and validate a bounded SpriteDocument JSON and inspect its explicitly
/// selected PNG. This is read-only and does not retain source bytes.
pub(crate) fn inspect_sources(json_path: &Path, png_path: &Path) -> Result<Inspection, String> {
    validate_path_shape(json_path, "sprite JSON")?;
    validate_path_shape(png_path, "PNG")?;

    let json = read_bounded_regular(json_path, MAX_JSON_BYTES, "sprite JSON")?;
    let json_sha256 = hash(&json);
    let document = parse_document(&json)?;
    let png = authoring::inspect_png(png_path)?;
    let summary = summary(&document);
    if (summary.width, summary.height) != (png.width, png.height) {
        return Err(format!(
            "sprite JSON atlas dimensions {}x{} do not match inspected PNG dimensions {}x{}",
            summary.width, summary.height, png.width, png.height
        ));
    }

    Ok(Inspection {
        json_path: json_path.to_path_buf(),
        png_path: png_path.to_path_buf(),
        json_sha256,
        png,
        summary,
    })
}

/// Create one new sprite package after a fresh JSON capture matches the
/// inspected source exactly. The checked core writer captures the PNG and
/// license and preserves all source bytes. Failed publication is never cleaned
/// up or retried here.
pub(crate) fn create_package(request: Request) -> Result<Report, String> {
    if !request.license_acknowledged {
        return Err(
            "explicit license and redistribution-rights acknowledgement is required".into(),
        );
    }

    validate_path_shape(&request.inspection.json_path, "sprite JSON")?;
    validate_path_shape(&request.inspection.png_path, "PNG")?;
    validate_path_shape(&request.license_path, "license")?;
    validate_new_destination(&request.destination)?;

    let captured_json =
        read_bounded_regular(&request.inspection.json_path, MAX_JSON_BYTES, "sprite JSON")?;
    let current_hash = hash(&captured_json);
    if current_hash != request.inspection.json_sha256 {
        return Err(
            "sprite JSON changed since inspection; inspect again before creating the package"
                .into(),
        );
    }
    let current_document = parse_document(&captured_json)?;
    let current_summary = summary(&current_document);
    if current_summary != request.inspection.summary {
        return Err("sprite document summary changed since inspection; inspect again before creating the package".into());
    }
    if (current_summary.width, current_summary.height)
        != (request.inspection.png.width, request.inspection.png.height)
    {
        return Err("sprite JSON atlas dimensions do not match the inspected PNG".into());
    }

    let core_request = authoring::Request {
        png: request.inspection.png_path.clone(),
        // The checked byte entry point never opens this path. Preserve the
        // originating path in the request for diagnostics and API consistency.
        json: request.inspection.json_path.clone(),
        license: request.license_path,
        name: request.name.clone(),
        version: request.version,
        out: request.destination.clone(),
    };
    let packed = authoring::import_package_bytes_checked(
        &core_request,
        &captured_json,
        &request.inspection.png.sha256,
    )
    .map_err(|error| {
        format!(
            "sprite package destination {}: {error}. If partial output exists, retain it for inspection; choose a new destination before retrying.",
            request.destination.display()
        )
    })?;

    Ok(Report {
        root: request.destination,
        package: request.name,
        document: "sprites.json".into(),
        image_filename: packed.image,
        width: packed.width,
        height: packed.height,
        region_count: current_summary.region_count,
        clip_count: current_summary.clip_count,
        json_sha256: current_hash,
        png_sha256: request.inspection.png.sha256,
    })
}

fn parse_document(json: &[u8]) -> Result<SpriteDocument, String> {
    let text = std::str::from_utf8(json).map_err(|error| format!("sprite JSON UTF-8: {error}"))?;
    SpriteDocument::from_json(text).map_err(|error| format!("sprite document: {error}"))
}

fn summary(document: &SpriteDocument) -> Summary {
    Summary {
        image_filename: document.atlas().image.clone(),
        width: document.atlas().width,
        height: document.atlas().height,
        region_count: document.regions().len(),
        clip_count: document.clips().len(),
    }
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn read_bounded_regular(path: &Path, limit: usize, label: &str) -> Result<Vec<u8>, String> {
    validate_path_shape(path, label)?;
    checked_ancestry(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| format!("{label}: {error}"))?;
    if !regular(&metadata) || metadata.len() > limit as u64 {
        return Err(format!("{label} must be a bounded regular file"));
    }
    let mut file = File::open(path).map_err(|error| format!("{label}: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("{label}: {error}"))?;
    if !regular(&opened) || opened.len() > limit as u64 {
        return Err(format!("{label} changed or exceeds its byte limit"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{label}: {error}"))?;
    if bytes.len() > limit {
        return Err(format!("{label} exceeds its byte limit"));
    }
    Ok(bytes)
}

fn validate_destination(path: &Path) -> Result<(), String> {
    // Windows strips trailing dots/spaces from path components. Reject all
    // spellings that resolve to the protected `.orr` package store before any
    // source or license file is opened.
    if path.components().any(|component| {
        matches!(component, Component::Normal(name)
            if name.to_string_lossy().trim_end_matches(['.', ' ']).eq_ignore_ascii_case(".orr"))
    }) {
        return Err("sprite package destination cannot be inside the .orr store".into());
    }
    validate_path_shape(path, "package destination")
}

fn validate_new_destination(path: &Path) -> Result<(), String> {
    validate_destination(path)?;
    let parent = path
        .parent()
        .ok_or("package destination must have an existing parent directory")?;
    checked_ancestry(parent)?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|error| format!("package destination parent: {error}"))?;
    if !parent_metadata.is_dir() || linked(&parent_metadata) {
        return Err("package destination parent must be a regular directory".into());
    }
    match fs::symlink_metadata(path) {
        Ok(_) => Err("package destination already exists; choose a new directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("package destination: {error}")),
    }
}

fn validate_path_shape(path: &Path, label: &str) -> Result<(), String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("{label} path must be Unicode"))?;
    if !path.is_absolute()
        || text.len() > 4096
        || path.components().count() > 64
        || text.chars().any(char::is_control)
        || text
            .split(['/', '\\'])
            .any(|part| part == "." || part == "..")
    {
        return Err(format!(
            "{label} path must be bounded, absolute and normalized"
        ));
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(format!("{label} path cannot contain traversal"));
        }
        if let Component::Prefix(prefix) = component {
            if !matches!(
                prefix.kind(),
                std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
            ) {
                return Err(format!("{label} path must use a local filesystem volume"));
            }
        }
        if let Component::Normal(name) = component {
            let name = name
                .to_str()
                .ok_or_else(|| format!("{label} path must be Unicode"))?;
            if name.contains([':', '\\']) || name.ends_with(['.', ' ']) {
                return Err(format!("{label} path has a nonportable component"));
            }
        }
    }
    Ok(())
}

fn checked_ancestry(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    let components: Vec<_> = path.components().collect();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("{}: {error}", current.display()))?;
        if linked(&metadata) {
            return Err("sprite authoring refuses linked or reparse ancestry".into());
        }
        if index + 1 < components.len() && !metadata.is_dir() {
            return Err(format!(
                "{}: path ancestor is not a directory",
                current.display()
            ));
        }
    }
    Ok(())
}

fn linked(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x0400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn regular(metadata: &Metadata) -> bool {
    metadata.is_file() && !linked(metadata)
}

#[cfg(test)]
#[path = "sprite_json_authoring/tests.rs"]
mod tests;
