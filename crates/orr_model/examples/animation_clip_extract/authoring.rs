//! Private authoring code, shared only by this example and its acceptance tests.
//! No runtime/importer behavior is changed. Source and output ancestry must be
//! trusted and stable; portable std path checks do not defeat hostile same-user
//! replacement. Like the existing RAW16 publisher, publication uses a hard link
//! to a completely written private stage, never replacement or a copy fallback.
#![allow(clippy::float_arithmetic)]

use orr_model::{
    animation::{
        AnimatedModel, ChannelValues, Interpolation, Trs, MAX_ANIMATION_BYTES, MAX_ANIMATION_KEYS,
        MAX_CLIPS,
    },
    animation_import::import_with_resolver,
    MAX_FILE_BYTES,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_TABLE_ENTRIES: usize = 1024;
const SOURCE_ID: &str = "clip-source.glb";
static STAGE_ID: AtomicU64 = AtomicU64::new(0);

/// Offline traversal direction; runtime playback remains unchanged.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Direction {
    #[default]
    Forward,
    Reverse,
}

#[derive(Clone, Debug)]
pub struct ExtractOptions {
    pub clip_index: u32,
    pub start: f32,
    pub end: f32,
    pub name: String,
    /// Fixed positive playback ratio; 2 means twice as fast in the authored clip.
    pub speed: f32,
    pub direction: Direction,
}

impl ExtractOptions {
    fn validate(&self) -> Result<(), String> {
        if ![0.25, 0.5, 1.0, 2.0, 4.0].contains(&self.speed) {
            return Err("clip speed must be one of 0.25, 0.5, 1, 2, 4".into());
        }
        if !self.start.is_finite()
            || !self.end.is_finite()
            || self.start < 0.0
            || self.end <= self.start
            || !(self.end - self.start).is_finite()
        {
            return Err("clip range must be finite with 0 <= start < end".into());
        }
        if self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
        {
            return Err(
                "new clip name must contain 1..=128 UTF-8 bytes and no control characters".into(),
            );
        }
        Ok(())
    }
}

// Powers-of-two rates preserve binary time values except at overflow/underflow
// edges. Reject even a still-positive rounded subnormal rather than silently
// changing an authored key's relative timing. f64 represents every f32 / rate
// exactly for these five rates, so this check does not introduce a tolerance.
fn retime(time: f32, speed: f32) -> Result<f32, String> {
    let result = time / speed;
    if !result.is_finite() || f64::from(result) != f64::from(time) / f64::from(speed) {
        return Err("clip speed cannot preserve finite, exact key timing".into());
    }
    Ok(result)
}

fn word(bytes: &[u8], offset: usize) -> Result<usize, String> {
    let end = offset.checked_add(4).ok_or("GLB offset overflow")?;
    let bytes = bytes.get(offset..end).ok_or("truncated GLB")?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("four bytes")) as usize)
}

fn unpack(bytes: &[u8]) -> Result<(Value, &[u8]), String> {
    if bytes.len() > MAX_FILE_BYTES
        || bytes.len() < 28
        || &bytes[..4] != b"glTF"
        || word(bytes, 4)? != 2
        || word(bytes, 8)? != bytes.len()
    {
        return Err("input must be a bounded, exact-length GLB 2 file".into());
    }
    let json_len = word(bytes, 12)?;
    if json_len > MAX_JSON_BYTES || json_len % 4 != 0 || &bytes[16..20] != b"JSON" {
        return Err("GLB must begin with a bounded aligned JSON chunk".into());
    }
    let bin_header = 20usize.checked_add(json_len).ok_or("GLB length overflow")?;
    let json_bytes = bytes.get(20..bin_header).ok_or("truncated GLB JSON")?;
    let bin_len = word(bytes, bin_header)?;
    let bin_start = bin_header.checked_add(8).ok_or("GLB length overflow")?;
    if bytes.get(bin_header + 4..bin_start) != Some(b"BIN\0")
        || bin_len % 4 != 0
        || bin_start.checked_add(bin_len) != Some(bytes.len())
    {
        return Err(
            "GLB requires exactly one aligned BIN chunk; extra chunks are unsupported".into(),
        );
    }
    let document: Value =
        serde_json::from_slice(json_bytes).map_err(|e| format!("GLB JSON: {e}"))?;
    let buffers = document
        .get("buffers")
        .and_then(Value::as_array)
        .ok_or("GLB requires one embedded buffer")?;
    if buffers.len() != 1 || buffers[0].get("uri").is_some() {
        return Err("only one embedded BIN buffer without a URI is supported".into());
    }
    if let Some(images) = document.get("images").and_then(Value::as_array) {
        if images
            .iter()
            .any(|image| image.get("uri").is_some() || image.get("bufferView").is_none())
        {
            return Err("images must use embedded bufferViews; every URI is unsupported".into());
        }
    }
    Ok((document, &bytes[bin_start..]))
}

fn import(bytes: &[u8]) -> Result<AnimatedModel, String> {
    import_with_resolver(SOURCE_ID, bytes, |_| {
        Err(orr_model::Error::message(
            "external resources are unsupported",
        ))
    })
    .map_err(|e| format!("animation import: {e}"))
}

fn table<'a>(document: &'a Value, name: &str) -> Result<&'a Vec<Value>, String> {
    document
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing {name} table"))
}

fn table_mut<'a>(document: &'a mut Value, name: &str) -> Result<&'a mut Vec<Value>, String> {
    document
        .get_mut(name)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("missing {name} table"))
}

fn add(total: &mut usize, amount: usize, limit: usize, label: &str) -> Result<(), String> {
    *total = total
        .checked_add(amount)
        .ok_or_else(|| format!("{label} overflow"))?;
    if *total > limit {
        return Err(format!("{label} limit exceeded"));
    }
    Ok(())
}

fn append_accessor(
    document: &mut Value,
    binary: &mut Vec<u8>,
    data: &[f32],
    width: usize,
    times: bool,
) -> Result<usize, String> {
    let view = table(document, "bufferViews")?.len();
    let accessor = table(document, "accessors")?.len();
    let offset = binary.len();
    let length = data.len().checked_mul(4).ok_or("accessor size overflow")?;
    if view >= MAX_TABLE_ENTRIES
        || accessor >= MAX_TABLE_ENTRIES
        || offset
            .checked_add(length)
            .is_none_or(|n| n > MAX_FILE_BYTES)
    {
        return Err("generated accessor/binary limit exceeded".into());
    }
    for value in data {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    table_mut(document, "bufferViews")?
        .push(json!({"buffer":0,"byteOffset":offset,"byteLength":length}));
    let kind = match width {
        1 => "SCALAR",
        3 => "VEC3",
        4 => "VEC4",
        _ => return Err("invalid output width".into()),
    };
    let mut entry =
        json!({"bufferView":view,"componentType":5126,"count":data.len()/width,"type":kind});
    if times {
        entry["min"] = json!([data[0]]);
        entry["max"] = json!([data[data.len() - 1]]);
    }
    table_mut(document, "accessors")?.push(entry);
    Ok(accessor)
}

fn values_at(values: &ChannelValues, sample: &Trs) -> Vec<f32> {
    match values {
        ChannelValues::Translation(_) => sample.translation.to_vec(),
        ChannelValues::Rotation(_) => sample.rotation.to_vec(),
        ChannelValues::Scale(_) => sample.scale.to_vec(),
    }
}

/// Append a named subclip to a self-contained GLB. Every original accessor,
/// sampler, channel and BIN byte is retained. Inputs are never modified.
pub fn extract_glb(input: &[u8], options: &ExtractOptions) -> Result<Vec<u8>, String> {
    options.validate()?;
    let duration = retime(options.end - options.start, options.speed)?;
    if duration <= 0.0 {
        return Err("retimed clip duration must be positive".into());
    }
    let (mut document, original_binary) = unpack(input)?;
    let model = import(input)?;
    let source = model.source();
    let clip = source
        .clips
        .get(options.clip_index as usize)
        .ok_or("clip index is out of range")?;
    // STEP holds the earlier key, so reversing its key order would put a jump
    // on the wrong side. Reject the entire selected clip, including mixed tracks.
    if options.direction == Direction::Reverse
        && clip
            .channels
            .iter()
            .any(|channel| channel.interpolation == Interpolation::Step)
    {
        return Err(
            "reverse extraction requires LINEAR channels; selected clip contains STEP".into(),
        );
    }
    if options.end > clip.duration() {
        return Err("clip end exceeds the selected clip duration".into());
    }
    if source.clips.len() >= MAX_CLIPS {
        return Err("clip count limit exceeded".into());
    }
    if source.clips.iter().any(|c| c.name == options.name) {
        return Err("new clip name already exists".into());
    }

    // The original sampler inventory includes unused samplers; retain its budget
    // independently from the runtime channel inventory before allocating keys.
    let accessors = table(&document, "accessors")?;
    let mut sampler_keys = 0;
    for animation in table(&document, "animations")? {
        for sampler in animation["samplers"].as_array().ok_or("missing samplers")? {
            let index = sampler["input"].as_u64().ok_or("invalid sampler input")? as usize;
            let count = accessors
                .get(index)
                .and_then(|a| a["count"].as_u64())
                .ok_or("invalid sampler count")?;
            add(
                &mut sampler_keys,
                usize::try_from(count).map_err(|_| "sampler count overflow")?,
                MAX_ANIMATION_KEYS,
                "sampler key",
            )?;
        }
    }
    let mut keys = 0;
    let mut animation_bytes = 0;
    for channel in source.clips.iter().flat_map(|c| &c.channels) {
        add(
            &mut keys,
            channel.times.len(),
            MAX_ANIMATION_KEYS,
            "animation key",
        )?;
        let stride = if matches!(channel.values, ChannelValues::Rotation(_)) {
            20
        } else {
            16
        };
        add(
            &mut animation_bytes,
            channel.times.len() * stride,
            MAX_ANIMATION_BYTES,
            "animation byte",
        )?;
    }
    let mut appended_bytes = 0;
    for channel in &clip.channels {
        let count = 2 + channel
            .times
            .iter()
            .filter(|&&t| t > options.start && t < options.end)
            .count();
        add(&mut keys, count, MAX_ANIMATION_KEYS, "animation key")?;
        add(&mut sampler_keys, count, MAX_ANIMATION_KEYS, "sampler key")?;
        let stride = if matches!(channel.values, ChannelValues::Rotation(_)) {
            20
        } else {
            16
        };
        add(
            &mut appended_bytes,
            count * stride,
            MAX_ANIMATION_BYTES,
            "new animation byte",
        )?;
    }
    add(
        &mut animation_bytes,
        appended_bytes,
        MAX_ANIMATION_BYTES,
        "animation byte",
    )?;
    for name in ["accessors", "bufferViews"] {
        let mut count = table(&document, name)?.len();
        add(&mut count, 2 * clip.channels.len(), MAX_TABLE_ENTRIES, name)?;
    }
    let mut binary_size = original_binary.len();
    add(
        &mut binary_size,
        appended_bytes,
        MAX_FILE_BYTES,
        "binary byte",
    )?;
    let start_pose = model
        .sample_clip(options.clip_index, options.start)
        .map_err(|e| e.to_string())?;
    let end_pose = model
        .sample_clip(options.clip_index, options.end)
        .map_err(|e| e.to_string())?;
    let (first_pose, last_pose) = match options.direction {
        Direction::Forward => (&start_pose, &end_pose),
        Direction::Reverse => (&end_pose, &start_pose),
    };
    let mut binary = Vec::with_capacity(binary_size);
    binary.extend_from_slice(original_binary);
    let mut samplers = Vec::with_capacity(clip.channels.len());
    let mut channels = Vec::with_capacity(clip.channels.len());
    for channel in &clip.channels {
        let mut times = vec![0.0];
        let mut values = values_at(&channel.values, &first_pose.local()[channel.node as usize]);
        for position in 0..channel.times.len() {
            let i = match options.direction {
                Direction::Forward => position,
                Direction::Reverse => channel.times.len() - 1 - position,
            };
            let time = channel.times[i];
            if time <= options.start || time >= options.end {
                continue;
            }
            // Preserve forward f32 subtraction exactly. Mirroring has its own
            // f32 grid: reject collisions, without promising exact subtraction.
            let relative = match options.direction {
                Direction::Forward => time - options.start,
                Direction::Reverse => options.end - time,
            };
            let rebased = retime(relative, options.speed)?;
            if rebased <= *times.last().expect("initial key") || rebased >= duration {
                return Err("rebased key precision cannot preserve strict time ordering".into());
            }
            times.push(rebased);
            match &channel.values {
                ChannelValues::Translation(v) | ChannelValues::Scale(v) => {
                    values.extend_from_slice(&v[i])
                }
                ChannelValues::Rotation(v) => values.extend_from_slice(&v[i]),
            }
        }
        times.push(duration);
        values.extend(values_at(
            &channel.values,
            &last_pose.local()[channel.node as usize],
        ));
        let (path, width) = match &channel.values {
            ChannelValues::Translation(_) => ("translation", 3),
            ChannelValues::Rotation(_) => ("rotation", 4),
            ChannelValues::Scale(_) => ("scale", 3),
        };
        let input_index = append_accessor(&mut document, &mut binary, &times, 1, true)?;
        let output_index = append_accessor(&mut document, &mut binary, &values, width, false)?;
        let interpolation = match channel.interpolation {
            Interpolation::Step => "STEP",
            Interpolation::Linear => "LINEAR",
        };
        channels.push(json!({"sampler":samplers.len(),"target":{"node":channel.node,"path":path}}));
        samplers
            .push(json!({"input":input_index,"output":output_index,"interpolation":interpolation}));
    }
    table_mut(&mut document, "animations")?
        .push(json!({"name":options.name,"samplers":samplers,"channels":channels}));
    document["buffers"][0]["byteLength"] = json!(binary.len());
    let output = pack(&document, &binary)?;
    // The production importer, not our JSON writer, decides runtime admission.
    let generated = import(&output)?;
    let cooked = generated.to_bytes().map_err(|e| e.to_string())?;
    AnimatedModel::from_bytes(&cooked).map_err(|e| format!("generated cooked reload: {e}"))?;
    Ok(output)
}

struct BoundedJson(Vec<u8>);
impl Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > MAX_JSON_BYTES)
        {
            return Err(io::Error::other("generated JSON byte limit exceeded"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn pack(document: &Value, binary: &[u8]) -> Result<Vec<u8>, String> {
    let mut json = BoundedJson(Vec::new());
    serde_json::to_writer(&mut json, document).map_err(|e| e.to_string())?;
    while json.0.len() % 4 != 0 {
        json.write_all(b" ").map_err(|e| e.to_string())?;
    }
    let total = 28usize
        .checked_add(json.0.len())
        .and_then(|n| n.checked_add(binary.len()))
        .ok_or("GLB size overflow")?;
    if total > MAX_FILE_BYTES || binary.len() % 4 != 0 {
        return Err("generated GLB byte/alignment limit exceeded".into());
    }
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(b"glTF");
    output.extend_from_slice(&2u32.to_le_bytes());
    output.extend_from_slice(&(total as u32).to_le_bytes());
    output.extend_from_slice(&(json.0.len() as u32).to_le_bytes());
    output.extend_from_slice(b"JSON");
    output.extend_from_slice(&json.0);
    output.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    output.extend_from_slice(b"BIN\0");
    output.extend_from_slice(binary);
    Ok(output)
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
    let mut result = PathBuf::new();
    let components: Vec<_> = absolute
        .components()
        .filter(|p| !matches!(p, Component::CurDir))
        .collect();
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
fn read_input(path: &Path) -> Result<Vec<u8>, String> {
    let path = checked_path(path, false)?;
    let before = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !before.file_type().is_file() || before.len() > MAX_FILE_BYTES as u64 {
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
    let mut limited = file.take(MAX_FILE_BYTES as u64 + 1);
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    limited.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let after = limited.get_ref().metadata().map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE_BYTES
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
        || !name.ends_with(".glb")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err("output must be a portable ASCII .glb filename of at most 100 bytes, not a reserved device name".into());
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
            ".orr-clip-{}-{}.tmp",
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

/// Parse six required pairs and optional speed/direction pairs. No runtime/package writes.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let args: Vec<_> = args.into_iter().take(18).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return writeln!(std::io::stdout(),"Usage: animation_clip_extract --input FILE.glb --output NEW.glb --clip N --start SECONDS --end SECONDS --name NAME [--speed 0.25|0.5|1|2|4] [--direction forward|reverse]\nSpeed defaults to 1; 2 plays twice as fast. Direction defaults to forward; reverse requires selected LINEAR channels. Requires animation,import; embedded GLB STEP/LINEAR TRS only. Trusted, stable, link-free parents; output parent must exist. No overwrite/install. Exit 0: help/published (check warnings); 2: not published/error.").map_err(|e|e.to_string());
    }
    if args.len() != 12 && args.len() != 14 && args.len() != 16 {
        return Err(
            "six required option/value pairs and optional --speed/--direction expected; use --help"
                .into(),
        );
    }
    let mut values = BTreeMap::new();
    for pair in args.chunks_exact(2) {
        let key = pair[0].to_str().ok_or("option names must be UTF-8")?;
        if ![
            "--input",
            "--output",
            "--clip",
            "--start",
            "--end",
            "--name",
            "--speed",
            "--direction",
        ]
        .contains(&key)
        {
            return Err(format!("unknown option {key}"));
        }
        if values.insert(key, pair[1].clone()).is_some() {
            return Err(format!("duplicate option {key}"));
        }
    }
    let get = |key: &str| values.get(key).ok_or_else(|| format!("missing {key}"));
    let text = |key: &str| -> Result<&str, String> {
        let s = get(key)?
            .to_str()
            .ok_or_else(|| format!("{key} must be UTF-8"))?;
        if s.len() > 4096 {
            return Err(format!("{key} is too long"));
        }
        Ok(s)
    };
    let clip = text("--clip")?;
    if clip.is_empty() || !clip.bytes().all(|b| b.is_ascii_digit()) {
        return Err("--clip must be an unsigned integer".into());
    }
    let options = ExtractOptions {
        clip_index: clip.parse().map_err(|_| "clip index overflow")?,
        start: text("--start")?.parse().map_err(|_| "invalid --start")?,
        end: text("--end")?.parse().map_err(|_| "invalid --end")?,
        name: text("--name")?.into(),
        direction: match values.get("--direction") {
            None => Direction::Forward,
            Some(_) => match text("--direction")? {
                "forward" => Direction::Forward,
                "reverse" => Direction::Reverse,
                _ => return Err("--direction must be forward or reverse".into()),
            },
        },
        speed: match values.get("--speed") {
            None => 1.0,
            Some(_) => match text("--speed")? {
                "0.25" => 0.25,
                "0.5" => 0.5,
                "1" => 1.0,
                "2" => 2.0,
                "4" => 4.0,
                _ => return Err("--speed must be one of 0.25, 0.5, 1, 2, 4".into()),
            },
        },
    };
    options.validate()?;
    let destination = output_path(Path::new(get("--output")?))?;
    let input = read_input(Path::new(get("--input")?))?;
    let bytes = extract_glb(&input, &options)?;
    publish(&destination, &bytes)?;
    let _ = writeln!(
        std::io::stdout(),
        "published {} bytes to {}",
        bytes.len(),
        destination.display()
    );
    Ok(())
}
