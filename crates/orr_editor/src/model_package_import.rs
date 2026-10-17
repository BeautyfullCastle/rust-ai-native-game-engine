use crate::{model_bindings::ModelKind, package_manager};
use orr_package::Manifest;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_SOURCE_BYTES: u64 = orr_model::MAX_FILE_BYTES as u64;
const MAX_LICENSE_BYTES: u64 = 1024 * 1024;
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
    pub kind: ModelKind,
    pub license_acknowledged: bool,
}

pub(crate) struct Report {
    pub root: PathBuf,
    pub package: String,
    pub asset: String,
    pub kind: ModelKind,
    pub source_sha256: String,
    pub license_sha256: String,
}

struct InputBytes {
    bytes: Vec<u8>,
    digest: String,
}

pub(crate) fn import_package(request: Request) -> Result<Report, String> {
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    validate_destination_policy(&request.destination)?;
    let (destination_parent, destination) = checked_new_directory(&request.destination)?;
    validate_destination_policy(&destination)?;
    let source_path = checked_file_path(&request.source, "GLB source")?;
    let license_path = checked_file_path(&request.license, "license source")?;
    if !source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("glb"))
    {
        return Err("source must be a self-contained .glb file".into());
    }

    let source = read_bounded_file(&source_path, MAX_SOURCE_BYTES, "GLB source")?;
    let license = read_bounded_file(&license_path, MAX_LICENSE_BYTES, "license source")?;
    if std::str::from_utf8(&license.bytes)
        .ok()
        .is_none_or(|text| text.trim().is_empty())
    {
        return Err("license must be nonempty UTF-8 text".into());
    }
    validate_glb(&source.bytes, request.kind)?;

    let stage = tempfile::Builder::new()
        .prefix(".orr-model-import-")
        .tempdir_in(&destination_parent)
        .map_err(|error| format!("create package staging directory: {error}"))?;
    let manifest = package_manifest(&request)?;
    let manifest_bytes = serde_json::to_vec(&manifest)
        .map_err(|error| format!("serialize package manifest: {error}"))?;

    write_new(&stage.path().join("model.glb"), &source.bytes)?;
    write_new(&stage.path().join("LICENSE.txt"), &license.bytes)?;
    write_new(&stage.path().join("orr.package.json"), &manifest_bytes)?;
    let runtime = orr_sample::room_project::compiled_runtime();
    let inspected = orr_package::inspect(stage.path(), &runtime)
        .map_err(|error| format!("package validation: {error}"))?;
    if inspected.manifest != manifest {
        return Err("package manifest changed during validation".into());
    }
    let parent_after_stage =
        package_manager::checked_directory(&destination_parent, "package destination parent")?;
    if parent_after_stage != destination_parent {
        return Err("package destination parent changed during import".into());
    }

    publish_directory(
        stage.path(),
        &destination,
        &source.bytes,
        &license.bytes,
        &manifest_bytes,
    )?;
    Ok(Report {
        root: destination,
        package: request.name,
        asset: "model.glb".into(),
        kind: request.kind,
        source_sha256: source.digest,
        license_sha256: license.digest,
    })
}

fn package_manifest(request: &Request) -> Result<Manifest, String> {
    let capabilities = match request.kind {
        ModelKind::Static => BTreeSet::from(["models".to_owned()]),
        ModelKind::Animated => {
            #[cfg(feature = "room-animated-models")]
            {
                BTreeSet::from(["animation".to_owned(), "models".to_owned()])
            }
            #[cfg(not(feature = "room-animated-models"))]
            return Err("animated model package import requires room-animated-models".into());
        }
    };
    let manifest = Manifest {
        schema: 1,
        name: request.name.clone(),
        version: request.version.clone(),
        engine: format!(
            "={}",
            orr_sample::room_project::compiled_runtime().engine_version
        ),
        capabilities,
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["LICENSE.txt".into(), "model.glb".into()]),
    };
    Ok(manifest)
}

fn validate_glb(bytes: &[u8], kind: ModelKind) -> Result<(), String> {
    match kind {
        ModelKind::Static => {
            orr_model::import::import_with_resolver("model.glb", bytes, |_| {
                Err(orr_model::Error::message(
                    "external resources are unsupported for raw model package import",
                ))
            })
            .map_err(|error| format!("static GLB import: {error}"))?;
        }
        ModelKind::Animated => {
            #[cfg(feature = "room-animated-models")]
            {
                let model =
                    orr_model::animation_import::import_with_resolver("model.glb", bytes, |_| {
                        Err(orr_model::Error::message(
                            "external resources are unsupported for raw model package import",
                        ))
                    })
                    .map_err(|error| format!("animated GLB import: {error}"))?;
                if model.source().clips.is_empty() {
                    return Err("animated GLB must contain at least one animation clip".into());
                }
            }
            #[cfg(not(feature = "room-animated-models"))]
            return Err("animated model package import requires room-animated-models".into());
        }
    }
    Ok(())
}

fn checked_file_path(path: &Path, label: &str) -> Result<PathBuf, String> {
    validate_path_shape(path, label)?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} needs a parent"))?;
    let parent = package_manager::checked_directory(parent, label)?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("{label} needs a filename"))?;
    let canonical = parent.join(name);
    let metadata =
        fs::symlink_metadata(&canonical).map_err(|error| format!("{label} metadata: {error}"))?;
    if !regular_non_reparse(&metadata) {
        return Err(format!("{label} must be a regular non-reparse file"));
    }
    Ok(canonical)
}

fn read_bounded_file(path: &Path, limit: u64, label: &str) -> Result<InputBytes, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("{label} metadata: {error}"))?;
    if !regular_non_reparse(&metadata) || metadata.len() > limit {
        return Err(format!(
            "{label} must be a regular file within its byte limit"
        ));
    }
    let mut file = open_without_following(path).map_err(|error| format!("{label}: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("{label} opened metadata: {error}"))?;
    if !regular_non_reparse(&opened) || opened.len() > limit {
        return Err(format!(
            "{label} changed to a non-regular or oversized file"
        ));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{label} read: {error}"))?;
    let after = fs::symlink_metadata(path).map_err(|error| format!("{label} recheck: {error}"))?;
    let handle_after = file
        .metadata()
        .map_err(|error| format!("{label} opened recheck: {error}"))?;
    if !regular_non_reparse(&after)
        || !regular_non_reparse(&handle_after)
        || bytes.len() as u64 > limit
        || bytes.len() as u64 != opened.len()
        || !same_file_observation(&metadata, &opened)
        || !same_file_observation(&opened, &handle_after)
        || !same_file_observation(&metadata, &after)
        || !same_file_observation(&after, &handle_after)
    {
        return Err(format!("{label} changed while it was read"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} needs a parent"))?;
    let checked_parent = package_manager::checked_directory(parent, label)?;
    if checked_parent != parent {
        return Err(format!("{label} parent changed while it was read"));
    }
    let digest = digest(&bytes);
    Ok(InputBytes { bytes, digest })
}

fn same_file_observation(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    first.len() == second.len()
        && match (first.modified(), second.modified()) {
            (Ok(first_modified), Ok(second_modified)) => first_modified == second_modified,
            _ => true,
        }
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
    let stem = name.split('.').next().unwrap_or_default();
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            upper.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.chars().count() == 1
                    && matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
            })
        })
}

fn validate_destination_policy(path: &Path) -> Result<(), String> {
    validate_path_shape(path, "package destination")?;
    if path.components().any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy().eq_ignore_ascii_case(".orr"))
    }) {
        return Err("package destination cannot be inside the .orr store".into());
    }
    Ok(())
}

fn checked_new_directory(path: &Path) -> Result<(PathBuf, PathBuf), String> {
    let parent = path
        .parent()
        .ok_or("package destination needs an existing parent")?;
    let parent = package_manager::checked_directory(parent, "package destination parent")?;
    let name = path
        .file_name()
        .ok_or("package destination needs a directory name")?;
    let destination = parent.join(name);
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err("package destination already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("package destination: {error}")),
    }
    Ok((parent, destination))
}

fn publish_directory(
    stage: &Path,
    destination: &Path,
    source: &[u8],
    license: &[u8],
    manifest: &[u8],
) -> Result<(), String> {
    fs::create_dir(destination).map_err(|error| format!("create package destination: {error}"))?;
    let mut created = Vec::new();
    let result = (|| {
        for (name, bytes) in [("model.glb", source), ("LICENSE.txt", license)] {
            let staged = stage.join(name);
            let staged_bytes =
                fs::read(&staged).map_err(|error| format!("read staged {name}: {error}"))?;
            if staged_bytes != bytes {
                return Err(format!("staged {name} changed before publication"));
            }
            let path = destination.join(name);
            write_new_tracked(&path, bytes, &mut created)?;
        }
        let manifest_path = destination.join("orr.package.json");
        write_new_tracked(&manifest_path, manifest, &mut created)?;
        Ok(())
    })();
    if let Err(error) = result {
        let cleanup = cleanup_owned_output(destination, &created);
        return Err(match cleanup {
            Ok(()) => error,
            Err(cleanup_error) => {
                format!("{error}; partial output cleanup failed: {cleanup_error}")
            }
        });
    }
    Ok(())
}

fn cleanup_owned_output(destination: &Path, created: &[PathBuf]) -> Result<(), String> {
    let mut errors = Vec::new();
    for path in created.iter().rev() {
        if let Err(error) = fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                errors.push(format!("{}: {error}", path.display()));
            }
        }
    }
    if let Err(error) = fs::remove_dir(destination) {
        if error.kind() != std::io::ErrorKind::NotFound {
            errors.push(format!("{}: {error}", destination.display()));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("write {}: {error}", path.display()))
}

fn write_new_tracked(path: &Path, bytes: &[u8], created: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create {}: {error}", path.display()))?;
    created.push(path.to_path_buf());
    file.write_all(bytes)
        .map_err(|error| format!("write {}: {error}", path.display()))
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

fn regular_non_reparse(metadata: &fs::Metadata) -> bool {
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

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
#[path = "model_package_import/tests.rs"]
mod tests;
