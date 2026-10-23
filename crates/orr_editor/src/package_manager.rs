mod removal;
mod update;
pub use removal::{
    apply_remove, apply_remove_with_receipt, apply_restore, plan_remove, plan_restore, RemovalPlan,
    RemovalReceipt,
};
pub use update::{apply_update, plan_update, UpdatePlan};

use orr_package::{Lock, LockedPackage, PackagePreview, Project, Runtime};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 64;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

pub struct Plan {
    pub root: PathBuf,
    pub source: PathBuf,
    pub preview: PackagePreview,
}

pub fn inspect(root: &Path) -> Result<Lock, String> {
    let (root, lock) = open_room(root)?;
    let project = Project::open(&root, room_runtime()).map_err(|error| error.to_string())?;
    let verified = project.verify().map_err(|error| error.to_string())?;
    if verified != lock {
        return Err("Room package lock changed during inspection".into());
    }
    Ok(verified)
}

pub fn plan_add(root: &Path, source: &Path) -> Result<Plan, String> {
    let (root, previous) = open_room(root)?;
    let source = checked_directory(source, "package source")?;
    let runtime = room_runtime();
    let source_package = inspect_source(&source, &runtime)?;
    reject_used_name(&previous, &source_package.manifest.name)?;

    let project = Project::open(&root, runtime).map_err(|error| error.to_string())?;
    let preview = project
        .preview_install(std::slice::from_ref(&source))
        .map_err(|error| error.to_string())?;
    if preview.previous != previous {
        return Err("Room package lock changed during planning; inspect and plan again".into());
    }
    validate_addition(&preview.previous, &preview.candidate, &source_package)?;
    Ok(Plan {
        root,
        source,
        preview,
    })
}

pub fn apply_add(plan: &Plan) -> Result<Lock, String> {
    let root = checked_directory(&plan.root, "Room project")?;
    if root != plan.root {
        return Err("Room project path identity changed since planning".into());
    }
    let source = checked_directory(&plan.source, "package source")?;
    if source != plan.source {
        return Err("package source path identity changed since planning".into());
    }

    let (_, current) = open_room(&root)?;
    let runtime = room_runtime();
    let source_package = inspect_source(&source, &runtime)?;
    validate_addition(
        &plan.preview.previous,
        &plan.preview.candidate,
        &source_package,
    )?;
    if current != plan.preview.previous {
        return Err("active Room package lock changed since planning".into());
    }

    let project = Project::open(&root, runtime).map_err(|error| error.to_string())?;
    let candidate = plan.preview.candidate.clone();
    let installed = project
        .install_checked(
            std::slice::from_ref(&source),
            &plan.preview.previous,
            &candidate,
            || {
                let checked_root = checked_directory(&root, "Room project")?;
                if checked_root != root {
                    return Err(
                        "Room project path identity changed before package publication".into(),
                    );
                }
                let prepared = orr_sample::room_project::PreparedProject::open(&root)?;
                if prepared.root() != root {
                    return Err("Room project root changed before package publication".into());
                }
                Ok(())
            },
        )
        .map_err(|error| error.to_string())?;

    let prepared = orr_sample::room_project::PreparedProject::open(&root).map_err(|error| {
        format!(
            "package was installed at {} but Room re-admission failed; the package lock is retained: {error}",
            root.display()
        )
    })?;
    if prepared.root() != root {
        return Err(format!(
            "package was installed at {} but Room re-admission returned a different project root; the package lock is retained",
            root.display()
        ));
    }
    let verified = Project::open(&root, room_runtime())
        .and_then(|project| project.verify())
        .map_err(|error| {
            format!(
                "package was installed at {} but the resulting lock could not be verified; the package lock is retained: {error}",
                root.display()
            )
        })?;
    if verified != installed || verified != candidate {
        return Err(format!(
            "package was installed at {} but its resulting lock differs from the approved candidate; the package lock is retained",
            root.display()
        ));
    }
    Ok(verified)
}

fn room_runtime() -> Runtime {
    orr_sample::room_project::compiled_runtime()
}

fn open_room(root: &Path) -> Result<(PathBuf, Lock), String> {
    let root = checked_directory(root, "Room project")?;
    let prepared = orr_sample::room_project::PreparedProject::open(&root)?;
    if prepared.root() != root {
        return Err("Room project root changed during admission".into());
    }
    let project = Project::open(&root, room_runtime()).map_err(|error| error.to_string())?;
    let lock = project.verify().map_err(|error| error.to_string())?;
    Ok((root, lock))
}

fn inspect_source(source: &Path, runtime: &Runtime) -> Result<LockedPackage, String> {
    let manifest_path = source.join("orr.package.json");
    let metadata = fs::symlink_metadata(&manifest_path)
        .map_err(|error| format!("package manifest: {error}"))?;
    ensure_regular(&metadata, "package manifest")?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err("package manifest exceeds 1 MiB".into());
    }
    let bytes = fs::read(&manifest_path).map_err(|error| format!("package manifest: {error}"))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("package manifest exceeds 1 MiB".into());
    }
    let manifest: orr_package::Manifest =
        serde_json::from_slice(&bytes).map_err(|error| format!("package manifest: {error}"))?;
    if manifest.files.len() > 4096 {
        return Err("package manifest exceeds the existing 4096 file limit".into());
    }
    if !manifest.files.contains("LICENSE.txt") {
        return Err("new packages must declare LICENSE.txt (this is not a legal review)".into());
    }
    validate_declared_paths(source, &manifest.files)?;
    orr_package::inspect(source, runtime).map_err(|error| error.to_string())
}

fn validate_declared_paths(source: &Path, files: &BTreeSet<String>) -> Result<(), String> {
    for relative in files {
        validate_portable_path(relative)?;
        let parts: Vec<_> = relative.split('/').collect();
        let mut current = source.to_path_buf();
        for (index, part) in parts.iter().enumerate() {
            current.push(part);
            let metadata = fs::symlink_metadata(&current)
                .map_err(|error| format!("package source path {}: {error}", current.display()))?;
            if index + 1 == parts.len() {
                ensure_regular(&metadata, "declared package file")?;
            } else {
                ensure_directory(&metadata, "package source ancestor")?;
            }
        }
    }
    Ok(())
}

fn validate_portable_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 240 || !path.is_ascii() || path.split('/').count() > 16 {
        return Err(format!("invalid portable package path: {path}"));
    }
    for part in path.split('/') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with('.')
            || part.len() > 100
            || !part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err(format!("invalid portable package path: {path}"));
        }
    }
    Ok(())
}

fn reject_used_name(previous: &Lock, name: &str) -> Result<(), String> {
    if previous.direct.contains_key(name) || previous.packages.contains_key(name) {
        return Err(format!("package name is already in use: {name}"));
    }
    Ok(())
}

fn validate_addition(
    previous: &Lock,
    candidate: &Lock,
    source: &LockedPackage,
) -> Result<(), String> {
    let name = &source.manifest.name;
    reject_used_name(previous, name)?;
    if candidate.schema != previous.schema
        || candidate.direct.len() != previous.direct.len() + 1
        || candidate.packages.len() != previous.packages.len() + 1
    {
        return Err("package preview must add exactly one direct package".into());
    }
    for (old_name, old_version) in &previous.direct {
        if candidate.direct.get(old_name) != Some(old_version) {
            return Err(format!(
                "package preview changed existing direct selection: {old_name}"
            ));
        }
    }
    for (old_name, old_package) in &previous.packages {
        if candidate.packages.get(old_name) != Some(old_package) {
            return Err(format!(
                "package preview changed existing package: {old_name}"
            ));
        }
    }
    if candidate.direct.get(name) != Some(&source.manifest.version)
        || candidate.packages.get(name) != Some(source)
    {
        return Err("package preview does not match the inspected source package".into());
    }
    if candidate.packages.keys().any(|candidate_name| {
        !previous.packages.contains_key(candidate_name) && candidate_name != name
    }) {
        return Err("package preview added packages beyond the selected source".into());
    }
    Ok(())
}

pub(crate) fn checked_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("{label} path must be Unicode"))?;
    if !path.is_absolute()
        || text.is_empty()
        || text.len() > MAX_PATH_BYTES
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err(format!(
            "{label} path must be bounded, absolute and traversal-free"
        ));
    }
    let components: Vec<_> = path.components().collect();
    if components.len() > MAX_PATH_COMPONENTS {
        return Err(format!("{label} path has too many components"));
    }

    #[cfg(windows)]
    let normal_count = validate_windows_components(&components, label)?;
    #[cfg(not(windows))]
    let normal_count = validate_native_components(path, &components, label)?;
    if normal_count == 0 {
        return Err(format!(
            "{label} path must name a directory below the filesystem root"
        ));
    }

    let canonical = fs::canonicalize(path).map_err(|error| format!("{label} path: {error}"))?;
    if canonical.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
        return Err(format!("canonical {label} path exceeds the path bound"));
    }
    Ok(canonical)
}

#[cfg(windows)]
fn validate_windows_components(components: &[Component<'_>], label: &str) -> Result<usize, String> {
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
        _ => return Err(format!("{label} path must use an absolute local disk path")),
    };
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err(format!("{label} path must be absolute"));
    }
    let mut current = PathBuf::from(prefix.as_os_str());
    current.push("\\");
    let root_metadata = fs::symlink_metadata(&current)
        .map_err(|error| format!("{label} path {}: {error}", current.display()))?;
    ensure_windows_directory(&root_metadata, label)?;
    let mut normal_count = 0;
    for component in components {
        let Component::Normal(name) = component else {
            return Err(format!("{label} path must be normalized"));
        };
        current.push(name);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("{label} path {}: {error}", current.display()))?;
        ensure_windows_directory(&metadata, label)?;
        normal_count += 1;
    }
    Ok(normal_count)
}

#[cfg(not(windows))]
fn validate_native_components(
    _path: &Path,
    components: &[Component<'_>],
    label: &str,
) -> Result<usize, String> {
    let mut current = PathBuf::new();
    let mut normal_count = 0;
    for component in components {
        match component {
            Component::RootDir => current.push(component.as_os_str()),
            Component::Normal(name) => {
                current.push(name);
                let metadata = fs::symlink_metadata(&current)
                    .map_err(|error| format!("{label} path {}: {error}", current.display()))?;
                ensure_native_directory(&metadata, label)?;
                normal_count += 1;
            }
            _ => return Err(format!("{label} path must be normalized")),
        }
    }
    Ok(normal_count)
}

#[cfg(windows)]
fn ensure_directory(metadata: &fs::Metadata, label: &str) -> Result<(), String> {
    ensure_windows_directory(metadata, label)
}

#[cfg(not(windows))]
fn ensure_directory(metadata: &fs::Metadata, label: &str) -> Result<(), String> {
    ensure_native_directory(metadata, label)
}

#[cfg(windows)]
fn ensure_regular(metadata: &fs::Metadata, label: &str) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(format!("{label} must be a regular non-reparse file"));
    }
    Ok(())
}

#[cfg(not(windows))]
fn ensure_regular(metadata: &fs::Metadata, label: &str) -> Result<(), String> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{label} must be a regular non-symlink file"));
    }
    Ok(())
}

#[cfg(windows)]
fn ensure_windows_directory(metadata: &fs::Metadata, label: &str) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(format!(
            "{label} path must contain only non-reparse directories"
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn ensure_native_directory(metadata: &fs::Metadata, label: &str) -> Result<(), String> {
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "{label} path must contain only non-symlink directories"
        ));
    }
    Ok(())
}

#[cfg(all(test, windows, target_arch = "x86_64", feature = "windows-projects"))]
mod tests {
    use super::*;
    use orr_package::Manifest;
    use std::collections::{BTreeMap, BTreeSet};

    fn room_root(parent: &Path, name: &str) -> PathBuf {
        let root = parent.join(name);
        orr_sample::windows_project::create_room_project(&root, "package-manager-tests")
            .expect("create admitted Room fixture");
        root
    }

    fn package_source(parent: &Path, name: &str, capabilities: &[&str]) -> PathBuf {
        let source = parent.join(format!("{name}-source"));
        fs::create_dir(&source).unwrap();
        fs::write(source.join("LICENSE.txt"), b"Local fixture license text\n").unwrap();
        fs::write(source.join("asset.dat"), b"local content\n").unwrap();
        let manifest = Manifest {
            schema: 1,
            name: name.into(),
            version: "1.0.0".into(),
            engine: "*".into(),
            capabilities: capabilities.iter().map(|value| (*value).into()).collect(),
            dependencies: BTreeMap::new(),
            files: BTreeSet::from(["LICENSE.txt".into(), "asset.dat".into()]),
        };
        fs::write(
            source.join("orr.package.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        source
    }

    #[test]
    fn add_preserves_existing_packages_and_room_admission() {
        let temporary = tempfile::tempdir().unwrap();
        let root = room_root(temporary.path(), "room");
        let source = package_source(temporary.path(), "local-art", &["models"]);
        let before = inspect(&root).unwrap();
        let prepared_before = orr_sample::room_project::PreparedProject::open(&root).unwrap();
        let checksum = prepared_before.scene().frame().checksum();
        let plan = plan_add(&root, &source).unwrap();
        assert_eq!(plan.preview.previous, before);
        assert_eq!(
            plan.preview.candidate.direct.get("local-art"),
            Some(&"1.0.0".to_owned())
        );
        let installed = apply_add(&plan).unwrap();
        assert_eq!(installed, plan.preview.candidate);
        for (name, package) in &before.packages {
            assert_eq!(installed.packages.get(name), Some(package));
        }
        assert_eq!(installed.direct.get("local-art"), Some(&"1.0.0".to_owned()));
        assert_eq!(
            orr_sample::room_project::PreparedProject::open(&root)
                .unwrap()
                .scene()
                .frame()
                .checksum(),
            checksum
        );
        assert!(apply_add(&plan).is_err(), "a plan cannot be applied twice");
        assert_eq!(inspect(&root).unwrap(), installed);
    }

    #[test]
    fn rejects_missing_license_unsupported_capability_and_existing_name() {
        let temporary = tempfile::tempdir().unwrap();
        let root = room_root(temporary.path(), "room");
        let initial = inspect(&root).unwrap();

        let missing_license = package_source(temporary.path(), "missing-license", &[]);
        fs::remove_file(missing_license.join("LICENSE.txt")).unwrap();
        let mut manifest: Manifest =
            serde_json::from_slice(&fs::read(missing_license.join("orr.package.json")).unwrap())
                .unwrap();
        manifest.files.remove("LICENSE.txt");
        fs::write(
            missing_license.join("orr.package.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(plan_add(&root, &missing_license).is_err());

        let unsupported = package_source(temporary.path(), "unsupported", &["unknown-host"]);
        assert!(plan_add(&root, &unsupported).is_err());

        let used_name = initial
            .direct
            .keys()
            .next()
            .cloned()
            .expect("Room fixture selects a package");
        let duplicate = package_source(temporary.path(), &used_name, &[]);
        assert!(plan_add(&root, &duplicate).is_err());
        assert_eq!(inspect(&root).unwrap(), initial);
    }

    #[test]
    fn stale_plan_cannot_change_a_newer_package_lock() {
        let temporary = tempfile::tempdir().unwrap();
        let root = room_root(temporary.path(), "room");
        let first_source = package_source(temporary.path(), "first-local", &[]);
        let stale = plan_add(&root, &first_source).unwrap();
        let second_source = package_source(temporary.path(), "second-local", &[]);
        let current = apply_add(&plan_add(&root, &second_source).unwrap()).unwrap();
        assert!(apply_add(&stale).is_err());
        assert_eq!(inspect(&root).unwrap(), current);
    }
}
