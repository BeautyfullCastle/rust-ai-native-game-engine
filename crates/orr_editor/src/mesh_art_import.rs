use crate::mesh_art::Snapshot;
use orr_package::Manifest;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

#[cfg(test)]
#[path = "mesh_art_import/tests.rs"]
mod tests;

const MAX_SOURCE_BYTES: u64 = orr_model::MAX_FILE_BYTES as u64;
const MAX_LICENSE_BYTES: u64 = 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 64;
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

pub(crate) struct Request {
    pub source: PathBuf,
    pub license: PathBuf,
    pub destination: PathBuf,
    pub name: String,
    pub version: String,
    pub license_acknowledged: bool,
    pub graph_revision: [u8; 32],
}

pub(crate) struct Report {
    pub root: PathBuf,
    pub asset: String,
    pub license: String,
    pub source_sha256: String,
    pub license_sha256: String,
    pub snapshot: Snapshot,
}

struct InputFile {
    path: PathBuf,
    bytes: Vec<u8>,
    sha256: String,
}

pub(crate) fn import_package(request: Request) -> Result<Report, String> {
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    validate_package_identity(&request.name, &request.version)?;
    let source = read_input_file(&request.source, MAX_SOURCE_BYTES, "GLB source")?;
    if !source
        .path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("glb"))
    {
        return Err("mesh art import accepts only self-contained .glb sources".into());
    }
    let license = read_input_file(&request.license, MAX_LICENSE_BYTES, "license source")?;
    if source.path == license.path {
        return Err("GLB and license must be separate files".into());
    }
    let license_text =
        std::str::from_utf8(&license.bytes).map_err(|_| "license must be nonempty UTF-8 text")?;
    if license_text.trim().is_empty() {
        return Err("license must be nonempty UTF-8 text".into());
    }

    let (destination_parent, destination) = checked_new_destination(&request.destination)?;
    let stage = tempfile::Builder::new()
        .prefix(".orr-mesh-art-import-")
        .tempdir_in(&destination_parent)
        .map_err(|error| format!("create mesh art package staging directory: {error}"))?;
    let mut runtime = orr_package::Runtime::content_only();
    runtime.capabilities.insert("models".into());
    let manifest = Manifest {
        schema: 1,
        name: request.name,
        version: request.version,
        engine: format!("={}", runtime.engine_version),
        capabilities: BTreeSet::from(["models".to_owned()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["LICENSE.txt".into(), "model.glb".into()]),
    };
    let manifest_bytes = serde_json::to_vec(&manifest)
        .map_err(|error| format!("serialize mesh art package manifest: {error}"))?;
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err("mesh art package manifest exceeds 1 MiB".into());
    }
    write_new_file(&stage.path().join("model.glb"), &source.bytes)?;
    write_new_file(&stage.path().join("LICENSE.txt"), &license.bytes)?;
    write_new_file(&stage.path().join("orr.package.json"), &manifest_bytes)?;
    let inspected = orr_package::inspect(stage.path(), &runtime)
        .map_err(|error| format!("inspect imported mesh art package: {error}"))?;
    if inspected.manifest != manifest {
        return Err("mesh art package manifest changed during inspection".into());
    }
    let snapshot = Snapshot::from_package(
        stage.path(),
        "model.glb",
        "LICENSE.txt",
        request.graph_revision,
    )?;
    recheck_input(&source, MAX_SOURCE_BYTES, "GLB source")?;
    recheck_input(&license, MAX_LICENSE_BYTES, "license source")?;
    let parent_after_stage = checked_directory(&destination_parent, "package destination parent")?;
    if parent_after_stage != destination_parent {
        return Err("package destination parent changed before publication".into());
    }
    refuse_existing(&destination)?;
    publish_new_directory(stage.path(), &destination)?;

    Ok(Report {
        root: destination,
        asset: "model.glb".into(),
        license: "LICENSE.txt".into(),
        source_sha256: source.sha256,
        license_sha256: license.sha256,
        snapshot,
    })
}

fn validate_package_identity(name: &str, version: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte))
    {
        return Err(
            "package name must use 1-64 lowercase ASCII letters, digits, '-' or '_'".into(),
        );
    }
    if version.is_empty() || version.len() > 128 || version.chars().any(char::is_control) {
        return Err("package version must be bounded printable semantic version text".into());
    }
    Ok(())
}

fn read_input_file(path: &Path, limit: u64, label: &str) -> Result<InputFile, String> {
    let canonical = checked_file_path(path, label)?;
    let before =
        fs::symlink_metadata(&canonical).map_err(|error| format!("inspect {label}: {error}"))?;
    if !regular_non_reparse(&before) || before.len() > limit {
        return Err(format!(
            "{label} must be a bounded regular non-reparse file"
        ));
    }
    let mut file = open_without_following(&canonical)
        .map_err(|error| format!("open {label} without following links: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("inspect opened {label}: {error}"))?;
    if !regular_non_reparse(&opened) || opened.len() > limit || !same_observation(&before, &opened)
    {
        return Err(format!("{label} changed before reading"));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {label}: {error}"))?;
    let opened_after = file
        .metadata()
        .map_err(|error| format!("recheck opened {label}: {error}"))?;
    let current =
        fs::symlink_metadata(&canonical).map_err(|error| format!("recheck {label}: {error}"))?;
    let parent = canonical
        .parent()
        .ok_or_else(|| format!("{label} has no parent"))?;
    if bytes.len() as u64 > limit
        || bytes.len() as u64 != opened.len()
        || !regular_non_reparse(&opened_after)
        || !regular_non_reparse(&current)
        || !same_observation(&opened, &opened_after)
        || !same_observation(&before, &current)
        || checked_directory(parent, label)?.as_path() != parent
    {
        return Err(format!("{label} changed while it was read"));
    }
    Ok(InputFile {
        path: canonical,
        sha256: digest(&bytes),
        bytes,
    })
}

fn recheck_input(input: &InputFile, limit: u64, label: &str) -> Result<(), String> {
    let current = read_input_file(&input.path, limit, label)?;
    if current.sha256 != input.sha256 || current.bytes != input.bytes {
        return Err(format!("{label} changed before package publication"));
    }
    Ok(())
}

fn checked_file_path(path: &Path, label: &str) -> Result<PathBuf, String> {
    validate_path_shape(path, label)?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} needs a parent"))?;
    let parent = checked_directory(parent, label)?;
    let filename = path
        .file_name()
        .and_then(|filename| filename.to_str())
        .ok_or_else(|| format!("{label} filename must be Unicode"))?;
    validate_leaf(filename, label)?;
    let canonical = parent.join(filename);
    if canonical.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
        return Err(format!("canonical {label} path exceeds the path bound"));
    }
    let metadata =
        fs::symlink_metadata(&canonical).map_err(|error| format!("inspect {label}: {error}"))?;
    if !regular_non_reparse(&metadata) {
        return Err(format!("{label} must be a regular non-reparse file"));
    }
    Ok(canonical)
}

fn checked_new_destination(path: &Path) -> Result<(PathBuf, PathBuf), String> {
    validate_path_shape(path, "package destination")?;
    if path.components().any(|component| {
        matches!(component, Component::Normal(part) if part.to_string_lossy().eq_ignore_ascii_case(".orr"))
    }) {
        return Err("mesh art package destination cannot be inside the .orr store".into());
    }
    let parent = path
        .parent()
        .ok_or("package destination needs an existing parent")?;
    let parent = checked_directory(parent, "package destination parent")?;
    let filename = path
        .file_name()
        .and_then(|filename| filename.to_str())
        .ok_or("package destination needs a Unicode directory name")?;
    validate_leaf(filename, "package destination")?;
    let destination = parent.join(filename);
    if destination.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
        return Err("canonical package destination exceeds the path bound".into());
    }
    refuse_existing(&destination)?;
    Ok((parent, destination))
}

fn validate_path_shape(path: &Path, label: &str) -> Result<(), String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("{label} path must be Unicode"))?;
    let components: Vec<_> = path.components().collect();
    if !path.is_absolute()
        || text.is_empty()
        || text.len() > MAX_PATH_BYTES
        || components.len() > MAX_PATH_COMPONENTS
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err(format!(
            "{label} path must be bounded, absolute and normalized"
        ));
    }
    for component in components {
        if let Component::Normal(name) = component {
            validate_leaf(
                name.to_str()
                    .ok_or_else(|| format!("{label} path must be Unicode"))?,
                label,
            )?;
        }
    }
    Ok(())
}

fn validate_leaf(name: &str, label: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 200
        || matches!(name.chars().last(), Some('.' | ' '))
        || name.contains([':', '\\', '/', '\0'])
        || name.chars().any(char::is_control)
        || reserved_device_name(name)
    {
        return Err(format!("{label} path contains an invalid Windows filename"));
    }
    Ok(())
}

fn reserved_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' '])
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.chars().count() == 1
                    && matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
            })
        })
}

fn checked_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    validate_path_shape(path, label)?;
    let components: Vec<_> = path.components().collect();
    let mut current = PathBuf::new();
    #[cfg(windows)]
    {
        let mut components = components.iter().copied();
        let prefix = match components.next() {
            Some(Component::Prefix(prefix))
                if matches!(
                    prefix.kind(),
                    std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
                ) =>
            {
                prefix
            }
            _ => return Err(format!("{label} must use an absolute local disk path")),
        };
        current.push(prefix.as_os_str());
        match components.next() {
            Some(Component::RootDir) => current.push("\\"),
            _ => return Err(format!("{label} must be rooted")),
        }
        ensure_directory(&current, label)?;
        for component in components {
            let Component::Normal(name) = component else {
                return Err(format!("{label} path is not normalized"));
            };
            validate_leaf(
                name.to_str().ok_or("path component must be Unicode")?,
                label,
            )?;
            current.push(name);
            ensure_directory(&current, label)?;
        }
    }
    #[cfg(not(windows))]
    {
        for component in components {
            match component {
                Component::RootDir => current.push(component.as_os_str()),
                Component::Normal(name) => {
                    validate_leaf(
                        name.to_str().ok_or("path component must be Unicode")?,
                        label,
                    )?;
                    current.push(name);
                    ensure_directory(&current, label)?;
                }
                _ => return Err(format!("{label} path is not normalized")),
            }
        }
    }
    let canonical =
        fs::canonicalize(path).map_err(|error| format!("canonicalize {label}: {error}"))?;
    if canonical.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
        return Err(format!("canonical {label} path exceeds the path bound"));
    }
    Ok(canonical)
}

fn ensure_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect {label} directory: {error}"))?;
    if !metadata.is_dir() {
        return Err(format!("{label} path must contain directories only"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(format!("{label} path cannot contain reparse points"));
        }
    }
    #[cfg(not(windows))]
    if metadata.file_type().is_symlink() {
        return Err(format!("{label} path cannot contain symlinks"));
    }
    Ok(())
}

fn regular_non_reparse(metadata: &Metadata) -> bool {
    if !metadata.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    #[cfg(not(windows))]
    {
        !metadata.file_type().is_symlink()
    }
}

fn same_observation(first: &Metadata, second: &Metadata) -> bool {
    first.len() == second.len()
        && match (first.modified(), second.modified()) {
            (Ok(first_modified), Ok(second_modified)) => first_modified == second_modified,
            _ => true,
        }
}

fn open_without_following(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create staged package file: {error}"))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("write staged package file: {error}"))
}

fn refuse_existing(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err("mesh art package destination already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect package destination: {error}")),
    }
}

fn publish_new_directory(stage: &Path, destination: &Path) -> Result<(), String> {
    let stage_metadata = fs::symlink_metadata(stage).map_err(|error| error.to_string())?;
    if !regular_directory(&stage_metadata) {
        return Err("mesh art staging directory changed before publication".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        refuse_existing(destination)?;
        let stage_wide: Vec<u16> = stage.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination_wide: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let result = unsafe { MoveFileExW(stage_wide.as_ptr(), destination_wide.as_ptr(), 0) };
        if result == 0 {
            return Err(format!(
                "native no-replace package publication failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        unsafe extern "C" {
            fn renameat2(
                old_dirfd: i32,
                old_path: *const i8,
                new_dirfd: i32,
                new_path: *const i8,
                flags: u32,
            ) -> i32;
        }
        let old_path =
            CString::new(stage.as_os_str().as_bytes()).map_err(|_| "staging path contains NUL")?;
        let new_path = CString::new(destination.as_os_str().as_bytes())
            .map_err(|_| "destination path contains NUL")?;
        let result = unsafe { renameat2(-100, old_path.as_ptr(), -100, new_path.as_ptr(), 1) };
        if result != 0 {
            return Err(format!(
                "native no-replace package publication failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (stage, destination);
        Err("native no-replace package publication is unavailable on this platform".into())
    }
}

fn regular_directory(metadata: &Metadata) -> bool {
    if !metadata.is_dir() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    #[cfg(not(windows))]
    {
        !metadata.file_type().is_symlink()
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
