//! Private GPU-free cooked material texture authoring. No runtime changes.
//! Paths require trusted stable ancestry; these portable checks are not a
//! sandbox against hostile concurrent directory replacement. Never overwrites.
use orr_model::{
    Dependency, Image, StaticModel, MAX_DECODED_BYTES, MAX_FILE_BYTES, MAX_IMAGES, MAX_IMAGE_EXTENT,
};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
pub const MAX_RECIPE_BYTES: usize = 16 * 1024;
static STAGE_ID: AtomicU64 = AtomicU64::new(0);

struct Recipe {
    format: String,
    version: u32,
    source: String,
    texture: String,
    material: u32,
}
// A map-only visitor also excludes serde's positional sequence-to-struct alias.
// Values retain their exact JSON types and duplicate keys never get discarded.
impl<'de> Deserialize<'de> for Recipe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RecipeVisitor;
        impl<'de> Visitor<'de> for RecipeVisitor {
            type Value = Recipe;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a texture recipe object with exactly five required fields")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Recipe, M::Error> {
                let (mut format, mut version, mut source, mut texture, mut material) =
                    (None, None, None, None, None);
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "format" => {
                            if format.is_some() {
                                return Err(de::Error::duplicate_field("format"));
                            }
                            format = Some(map.next_value()?);
                        }
                        "version" => {
                            if version.is_some() {
                                return Err(de::Error::duplicate_field("version"));
                            }
                            version = Some(map.next_value()?);
                        }
                        "source" => {
                            if source.is_some() {
                                return Err(de::Error::duplicate_field("source"));
                            }
                            source = Some(map.next_value()?);
                        }
                        "texture" => {
                            if texture.is_some() {
                                return Err(de::Error::duplicate_field("texture"));
                            }
                            texture = Some(map.next_value()?);
                        }
                        "material" => {
                            if material.is_some() {
                                return Err(de::Error::duplicate_field("material"));
                            }
                            material = Some(map.next_value()?);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &["format", "version", "source", "texture", "material"],
                            ))
                        }
                    }
                }
                Ok(Recipe {
                    format: format.ok_or_else(|| de::Error::missing_field("format"))?,
                    version: version.ok_or_else(|| de::Error::missing_field("version"))?,
                    source: source.ok_or_else(|| de::Error::missing_field("source"))?,
                    texture: texture.ok_or_else(|| de::Error::missing_field("texture"))?,
                    material: material.ok_or_else(|| de::Error::missing_field("material"))?,
                })
            }
        }
        deserializer.deserialize_map(RecipeVisitor)
    }
}
fn logical_path(path: &str, suffix: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 240
        || !path.ends_with(suffix)
        || !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
    {
        return Err(format!(
            "expected a portable relative ASCII {suffix} path of at most 240 bytes"
        ));
    }
    for segment in path.split('/') {
        let device = segment.split('.').next().unwrap_or("").to_ascii_uppercase();
        if segment.is_empty()
            || segment.starts_with('.')
            || segment.ends_with('.')
            || ["CON", "PRN", "AUX", "NUL"].contains(&device.as_str())
            || (device.len() == 4
                && (device.starts_with("COM") || device.starts_with("LPT"))
                && device.as_bytes()[3].is_ascii_digit())
        {
            return Err("path has an empty, hidden, traversal or reserved device segment".into());
        }
    }
    Ok(())
}
fn recipe(bytes: &[u8], asset_id: &str) -> Result<Recipe, String> {
    if bytes.len() > MAX_RECIPE_BYTES {
        return Err("recipe exceeds 16 KiB limit".into());
    }
    let recipe: Recipe = serde_json::from_slice(bytes).map_err(|e| format!("recipe JSON: {e}"))?;
    if recipe.format != "orr_static_material_texture" || recipe.version != 1 {
        return Err("unsupported texture recipe format/version".into());
    }
    logical_path(asset_id, ".retexture.json")?;
    logical_path(&recipe.source, ".model.json")?;
    logical_path(&recipe.texture, ".png")?;
    if recipe.source.eq_ignore_ascii_case(&recipe.texture)
        || asset_id.eq_ignore_ascii_case(&recipe.source)
        || asset_id.eq_ignore_ascii_case(&recipe.texture)
    {
        return Err("recipe identity and direct input paths must be distinct".into());
    }
    Ok(recipe)
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn charge(total: usize, bytes: usize) -> Result<usize, String> {
    total
        .checked_add(bytes)
        .filter(|&sum| sum <= MAX_FILE_BYTES)
        .ok_or_else(|| "recipe plus cooked source plus PNG exceeds 64 MiB input budget".into())
}
fn decode_png(bytes: &[u8], remaining: usize) -> Result<Image, String> {
    // Same admission policy as the existing importer, without changing its API.
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: MAX_DECODED_BYTES,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("PNG: {e}"))?;
    let (width, height) = (reader.info().width, reader.info().height);
    if width == 0
        || height == 0
        || width > MAX_IMAGE_EXTENT
        || height > MAX_IMAGE_EXTENT
        || reader.info().animation_control.is_some()
    {
        return Err("PNG extent or APNG unsupported".into());
    }
    let expected = width as usize * height as usize * 4;
    if expected > remaining {
        return Err("decoded image budget exceeded".into());
    }
    let len = reader.output_buffer_size().ok_or("PNG size overflow")?;
    if len != expected {
        return Err("PNG must decode to exact RGBA8".into());
    }
    let mut rgba8 = vec![0; len];
    let info = reader
        .next_frame(&mut rgba8)
        .map_err(|e| format!("PNG: {e}"))?;
    if info.width != width
        || info.height != height
        || info.buffer_size() != expected
        || info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
    {
        return Err("PNG frame does not match static RGBA8 image".into());
    }
    reader.finish().map_err(|e| format!("PNG: {e}"))?;
    Ok(Image {
        width,
        height,
        rgba8,
    })
}

/// Append one decoded PNG and redirect one used material in an admitted cooked
/// source. Recipe identity namespaces primitive IDs; source metadata is bound by
/// the exact cooked-input digest. Never changes source geometry or sampler state.
pub fn retexture_model(
    recipe_bytes: &[u8],
    recipe_asset_id: &str,
    mut resolver: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<StaticModel, String> {
    let recipe = recipe(recipe_bytes, recipe_asset_id)?;
    let source_bytes = resolver(&recipe.source)?;
    let total = charge(recipe_bytes.len(), source_bytes.len())?;
    let original =
        StaticModel::from_bytes(&source_bytes).map_err(|e| format!("source model: {e}"))?;
    let source = original.source();
    let material = usize::try_from(recipe.material).map_err(|_| "material index overflow")?;
    if material >= source.materials.len()
        || !source
            .primitives
            .iter()
            .any(|p| p.material == recipe.material)
    {
        return Err("selected material must exist and be used by a primitive".into());
    }
    if source.images.len() >= MAX_IMAGES {
        return Err("appended image exceeds image count limit".into());
    }
    let decoded: usize = source.images.iter().map(|image| image.rgba8.len()).sum();
    let remaining = MAX_DECODED_BYTES
        .checked_sub(decoded)
        .ok_or("decoded image budget exceeded")?;
    let texture_bytes = resolver(&recipe.texture)?;
    charge(total, texture_bytes.len())?;
    let image = decode_png(&texture_bytes, remaining)?;
    let mut result = source.clone();
    result.asset_id = recipe_asset_id.to_owned();
    for primitive in &mut result.primitives {
        let suffix = primitive
            .id
            .strip_prefix(&source.asset_id)
            .filter(|s| s.starts_with("#node="))
            .ok_or("unexpected source primitive identity")?;
        primitive.id = format!("{recipe_asset_id}{suffix}");
    }
    result.materials[material].image = result.images.len() as u32;
    result.images.push(image);
    result.dependencies = vec![
        Dependency {
            uri: "$source".into(),
            sha256: digest(recipe_bytes),
        },
        Dependency {
            uri: recipe.source,
            sha256: digest(&source_bytes),
        },
        Dependency {
            uri: recipe.texture,
            sha256: digest(&texture_bytes),
        },
    ];
    let model =
        StaticModel::new(result).map_err(|e| format!("retextured model validation: {e}"))?;
    let cooked = model
        .to_bytes()
        .map_err(|e| format!("retextured model cook: {e}"))?;
    let reloaded =
        StaticModel::from_bytes(&cooked).map_err(|e| format!("retextured model reload: {e}"))?;
    if reloaded.source() != model.source() {
        return Err("retextured model changed during cooked round-trip".into());
    }
    Ok(reloaded)
}

fn checked_path(path: &Path, output: bool) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("empty paths and parent traversal are unsupported".into());
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    if !absolute.is_absolute() {
        return Err("drive-relative paths are unsupported".into());
    }
    // Reject network/device namespaces before even querying their metadata.
    // Only local drive prefixes are admitted on Windows.
    for component in absolute.components() {
        if let Component::Prefix(prefix) = component {
            if !matches!(
                prefix.kind(),
                std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
            ) {
                return Err("nonlocal Windows path prefixes are unsupported".into());
            }
        }
        if let Component::Normal(name) = component {
            if name.to_str().is_some_and(|s| s.ends_with(['.', ' '])) {
                return Err("path components ending in a dot or space are unsupported".into());
            }
        }
    }
    let mut result = PathBuf::new();
    let components: Vec<_> = absolute
        .components()
        .filter(|p| !matches!(p, Component::CurDir))
        .collect();
    // Store exclusion is lexical; trusted callers must not use alternate
    // directory aliases or mounts to address an installed package store.
    for (i, part) in components.iter().enumerate() {
        if output
            && part
                .as_os_str()
                .to_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(".orr"))
        {
            return Err("output cannot be inside an .orr package store".into());
        }
        result.push(part.as_os_str());
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&result) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err("symlink paths or ancestry are unsupported".into())
            }
            Ok(m) if i + 1 < components.len() && !m.is_dir() => {
                return Err("path ancestor is not a directory".into())
            }
            Ok(_) => {}
            Err(e)
                if output && i + 1 == components.len() && e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("path metadata: {e}")),
        }
    }
    Ok(result)
}
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        a.len() == b.len() && a.modified().ok() == b.modified().ok()
    }
}
fn read_input(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let path = checked_path(path, false)?;
    let before = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !before.file_type().is_file() || before.len() > limit as u64 {
        return Err("input must be a bounded regular file".into());
    }
    let file = File::open(&path).map_err(|e| e.to_string())?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    let current = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !opened.is_file()
        || !current.file_type().is_file()
        || !same_file(&before, &opened)
        || !same_file(&opened, &current)
        || before.len() != opened.len()
    {
        return Err("input changed during open".into());
    }
    let mut limited = file.take(limit as u64 + 1);
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    limited.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let after = limited.get_ref().metadata().map_err(|e| e.to_string())?;
    if bytes.len() > limit
        || bytes.len() as u64 != opened.len()
        || after.len() != opened.len()
        || after.modified().ok() != opened.modified().ok()
    {
        return Err("input changed during read or exceeds byte limit".into());
    }
    Ok(bytes)
}
fn output_path(path: &Path) -> Result<PathBuf, String> {
    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if name.is_empty()
        || name.len() > 100
        || !name.ends_with(".model.json")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err("output must be a portable ASCII .model.json filename of at most 100 bytes, not a reserved device name".into());
    }
    let path = checked_path(path, true)?;
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(path),
        Ok(_) => Err("output already exists; no overwrite or --force is supported".into()),
        Err(e) => Err(e.to_string()),
    }
}
fn publish(output: &Path, bytes: &[u8]) -> Result<(), String> {
    let destination = output_path(output)?;
    let parent = destination
        .parent()
        .ok_or("output requires an existing trusted parent")?;
    let mut stage = None;
    for _ in 0..32 {
        let path = parent.join(format!(
            ".orr-static-retexture-{}-{}.tmp",
            std::process::id(),
            STAGE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                stage = Some((path, file));
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("output stage: {e}")),
        }
    }
    let (stage, mut file) = stage.ok_or("owned stage-name budget exhausted")?;
    let commit = (|| -> Result<(), String> {
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut readback = Vec::with_capacity(bytes.len());
        (&mut file)
            .take(bytes.len() as u64 + 1)
            .read_to_end(&mut readback)
            .map_err(|e| e.to_string())?;
        if readback != bytes {
            return Err("stage readback differs".into());
        }
        output_path(&destination)?;
        fs::hard_link(&stage, &destination)
            .map_err(|e| format!("no-replace hard-link publication required: {e}"))?;
        Ok(())
    })();
    drop(file);
    let cleanup = fs::remove_file(&stage);
    if let Err(error) = commit {
        return Err(format!(
            "not published: {error}{}",
            cleanup
                .err()
                .map(|e| format!("; owned stage {stage:?} cleanup failed: {e}"))
                .unwrap_or_default()
        ));
    }
    if let Err(error) = cleanup {
        let _ = writeln!(
            std::io::stderr(),
            "warning: published; owned stage {stage:?} cleanup failed: {error}"
        );
    }
    #[cfg(unix)]
    if let Err(error) = File::open(parent).and_then(|f| f.sync_all()) {
        let _ = writeln!(
            std::io::stderr(),
            "warning: published; directory durability uncertain: {error}"
        );
    }
    #[cfg(not(unix))]
    {
        let _ = writeln!(
            std::io::stderr(),
            "warning: published; directory sync unavailable on this platform"
        );
    }
    Ok(())
}

/// Strict bounded CLI. The recipe parent anchors both normalized direct inputs.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let args: Vec<_> = args.into_iter().take(8).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return writeln!(std::io::stdout(), "Usage: static_model_retexture --recipe FILE.retexture.json --asset-id variants/blue.retexture.json --output NEW.model.json\nRequires import. One used material receives an appended RGBA8 PNG; source geometry and other material state stay unchanged. Recipe-relative inputs; trusted stable link-free parents; no overwrite/install. Exit 0: help/published (check warnings); 2: error/not published.").map_err(|e| e.to_string());
    }
    if args.len() != 6 {
        return Err("expected exactly three unique option/value pairs; use --help".into());
    }
    let mut values = BTreeMap::new();
    for pair in args.chunks_exact(2) {
        let key = pair[0].to_str().ok_or("option names must be UTF-8")?;
        if !["--recipe", "--asset-id", "--output"].contains(&key) {
            return Err(format!("unknown option {key}"));
        }
        if pair[1].as_os_str().len() > 4096 {
            return Err(format!("{key} value is too long"));
        }
        if values.insert(key, &pair[1]).is_some() {
            return Err(format!("duplicate option {key}"));
        }
    }
    let get = |key: &str| {
        values
            .get(key)
            .copied()
            .ok_or_else(|| format!("missing {key}"))
    };
    let asset_id = get("--asset-id")?
        .to_str()
        .ok_or("--asset-id must be UTF-8")?;
    let output = output_path(Path::new(get("--output")?))?;
    let recipe_path = checked_path(Path::new(get("--recipe")?), false)?;
    let bytes = read_input(&recipe_path, MAX_RECIPE_BYTES)?;
    let parent = recipe_path
        .parent()
        .ok_or("recipe requires a trusted parent")?;
    let mut remaining = MAX_FILE_BYTES - bytes.len();
    let model = retexture_model(&bytes, asset_id, |relative| {
        let input = read_input(&parent.join(relative), remaining)?;
        remaining -= input.len();
        Ok(input)
    })?;
    let cooked = model.to_bytes().map_err(|e| e.to_string())?;
    publish(&output, &cooked)?;
    let _ = writeln!(
        std::io::stdout(),
        "published {} bytes to {}",
        cooked.len(),
        output.display()
    );
    Ok(())
}
