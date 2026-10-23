//! Private offline cooked static-model assembly; no runtime or renderer changes.
//! Paths require trusted stable ancestry; these portable checks are not a
//! sandbox against hostile concurrent directory replacement. Never overwrites.
#![allow(clippy::float_arithmetic)]
use orr_model::{
    normal_matrix, Dependency, ModelSource, StaticModel, IDENTITY, MAX_DECODED_BYTES,
    MAX_FILE_BYTES, MAX_IMAGES, MAX_INDICES, MAX_MATERIALS, MAX_PRIMITIVES, MAX_VERTICES,
};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fmt,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
pub const MAX_RECIPE_BYTES: usize = 16 * 1024;
pub const MAX_PARTS: usize = 32;
static STAGE_ID: AtomicU64 = AtomicU64::new(0);

// Explicit map visitors reject positional sequence aliases, unknown fields,
// duplicate fields, and missing fields without discarding JSON keys.
macro_rules! strict_map {
    ($name:ident { $($field:ident: $ty:ty),+ $(,)? }) => {
        struct $name { $($field: $ty),+ }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct ObjectVisitor;
                impl<'de> Visitor<'de> for ObjectVisitor {
                    type Value = $name;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str(concat!("a strict ", stringify!($name), " object"))
                    }
                    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<$name, M::Error> {
                        $(let mut $field = None;)+
                        while let Some(key) = map.next_key::<String>()? {
                            match key.as_str() {
                                $(stringify!($field) => {
                                    if $field.is_some() { return Err(de::Error::duplicate_field(stringify!($field))); }
                                    $field = Some(map.next_value()?);
                                },)+
                                _ => return Err(de::Error::unknown_field(&key, &[$(stringify!($field)),+])),
                            }
                        }
                        Ok($name { $($field: $field.ok_or_else(|| de::Error::missing_field(stringify!($field)))?),+ })
                    }
                }
                deserializer.deserialize_map(ObjectVisitor)
            }
        }
    };
}
strict_map!(Recipe { format: String, version: u32, parts: Vec<Part> });
strict_map!(Part {
    key: String,
    source: String,
    placement: [[f32; 4]; 4]
});

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
    if recipe.format != "orr_static_model_assemble" || recipe.version != 1 {
        return Err("unsupported assembly recipe format/version".into());
    }
    logical_path(asset_id, ".assemble.json")?;
    if recipe.parts.is_empty() || recipe.parts.len() > MAX_PARTS {
        return Err("assembly requires 1–32 parts".into());
    }
    let mut keys = BTreeSet::new();
    let mut sources = BTreeMap::new();
    for part in &recipe.parts {
        if part.key.is_empty()
            || part.key.len() > 64
            || !part.key.as_bytes()[0].is_ascii_alphanumeric()
            || !part
                .key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
            || !keys.insert(part.key.to_ascii_lowercase())
        {
            return Err("part keys must be unique portable ASCII identifiers of 1–64 bytes".into());
        }
        logical_path(&part.source, ".model.json")?;
        if asset_id.eq_ignore_ascii_case(&part.source) {
            return Err("recipe identity and input path must be distinct".into());
        }
        if let Some(previous) = sources.insert(part.source.to_ascii_lowercase(), &part.source) {
            if previous != &part.source {
                return Err("case-folded source path collision".into());
            }
        }
        normal_matrix(part.placement).map_err(|e| format!("part {} placement: {e}", part.key))?;
    }
    Ok(recipe)
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn charge(total: &mut usize, amount: usize, limit: usize, label: &str) -> Result<(), String> {
    *total = total
        .checked_add(amount)
        .filter(|&sum| sum <= limit)
        .ok_or_else(|| format!("assembly {label} budget exceeded"))?;
    Ok(())
}
fn compose(placement: [[f32; 4]; 4], original: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    // Numeric identity includes either sign of zero. Preserve source bits.
    if placement == IDENTITY {
        return original;
    }
    let mut result = [[0.0; 4]; 4];
    for (c, column) in result.iter_mut().enumerate() {
        for (r, value) in column.iter_mut().enumerate() {
            *value = placement[0][r] * original[c][0]
                + placement[1][r] * original[c][1]
                + placement[2][r] * original[c][2]
                + placement[3][r] * original[c][3];
        }
    }
    result
}
/// Fully admit each exact cooked source once, including unused resources. Append
/// full tables in first-use order; repeated placements only duplicate primitives.
/// Resolve no historical dependencies. Mirrored winding belongs to the renderer.
pub fn assemble_model(
    recipe_bytes: &[u8],
    recipe_asset_id: &str,
    mut resolver: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<StaticModel, String> {
    let recipe = recipe(recipe_bytes, recipe_asset_id)?;
    let mut result = ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: recipe_asset_id.into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: digest(recipe_bytes),
        }],
        primitives: Vec::new(),
        materials: Vec::new(),
        images: Vec::new(),
    };
    let mut sources: BTreeMap<String, (StaticModel, u32)> = BTreeMap::new();
    let (mut input_bytes, mut decoded_bytes, mut vertices, mut indices) =
        (recipe_bytes.len(), 0, 0, 0);
    for part in recipe.parts {
        if !sources.contains_key(&part.source) {
            let bytes = resolver(&part.source)?;
            charge(
                &mut input_bytes,
                bytes.len(),
                MAX_FILE_BYTES,
                "unique input byte",
            )?;
            let model = StaticModel::from_bytes(&bytes)
                .map_err(|e| format!("source {}: {e}", part.source))?;
            let source = model.source();
            let image_base =
                u32::try_from(result.images.len()).map_err(|_| "image slot overflow")?;
            let material_base =
                u32::try_from(result.materials.len()).map_err(|_| "material slot overflow")?;
            let mut image_count = result.images.len();
            let mut material_count = result.materials.len();
            charge(
                &mut image_count,
                source.images.len(),
                MAX_IMAGES,
                "image count",
            )?;
            charge(
                &mut material_count,
                source.materials.len(),
                MAX_MATERIALS,
                "material count",
            )?;
            for image in &source.images {
                charge(
                    &mut decoded_bytes,
                    image.rgba8.len(),
                    MAX_DECODED_BYTES,
                    "decoded image",
                )?;
            }
            result.images.extend(source.images.iter().cloned());
            for material in &source.materials {
                let mut material = material.clone();
                material.image = material
                    .image
                    .checked_add(image_base)
                    .ok_or("image slot overflow")?;
                result.materials.push(material);
            }
            result.dependencies.push(Dependency {
                uri: part.source.clone(),
                sha256: digest(&bytes),
            });
            sources.insert(part.source.clone(), (model, material_base));
        }
        let (model, material_base) = sources.get(&part.source).ok_or("missing admitted source")?;
        let source = model.source();
        let mut primitive_count = result.primitives.len();
        charge(
            &mut primitive_count,
            source.primitives.len(),
            MAX_PRIMITIVES,
            "primitive count",
        )?;
        for primitive in &source.primitives {
            charge(
                &mut vertices,
                primitive.vertices.len(),
                MAX_VERTICES,
                "vertex count",
            )?;
            charge(
                &mut indices,
                primitive.indices.len(),
                MAX_INDICES,
                "index count",
            )?;
            let suffix = primitive
                .id
                .strip_prefix(&source.asset_id)
                .filter(|s| s.starts_with("#node="))
                .ok_or("unexpected source primitive identity")?;
            let mut primitive = primitive.clone();
            primitive.id = format!("{recipe_asset_id}#node=part:{}{suffix}", part.key);
            primitive.material = primitive
                .material
                .checked_add(*material_base)
                .ok_or("material slot overflow")?;
            primitive.transform = compose(part.placement, primitive.transform);
            normal_matrix(primitive.transform)
                .map_err(|e| format!("composed part {}: {e}", part.key))?;
            result.primitives.push(primitive);
        }
    }
    // Static admission checks composed world bounds as well as all table slots.
    let model = StaticModel::new(result).map_err(|e| format!("assembled model validation: {e}"))?;
    let cooked = model
        .to_bytes()
        .map_err(|e| format!("assembled model cook: {e}"))?;
    let reloaded =
        StaticModel::from_bytes(&cooked).map_err(|e| format!("assembled model reload: {e}"))?;
    // Canonical serialization distinguishes signed zeros (PartialEq does not).
    if reloaded.to_bytes().map_err(|e| e.to_string())? != cooked {
        return Err("assembled model changed during exact cooked round-trip".into());
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
            ".orr-static-assemble-{}-{}.tmp",
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

/// Strict bounded CLI. The recipe parent anchors the normalized cooked source input.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let args: Vec<_> = args.into_iter().take(8).collect();
    if args.len() == 1 && args[0] == "--help" {
        return writeln!(std::io::stdout(), "Usage: static_model_assemble --recipe FILE.assemble.json --asset-id variants/chair.assemble.json --output NEW.model.json\nRequires import. Each part receives placement * M; geometry and winding stay unchanged; source tables are reused. Recipe-relative inputs; trusted stable link-free parents; no overwrite/install. Exit 0: help/published (check warnings); 2: error/not published.").map_err(|e| e.to_string());
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
    let model = assemble_model(&bytes, asset_id, |relative| {
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
