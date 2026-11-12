//! Private GPU-free capped-cone authoring. No runtime or collision changes.
//! Paths require trusted stable ancestry; these portable checks are not a
//! sandbox against hostile concurrent directory replacement. Never overwrites.
#![allow(clippy::float_arithmetic)]
use orr_model::{
    Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY,
};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
pub const MAX_RECIPE_BYTES: usize = 16 * 1024;
static STAGE_ID: AtomicU64 = AtomicU64::new(0);

struct Recipe {
    format: String,
    version: u32,
    radius: f64,
    height: f64,
    segments: u32,
}
// Require a map, rather than accepting serde's positional sequence-to-struct
// alias. The visitor retains JSON types and rejects duplicate keys directly.
impl<'de> Deserialize<'de> for Recipe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RecipeVisitor;
        impl<'de> Visitor<'de> for RecipeVisitor {
            type Value = Recipe;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a cone recipe object with exactly five required fields")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Recipe, M::Error> {
                let (mut format, mut version, mut radius, mut height, mut segments) =
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
                        "radius" => {
                            if radius.is_some() {
                                return Err(de::Error::duplicate_field("radius"));
                            }
                            radius = Some(map.next_value()?);
                        }
                        "height" => {
                            if height.is_some() {
                                return Err(de::Error::duplicate_field("height"));
                            }
                            height = Some(map.next_value()?);
                        }
                        "segments" => {
                            if segments.is_some() {
                                return Err(de::Error::duplicate_field("segments"));
                            }
                            segments = Some(map.next_value()?);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &["format", "version", "radius", "height", "segments"],
                            ))
                        }
                    }
                }
                Ok(Recipe {
                    format: format.ok_or_else(|| de::Error::missing_field("format"))?,
                    version: version.ok_or_else(|| de::Error::missing_field("version"))?,
                    radius: radius.ok_or_else(|| de::Error::missing_field("radius"))?,
                    height: height.ok_or_else(|| de::Error::missing_field("height"))?,
                    segments: segments.ok_or_else(|| de::Error::missing_field("segments"))?,
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
    if recipe.format != "orr_static_model_cone" || recipe.version != 1 {
        return Err("unsupported cone recipe format/version".into());
    }
    logical_path(asset_id, ".cone.json")?;
    // Check the parsed f64 before narrowing: an out-of-bounds value that rounds
    // to a valid f32 boundary must still be rejected, never clamped.
    for (name, value) in [("radius", recipe.radius), ("height", recipe.height)] {
        if !value.is_finite() || !(0.001..=1_000_000.0).contains(&value) {
            return Err(format!("{name} must be finite and within [0.001,1000000]"));
        }
    }
    if !matches!(recipe.segments, 8 | 16 | 32 | 64 | 128) {
        return Err("segments must be an integer in {8,16,32,64,128}".into());
    }
    Ok(recipe)
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
/// Exact semantic comparison, including every floating-point sign bit. Derived
/// PartialEq alone treats positive and negative zero as equal.
fn same_source_bits(a: &ModelSource, b: &ModelSource) -> bool {
    a == b
        && a.materials
            .iter()
            .zip(&b.materials)
            .all(|(a, b)| a.base_color.map(f32::to_bits) == b.base_color.map(f32::to_bits))
        && a.primitives.iter().zip(&b.primitives).all(|(a, b)| {
            a.transform.map(|column| column.map(f32::to_bits))
                == b.transform.map(|column| column.map(f32::to_bits))
                && a.vertices.iter().zip(&b.vertices).all(|(a, b)| {
                    a.position.map(f32::to_bits) == b.position.map(f32::to_bits)
                        && a.normal.map(f32::to_bits) == b.normal.map(f32::to_bits)
                        && a.uv.map(f32::to_bits) == b.uv.map(f32::to_bits)
                })
        })
}

// Definition of the angular grid: IEEE-754 binary32 sine values for the first
// quadrant at multiples of pi/64, rounded once and stored as bits. All allowed
// segment counts are exact subdivisions of this fixed 128-point circle. Neither
// cooking nor loading calls libm; this table is part of this private recipe v1
// contract. The cardinal endpoints are exactly 0 and 1. This is presentation
// geometry, not simulation math or a claim about untested platform serializers.
const QUARTER_SIN_BITS: [u32; 33] = [
    0x00000000, 0x3d48fb30, 0x3dc8bd36, 0x3e164083, 0x3e47c5c2, 0x3e78cfcc, 0x3e94a031, 0x3eac7cd4,
    0x3ec3ef15, 0x3edae880, 0x3ef15aea, 0x3f039c3d, 0x3f0e39da, 0x3f187fc0, 0x3f226799, 0x3f2beb4a,
    0x3f3504f3, 0x3f3daef9, 0x3f45e403, 0x3f4d9f02, 0x3f54db31, 0x3f5b941a, 0x3f61c598, 0x3f676bd8,
    0x3f6c835e, 0x3f710908, 0x3f74fa0b, 0x3f7853f8, 0x3f7b14be, 0x3f7d3aac, 0x3f7ec46d, 0x3f7fb10f,
    0x3f800000,
];
fn circle_point(index: u32, segments: u32) -> [f32; 2] {
    let grid = (index % segments) * (128 / segments);
    let quadrant = grid / 32;
    let step = (grid % 32) as usize;
    let sine = f32::from_bits(QUARTER_SIN_BITS[step]);
    let cosine = f32::from_bits(QUARTER_SIN_BITS[32 - step]);
    let point = match quadrant {
        0 => [cosine, sine],
        1 => [-sine, cosine],
        2 => [-cosine, -sine],
        3 => [sine, -cosine],
        _ => unreachable!("bounded 128-point angular grid"),
    };
    // Exact positive-zero cardinal coordinates make welded positions and the
    // duplicate side seam bit-identical across both primitives.
    point.map(|value| if value == 0.0 { 0.0 } else { value })
}

/// Closed Y-centered cone with faceted sides (slot 0) and flat bottom (slot 1).
/// Each side triangle owns its vertices, including its midpoint-U apex. The
/// two seam positions are identical but carry U=0 and U=1. No top cap exists.
pub fn author_cone(recipe_bytes: &[u8], recipe_asset_id: &str) -> Result<StaticModel, String> {
    let recipe = recipe(recipe_bytes, recipe_asset_id)?;
    let radius = recipe.radius as f32;
    let half_height = (recipe.height as f32) * 0.5;
    let segments = recipe.segments;
    let mut sides = Primitive {
        id: format!("{recipe_asset_id}#node=0/mesh=0/primitive=0"),
        vertices: Vec::with_capacity(3 * segments as usize),
        indices: Vec::with_capacity(3 * segments as usize),
        material: 0,
        transform: IDENTITY,
    };
    for index in 0..segments {
        let [x, z] = circle_point(index, segments);
        let [nx, nz] = circle_point(index + 1, segments);
        let positions = [
            [radius * x, -half_height, radius * z],
            [0.0, half_height, 0.0],
            [radius * nx, -half_height, radius * nz],
        ];
        // Derive the face normal from the actual narrowed positions, not ideal
        // trigonometry. f64 intermediates avoid overflow/underflow across the
        // supported proportions. Fail rather than invent a normal on collapse.
        let p = positions.map(|p| p.map(f64::from));
        let a = std::array::from_fn::<_, 3, _>(|i| p[1][i] - p[0][i]);
        let b = std::array::from_fn::<_, 3, _>(|i| p[2][i] - p[0][i]);
        let cross = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
        if !length.is_finite() || length <= 0.0 {
            return Err("cone triangle collapsed during f32 conversion".into());
        }
        let normal = cross.map(|v| (v / length) as f32);
        if !normal.iter().all(|v| v.is_finite()) || normal == [0.0; 3] {
            return Err("cone normal collapsed during f32 conversion".into());
        }
        let uvs = [
            [index as f32 / segments as f32, 1.0],
            [(index as f32 + 0.5) / segments as f32, 0.0],
            [(index + 1) as f32 / segments as f32, 1.0],
        ];
        for (position, uv) in positions.into_iter().zip(uvs) {
            sides.vertices.push(Vertex {
                position,
                normal,
                uv,
            });
        }
        sides
            .indices
            .extend_from_slice(&[3 * index, 3 * index + 1, 3 * index + 2]);
    }
    let mut cap = Primitive {
        id: format!("{recipe_asset_id}#node=0/mesh=0/primitive=1"),
        vertices: Vec::with_capacity(segments as usize + 1),
        indices: Vec::with_capacity(3 * segments as usize),
        material: 1,
        transform: IDENTITY,
    };
    cap.vertices.push(Vertex {
        position: [0.0, -half_height, 0.0],
        normal: [0.0, -1.0, 0.0],
        uv: [0.5, 0.5],
    });
    for index in 0..segments {
        let [x, z] = circle_point(index, segments);
        cap.vertices.push(Vertex {
            position: [radius * x, -half_height, radius * z],
            normal: [0.0, -1.0, 0.0],
            uv: [0.5 + 0.5 * x, 0.5 + 0.5 * z],
        });
        cap.indices
            .extend_from_slice(&[0, index + 1, (index + 1) % segments + 1]);
    }
    let primitives = vec![sides, cap];
    let source = ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: recipe_asset_id.into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: digest(recipe_bytes),
        }],
        primitives,
        materials: vec![
            Material {
                base_color: [1.0; 4],
                image: 0,
                linear_filter: false,
                wrap_s: Wrap::Clamp,
                wrap_t: Wrap::Clamp,
            };
            2
        ],
        images: vec![Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4],
        }],
    };
    let model = StaticModel::new(source).map_err(|e| format!("cone model validation: {e}"))?;
    let cooked = model
        .to_bytes()
        .map_err(|e| format!("cone model cook: {e}"))?;
    let reloaded =
        StaticModel::from_bytes(&cooked).map_err(|e| format!("cone model reload: {e}"))?;
    if !same_source_bits(reloaded.source(), model.source()) {
        return Err("cone model changed during cooked round-trip".into());
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
        || !name.ends_with(".orrmodel")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err("output must be a portable ASCII .orrmodel filename of at most 100 bytes, not a reserved device name".into());
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
            ".orr-static-cone-{}-{}.tmp",
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

/// Strict bounded CLI. The recipe is the only input; nothing is installed.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let args: Vec<_> = args.into_iter().take(8).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return writeln!(std::io::stdout(), "Usage: static_model_cone --recipe FILE.cone.json --asset-id props/key.cone.json --output NEW.orrmodel\nRequires import. Closed Y-centered cone with radius/height in [0.001,1000000] and segments 8/16/32/64/128. Two stable side/bottom primitive and material slots; faceted side normals, hard bottom cap, explicit UV seam. No collision generation. Trusted stable link-free parents; no overwrite/install. Exit 0: help/published (check warnings); 2: error/not published.").map_err(|e| e.to_string());
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
    let model = author_cone(&bytes, asset_id)?;
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
