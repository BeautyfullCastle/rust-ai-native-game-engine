//! Private implementation shared only with this example's acceptance tests.
//! Production Room admission remains authoritative.

use orr_edit::{Op, Origin};
use orr_fp::{FPVec3, FP};
use orr_reflect::{decimal, Value};
use orr_sample::{room_game, room_project::PreparedProject};
use serde_json::{json, Value as Json};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const MAX_NUMBER_BYTES: usize = 48;
pub const MAX_OUTPUT_BYTES: usize = 4 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const BODY: &str = "orr_physics3d::Body";
const USAGE: &str =
    "usage: room_layout --project ROOT --player X Z --key X Z --exit X Z --output FILE";

#[derive(Clone, Debug)]
pub struct Options {
    pub project: PathBuf,
    pub player: [FP; 2],
    pub key: [FP; 2],
    pub exit: [FP; 2],
    pub output: PathBuf,
}

impl Options {
    /// Arguments exclude the program name. Each option appears exactly once.
    pub fn parse(args: Vec<OsString>) -> Result<Self, String> {
        if args.len() != 13 {
            return Err(USAGE.into());
        }
        let (mut project, mut player, mut key, mut exit, mut output) =
            (None, None, None, None, None);
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let flag = logical(&flag, 14, "option")?;
            match flag {
                "--project" if project.is_none() => {
                    project = Some(path_arg(args.next(), "--project")?);
                }
                "--player" if player.is_none() => {
                    player = Some(number_pair(&mut args, "--player")?);
                }
                "--key" if key.is_none() => {
                    key = Some(number_pair(&mut args, "--key")?);
                }
                "--exit" if exit.is_none() => {
                    exit = Some(number_pair(&mut args, "--exit")?);
                }
                "--output" if output.is_none() => {
                    output = Some(path_arg(args.next(), "--output")?);
                }
                _ => return Err(format!("unknown or duplicate option; {USAGE}")),
            }
        }
        Ok(Self {
            project: project.ok_or(USAGE)?,
            player: player.ok_or(USAGE)?,
            key: key.ok_or(USAGE)?,
            exit: exit.ok_or(USAGE)?,
            output: output.ok_or(USAGE)?,
        })
    }
}

fn logical<'a>(value: &'a OsStr, max: usize, what: &str) -> Result<&'a str, String> {
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

/// Produce bounded patch JSON and the admitted project root without writing files.
pub fn generate(options: &Options) -> Result<(Vec<u8>, PathBuf), String> {
    if options.project.as_os_str().is_empty()
        || options.project.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES
    {
        return Err("project path must contain 1..=4096 native path bytes".into());
    }
    let project = PreparedProject::open(&options.project)?;
    let frame = project.scene().frame();
    let index = project.scene().index();
    let mut roles: [Option<(orr_reflect::Guid, FP)>; 3] = [None, None, None];
    for entity in frame.entities() {
        let Some(actor) = frame.get::<room_game::RoomActor>(entity) else {
            continue;
        };
        let slot = match actor.kind {
            room_game::PLAYER => 0,
            room_game::KEY => 1,
            room_game::EXIT => 2,
            _ => continue,
        };
        if roles[slot].is_some() {
            return Err("admitted Room frame contains a duplicate objective role".into());
        }
        let guid = index
            .guid(entity)
            .cloned()
            .ok_or("objective role is missing its admitted GUID")?;
        let body = orr_sample::room_project::types()
            .read_component(frame, entity, BODY)
            .map_err(|error| error.to_string())?;
        let Value::Struct(fields) = body else {
            return Err("admitted Body is not a struct".into());
        };
        let position = fields
            .iter()
            .find(|(name, _)| name == "pos")
            .map(|(_, value)| value)
            .ok_or("admitted Body position is missing")?;
        let Value::Vec3(position) = position else {
            return Err("admitted Body position is not a vec3".into());
        };
        roles[slot] = Some((guid, position.y));
    }
    let positions = [options.player, options.key, options.exit];
    let mut ops = Vec::with_capacity(3);
    let mut json_ops = Vec::with_capacity(3);
    for (slot, role) in ["player", "key", "exit"].into_iter().enumerate() {
        let (guid, position_y) = roles[slot]
            .take()
            .ok_or_else(|| format!("admitted Room frame is missing the {role} role"))?;
        let [position_x, position_z] = positions[slot];
        let value = Value::Vec3(FPVec3::new(position_x, position_y, position_z));
        ops.push(Op::SetField {
            guid: guid.clone(),
            component: BODY.to_owned(),
            path: "pos".into(),
            value: value.clone(),
        });
        json_ops.push(json!({
            "op": "patch",
            "entity": guid.as_str(),
            "component": BODY,
            "path": "pos",
            "value": orr_remote::json::value_to_json(&value),
        }));
    }
    let mut document = orr_remote::room_escape::document(project.scene().text())?;
    document
        .apply_batch(
            "author Room layout",
            ops,
            Origin::Agent("room_layout".into()),
        )
        .map_err(|error| format!("Room layout admission: {error}"))?;
    orr_sample::room_view::admit_presentation(
        orr_bridge::FrameView::of(document.frame()),
        document.index(),
        project.models(),
    )?;
    let mut output = BoundedOutput(Vec::new());
    serde_json::to_writer_pretty(&mut output, &Json::Array(json_ops))
        .map_err(|error| error.to_string())?;
    output.write_all(b"\n").map_err(|error| error.to_string())?;
    Ok((output.0, project.root().to_path_buf()))
}

struct BoundedOutput(Vec<u8>);
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_OUTPUT_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("room layout operations exceed 4 KiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Admit the complete layout before atomically publishing a new regular file.
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
        let _ = fs::remove_file(&self.0);
    }
}

fn temporary(parent: &Path, destination: &Path) -> Result<(TemporaryOutput, File), String> {
    for _ in 0..32 {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".room-layout-{}-{serial}.tmp", std::process::id()));
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
        return Err("room layout operations exceed 4 KiB".into());
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
    fs::hard_link(&temporary.0, destination)
        .map_err(|error| format!("output publication (no overwrite): {error}"))?;
    Ok(())
}
