//! Private offline sequence authoring. Directories must remain trusted during use.
use orr_sprite::{Atlas, ClipSource, Frame, PlaybackMode, Region, SpriteDocument, SpriteSource};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub const MAX_RECIPE_BYTES: usize = 1024 * 1024;
pub const MAX_FRAMES: usize = 256;
pub const MAX_PNG_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL_PNG_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_RGBA_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_LICENSE_BYTES: usize = 64 * 1024;

pub struct Request {
    pub recipe: PathBuf,
    pub out: PathBuf,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub width: u32,
    pub height: u32,
    pub frames: usize,
    pub rgba_bytes: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    schema: u32,
    name: String,
    version: String,
    license: String,
    clip: String,
    mode: PlaybackMode,
    columns: u32,
    #[serde(deserialize_with = "bounded_frames")]
    frames: Vec<InputFrame>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputFrame {
    png: String,
    duration_ms: u64,
}
fn bounded_frames<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<InputFrame>, D::Error> {
    struct FramesVisitor;
    impl<'de> serde::de::Visitor<'de> for FramesVisitor {
        type Value = Vec<InputFrame>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("at most 256 ordered frames")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut frames = Vec::new();
            while let Some(frame) = sequence.next_element::<InputFrame>()? {
                if frames.len() == MAX_FRAMES {
                    return Err(serde::de::Error::custom("frame count exceeds 256"));
                }
                frames.push(frame);
            }
            Ok(frames)
        }
    }
    deserializer.deserialize_seq(FramesVisitor)
}
#[derive(Serialize)]
struct Package<'a> {
    schema: u32,
    name: &'a str,
    version: &'a str,
    engine: &'static str,
    capabilities: [&'static str; 1],
    dependencies: BTreeMap<String, String>,
    files: [&'static str; 3],
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

fn relative_input(base: &Path, value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || value.contains(':')
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("recipe inputs must be relative paths without traversal".into());
    }
    checked_path(&base.join(path), false)
}

fn rgba_size(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 || width > 2048 || height > 2048 {
        return Err("image or atlas dimensions must be 1..=2048".into());
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n <= MAX_RGBA_BYTES)
        .ok_or_else(|| "RGBA byte budget exceeded".into())
}
fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    if bytes.len() < 24
        || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || &bytes[8..12] != 13u32.to_be_bytes().as_slice()
        || &bytes[12..16] != b"IHDR"
    {
        return Err("frame must be a PNG with an IHDR header".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().map_err(|_| "PNG width")?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().map_err(|_| "PNG height")?);
    rgba_size(width, height)?;
    Ok((width, height))
}
fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn import_package(request: &Request) -> Result<Report, String> {
    let recipe_path = checked_path(&request.recipe, false)?;
    let recipe: Recipe = serde_json::from_slice(&read_regular(&recipe_path, MAX_RECIPE_BYTES)?)
        .map_err(|e| format!("recipe: {e}"))?;
    if recipe.schema != 1 || !validate_name(&recipe.name) || !validate_version(&recipe.version) {
        return Err("recipe requires schema 1 and valid package name/canonical version".into());
    }
    if recipe.frames.is_empty()
        || recipe.frames.len() > MAX_FRAMES
        || recipe.columns == 0
        || recipe.columns as usize > recipe.frames.len()
    {
        return Err("require 1..=256 frames and columns in 1..=frame count".into());
    }
    if recipe.clip.trim().is_empty() || recipe.clip.len() > orr_sprite::MAX_CLIP_ID_BYTES {
        return Err("invalid clip name".into());
    }
    let mut duration = 0u64;
    for frame in &recipe.frames {
        if frame.duration_ms == 0 {
            return Err("frame duration must be positive".into());
        }
        duration = duration
            .checked_add(frame.duration_ms)
            .ok_or("clip duration overflow")?;
    }
    let out = checked_path(&request.out, true)?;
    if out.try_exists().map_err(|e| e.to_string())? {
        return Err("output already exists".into());
    }
    let base = recipe_path.parent().ok_or("recipe parent missing")?;
    let license = read_regular(&relative_input(base, &recipe.license)?, MAX_LICENSE_BYTES)?;
    if std::str::from_utf8(&license)
        .map_err(|_| "license must be UTF-8")?
        .trim()
        .is_empty()
    {
        return Err("license must be nonempty UTF-8 text".into());
    }
    // Metadata preflight bounds all compressed reads before retaining any PNG bytes.
    let mut paths = Vec::with_capacity(recipe.frames.len());
    let mut compressed = 0usize;
    for frame in &recipe.frames {
        let path = relative_input(base, &frame.png)?;
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > MAX_PNG_BYTES as u64 {
            return Err("PNG must be a bounded regular file".into());
        }
        compressed = compressed
            .checked_add(meta.len() as usize)
            .filter(|n| *n <= MAX_TOTAL_PNG_BYTES)
            .ok_or("aggregate compressed PNG budget exceeded")?;
        paths.push(path);
    }
    let mut images = Vec::with_capacity(paths.len());
    let mut dimensions = None;
    let mut actual_compressed = 0usize;
    let mut decoded_total = 0usize;
    for path in paths {
        let remaining = MAX_TOTAL_PNG_BYTES - actual_compressed;
        let bytes = read_regular(&path, remaining.min(MAX_PNG_BYTES))?;
        actual_compressed += bytes.len();
        let size = png_dimensions(&bytes)?;
        if dimensions.is_some_and(|expected| expected != size) {
            return Err("PNG frames must have equal dimensions".into());
        }
        dimensions = Some(size);
        decoded_total = decoded_total
            .checked_add(rgba_size(size.0, size.1)?)
            .filter(|n| *n <= MAX_RGBA_BYTES)
            .ok_or("aggregate decoded RGBA budget exceeded")?;
        images.push(bytes);
    }
    let (frame_width, frame_height) = dimensions.ok_or("no frames")?;
    let rows = (recipe.frames.len() as u32).div_ceil(recipe.columns);
    let width = frame_width
        .checked_mul(recipe.columns)
        .ok_or("atlas width overflow")?;
    let height = frame_height
        .checked_mul(rows)
        .ok_or("atlas height overflow")?;
    let rgba_bytes = rgba_size(width, height)?;
    // Both decoded and atlas budgets are established before PNG decoding or atlas allocation.
    let mut atlas = vec![0u8; rgba_bytes];
    let mut regions = Vec::with_capacity(recipe.frames.len());
    let mut frames = Vec::with_capacity(recipe.frames.len());
    for (index, (bytes, frame)) in images.iter().zip(&recipe.frames).enumerate() {
        let rgba = orr_sample::project_sprites::decode_atlas(bytes, frame_width, frame_height)?;
        let x = (index as u32 % recipe.columns) * frame_width;
        let y = (index as u32 / recipe.columns) * frame_height;
        let stride = frame_width as usize * 4;
        for row in 0..frame_height as usize {
            let destination = ((y as usize + row) * width as usize + x as usize) * 4;
            atlas[destination..destination + stride]
                .copy_from_slice(&rgba[row * stride..(row + 1) * stride]);
        }
        regions.push(Region {
            id: index as u32,
            x,
            y,
            width: frame_width,
            height: frame_height,
        });
        frames.push(Frame {
            region: index as u32,
            duration_ms: frame.duration_ms,
        });
    }
    let source = SpriteSource {
        format: orr_sprite::FORMAT.into(),
        version: orr_sprite::VERSION,
        atlas: Atlas {
            image: "atlas.png".into(),
            width,
            height,
        },
        regions,
        clips: vec![ClipSource {
            id: recipe.clip,
            mode: recipe.mode,
            frames,
        }],
    };
    SpriteDocument::new(source.clone()).map_err(|e| e.to_string())?;
    let json = json_bytes(&source)?;
    let mut png = Vec::new();
    {
        let mut encoder = png17::Encoder::new(&mut png, width, height);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&atlas).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    if png.len() > MAX_PNG_BYTES {
        return Err("encoded atlas exceeds PNG budget".into());
    }
    let package = Package {
        schema: 1,
        name: &recipe.name,
        version: &recipe.version,
        engine: "^0.0.1",
        capabilities: ["sprite"],
        dependencies: BTreeMap::new(),
        files: ["sprites.json", "atlas.png", "LICENSE.txt"],
    };
    publish(
        &out,
        &[
            ("sprites.json", &json),
            ("atlas.png", &png),
            ("LICENSE.txt", &license),
            ("orr.package.json", &json_bytes(&package)?),
        ],
    )?;
    Ok(Report {
        width,
        height,
        frames: recipe.frames.len(),
        rgba_bytes,
    })
}

fn publish(out: &Path, files: &[(&str, &[u8])]) -> Result<(), String> {
    let parent = out.parent().ok_or("output parent missing")?;
    let stage = tempfile::Builder::new()
        .prefix(".orr-png-sequence-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    for (name, bytes) in files {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.path().join(name))
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    if checked_path(out, true)? != out {
        return Err("output parent changed".into());
    }
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        stage.path(),
        rustix::fs::CWD,
        out,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|e| format!("atomic no-replace publication failed: {e}"))?;
    // Stage now has no path. TempDir cleanup cannot target the published destination.
    Ok(())
}
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
