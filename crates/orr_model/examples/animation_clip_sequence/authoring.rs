//! Private offline authoring. No runtime or package-store mutations.
//! File operations require trusted stable ancestry, not a hostile-user sandbox.
#![allow(clippy::float_arithmetic)]
use orr_model::{
    animation::{
        AnimatedModel, AnimationChannel, AnimationClip, ChannelValues, Interpolation,
        MAX_ANIMATION_BYTES, MAX_ANIMATION_KEYS, MAX_CLIPS,
    },
    Dependency, MAX_FILE_BYTES,
};
use serde::{
    de::{self, MapAccess, Visitor},
    Deserialize, Deserializer,
};
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
    source: String,
    first_clip: u32,
    second_clip: u32,
    transition_seconds: f64,
    name: String,
}
impl<'de> Deserialize<'de> for Recipe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Object;
        impl<'de> Visitor<'de> for Object {
            type Value = Recipe;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a sequence recipe object with seven required fields")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Recipe, M::Error> {
                let (
                    mut format,
                    mut version,
                    mut source,
                    mut first_clip,
                    mut second_clip,
                    mut transition_seconds,
                    mut name,
                ) = (None, None, None, None, None, None, None);
                while let Some(key) = map.next_key::<String>()? {
                    macro_rules! field {
                        ($field:ident) => {{
                            if $field.is_some() {
                                return Err(de::Error::duplicate_field(stringify!($field)));
                            }
                            $field = Some(map.next_value()?);
                        }};
                    }
                    match key.as_str() {
                        "format" => field!(format),
                        "version" => field!(version),
                        "source" => field!(source),
                        "first_clip" => field!(first_clip),
                        "second_clip" => field!(second_clip),
                        "transition_seconds" => field!(transition_seconds),
                        "name" => field!(name),
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &[
                                    "format",
                                    "version",
                                    "source",
                                    "first_clip",
                                    "second_clip",
                                    "transition_seconds",
                                    "name",
                                ],
                            ))
                        }
                    }
                }
                macro_rules! required {
                    ($field:ident) => {
                        $field.ok_or_else(|| de::Error::missing_field(stringify!($field)))?
                    };
                }
                Ok(Recipe {
                    format: required!(format),
                    version: required!(version),
                    source: required!(source),
                    first_clip: required!(first_clip),
                    second_clip: required!(second_clip),
                    transition_seconds: required!(transition_seconds),
                    name: required!(name),
                })
            }
        }
        deserializer.deserialize_map(Object)
    }
}
fn reserved(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or("").to_ascii_uppercase();
    ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
}
fn logical_path(path: &str, suffix: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 240
        || !path.ends_with(suffix)
        || !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
        || path
            .split('/')
            .any(|s| s.is_empty() || s.starts_with('.') || s.ends_with('.') || reserved(s))
    {
        return Err(format!(
            "expected normalized portable relative {suffix} path (at most 240 bytes)"
        ));
    }
    Ok(())
}
fn parse_recipe(bytes: &[u8], asset_id: &str) -> Result<Recipe, String> {
    if bytes.len() > MAX_RECIPE_BYTES {
        return Err("recipe exceeds 16 KiB limit".into());
    }
    let r: Recipe = serde_json::from_slice(bytes).map_err(|e| format!("recipe JSON: {e}"))?;
    if r.format != "orr_animation_clip_sequence" || r.version != 1 {
        return Err("unsupported sequence recipe format/version".into());
    }
    logical_path(asset_id, ".sequence.json")?;
    logical_path(&r.source, ".animated.json")?;
    if r.name.trim().is_empty() || r.name.len() > 128 || r.name.chars().any(char::is_control) {
        return Err("new clip name must have 1..=128 UTF-8 bytes and no control characters".into());
    }
    if !r.transition_seconds.is_finite()
        || r.transition_seconds <= 0.0
        || r.transition_seconds > 60.0
        || r.transition_seconds as f32 == 0.0
    {
        return Err("transition_seconds must be finite in (0,60] and positive as f32".into());
    }
    Ok(r)
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn target(c: &AnimationChannel) -> (u32, u8) {
    (
        c.node,
        match c.values {
            ChannelValues::Translation(_) => 0,
            ChannelValues::Rotation(_) => 1,
            ChannelValues::Scale(_) => 2,
        },
    )
}
fn add_budget(total: &mut usize, n: usize, limit: usize) -> Result<(), String> {
    *total = total
        .checked_add(n)
        .filter(|&v| v <= limit)
        .ok_or("animation budget exceeded")?;
    Ok(())
}
// Endpoint clamping is explicit even for a channel shorter than its clip or
// whose first key is later than zero. Original values are retained, including
// quaternion signs; the existing LINEAR runtime uses shortest-path slerp.
fn endpoints(c: &AnimationChannel, duration: f32) -> Vec<(f32, usize)> {
    let mut out = Vec::with_capacity(c.times.len() + 2);
    if c.times[0] > 0.0 {
        out.push((0.0, 0));
    }
    out.extend(c.times.iter().copied().enumerate().map(|(i, t)| (t, i)));
    if *c.times.last().expect("validated channel") < duration {
        out.push((duration, c.times.len() - 1));
    }
    out
}
fn concatenate_values(
    a: &ChannelValues,
    b: &ChannelValues,
    ai: &[(f32, usize)],
    bi: &[(f32, usize)],
) -> Result<ChannelValues, String> {
    macro_rules! combine {
        ($a:ident,$b:ident,$kind:ident) => {
            ChannelValues::$kind(
                ai.iter()
                    .map(|&(_, i)| $a[i])
                    .chain(bi.iter().map(|&(_, i)| $b[i]))
                    .collect(),
            )
        };
    }
    Ok(match (a, b) {
        (ChannelValues::Translation(a), ChannelValues::Translation(b)) => {
            combine!(a, b, Translation)
        }
        (ChannelValues::Rotation(a), ChannelValues::Rotation(b)) => combine!(a, b, Rotation),
        (ChannelValues::Scale(a), ChannelValues::Scale(b)) => combine!(a, b, Scale),
        _ => return Err("channel property mismatch".into()),
    })
}
struct BoundedBytes(Vec<u8>);
impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > MAX_FILE_BYTES)
        {
            return Err(io::Error::other("output exceeds 64 MiB limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn cooked(model: &AnimatedModel) -> Result<Vec<u8>, String> {
    let mut out = BoundedBytes(Vec::new());
    serde_json::to_writer(&mut out, model.source())
        .map_err(|e| format!("bounded cooked output: {e}"))?;
    Ok(out.0)
}
/// Append A, an explicit endpoint transition, and B. This is not crossfade.
/// The resolver is invoked exactly once, only after strict recipe validation.
pub fn sequence_clips(
    recipe_bytes: &[u8],
    recipe_asset_id: &str,
    mut resolver: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<AnimatedModel, String> {
    let recipe = parse_recipe(recipe_bytes, recipe_asset_id)?;
    let input = resolver(&recipe.source)?;
    recipe_bytes
        .len()
        .checked_add(input.len())
        .filter(|&n| n <= MAX_FILE_BYTES)
        .ok_or("recipe plus source exceeds 64 MiB input budget")?;
    let original =
        AnimatedModel::from_bytes(&input).map_err(|e| format!("source animated model: {e}"))?;
    let source = original.source();
    if recipe_asset_id.eq_ignore_ascii_case(&source.asset_id) {
        return Err("new asset identity must differ from source identity".into());
    }
    if source.clips.len() >= MAX_CLIPS {
        return Err("clip count limit exceeded".into());
    }
    if source.clips.iter().any(|c| c.name == recipe.name) {
        return Err("new clip name already exists".into());
    }
    let a = source
        .clips
        .get(recipe.first_clip as usize)
        .ok_or("first_clip index out of range")?;
    let b = source
        .clips
        .get(recipe.second_clip as usize)
        .ok_or("second_clip index out of range")?;
    if a.channels
        .iter()
        .chain(&b.channels)
        .any(|c| c.interpolation != Interpolation::Linear)
    {
        return Err("both selected clips must use only LINEAR channels".into());
    }
    let ac: BTreeMap<_, _> = a.channels.iter().map(|c| (target(c), c)).collect();
    let bc: BTreeMap<_, _> = b.channels.iter().map(|c| (target(c), c)).collect();
    if ac.keys().ne(bc.keys()) {
        return Err("selected clips must have identical node/property target sets".into());
    }
    let da = a.duration();
    let db = b.duration();
    let offset = da + recipe.transition_seconds as f32;
    let duration = offset + db;
    if !offset.is_finite()
        || offset <= da
        || !duration.is_finite()
        || (db > 0.0 && duration <= offset)
        || duration > 3600.0
        || f64::from(da) + recipe.transition_seconds + f64::from(db) > 3600.0
    {
        return Err(
            "sequence duration exceeds 3600 seconds or f32 transition/timeline collapses".into(),
        );
    }
    let (mut keys, mut bytes) = (0, 0);
    for c in source.clips.iter().flat_map(|c| &c.channels) {
        add_budget(&mut keys, c.times.len(), MAX_ANIMATION_KEYS)?;
        add_budget(
            &mut bytes,
            c.times.len() * if target(c).1 == 1 { 20 } else { 16 },
            MAX_ANIMATION_BYTES,
        )?;
    }
    let mut channels = Vec::with_capacity(ac.len());
    for (key, a) in ac {
        let b = bc[&key];
        let ai = endpoints(a, da);
        let bi = endpoints(b, db);
        let count = ai.len() + bi.len();
        add_budget(&mut keys, count, MAX_ANIMATION_KEYS)?;
        add_budget(
            &mut bytes,
            count * if key.1 == 1 { 20 } else { 16 },
            MAX_ANIMATION_BYTES,
        )?;
        let mut times = Vec::with_capacity(count);
        for t in ai
            .iter()
            .map(|&(t, _)| t)
            .chain(bi.iter().map(|&(t, _)| offset + t))
        {
            if !t.is_finite() || times.last().is_some_and(|&last| t <= last) {
                return Err(
                    "f32 shifted key timing collapses or is not strictly increasing".into(),
                );
            }
            times.push(t);
        }
        channels.push(AnimationChannel {
            node: key.0,
            interpolation: Interpolation::Linear,
            times,
            values: concatenate_values(&a.values, &b.values, &ai, &bi)?,
        });
    }
    let mut result = source.clone();
    for primitive in &mut result.primitives {
        let suffix = primitive
            .id
            .strip_prefix(&source.asset_id)
            .filter(|s| s.starts_with("#node="))
            .ok_or("unexpected source primitive identity")?;
        primitive.id = format!("{recipe_asset_id}{suffix}");
    }
    result.asset_id = recipe_asset_id.into();
    result.dependencies = vec![
        Dependency {
            uri: "$source".into(),
            sha256: digest(recipe_bytes),
        },
        Dependency {
            uri: recipe.source,
            sha256: digest(&input),
        },
    ];
    result.clips.push(AnimationClip {
        name: recipe.name,
        channels,
    });
    let model =
        AnimatedModel::new(result).map_err(|e| format!("sequence model validation: {e}"))?;
    let bytes = cooked(&model)?;
    let reloaded =
        AnimatedModel::from_bytes(&bytes).map_err(|e| format!("sequence cooked reload: {e}"))?;
    if cooked(&reloaded)? != bytes {
        return Err("sequence changed during cooked round-trip".into());
    }
    Ok(reloaded)
}

fn checked_path(path: &Path, output: bool) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        return Err("empty, parent-traversal or platform-prefixed paths unsupported".into());
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let parts: Vec<_> = absolute
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    let mut checked = PathBuf::new();
    for (i, c) in parts.iter().enumerate() {
        if let Component::Normal(name) = c {
            let name = name.to_str().ok_or("filesystem paths must be UTF-8")?;
            if name.ends_with(['.', ' ']) || reserved(name) {
                return Err("nonportable path component".into());
            }
            if output && name.eq_ignore_ascii_case(".orr") {
                return Err("output cannot be inside an .orr package store".into());
            }
        }
        checked.push(c.as_os_str());
        match fs::symlink_metadata(&checked) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err("symlink path/ancestry unsupported".into())
            }
            Ok(m) if i + 1 < parts.len() && !m.is_dir() => {
                return Err("path ancestor is not a directory".into())
            }
            Ok(_) => {}
            Err(e) if output && i + 1 == parts.len() && e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("path metadata: {e}")),
        }
    }
    Ok(checked)
}
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}
fn owned_path(path: &Path, file: &File) -> Result<(), String> {
    let path_meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let file_meta = file.metadata().map_err(|e| e.to_string())?;
    if !path_meta.file_type().is_file()
        || !file_meta.is_file()
        || !same_file(&path_meta, &file_meta)
    {
        return Err("file path identity changed; refusing to operate on replacement".into());
    }
    Ok(())
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn safe_open(path: &Path) -> Result<File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    // Linux x86_64 O_NOFOLLOW | O_NONBLOCK: reject final symlinks atomically and
    // avoid blocking if a regular input was replaced by a FIFO before open.
    OpenOptions::new()
        .read(true)
        .custom_flags(0x20000 | 0x800)
        .open(path)
        .map_err(|e| format!("nofollow input open: {e}"))
}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn safe_open(_path: &Path) -> Result<File, String> {
    Err("safe nofollow/nonblocking file operations currently require Linux x86_64; pure sequence API is portable".into())
}
fn read_input(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let path = checked_path(path, false)?;
    let before = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !before.file_type().is_file() || before.len() > limit as u64 {
        return Err("input must be a bounded regular file".into());
    }
    let file = safe_open(&path)?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if !opened.is_file()
        || !same_file(&before, &opened)
        || before.len() != opened.len()
        || before.modified().ok() != opened.modified().ok()
    {
        return Err("input changed during open".into());
    }
    owned_path(&path, &file)?;
    let mut bounded = file.take(limit as u64 + 1);
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    bounded.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let after = bounded.get_ref().metadata().map_err(|e| e.to_string())?;
    if bytes.len() > limit
        || bytes.len() as u64 != opened.len()
        || after.len() != opened.len()
        || after.modified().ok() != opened.modified().ok()
    {
        return Err("input changed during bounded read".into());
    }
    checked_path(&path, false)?;
    owned_path(&path, bounded.get_ref())?;
    Ok(bytes)
}
fn output_path(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("output needs portable filename")?;
    logical_path(name, ".animated.json")?;
    if name.len() > 100 {
        return Err("output filename exceeds 100 bytes".into());
    }
    let path = checked_path(path, true)?;
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(path),
        Ok(_) => Err("output already exists; no overwrite/force supported".into()),
        Err(e) => Err(e.to_string()),
    }
}
fn publish(output: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("output exceeds 64 MiB limit".into());
    }
    let destination = output_path(output)?;
    let parent = destination
        .parent()
        .ok_or("output requires an existing parent")?;
    let mut stage = None;
    for _ in 0..32 {
        let path = parent.join(format!(
            ".orr-animation-sequence-{}-{}.tmp",
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
            Err(e) => return Err(format!("create owned output stage: {e}")),
        }
    }
    let (path, mut file) = stage.ok_or("stage-name budget exhausted")?;
    let commit = (|| {
        owned_path(&path, &file)?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut readback = Vec::with_capacity(bytes.len());
        (&mut file)
            .take(bytes.len() as u64 + 1)
            .read_to_end(&mut readback)
            .map_err(|e| e.to_string())?;
        if readback != bytes {
            return Err("owned stage readback differs".into());
        }
        output_path(&destination)?;
        checked_path(&path, false)?;
        owned_path(&path, &file)?;
        fs::hard_link(&path, &destination)
            .map_err(|e| format!("no-replace hard-link publication required: {e}"))?;
        Ok::<(), String>(())
    })();
    // Only remove the entry if it still identifies our open stage. A replacement
    // is never unlinked, including after a failed publication attempt.
    let cleanup =
        owned_path(&path, &file).and_then(|()| fs::remove_file(&path).map_err(|e| e.to_string()));
    if let Err(error) = commit {
        return Err(format!(
            "not published: {error}{}",
            cleanup
                .err()
                .map(|e| format!("; stage cleanup refused/failed: {e}"))
                .unwrap_or_default()
        ));
    }
    if let Err(error) = cleanup {
        let _ = writeln!(
            io::stderr(),
            "warning: published; stage cleanup refused/failed: {error}"
        );
    }
    if let Err(error) = File::open(parent).and_then(|f| f.sync_all()) {
        let _ = writeln!(
            io::stderr(),
            "warning: published; directory durability uncertain: {error}"
        );
    }
    Ok(())
}
/// Strict CLI; no imports, installs, or writes to the source files.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let args: Vec<_> = args.into_iter().take(8).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return writeln!(io::stdout(),"Usage: animation_clip_sequence --recipe FILE.sequence.json --asset-id sequences/action.sequence.json --output NEW.animated.json\nRequires animation,import. Append A, positive endpoint transition, B from two matching LINEAR cooked clips. Not crossfade. Linux x86_64 filesystem CLI; trusted stable link-free ancestry; absent output; no overwrite/install. Exit 0: help/published (check warnings); 2: error/not published.").map_err(|e|e.to_string());
    }
    if args.len() != 6 {
        return Err("expected three unique option/value pairs; use --help".into());
    }
    let mut options = BTreeMap::new();
    for pair in args.chunks_exact(2) {
        let key = pair[0].to_str().ok_or("option names must be UTF-8")?;
        if !["--recipe", "--asset-id", "--output"].contains(&key) {
            return Err(format!("unknown option {key}"));
        }
        if pair[1].as_os_str().len() > 4096 {
            return Err(format!("{key} exceeds path/text limit"));
        }
        if options.insert(key, &pair[1]).is_some() {
            return Err(format!("duplicate option {key}"));
        }
    }
    let get = |key: &str| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| format!("missing {key}"))
    };
    let identity = get("--asset-id")?
        .to_str()
        .ok_or("asset identity must be UTF-8")?;
    let output = output_path(Path::new(get("--output")?))?;
    let recipe_path = checked_path(Path::new(get("--recipe")?), false)?;
    let recipe = read_input(&recipe_path, MAX_RECIPE_BYTES)?;
    let parent = recipe_path.parent().ok_or("recipe parent missing")?;
    let model = sequence_clips(&recipe, identity, |relative| {
        read_input(&parent.join(relative), MAX_FILE_BYTES - recipe.len())
    })?;
    let bytes = cooked(&model)?;
    publish(&output, &bytes)?;
    let _ = writeln!(
        io::stdout(),
        "published {} bytes to {}",
        bytes.len(),
        output.display()
    );
    Ok(())
}
