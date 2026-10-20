//! Private, offline example implementation; not a library authoring API.
use orr_sprite::SpriteDocument;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PNG_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_LICENSE_BYTES: usize = 64 * 1024;
pub const MAX_RGBA_BYTES: usize = 16 * 1024 * 1024;

pub struct Request {
    pub png: PathBuf,
    pub json: PathBuf,
    pub license: PathBuf,
    pub name: String,
    pub version: String,
    pub out: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub width: u32,
    pub height: u32,
    pub rgba_bytes: usize,
    pub image: String,
}

#[derive(Serialize)]
struct Package<'a> {
    schema: u32,
    name: &'a str,
    version: &'a str,
    engine: &'static str,
    capabilities: [&'static str; 1],
    dependencies: BTreeMap<String, String>,
    files: Vec<String>,
}

fn validate_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
}

// Canonical SemVer 2.0.0, including u64 core fields as used by orr_package.
// Keep this example independent of the optional image-reimport feature/dependency.
fn validate_version(value: &str) -> bool {
    if value.is_empty() || value.len() > 256 {
        return false;
    }
    let (without_build, build) = value
        .split_once('+')
        .map_or((value, None), |(a, b)| (a, Some(b)));
    let (core, pre) = without_build
        .split_once('-')
        .map_or((without_build, None), |(a, b)| (a, Some(b)));
    let numbers: Vec<_> = core.split('.').collect();
    if numbers.len() != 3
        || !numbers
            .iter()
            .all(|s| numeric(s) && s.parse::<u64>().is_ok())
    {
        return false;
    }
    let identifiers = |s: &str, prerelease: bool| {
        s.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && (!prerelease || !part.bytes().all(|b| b.is_ascii_digit()) || numeric(part))
        })
    };
    pre.is_none_or(|s| identifiers(s, true)) && build.is_none_or(|s| identifiers(s, false))
}

fn numeric(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0'))
}

fn validate_image_name(name: &str) -> Result<(), String> {
    // Deliberately narrower than arbitrary host filenames: one ASCII portable leaf.
    if name.is_empty()
        || name.len() > 255
        || name.ends_with(['.', ' '])
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        || matches!(name, "." | "..")
    {
        return Err("atlas.image must be a portable ASCII leaf PNG filename".into());
    }
    if ["sprites.json", "LICENSE.txt", "orr.package.json"]
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
    {
        return Err("atlas.image collides with a package metadata filename".into());
    }
    let (stem, extension) = name
        .rsplit_once('.')
        .ok_or("atlas.image must have a .png extension")?;
    if stem.is_empty() || !extension.eq_ignore_ascii_case("png") {
        return Err("atlas.image must have a nonempty stem and .png extension".into());
    }
    let device = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            device.strip_prefix(*prefix).is_some_and(|suffix| {
                suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
            })
        })
    {
        return Err("atlas.image is a reserved Windows device filename".into());
    }
    Ok(())
}

// Static checks only: no race-proof no-follow/openat guarantee. All input and
// output directories must remain trusted and unchanged throughout the command.
fn checked_path(path: &Path, allow_missing_leaf: bool) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("empty path".into());
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let parts: Vec<_> = absolute.components().collect();
    let mut checked = PathBuf::new();
    for (index, part) in parts.iter().enumerate() {
        if matches!(part, Component::ParentDir) {
            return Err("parent traversal is not allowed".into());
        }
        checked.push(part.as_os_str());
        // A prefix alone is not the rooted Windows volume. The following
        // RootDir iteration inspects the full root before any descendants.
        if matches!(part, Component::Prefix(_))
            && matches!(parts.get(index + 1), Some(Component::RootDir))
        {
            continue;
        }
        match fs::symlink_metadata(&checked) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err("symlink paths are not allowed".into());
                }
                if index + 1 < parts.len() && !meta.is_dir() {
                    return Err("path ancestor is not a directory".into());
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && allow_missing_leaf
                    && index + 1 == parts.len() => {}
            Err(error) => return Err(format!("{}: {error}", checked.display())),
        }
    }
    Ok(checked)
}

fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let path = checked_path(path, false)?;
    let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return Err("input must be a bounded regular file".into());
    }
    let mut file = File::open(&path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return Err("input changed or exceeds byte budget".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("input exceeds byte budget".into());
    }
    Ok(bytes)
}

pub fn import_package(request: &Request) -> Result<Report, String> {
    if !validate_name(&request.name) {
        return Err(
            "invalid lowercase package name (1..=64 ASCII letters, digits, '-' or '_')".into(),
        );
    }
    if !validate_version(&request.version) {
        return Err("canonical semantic version required (maximum 256 bytes)".into());
    }
    let out = checked_path(&request.out, true)?;
    match fs::symlink_metadata(&out) {
        Ok(_) => return Err("output already exists; choose a new directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let json =
        read_regular(&request.json, MAX_JSON_BYTES).map_err(|e| format!("sprite JSON: {e}"))?;
    let document = SpriteDocument::from_json(
        std::str::from_utf8(&json).map_err(|e| format!("sprite JSON UTF-8: {e}"))?,
    )
    .map_err(|e| format!("sprite document: {e}"))?;
    let atlas = document.atlas();
    validate_image_name(&atlas.image)?;
    let png = read_regular(&request.png, MAX_PNG_BYTES).map_err(|e| format!("PNG: {e}"))?;
    let rgba_bytes = (atlas.width as usize)
        .checked_mul(atlas.height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n <= MAX_RGBA_BYTES)
        .ok_or("decoded RGBA exceeds 16 MiB")?;
    let rgba = orr_sample::project_sprites::decode_atlas(&png, atlas.width, atlas.height)
        .map_err(|e| format!("atlas PNG: {e}"))?;
    if rgba.len() != rgba_bytes {
        return Err("decoded RGBA size differs from document".into());
    }
    drop(rgba);
    let license =
        read_regular(&request.license, MAX_LICENSE_BYTES).map_err(|e| format!("license: {e}"))?;
    if std::str::from_utf8(&license)
        .map_err(|_| "license must be UTF-8 text")?
        .trim()
        .is_empty()
    {
        return Err("license must be nonempty UTF-8 text".into());
    }
    let package = Package {
        schema: 1,
        name: &request.name,
        version: &request.version,
        engine: "^0.0.1",
        capabilities: ["sprite"],
        dependencies: BTreeMap::new(),
        files: vec![
            "sprites.json".into(),
            atlas.image.clone(),
            "LICENSE.txt".into(),
        ],
    };
    let mut metadata = serde_json::to_vec_pretty(&package).map_err(|e| e.to_string())?;
    metadata.push(b'\n');
    // Original JSON, PNG and license bytes are copied, never re-encoded.
    publish(
        &out,
        &[
            ("sprites.json", json.as_slice()),
            (&atlas.image, png.as_slice()),
            ("LICENSE.txt", license.as_slice()),
            ("orr.package.json", metadata.as_slice()),
        ],
    )?;
    Ok(Report {
        width: atlas.width,
        height: atlas.height,
        rgba_bytes,
        image: atlas.image.clone(),
    })
}

// mkdir claims a new destination without clobbering an existing path. The
// directory is visible before completion; this is not atomic/crash-durable.
fn publish(out: &Path, files: &[(&str, &[u8])]) -> Result<(), String> {
    fs::create_dir(out).map_err(|e| format!("claim new output: {e}"))?;
    let result = (|| -> std::io::Result<()> {
        for (name, bytes) in files {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(out.join(name))?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(cleanup) = fs::remove_dir_all(out) {
            return Err(format!(
                "publication failed: {error}; cleanup also failed: {cleanup}; inspect {}",
                out.display()
            ));
        }
        return Err(format!("publication failed: {error}"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
