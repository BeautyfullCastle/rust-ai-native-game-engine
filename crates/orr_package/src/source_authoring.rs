use orr_package::{inspect_for_install, Manifest, Runtime};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, Metadata, OpenOptions},
    io::{Read, Write},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf, Prefix},
};

pub const USAGE: &str = "package_source --source DIR --name NAME --version EXACT --engine REQUIREMENT [--capability NAME]... [--dependency NAME=EXACT]... --file PORTABLE_PATH...";
const MANIFEST: &str = "orr.package.json";
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 4096;
const MAX_CAPABILITIES: usize = 128;
const MAX_DEPENDENCIES: usize = 128;
const MAX_SOURCE_PATH_BYTES: usize = 4096;
const MAX_SOURCE_COMPONENTS: usize = 64;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

#[derive(Clone, Debug)]
pub struct Options {
    pub source: PathBuf,
    pub name: String,
    pub version: String,
    pub engine: String,
    pub capabilities: Vec<String>,
    pub dependencies: Vec<(String, String)>,
    pub files: Vec<String>,
}

pub struct Report {
    pub manifest: PathBuf,
    pub declared_files: usize,
    pub declared_bytes: u64,
}

impl Options {
    pub fn parse(args: Vec<OsString>) -> Result<Self, String> {
        if args.len() > 8500 {
            return Err("too many arguments".into());
        }
        let mut total_bytes = 0usize;
        for argument in &args {
            let length = argument.as_encoded_bytes().len();
            if length > MAX_SOURCE_PATH_BYTES {
                return Err("argument exceeds 4096 bytes".into());
            }
            total_bytes = total_bytes
                .checked_add(length)
                .ok_or("argument byte limit exceeded")?;
            if total_bytes > 4 * MAX_MANIFEST_BYTES {
                return Err("argument byte limit exceeded".into());
            }
        }

        let mut source = None;
        let mut name = None;
        let mut version = None;
        let mut engine = None;
        let mut capabilities = Vec::new();
        let mut dependencies = Vec::new();
        let mut files = Vec::new();
        let mut arguments = args.into_iter();
        while let Some(flag) = arguments.next() {
            let flag = flag
                .to_str()
                .ok_or_else(|| format!("unknown option; {USAGE}"))?;
            match flag {
                "--source" if source.is_none() => {
                    let value = arguments.next().ok_or(USAGE)?;
                    if value.is_empty() || value.as_encoded_bytes().len() > MAX_SOURCE_PATH_BYTES {
                        return Err("source path must contain 1..=4096 bytes".into());
                    }
                    source = Some(PathBuf::from(value));
                }
                "--name" if name.is_none() => {
                    name = Some(string_value(arguments.next(), "package name")?);
                }
                "--version" if version.is_none() => {
                    version = Some(string_value(arguments.next(), "package version")?);
                }
                "--engine" if engine.is_none() => {
                    engine = Some(string_value(arguments.next(), "engine requirement")?);
                }
                "--capability" => {
                    capabilities.push(string_value(arguments.next(), "capability")?);
                }
                "--dependency" => {
                    let value = string_value(arguments.next(), "dependency")?;
                    let (name, version) = value
                        .split_once('=')
                        .ok_or("dependency must be NAME=EXACT")?;
                    dependencies.push((name.into(), version.into()));
                }
                "--file" => files.push(string_value(arguments.next(), "asset path")?),
                _ => return Err(format!("unknown, duplicate or misplaced option; {USAGE}")),
            }
        }
        let options = Self {
            source: source.ok_or(USAGE)?,
            name: name.ok_or(USAGE)?,
            version: version.ok_or(USAGE)?,
            engine: engine.ok_or(USAGE)?,
            capabilities,
            dependencies,
            files,
        };
        validate_options(&options)?;
        Ok(options)
    }
}

pub fn create(options: &Options) -> Result<Report, String> {
    validate_options(options)?;
    let source = source_root(&options.source)?;
    let manifest = manifest(options)?;
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(error)?;
    manifest_bytes.push(b'\n');
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err("package manifest exceeds 1 MiB".into());
    }

    let target = source.join(MANIFEST);
    if exists(&target)? {
        return Err("package manifest already exists".into());
    }

    let snapshot = tempfile::tempdir().map_err(error)?;
    write_snapshot_manifest(snapshot.path(), &manifest_bytes)?;
    for path in &manifest.files {
        let target = snapshot.path().join(path);
        create_snapshot_placeholder(&target)?;
    }
    inspect_for_install(snapshot.path(), Runtime::content_only().engine_version)
        .map_err(|error| format!("package manifest validation: {error}"))?;

    let mut source_hashes = BTreeMap::new();
    let mut declared_bytes = 0u64;
    for relative in &manifest.files {
        let bytes = read_source_file(&source, relative)?;
        declared_bytes = declared_bytes
            .checked_add(bytes.len() as u64)
            .ok_or("package byte limit exceeded")?;
        if declared_bytes > MAX_TOTAL_BYTES {
            return Err("package byte limit exceeded".into());
        }
        let digest = hash(&bytes);
        source_hashes.insert(relative.clone(), (bytes.len() as u64, digest));
        fs::write(snapshot.path().join(relative), &bytes).map_err(error)?;
    }

    let inspected = inspect_for_install(snapshot.path(), Runtime::content_only().engine_version)
        .map_err(|error| format!("package source validation: {error}"))?;
    let expected_hashes: BTreeMap<_, _> = source_hashes
        .iter()
        .map(|(path, (_, digest))| (path.clone(), digest.clone()))
        .collect();
    if inspected.manifest != manifest || inspected.files != expected_hashes {
        return Err("staged package bytes differ from the declared source".into());
    }

    for relative in &manifest.files {
        let bytes = read_source_file(&source, relative)?;
        let digest = hash(&bytes);
        if !source_hashes
            .get(relative)
            .is_some_and(|(length, expected)| *length == bytes.len() as u64 && *expected == digest)
        {
            return Err(format!(
                "source changed during package authoring: {relative}"
            ));
        }
    }
    if exists(&target)? {
        return Err("package manifest already exists".into());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(&source).map_err(error)?;
    temporary.write_all(&manifest_bytes).map_err(error)?;
    temporary.as_file().sync_all().map_err(error)?;
    temporary.persist_noclobber(&target).map_err(|failure| {
        format!(
            "native no-replace manifest publication failed: {}",
            failure.error
        )
    })?;

    Ok(Report {
        manifest: target,
        declared_files: manifest.files.len(),
        declared_bytes,
    })
}

fn validate_options(options: &Options) -> Result<(), String> {
    let source_text = options
        .source
        .to_str()
        .ok_or("source path must be Unicode")?;
    if source_text.is_empty()
        || source_text.len() > MAX_SOURCE_PATH_BYTES
        || source_text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || options.name.len() > 64
        || options.version.len() > 128
        || options.engine.is_empty()
        || options.engine.len() > 256
        || options.capabilities.len() > MAX_CAPABILITIES
        || options.dependencies.len() > MAX_DEPENDENCIES
        || options.files.is_empty()
        || options.files.len() > MAX_FILES
        || !absolute_disk_path(&options.source)
    {
        return Err("package source options exceed bounds".into());
    }
    let mut capabilities = BTreeSet::new();
    for capability in &options.capabilities {
        if capability.len() > 64 || !capabilities.insert(capability) {
            return Err("capability exceeds bounds or is duplicated".into());
        }
    }
    let mut dependencies = BTreeSet::new();
    for (name, version) in &options.dependencies {
        if name.len() > 64 || version.len() > 128 || !dependencies.insert(name) {
            return Err("dependency exceeds bounds or is duplicated".into());
        }
    }
    let mut files = BTreeSet::new();
    for path in &options.files {
        validate_asset_path(path)?;
        if !files.insert(path) {
            return Err("duplicate asset path".into());
        }
    }
    validate_asset_collisions(&files)?;
    Ok(())
}

fn manifest(options: &Options) -> Result<Manifest, String> {
    Ok(Manifest {
        schema: 1,
        name: options.name.clone(),
        version: options.version.clone(),
        engine: options.engine.clone(),
        capabilities: options.capabilities.iter().cloned().collect(),
        dependencies: options.dependencies.iter().cloned().collect(),
        files: options.files.iter().cloned().collect(),
    })
}

fn validate_asset_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 240
        || !path.is_ascii()
        || path.split('/').count() > 16
        || path
            .split('/')
            .next()
            .is_some_and(|part| part.eq_ignore_ascii_case(MANIFEST))
    {
        return Err(format!("invalid portable asset path: {path}"));
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
            return Err(format!("nonportable asset path: {path}"));
        }
    }
    Ok(())
}

fn validate_asset_collisions(files: &BTreeSet<&String>) -> Result<(), String> {
    let folded: BTreeSet<_> = files.iter().map(|path| path.to_ascii_lowercase()).collect();
    if folded.len() != files.len() {
        return Err("case-colliding asset paths".into());
    }
    for path in &folded {
        for (index, _) in path.match_indices('/') {
            if folded.contains(&path[..index]) {
                return Err("file/directory asset path collision".into());
            }
        }
    }
    Ok(())
}

fn source_root(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("source path must be Unicode")?;
    if text.len() > MAX_SOURCE_PATH_BYTES
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err("source must be a bounded absolute disk path without traversal".into());
    }
    let mut components = path.components();
    let prefix = match components.next() {
        Some(Component::Prefix(prefix)) => prefix.kind(),
        _ => return Err("source must be an absolute disk path".into()),
    };
    if !matches!(prefix, Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        || !matches!(components.next(), Some(Component::RootDir))
    {
        return Err("source must be an absolute disk path".into());
    }
    let mut current = PathBuf::new();
    current.push(match path.components().next() {
        Some(component @ Component::Prefix(_)) => component,
        _ => return Err("source must be an absolute disk path".into()),
    });
    let mut component_count = 1usize;
    let mut normal_count = 0usize;
    for component in path.components().skip(1) {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err("source path must be normalized".into());
        }
        current.push(component);
        component_count += 1;
        if component_count > MAX_SOURCE_COMPONENTS {
            return Err("source path has too many components".into());
        }
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("source path {}: {error}", current.display()))?;
        if !is_directory(&metadata) {
            return Err("source ancestors must be non-reparse directories".into());
        }
        if matches!(component, Component::Normal(_)) {
            normal_count += 1;
        }
    }
    if normal_count == 0 {
        return Err("source must be a project directory, not a disk root".into());
    }
    let canonical = fs::canonicalize(path).map_err(error)?;
    let metadata = fs::symlink_metadata(&canonical).map_err(error)?;
    if !is_directory(&metadata) {
        return Err("source root must be a non-reparse directory".into());
    }
    Ok(canonical)
}

fn absolute_disk_path(path: &Path) -> bool {
    let Some(text) = path.to_str() else {
        return false;
    };
    if text
        .split(['\\', '/'])
        .any(|part| part == "." || part == "..")
    {
        return false;
    }
    let mut components = path.components();
    let prefix = match components.next() {
        Some(Component::Prefix(prefix)) => prefix.kind(),
        _ => return false,
    };
    matches!(prefix, Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        && matches!(components.next(), Some(Component::RootDir))
        && components.all(|component| matches!(component, Component::Normal(_)))
}

fn read_source_file(root: &Path, relative: &str) -> Result<Vec<u8>, String> {
    let mut path = root.to_path_buf();
    let parts: Vec<_> = relative.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        let metadata = fs::symlink_metadata(&path).map_err(error)?;
        let final_part = index + 1 == parts.len();
        if final_part {
            if !is_regular(&metadata) || metadata.len() > MAX_FILE_BYTES {
                return Err(format!(
                    "declared source must be a regular file within limits: {relative}"
                ));
            }
        } else if !is_directory(&metadata) {
            return Err(format!(
                "declared source ancestor is not a non-reparse directory: {relative}"
            ));
        }
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .map_err(error)?;
    let opened = file.metadata().map_err(error)?;
    if !is_regular(&opened) || opened.len() > MAX_FILE_BYTES {
        return Err(format!("declared source changed while opening: {relative}"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() as u64 > MAX_FILE_BYTES || bytes.len() as u64 != opened.len() {
        return Err(format!("declared source changed while reading: {relative}"));
    }
    Ok(bytes)
}

fn write_snapshot_manifest(root: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(MANIFEST))
        .map_err(error)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(error)
}

fn create_snapshot_placeholder(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path.parent().ok_or("invalid asset path")?).map_err(error)?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(error)?
        .sync_all()
        .map_err(error)
}

fn is_regular(metadata: &Metadata) -> bool {
    metadata.is_file() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

fn is_directory(metadata: &Metadata) -> bool {
    metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

fn exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn string_value(value: Option<OsString>, name: &str) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("missing {name}; {USAGE}"))?;
    if value.as_encoded_bytes().len() > 4096 {
        return Err(format!("{name} exceeds 4096 bytes"));
    }
    value
        .into_string()
        .map_err(|_| format!("{name} must be Unicode"))
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
