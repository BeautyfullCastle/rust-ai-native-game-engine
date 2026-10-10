//! Read-only export closure admission. This detects ordinary source changes;
//! it is not a sandbox against a hostile process replacing paths concurrently.
use crate::project_sprites::Document;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub(crate) const MAX_PROJECT_BYTES: u64 = 256 * 1024 * 1024;
pub(crate) const MAX_PROJECT_FILES: usize = 8192;
pub(crate) const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_JSON_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_SCENE_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 512;
const MAX_PATH_COMPONENTS: usize = 32;
const MAX_COMPONENT_BYTES: usize = 100;
const PROJECT_MANIFEST: &str = "orr.project.json";
const PACKAGE_LOCK: &str = "orr.packages.lock.json";
const PACKAGE_MANIFEST: &str = "orr.package.json";

/// Intentionally not serializable: source paths and source metadata never enter
/// the exported manifest. Hardlinks are allowed on input but are not retained.
#[derive(Debug)]
pub(crate) struct SnapshotFile {
    pub relative: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub role: &'static str,
    source: PathBuf,
    identity: FileIdentity,
    limit: u64,
}

#[derive(Debug, PartialEq, Eq)]
struct FileIdentity {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    unix: (u64, u64, u32, i64, i64, i64, i64),
}
impl FileIdentity {
    fn of(metadata: &fs::Metadata) -> Self {
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            unix: {
                use std::os::unix::fs::MetadataExt;
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.mode(),
                    metadata.mtime(),
                    metadata.mtime_nsec(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                )
            },
        }
    }
}

impl SnapshotFile {
    /// The caller may change the descriptive role before manifest construction.
    pub fn read(source: &Path, relative: &str, limit: u64) -> Result<Self, String> {
        Self::read_with(source, relative, limit, "binary", |source| {
            let (mut file, before) = open_regular(source, limit)?;
            let mut bytes = Vec::new();
            (&mut file)
                .take(limit + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("read {}: {e}", source.display()))?;
            ensure_stable_read(source, &file, &before, bytes.len() as u64, limit)?;
            Ok(bytes)
        })
    }

    fn read_with(
        source: &Path,
        relative: &str,
        limit: u64,
        role: &'static str,
        read: impl FnOnce(&Path) -> Result<Vec<u8>, String>,
    ) -> Result<Self, String> {
        validate_relative(relative)?;
        let source = check_path(source, false)?;
        let before = regular_metadata(&source, limit)?;
        let bytes = read(&source)?;
        if bytes.len() as u64 > limit {
            return Err(format!("export file exceeds {limit} bytes: {relative}"));
        }
        check_path(&source, false)?;
        let after = regular_metadata(&source, limit)?;
        if before != after || before.len != bytes.len() as u64 {
            return Err(format!("export source changed while reading: {relative}"));
        }
        let snapshot = Self {
            relative: relative.to_owned(),
            sha256: hash(&bytes),
            bytes,
            role,
            source,
            identity: before,
            limit,
        };
        // Same-length writes can share timestamps on coarse-grained filesystems.
        // Require current bytes as well as metadata before admitting the snapshot.
        snapshot.recheck().map_err(|error| {
            format!("export source changed while reading: {relative}: {error}")
        })?;
        Ok(snapshot)
    }

    pub fn recheck(&self) -> Result<(), String> {
        let (mut file, before) = open_regular(&self.source, self.limit)?;
        if before != self.identity {
            return Err(format!("export source metadata changed: {}", self.relative));
        }
        // Always read bytes again, even when metadata agrees. An edit through an
        // alias must not evade validation by preserving length or restoring mtime.
        let mut digest = Sha256::new();
        let mut length = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|e| format!("recheck {}: {e}", self.relative))?;
            if count == 0 {
                break;
            }
            length = length
                .checked_add(count as u64)
                .ok_or("export source size overflow")?;
            if length > self.limit || length > self.identity.len {
                return Err(format!("export source grew: {}", self.relative));
            }
            digest.update(&buffer[..count]);
        }
        ensure_stable_read(&self.source, &file, &before, length, self.limit)?;
        if length != self.bytes.len() as u64 || hex(&digest.finalize()) != self.sha256 {
            return Err(format!("export source bytes changed: {}", self.relative));
        }
        Ok(())
    }
}

pub(crate) struct ProjectSnapshot {
    pub root: PathBuf,
    pub files: Vec<SnapshotFile>,
    pub entry: orr_package::ProjectEntry,
    pub package_identities: BTreeMap<String, String>,
    pub initial_checksum: u64,
    absent_lock: bool,
}

impl ProjectSnapshot {
    pub fn open(root: &Path) -> Result<Self, String> { Self::open_profile(root, super::Profile::Arena) }
    pub fn open_profile(root: &Path, profile: super::Profile) -> Result<Self, String> {
        let root = check_path(root, true)?;
        // Pin exact bytes before any existing validator opens either authority.
        let mut manifest_file = SnapshotFile::read(
            &root.join(PROJECT_MANIFEST),
            PROJECT_MANIFEST,
            MAX_JSON_BYTES,
        )?;
        manifest_file.role = "project_manifest";
        let mut lock_file = match fs::symlink_metadata(root.join(PACKAGE_LOCK)) {
            Ok(_) => Some(SnapshotFile::read(
                &root.join(PACKAGE_LOCK),
                PACKAGE_LOCK,
                MAX_JSON_BYTES,
            )?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("inspect package lock: {e}")),
        };
        if let Some(file) = &mut lock_file {
            file.role = "package_lock";
        }
        let absent_lock = lock_file.is_none();
        let project = orr_package::Project::open(&root, profile.runtime())
            .map_err(|e| format!("export project: {e}"))?;
        let manifest = project
            .manifest()
            .ok_or("export requires a schema-2/3 authored project")?;
        let pinned_manifest: orr_package::ProjectManifest =
            serde_json::from_slice(&manifest_file.bytes)
                .map_err(|e| format!("export project manifest: {e}"))?;
        if manifest != &pinned_manifest {
            return Err("project manifest changed during export admission".into());
        }
        if !matches!(manifest.schema, 2 | 3) {
            return Err("export requires a schema-2/3 authored project".into());
        }
        let entry = manifest
            .entry
            .clone()
            .ok_or("export requires an authored game entry")?;
        // list performs the existing package validation/resolution without
        // reading object assets. Only that authoritative closure is enumerated.
        let lock = project
            .list()
            .map_err(|e| format!("export package lock: {e}"))?;
        let pinned_lock: orr_package::Lock = match &lock_file {
            Some(file) => serde_json::from_slice(&file.bytes)
                .map_err(|e| format!("export package lock: {e}"))?,
            None => orr_package::Lock::default(),
        };
        if lock != pinned_lock {
            return Err("package lock changed during export admission".into());
        }
        manifest_file.recheck()?;
        if let Some(file) = &lock_file {
            file.recheck()?;
        } else {
            require_absent_lock(&root)?;
        }

        let mut plan = Plan::default();
        plan.add(PROJECT_MANIFEST, MAX_JSON_BYTES)?;
        if lock_file.is_some() {
            plan.add(PACKAGE_LOCK, MAX_JSON_BYTES)?;
        }
        plan.add(&entry.scene, MAX_SCENE_BYTES)?;
        if let Some(sidecar) = &entry.models { plan.add(sidecar,MAX_JSON_BYTES)?; }
        if let Some(sidecar) = &entry.sprites {
            plan.add(sidecar, MAX_JSON_BYTES)?;
        }
        if let Some(document) = entry.ui.as_ref().and_then(|ui| ui.document.as_ref()) { plan.add(document, 64 * 1024)?; }
        for package in lock.packages.values() {
            let object = format!(".orr/packages/objects/{}", package.digest);
            plan.add(&format!("{object}/{PACKAGE_MANIFEST}"), MAX_JSON_BYTES)?;
            for path in &package.manifest.files {
                plan.add(&format!("{object}/{path}"), MAX_FILE_BYTES)?;
            }
        }
        // Before verify or PreparedRuntime can read all packages, enforce the
        // export-wide budget. Package verification itself has per-package limits.
        plan.preflight(&root)?;
        let verified = project
            .verify()
            .map_err(|e| format!("export package verification: {e}"))?;
        if verified != lock {
            return Err("package lock changed during export verification".into());
        }
        let prepared = super::Prepared::open(&root, profile)?;
        let initial_checksum = prepared.checksum();
        if prepared.root() != root {
            return Err("project root changed during export admission".into());
        }
        let mut files = vec![manifest_file];
        if let Some(file) = lock_file {
            files.push(file);
        }
        let mut scene =
            SnapshotFile::read(&root.join(&entry.scene), &entry.scene, MAX_SCENE_BYTES)?;
        scene.role = "scene";
        if scene.source != prepared.scene_path()
            || scene.bytes != prepared.scene_text().as_bytes()
        {
            return Err("entry scene changed after runtime admission".into());
        }
        files.push(scene);
        match (&entry.sprites, prepared.sprites()) {
            (Some(path), Some(sprites)) => {
                let mut file = SnapshotFile::read(&root.join(path), path, MAX_JSON_BYTES)?;
                file.role = "sprite_sidecar";
                let document = Document::from_bytes(&file.bytes)?;
                if file.source != sprites.path || document != sprites.document {
                    return Err("sprite sidecar changed after runtime admission".into());
                }
                files.push(file);
            }
            (None, None) => {}
            _ => return Err("sprite sidecar changed during runtime admission".into()),
        }
        #[cfg(feature = "room-project")]
        if let Some(path) = &entry.models {
            let super::Prepared::Room(room)=&prepared else { return Err("model sidecar requires Room consumer".into()); };
            let mut file=SnapshotFile::read(&root.join(path),path,MAX_JSON_BYTES)?;
            let document:orr_model_bindings::model_bindings::Document=serde_json::from_slice(&file.bytes).map_err(|e|format!("export model sidecar: {e}"))?;
            if file.source!=room.models().path || document!=room.models().document { return Err("model sidecar changed after runtime admission".into()); }
            file.role="model_sidecar"; files.push(file);
        }
        #[cfg(feature = "collect-ui")]
        if let Some(path) = entry.ui.as_ref().and_then(|ui| ui.document.as_ref()) {
            let super::Prepared::Collect(collect) = &prepared else { return Err("authored UI requires Collect consumer".into()); };
            let admitted = collect.ui().ok_or("missing admitted Collect UI")?;
            let mut file = SnapshotFile::read(&root.join(path), path, 64 * 1024)?;
            if file.source != admitted.path || crate::authored_ui::Document::parse(&file.bytes)? != admitted.document { return Err("UI document changed after runtime admission".into()); }
            file.role = "ui_document";
            files.push(file);
        }
        // The initial frame and decoded atlases are no longer needed. Release
        // them before retaining the whole package closure in the snapshot.
        drop(prepared);
        let mut retained_bytes: u64 = files.iter().map(|f| f.bytes.len() as u64).sum();
        for (name, package) in &lock.packages {
            let object = format!(".orr/packages/objects/{}", package.digest);
            let relative = format!("{object}/{PACKAGE_MANIFEST}");
            let mut file = SnapshotFile::read(&root.join(&relative), &relative, MAX_JSON_BYTES)?;
            file.role = "package_manifest";
            let stored: orr_package::Manifest = serde_json::from_slice(&file.bytes)
                .map_err(|e| format!("export installed manifest: {e}"))?;
            if stored != package.manifest {
                return Err("installed package manifest changed after verification".into());
            }
            add_snapshot(&mut files, &mut retained_bytes, file)?;
            for (path, expected) in &package.files {
                let relative = format!("{object}/{path}");
                let file = SnapshotFile::read_with(
                    &root.join(&relative),
                    &relative,
                    MAX_FILE_BYTES,
                    "package_asset",
                    |_| project.read_asset(name, path).map_err(|e| e.to_string()),
                )?;
                if &file.sha256 != expected {
                    return Err(format!("package asset changed from pinned lock: {path}"));
                }
                add_snapshot(&mut files, &mut retained_bytes, file)?;
            }
        }
        files.sort_by(|a, b| a.relative.cmp(&b.relative));
        let snapshot = Self {
            root,
            files,
            entry,
            package_identities: lock
                .packages
                .iter()
                .map(|(name, p)| (name.clone(), p.digest.clone()))
                .collect(),
            initial_checksum,
            absent_lock,
        };
        snapshot.recheck()?;
        Ok(snapshot)
    }

    pub fn recheck(&self) -> Result<(), String> {
        check_path(&self.root, true)?;
        for file in &self.files {
            file.recheck()?;
        }
        if self.absent_lock {
            require_absent_lock(&self.root)?;
        }
        Ok(())
    }
}

fn add_snapshot(
    files: &mut Vec<SnapshotFile>,
    retained_bytes: &mut u64,
    file: SnapshotFile,
) -> Result<(), String> {
    *retained_bytes = retained_bytes
        .checked_add(file.bytes.len() as u64)
        .ok_or("export project byte count overflow")?;
    if *retained_bytes > MAX_PROJECT_BYTES || files.len() >= MAX_PROJECT_FILES {
        return Err("export project exceeds total byte or file count bound".into());
    }
    files.push(file);
    Ok(())
}

#[derive(Default)]
struct Plan {
    paths: BTreeMap<String, u64>,
    folded: BTreeSet<String>,
}
impl Plan {
    fn add(&mut self, relative: &str, limit: u64) -> Result<(), String> {
        validate_relative(relative)?;
        if self.paths.len() >= MAX_PROJECT_FILES {
            return Err(format!("export project exceeds {MAX_PROJECT_FILES} files"));
        }
        let folded = relative.to_ascii_lowercase();
        if !self.folded.insert(folded) {
            return Err(format!("export path collision: {relative}"));
        }
        self.paths.insert(relative.to_owned(), limit);
        Ok(())
    }

    fn preflight(&self, root: &Path) -> Result<(), String> {
        for path in &self.folded {
            for (index, _) in path.match_indices('/') {
                if self.folded.contains(&path[..index]) {
                    return Err(format!("export file/directory collision: {path}"));
                }
            }
        }
        let mut total = 0u64;
        for (relative, limit) in &self.paths {
            let path = check_path(&root.join(relative), false)?;
            let metadata = regular_metadata(&path, *limit)?;
            total = total
                .checked_add(metadata.len)
                .ok_or("export project byte count overflow")?;
            if total > MAX_PROJECT_BYTES {
                return Err(format!("export project exceeds {MAX_PROJECT_BYTES} bytes"));
            }
        }
        Ok(())
    }
}

fn require_absent_lock(root: &Path) -> Result<(), String> {
    check_path(root, true)?;
    match fs::symlink_metadata(root.join(PACKAGE_LOCK)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("inspect absent package lock: {e}")),
        Ok(_) => Err("package lock appeared during export".into()),
    }
}

/// Check the supplied spelling, including every ancestor, before canonicalizing.
/// In particular, a symlink must not be made invisible through canonicalization.
pub(crate) fn check_path(path: &Path, expect_dir: bool) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("empty filesystem path".into());
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let mut current = PathBuf::new();
    let mut components = absolute.components().peekable();
    while let Some(component) = components.next() {
        if matches!(component, Component::ParentDir) {
            return Err("parent traversal rejected in export source path".into());
        }
        current.push(component);
        if matches!(component, Component::Prefix(_))
            && components.peek() == Some(&Component::RootDir)
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&current)
            .map_err(|e| format!("export path {}: {e}", current.display()))?;
        let directory = expect_dir || components.peek().is_some();
        if metadata.file_type().is_symlink()
            || (directory && !metadata.is_dir())
            || (!directory && !metadata.is_file())
        {
            return Err(format!(
                "export requires a regular {}, without symlinks: {}",
                if directory { "directory" } else { "file" },
                current.display()
            ));
        }
    }
    fs::canonicalize(&absolute).map_err(|e| format!("canonical export path: {e}"))
}

pub(crate) fn validate_relative(relative: &str) -> Result<(), String> {
    if relative.is_empty()
        || relative.len() > MAX_PATH_BYTES
        || !relative.is_ascii()
        || Path::new(relative).is_absolute()
        || relative.split('/').count() > MAX_PATH_COMPONENTS
    {
        return Err(format!("invalid relative export path: {relative}"));
    }
    for part in relative.split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.len() > MAX_COMPONENT_BYTES
            || part.ends_with('.')
            || !part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err(format!("invalid relative export path: {relative}"));
        }
    }
    Ok(())
}

fn regular_metadata(path: &Path, limit: u64) -> Result<FileIdentity, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|e| format!("export source {}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(format!(
            "export source must be a regular file within {limit} bytes: {}",
            path.display()
        ));
    }
    Ok(FileIdentity::of(&metadata))
}

fn open_regular(path: &Path, limit: u64) -> Result<(fs::File, FileIdentity), String> {
    check_path(path, false)?;
    let before = regular_metadata(path, limit)?;
    let file = fs::File::open(path).map_err(|e| format!("open export source: {e}"))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || before != FileIdentity::of(&metadata) {
        return Err("export source changed while opening".into());
    }
    Ok((file, before))
}

fn ensure_stable_read(
    path: &Path,
    file: &fs::File,
    before: &FileIdentity,
    length: u64,
    limit: u64,
) -> Result<(), String> {
    let after = file.metadata().map_err(|e| e.to_string())?;
    check_path(path, false)?;
    if length > limit
        || length != before.len
        || &FileIdentity::of(&after) != before
        || &regular_metadata(path, limit)? != before
    {
        return Err(format!(
            "export source changed while reading: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(crate) fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary() -> tempfile::TempDir {
        tempfile::TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
    }

    fn copy_tree(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    fn fixture() -> tempfile::TempDir {
        let dir = temporary();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/saved_arena_project")
            .canonicalize()
            .unwrap();
        copy_tree(&source, dir.path());
        dir
    }

    #[test]
    fn exact_nine_file_closure_excludes_unrelated_content() {
        let dir = fixture();
        fs::write(dir.path().join("editor.cache"), b"unrelated").unwrap();
        let snapshot = ProjectSnapshot::open(dir.path()).unwrap();
        assert_eq!(snapshot.files.len(), 9);
        assert_eq!(snapshot.package_identities.len(), 1);
        assert!(snapshot
            .files
            .windows(2)
            .all(|w| w[0].relative < w[1].relative));
        assert!(snapshot
            .files
            .iter()
            .any(|f| f.relative.ends_with("LICENSE.txt")));
        assert!(snapshot
            .files
            .iter()
            .any(|f| f.relative.ends_with("lantern_keeper.rgba")));
        assert!(!snapshot
            .files
            .iter()
            .any(|f| f.relative == "README.md" || f.relative == "editor.cache"));
        for file in &snapshot.files {
            assert_eq!(
                file.bytes,
                fs::read(dir.path().join(&file.relative)).unwrap()
            );
            assert_eq!(file.sha256, hash(&file.bytes));
        }
        snapshot.recheck().unwrap();
    }

    #[test]
    fn absent_lock_is_preserved_and_appearance_is_detected() {
        let dir = fixture();
        let path = dir.path().join(PROJECT_MANIFEST);
        let mut manifest: orr_package::ProjectManifest =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest.entry.as_mut().unwrap().sprites = None;
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        fs::remove_file(dir.path().join(PACKAGE_LOCK)).unwrap();
        let snapshot = ProjectSnapshot::open(dir.path()).unwrap();
        assert_eq!(snapshot.files.len(), 2);
        assert!(snapshot.absent_lock);
        fs::write(dir.path().join(PACKAGE_LOCK), b"{}").unwrap();
        assert!(snapshot.recheck().unwrap_err().contains("appeared"));
    }

    #[test]
    fn manifest_exact_byte_edits_are_detected() {
        let dir = fixture();
        let snapshot = ProjectSnapshot::open(dir.path()).unwrap();
        let path = dir.path().join(PROJECT_MANIFEST);
        let mut bytes = fs::read(&path).unwrap();
        bytes.push(b' ');
        fs::write(path, bytes).unwrap();
        assert!(snapshot.recheck().is_err());
    }

    #[test]
    fn mutation_during_snapshot_read_is_detected() {
        let dir = temporary();
        let source = dir.path().join("source");
        fs::write(&source, b"original").unwrap();
        let result = SnapshotFile::read_with(&source, "asset", 64, "package_asset", |path| {
            let bytes = fs::read(path).unwrap();
            fs::write(path, b"modified").unwrap();
            Ok(bytes)
        });
        assert!(result.unwrap_err().contains("changed while reading"));

        // A filesystem can report identical metadata for a same-length write.
        // Inject stale reader bytes without changing the current file at all,
        // so rejection must also compare bytes rather than rely on timestamps.
        assert_eq!(fs::read(&source).unwrap(), b"modified");
        let identity = regular_metadata(&source, 64).unwrap();
        let stale = SnapshotFile::read_with(&source, "asset", 64, "package_asset", |_| {
            Ok(b"original".to_vec())
        });
        assert_eq!(regular_metadata(&source, 64).unwrap(), identity);
        let error = stale.unwrap_err();
        assert!(error.contains("changed while reading"), "{error}");
        assert!(error.contains("bytes changed"), "{error}");
    }

    #[test]
    fn oversized_scene_and_manifest_are_rejected_before_parsing() {
        let dir = fixture();
        fs::OpenOptions::new()
            .write(true)
            .open(dir.path().join("arena.scene.yaml"))
            .unwrap()
            .set_len(MAX_SCENE_BYTES + 1)
            .unwrap();
        assert!(ProjectSnapshot::open(dir.path())
            .err()
            .unwrap()
            .contains("within 4194304 bytes"));
        fs::OpenOptions::new()
            .write(true)
            .open(dir.path().join(PROJECT_MANIFEST))
            .unwrap()
            .set_len(MAX_JSON_BYTES + 1)
            .unwrap();
        assert!(ProjectSnapshot::open(dir.path())
            .err()
            .unwrap()
            .contains("within 1048576 bytes"));
    }

    #[test]
    fn hardlink_inputs_are_accepted_but_alias_mutation_is_rejected() {
        let dir = temporary();
        let source = dir.path().join("source");
        let alias = dir.path().join("alias");
        fs::write(&source, b"original").unwrap();
        fs::hard_link(&source, &alias).unwrap();
        let snapshot = SnapshotFile::read(&source, "asset", 64).unwrap();
        snapshot.recheck().unwrap();
        fs::write(alias, b"modified").unwrap();
        assert!(snapshot.recheck().is_err());
    }

    #[test]
    fn recheck_hashes_bytes_even_when_identity_matches() {
        let dir = temporary();
        let source = dir.path().join("source");
        fs::write(&source, b"original").unwrap();
        let mut snapshot = SnapshotFile::read(&source, "asset", 64).unwrap();
        fs::write(&source, b"modified").unwrap();
        // Simulate a filesystem reporting matching metadata: the fresh byte
        // hash is an independent requirement, not a shortcut based on mtime.
        snapshot.identity = regular_metadata(&source, 64).unwrap();
        assert!(snapshot.recheck().unwrap_err().contains("bytes changed"));
    }

    #[test]
    fn checked_paths_reject_traversal_and_collisions() {
        for path in [
            "", "/root", "../asset", "a/../b", "a//b", "a\\b", "a:b", "./a", "a\nb", "a\0b", "a b",
            "file.", "CON.txt", "lpt1.txt",
        ] {
            assert!(validate_relative(path).is_err(), "{path}");
        }
        let mut plan = Plan::default();
        plan.add("a/File", 10).unwrap();
        assert!(plan.add("a/file", 10).is_err());
        let mut plan = Plan::default();
        plan.add("a", 10).unwrap();
        plan.add("a/file", 10).unwrap();
        assert!(plan
            .preflight(Path::new("/"))
            .unwrap_err()
            .contains("collision"));
        assert!(check_path(Path::new(""), true).is_err());
        assert!(validate_relative(&"a".repeat(MAX_COMPONENT_BYTES + 1)).is_err());
        assert!(validate_relative(&["a"; MAX_PATH_COMPONENTS + 1].join("/")).is_err());
        assert!(validate_relative(&vec!["a".repeat(100); 6].join("/")).is_err());
        assert!(validate_relative(&vec!["a".repeat(100); 5].join("/")).is_ok());
    }

    #[test]
    fn sparse_bounds_are_rejected_without_reading_payloads() {
        let dir = temporary();
        let file = fs::File::create(dir.path().join("large")).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(SnapshotFile::read(&dir.path().join("large"), "large", MAX_FILE_BYTES).is_err());
        let mut plan = Plan::default();
        for index in 0..5 {
            let relative = format!("large{index}");
            fs::File::create(dir.path().join(&relative))
                .unwrap()
                .set_len(MAX_FILE_BYTES)
                .unwrap();
            plan.add(&relative, MAX_FILE_BYTES).unwrap();
        }
        assert!(plan.preflight(dir.path()).unwrap_err().contains("exceeds"));
        let mut plan = Plan::default();
        for index in 0..MAX_PROJECT_FILES {
            plan.add(&format!("asset{index}"), 1).unwrap();
        }
        assert!(plan.add("too-many", 1).is_err());
    }

    #[test]
    fn inactive_tampered_object_does_not_enter_closure() {
        let dir = fixture();
        let inactive = dir.path().join(".orr/packages/objects/inactive");
        fs::create_dir(&inactive).unwrap();
        fs::write(inactive.join("orr.package.json"), b"broken").unwrap();
        assert_eq!(ProjectSnapshot::open(dir.path()).unwrap().files.len(), 9);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_in_files_or_ancestors_are_rejected() {
        use std::os::unix::fs::symlink;
        let dir = temporary();
        fs::create_dir(dir.path().join("real")).unwrap();
        let source = dir.path().join("real/source");
        fs::write(&source, b"bytes").unwrap();
        symlink(&source, dir.path().join("link")).unwrap();
        symlink(dir.path().join("real"), dir.path().join("directory")).unwrap();
        assert!(SnapshotFile::read(&dir.path().join("link"), "asset", 64).is_err());
        assert!(SnapshotFile::read(&dir.path().join("directory/source"), "asset", 64).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn fifo_and_socket_are_rejected_before_opening() {
        let dir = temporary();
        let fifo = dir.path().join("fifo");
        rustix::fs::mkfifoat(rustix::fs::CWD, &fifo, rustix::fs::Mode::RUSR).unwrap();
        assert!(SnapshotFile::read(&fifo, "asset", 64).is_err());
        let socket = dir.path().join("socket");
        // A socket filesystem node is sufficient for rejection; no listener or
        // network operation is needed for this filesystem-only test.
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &socket,
            rustix::fs::FileType::Socket,
            rustix::fs::Mode::RUSR,
            0,
        )
        .unwrap();
        assert!(SnapshotFile::read(&socket, "asset", 64).is_err());
    }
}
