//! Private implementation shared only with this example's acceptance tests.
//! Production Room admission and the existing spawn operation remain authoritative.

use orr_edit::{Op, Origin};
use orr_fp::{FPVec3, FP};
use orr_reflect::{decimal, Guid, Value};
use orr_sample::{room_game, room_project::PreparedProject};
use orr_sim::Simulation;
use serde_json::{json, Value as Json};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const MAX_NUMBER_BYTES: usize = 48;
pub const MAX_NAME_BYTES: usize = 128;
pub const MAX_GUID_BYTES: usize = 34;
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const BODY: &str = "orr_physics3d::Body";
const COLLIDER: &str = "orr_physics3d::Collider";
const USAGE: &str = "usage: room_wall --project ROOT --guid GUID --name NAME --center X Z --half-extents X Z --output FILE";

#[derive(Clone, Debug)]
pub struct Options {
    pub project: PathBuf,
    pub guid: Guid,
    pub name: String,
    pub center: [FP; 2],
    pub half_extents: [FP; 2],
    pub output: PathBuf,
}

impl Options {
    /// Arguments exclude the program name. Options may appear in any order,
    /// exactly once each; positional arguments and alternate spellings fail.
    pub fn parse(args: Vec<OsString>) -> Result<Self, String> {
        if args.len() != 14 {
            return Err(USAGE.into());
        }
        let (mut project, mut guid, mut name, mut center, mut half, mut output) =
            (None, None, None, None, None, None);
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let flag = logical(&flag, 14, "option")?;
            match flag {
                "--project" if project.is_none() => {
                    project = Some(path_arg(args.next(), "--project")?);
                }
                "--guid" if guid.is_none() => {
                    let value = next_arg(&mut args, "--guid")?;
                    guid = Some(Guid::parse(logical(&value, MAX_GUID_BYTES, "GUID")?)?);
                }
                "--name" if name.is_none() => {
                    let value = next_arg(&mut args, "--name")?;
                    name = Some(logical(&value, MAX_NAME_BYTES, "name")?.to_owned());
                }
                "--center" if center.is_none() => {
                    center = Some(number_pair(&mut args, "--center")?);
                }
                "--half-extents" if half.is_none() => {
                    half = Some(number_pair(&mut args, "--half-extents")?);
                }
                "--output" if output.is_none() => {
                    output = Some(path_arg(args.next(), "--output")?);
                }
                _ => return Err(format!("unknown or duplicate option; {USAGE}")),
            }
        }
        Ok(Self {
            project: project.ok_or(USAGE)?,
            guid: guid.ok_or(USAGE)?,
            name: name.ok_or(USAGE)?,
            center: center.ok_or(USAGE)?,
            half_extents: half.ok_or(USAGE)?,
            output: output.ok_or(USAGE)?,
        })
    }
}

fn logical<'a>(value: &'a OsStr, max: usize, what: &str) -> Result<&'a str, String> {
    // Bounds precede UTF-8 scanning, allocation, GUID parsing and FP parsing.
    if value.is_empty() || value.as_encoded_bytes().len() > max {
        return Err(format!("{what} must contain 1..={max} bytes"));
    }
    value
        .to_str()
        .ok_or_else(|| format!("{what} must be UTF-8"))
}

fn next_arg(args: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<OsString, String> {
    args.next()
        .ok_or_else(|| format!("missing value for {flag}"))
}

fn path_arg(value: Option<OsString>, flag: &str) -> Result<PathBuf, String> {
    let value = value.ok_or_else(|| format!("missing value for {flag}"))?;
    if value.is_empty() || value.as_encoded_bytes().len() > MAX_PATH_BYTES {
        return Err(format!(
            "{flag} must contain 1..={MAX_PATH_BYTES} native path bytes"
        ));
    }
    Ok(value.into())
}

fn number_pair(args: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<[FP; 2], String> {
    let mut result = [FP::ZERO; 2];
    for slot in &mut result {
        let value = next_arg(args, flag)?;
        let text = logical(&value, MAX_NUMBER_BYTES, "number")?;
        *slot = decimal::parse_fp(text)
            .map_err(|error| format!("{flag} requires an exact plain Q48.16 decimal: {error:?}"))?;
    }
    Ok(result)
}

/// Construct an ordinary existing spawn op, without reading or editing a project.
/// This alone is not Room admission; callers must apply it to a Room document.
pub fn wall_op(options: &Options) -> Result<Op, String> {
    logical(OsStr::new(&options.name), MAX_NAME_BYTES, "name")?;
    // The separate trusted setup frame supplies canonical Body/Collider values,
    // including hidden defaults. It never replaces an authored project's frame,
    // and it also works when that project has no existing wall to copy.
    let mut template = Simulation::<room_game::RoomEscapeV1>::new(
        room_game::RoomConfig,
        room_game::TICK_RATE,
        orr_sample::room_project::SEED,
    );
    let wall = template
        .frame()
        .entities()
        .find(|&entity| {
            template
                .frame()
                .get::<room_game::RoomActor>(entity)
                .is_some_and(|actor| actor.kind == room_game::WALL)
        })
        .ok_or("trusted Room setup has no wall template")?;
    let types = orr_sample::room_project::types();
    types
        .set_field(
            template.frame_mut(),
            wall,
            BODY,
            "pos",
            Value::Vec3(FPVec3::new(options.center[0], FP::HALF, options.center[1])),
        )
        .map_err(|error| error.to_string())?;
    types
        .set_field(
            template.frame_mut(),
            wall,
            COLLIDER,
            "shape",
            Value::Variant(
                "box".into(),
                vec![(
                    "half_extents".into(),
                    Value::Vec3(FPVec3::new(
                        options.half_extents[0],
                        FP::ONE,
                        options.half_extents[1],
                    )),
                )],
            ),
        )
        .map_err(|error| error.to_string())?;
    let components = [orr_sample::room_project::ACTOR, BODY, COLLIDER]
        .into_iter()
        .map(|name| {
            types
                .read_component(template.frame(), wall, name)
                .map(|value| (name.to_string(), value))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<_, _>>()?;
    Ok(Op::SpawnEntity {
        guid: Some(options.guid.clone()),
        name: Some(options.name.clone()),
        components,
    })
}

/// Produce bounded JSON and its admitted project root after private-edit admission.
/// No filesystem writes occur; the output path is used only by `run`.
pub fn generate(options: &Options) -> Result<(Vec<u8>, PathBuf), String> {
    // Validate inexpensive logical bounds even when tests construct Options.
    logical(OsStr::new(&options.name), MAX_NAME_BYTES, "name")?;
    let project = PreparedProject::open(&options.project)?;
    let mut document = orr_remote::room_escape::document(project.scene().text())?;
    if document.scene().entities.contains_key(&options.guid) {
        return Err("wall GUID already exists in the saved Room document".into());
    }
    document
        .apply(wall_op(options)?, Origin::Agent("room_wall".into()))
        .map_err(|error| format!("Room wall admission: {error}"))?;
    let entity = document
        .scene()
        .entities
        .get(&options.guid)
        .ok_or("admitted wall is missing from the private document")?;
    let components: serde_json::Map<String, Json> = entity
        .components
        .iter()
        .map(|(name, value)| (name.clone(), orr_remote::json::value_to_json(value)))
        .collect();
    let ops = json!([{
        "op": "spawn",
        "guid": options.guid.as_str(),
        "name": entity.name,
        "components": components,
    }]);
    let mut output = BoundedOutput(Vec::new());
    serde_json::to_writer_pretty(&mut output, &ops).map_err(|error| error.to_string())?;
    output.write_all(b"\n").map_err(|error| error.to_string())?;
    Ok((output.0, project.root().to_path_buf()))
}

struct BoundedOutput(Vec<u8>);
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_OUTPUT_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("wall operation exceeds 16 KiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Admit the complete proposal, then atomically publish a new regular file.
pub fn run(options: Options) -> Result<(), String> {
    let (bytes, root) = generate(&options)?;
    let destination = output_destination(&options.output, &root)?;
    publish(&destination, &bytes)
}

fn output_destination(path: &Path, project_root: &Path) -> Result<PathBuf, String> {
    check_path_spelling(path)?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("output current directory: {error}"))?
            .join(path)
    };
    check_path_spelling(&absolute)?;
    let parent = absolute
        .parent()
        .ok_or("output requires a parent directory")?;
    if absolute.file_name().is_none() {
        return Err("output requires a regular-file name".into());
    }
    // Inspect each ancestor without following symlinks. Existing output paths
    // of every kind, including dangling symlinks and hard-link aliases, fail.
    let mut ancestor = PathBuf::new();
    for component in parent.components() {
        ancestor.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let metadata =
            fs::symlink_metadata(&ancestor).map_err(|error| format!("output ancestor: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("output ancestry must contain only existing nonsymlink directories".into());
        }
    }
    let canonical_parent =
        fs::canonicalize(parent).map_err(|error| format!("output parent: {error}"))?;
    let canonical_root = fs::canonicalize(project_root)
        .map_err(|error| format!("admitted project root: {error}"))?;
    if canonical_parent.starts_with(&canonical_root) {
        return Err("output must be outside the admitted project root".into());
    }
    match fs::symlink_metadata(&absolute) {
        Ok(_) => return Err("output already exists; refusing to overwrite or follow it".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("output destination: {error}")),
    }
    Ok(absolute)
}

fn check_path_spelling(path: &Path) -> Result<(), String> {
    let mut bytes = path.as_os_str().as_encoded_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES {
        return Err("output path must contain 1..=4096 native path bytes".into());
    }
    if cfg!(windows) && path.has_root() && !path.is_absolute() {
        return Err("output does not support a drive-dependent rooted path".into());
    }
    if let Some(Component::Prefix(prefix)) = path.components().next() {
        #[cfg(windows)]
        if matches!(prefix.kind(), std::path::Prefix::Disk(_)) && path.is_absolute() {
            bytes = &bytes[2..];
        } else {
            return Err(
                "output does not support UNC, device, verbatim or drive-relative paths".into(),
            );
        }
        #[cfg(not(windows))]
        {
            let _ = prefix;
            return Err("output does not support platform path prefixes".into());
        }
    }
    let separator = |byte: u8| byte == b'/' || cfg!(windows) && byte == b'\\';
    if bytes.first().is_some_and(|&byte| separator(byte)) {
        bytes = &bytes[1..];
    }
    for part in bytes.split(|&byte| separator(byte)) {
        if part.is_empty()
            || part == b"."
            || part == b".."
            || part.contains(&b':')
            || (!cfg!(windows) && part.contains(&b'\\'))
            || part.last().is_some_and(|byte| matches!(byte, b'.' | b' '))
            || part.iter().any(|byte| *byte < 32 || *byte == 127)
        {
            return Err(
                "output path has an empty, dot, parent or platform-ambiguous component".into(),
            );
        }
        // Reject Windows device-name aliases on every platform so the same
        // spelling cannot become a device or alternate stream when copied.
        let stem = part.split(|&byte| byte == b'.').next().unwrap_or_default();
        let device = [b"CON".as_slice(), b"PRN", b"AUX", b"NUL"]
            .iter()
            .any(|name| stem.eq_ignore_ascii_case(name))
            || (stem.len() == 4
                && (stem[..3].eq_ignore_ascii_case(b"COM")
                    || stem[..3].eq_ignore_ascii_case(b"LPT"))
                && (b'1'..=b'9').contains(&stem[3]));
        if device {
            return Err("output path contains a reserved device name".into());
        }
    }
    Ok(())
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TemporaryOutput(PathBuf);
impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        // Never remove the requested destination, including after a failed link.
        let _ = fs::remove_file(&self.0);
    }
}

fn temporary(parent: &Path, destination: &Path) -> Result<(TemporaryOutput, File), String> {
    for _ in 0..32 {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".room-wall-{}-{serial}.tmp", std::process::id()));
        if path == destination {
            continue;
        }
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((TemporaryOutput(path), file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("output staging: {error}")),
        }
    }
    Err("output staging names exhausted after 32 attempts".into())
}

fn publish(destination: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_OUTPUT_BYTES {
        return Err("wall operation exceeds 16 KiB".into());
    }
    let parent = destination
        .parent()
        .ok_or("output requires a parent directory")?;
    let (temporary, mut file) = temporary(parent, destination)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("output staging write: {error}"))?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("output staging must be a regular file".into());
    }
    drop(file);
    // A same-directory hard link atomically fails if the destination exists.
    // Unlike rename, it can never replace a pre-existing leaf. There is no
    // overwrite fallback on filesystems which do not support hard links.
    fs::hard_link(&temporary.0, destination)
        .map_err(|error| format!("output publication (no overwrite): {error}"))?;
    Ok(())
}
