//! Offline, content-only packages. No script execution, downloads or Cargo edits.
//! The application supplies its real compiled capabilities through [`Runtime`].
mod project_identity;
pub use project_identity::{format_game_id, ProgressProfile, ProjectProgress};

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MANIFEST: &str = "orr.package.json";
const LOCK: &str = "orr.packages.lock.json";
const MAX_JSON: u64 = 1024 * 1024;
const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_TOTAL: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 4096;
const MAX_PACKAGES: usize = 128;

#[derive(Debug)]
pub struct Error(String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self(e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;
fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(Error(message.into()))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub name: String,
    pub version: String,
    pub engine: String,
    #[serde(default)]
    pub capabilities: BTreeSet<String>,
    /// Exact versions, never ranges. Dependencies are resolved from supplied local sources
    /// or already installed immutable objects.
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    /// Explicit portable paths; no directory globs or implicit executable hooks.
    pub files: BTreeSet<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LockedPackage {
    pub manifest: Manifest,
    pub digest: String,
    pub files: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    pub schema: u32,
    pub direct: BTreeMap<String, String>,
    pub packages: BTreeMap<String, LockedPackage>,
}
impl Default for Lock {
    fn default() -> Self {
        Self {
            schema: 1,
            direct: BTreeMap::new(),
            packages: BTreeMap::new(),
        }
    }
}
/// Capabilities must come from the application's compiled feature inventory, never a
/// package/project JSON claim. An empty inventory supports generic content only.
#[derive(Clone, Debug)]
pub struct Runtime {
    pub engine_version: Version,
    pub capabilities: BTreeSet<String>,
}
impl Runtime {
    pub fn content_only() -> Self {
        Self {
            engine_version: Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version"),
            capabilities: BTreeSet::new(),
        }
    }
}
/// Project metadata. Schema 1 remains metadata-only; schema 2 selects one
/// entry scene; schema 3 adds explicit CollectDodge progress identity. None
/// selects packages: the lock alone controls package activation and versions.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectManifest {
    pub schema: u32,
    pub engine: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_entry"
    )]
    pub entry: Option<ProjectEntry>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_progress"
    )]
    pub progress: Option<ProjectProgress>,
}

/// Closed launch profiles supported by the project contract. This is metadata,
/// not permission to enable optional code in a consuming application.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProjectGame {
    #[serde(rename = "arena")]
    Arena,
    #[serde(rename = "collect-dodge-v1")]
    CollectDodgeV1,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectEntry {
    pub game: ProjectGame,
    /// Portable path relative to the project root.
    pub scene: String,
    /// Optional sprite sidecar, also relative to the project root.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_sprites"
    )]
    pub sprites: Option<String>,
    /// Optional closed presentation preset. Assets are resolved only by the lock.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_ui"
    )]
    pub ui: Option<ProjectUi>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProjectUiProfile {
    #[serde(rename = "arena-korean-v1")]
    ArenaKoreanV1,
    #[serde(rename = "collect-authored-v1")]
    CollectAuthoredV1,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectUi {
    pub profile: ProjectUiProfile,
    pub font: ProjectFont,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present_sprites")]
    pub document: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectFont {
    pub package: String,
    pub asset: String,
}

// A missing optional field is valid. An explicitly supplied null is not a
// second spelling of missing launch metadata (including on schema 1).
fn present_entry<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<ProjectEntry>, D::Error> {
    ProjectEntry::deserialize(d).map(Some)
}
fn present_sprites<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
fn present_ui<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<ProjectUi>, D::Error> {
    ProjectUi::deserialize(d).map(Some)
}

fn present_progress<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<ProjectProgress>, D::Error> {
    ProjectProgress::deserialize(d).map(Some)
}

impl ProjectManifest {
    fn validate(&self, runtime: &Runtime) -> Result<()> {
        match (self.schema, &self.progress) {
            (1 | 2, None) => {}
            (3, Some(progress)) => {
                if let Err(message) = progress.validate() {
                    return fail(&message);
                }
                if self.entry.as_ref().map(|e| e.game) != Some(ProjectGame::CollectDodgeV1) {
                    return fail("schema-3 progress requires collect-dodge-v1");
                }
            }
            _ => return fail("progress metadata requires schema 3 and schema 3 requires progress"),
        }
        match (self.schema, &self.entry) {
            (1, None) => {}
            (2 | 3, Some(entry)) => {
                portable(&entry.scene)?;
                if let Some(sprites) = &entry.sprites {
                    portable(sprites)?;
                    if sprites.eq_ignore_ascii_case(&entry.scene) {
                        return fail("entry scene and sprite sidecar must be different files");
                    }
                }
                if let Some(ui) = &entry.ui {
                    match (entry.game, ui.profile, &ui.document) {
                        (ProjectGame::Arena, ProjectUiProfile::ArenaKoreanV1, None) => {},
                        (ProjectGame::CollectDodgeV1, ProjectUiProfile::CollectAuthoredV1, Some(document)) => {
                            portable(document)?;
                            if document.eq_ignore_ascii_case(&entry.scene) || entry.sprites.as_ref().is_some_and(|s| s.eq_ignore_ascii_case(document)) {
                                return fail("UI document must differ from scene and sprites");
                            }
                        },
                        _ => return fail("UI profile/document does not match project game"),
                    }
                    name(&ui.font.package)?;
                    portable(&ui.font.asset)?;
                }
            }
            (1, Some(_)) => return fail("schema-1 projects cannot contain entry metadata"),
            (2 | 3, None) => return fail("schema-2 projects require an entry"),
            _ => return fail("unsupported project schema"),
        }
        compatible(&self.engine, runtime)
    }
}

pub struct Project {
    root: PathBuf,
    manifest: Option<ProjectManifest>,
    runtime: Runtime,
    enforce_capabilities: bool,
}
impl Project {
    /// Opens an existing directory. Optional `orr.project.json` constrains the engine;
    /// the lock is the sole authority for direct selection and resolved package state.
    pub fn open(root: impl AsRef<Path>, runtime: Runtime) -> Result<Self> {
        no_links(root.as_ref())?;
        let root = fs::canonicalize(root)?;
        if !root.is_dir() {
            return fail("project root is not a directory");
        }
        let project = root.join("orr.project.json");
        let manifest = match fs::symlink_metadata(&project) {
            Ok(_) => {
                let p: ProjectManifest = json(&project)?;
                p.validate(&runtime)?;
                Some(p)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            root,
            manifest,
            runtime,
            enforce_capabilities: true,
        })
    }
    /// Canonical, symlink-free project root captured at open.
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Validated metadata captured at open, without rewriting it or the lock.
    pub fn manifest(&self) -> Option<&ProjectManifest> {
        self.manifest.as_ref()
    }
    /// Metadata-only installation tool. This cannot read assets: a consuming host must
    /// reopen with its compiled capability inventory before loading content.
    pub fn open_for_install(root: impl AsRef<Path>, engine_version: Version) -> Result<Self> {
        let mut project = Self::open(
            root,
            Runtime {
                engine_version,
                capabilities: BTreeSet::new(),
            },
        )?;
        project.enforce_capabilities = false;
        Ok(project)
    }
    pub fn list(&self) -> Result<Lock> {
        let path = self.root.join(LOCK);
        let lock: Lock = match fs::symlink_metadata(&path) {
            Ok(_) => json(&path)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Lock::default(),
            Err(e) => return Err(e.into()),
        };
        self.validate_lock(&lock)?;
        Ok(lock)
    }
    fn validate_lock(&self, lock: &Lock) -> Result<()> {
        if lock.schema != 1 || lock.packages.len() > MAX_PACKAGES {
            return fail("invalid lock schema or package limit");
        }
        for (name, p) in &lock.packages {
            validate_manifest(&p.manifest, &self.runtime, self.enforce_capabilities)?;
            if name != &p.manifest.name
                || p.files.keys().cloned().collect::<BTreeSet<_>>() != p.manifest.files
                || !hex_digest(&p.digest)
                || p.files.values().any(|s| !hex_digest(s))
                || identity(p)? != p.digest
            {
                return fail("invalid lock identity");
            }
        }
        let resolved = resolve(&lock.direct, &lock.packages)?;
        if resolved != lock.packages {
            return fail("lock contains unreachable packages");
        }
        Ok(())
    }
    /// Verify every installed byte against the authoritative lock before use.
    pub fn verify(&self) -> Result<Lock> {
        let lock = self.list()?;
        for p in lock.packages.values() {
            self.verify_object(p)?;
        }
        Ok(lock)
    }
    fn object(&self, p: &LockedPackage) -> PathBuf {
        self.root.join(".orr/packages/objects").join(&p.digest)
    }
    fn verify_object(&self, p: &LockedPackage) -> Result<()> {
        let root = self.object(p);
        no_links(&root)?;
        let stored: Manifest = json(&root.join(MANIFEST))?;
        if stored != p.manifest {
            return fail("installed manifest changed");
        }
        let mut total = 0;
        for (path, digest) in &p.files {
            let bytes = read_bounded(&root.join(path), MAX_FILE)?;
            total += bytes.len() as u64;
            if total > MAX_TOTAL || hash(&bytes) != *digest {
                return fail(format!("installed content changed: {path}"));
            }
        }
        Ok(())
    }
    /// Returns verified asset bytes, never an unchecked path vulnerable to later edits.
    pub fn read_asset(&self, package: &str, path: &str) -> Result<Vec<u8>> {
        if !self.enforce_capabilities {
            return fail("asset loading requires a compiled host capability inventory");
        }
        portable(path)?;
        let lock = self.list()?;
        let p = lock
            .packages
            .get(package)
            .ok_or_else(|| Error(format!("missing package: {package}")))?;
        let expected = p
            .files
            .get(path)
            .ok_or_else(|| Error(format!("asset not declared: {path}")))?;
        let bytes = read_bounded(&self.object(p).join(path), MAX_FILE)?;
        if hash(&bytes) != *expected {
            return fail(format!("installed content changed: {path}"));
        }
        Ok(bytes)
    }
    /// Select each supplied source directly, resolving dependencies from active objects.
    pub fn install(&self, sources: &[PathBuf]) -> Result<Lock> {
        self.install_with_dependencies(sources, &[])
    }
    /// Candidate sources supply dependencies without adding direct selections. Only
    /// reachable candidates become active. One exact version per package name.
    pub fn install_with_dependencies(
        &self,
        sources: &[PathBuf],
        candidates: &[PathBuf],
    ) -> Result<Lock> {
        self.install_transaction(sources, candidates, None, None, || Ok(()))
    }
    /// Activate precisely a preflighted candidate, only while the complete old
    /// lock still matches. The callback runs under the writer guard immediately
    /// before publication; an error retains the old active lock. Object staging
    /// may leave unreachable immutable objects. No fallible work follows rename.
    pub fn install_checked(
        &self,
        sources: &[PathBuf],
        expected: &Lock,
        approved: &Lock,
        before_publish: impl FnOnce() -> std::result::Result<(), String>,
    ) -> Result<Lock> {
        self.install_transaction(sources, &[], Some(expected), Some(approved), before_publish)
    }
    fn install_transaction(
        &self,
        sources: &[PathBuf],
        candidates: &[PathBuf],
        expected: Option<&Lock>,
        approved: Option<&Lock>,
        before_publish: impl FnOnce() -> std::result::Result<(), String>,
    ) -> Result<Lock> {
        if sources.is_empty() || sources.len() + candidates.len() > MAX_PACKAGES {
            return fail("supply 1..128 local package directories");
        }
        let _guard = self.writer()?;
        let previous = self.verify()?;
        if expected.is_some_and(|expected| expected != &previous) {
            return fail("active package lock changed since preparation");
        }
        let mut direct = previous.direct;
        let mut available = previous.packages;
        let mut snapshots = BTreeMap::new();
        let mut source_names = BTreeSet::new();
        let mut transaction_bytes = 0_u64;
        for (source, selected) in sources
            .iter()
            .map(|p| (p, true))
            .chain(candidates.iter().map(|p| (p, false)))
        {
            let (p, files) = snapshot(source, &self.runtime, self.enforce_capabilities)?;
            transaction_bytes += files.values().map(|b| b.len() as u64).sum::<u64>();
            if transaction_bytes > MAX_TOTAL {
                return fail("transaction byte limit exceeded");
            }
            if !source_names.insert(p.manifest.name.clone()) {
                return fail("duplicate package name in install");
            }
            if let Some(old) = available.get(&p.manifest.name) {
                if old.manifest.version == p.manifest.version && old.digest != p.digest {
                    return fail(
                        "same name/version has different content; bump the package version",
                    );
                }
            }
            if selected {
                direct.insert(p.manifest.name.clone(), p.manifest.version.clone());
            }
            snapshots.insert(p.digest.clone(), files);
            available.insert(p.manifest.name.clone(), p);
        }
        let packages = resolve(&direct, &available)?;
        let lock = Lock {
            schema: 1,
            direct,
            packages,
        };
        if approved.is_some_and(|approved| approved != &lock) {
            return fail("candidate package lock differs from preflighted content");
        }
        for p in lock.packages.values() {
            if let Some(files) = snapshots.get(&p.digest) {
                self.store(p, files)?;
            }
        }
        before_publish().map_err(Error)?;
        self.publish(&lock)?;
        Ok(lock)
    }
    /// Remove a direct selection. A required transitive package remains active.
    /// Stored objects and all source files are retained.
    pub fn remove(&self, name: &str) -> Result<Lock> {
        let _guard = self.writer()?;
        let old = self.verify()?;
        let mut direct = old.direct;
        if direct.remove(name).is_none() {
            return fail(format!("not a direct package: {name}"));
        }
        let packages = resolve(&direct, &old.packages)?;
        let lock = Lock {
            schema: 1,
            direct,
            packages,
        };
        self.publish(&lock)?;
        Ok(lock)
    }
    fn writer(&self) -> Result<Writer> {
        let dir = self.root.join(".orr/packages");
        safe_mkdir(&dir)?;
        let path = dir.join("writer.lock");
        // A stale guard after a crash is deliberately fail-closed. Never steal locks.
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| {
                Error(format!(
                    "package writer unavailable (stale guard requires manual recovery): {e}"
                ))
            })?;
        let guard = Writer(path);
        writeln!(f, "{}", std::process::id())?;
        f.sync_all()?;
        Ok(guard)
    }
    fn store(&self, p: &LockedPackage, files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
        let objects = self.root.join(".orr/packages/objects");
        safe_mkdir(&objects)?;
        let destination = self.object(p);
        if fs::symlink_metadata(&destination).is_ok() {
            return self.verify_object(p);
        }
        let stage = tempfile::Builder::new()
            .prefix("stage-")
            .tempdir_in(&objects)?;
        write_sync(
            &stage.path().join(MANIFEST),
            &serde_json::to_vec_pretty(&p.manifest)?,
        )?;
        for (path, bytes) in files {
            let target = stage.path().join(path);
            safe_mkdir(target.parent().expect("asset parent"))?;
            write_sync(&target, bytes)?;
        }
        #[cfg(test)]
        injected_failure("object-stage")?;
        // Publish an immutable object first; interrupted installs may leave unreachable
        // objects, but cannot expose a partial object through the active lock.
        fs::rename(stage.path(), &destination)?;
        Ok(())
    }
    fn publish(&self, lock: &Lock) -> Result<()> {
        self.validate_lock(lock)?;
        let path = self.root.join(LOCK);
        if fs::symlink_metadata(&path).is_ok() {
            no_links(&path)?;
        }
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        let encoded = serde_json::to_vec_pretty(lock)?;
        if encoded.len() as u64 >= MAX_JSON {
            return fail("lock byte limit exceeded");
        }
        file.write_all(&encoded)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        #[cfg(test)]
        injected_failure("lock-publish")?;
        file.persist(&path)
            .map_err(|e| Error(format!("atomic lock publish failed: {}", e.error)))?;
        // Process-interruption atomicity is guaranteed by rename. Power-loss durability
        // of directory metadata is filesystem-specific and is not claimed by v1.
        Ok(())
    }
}
struct Writer(PathBuf);
impl Drop for Writer {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn inspect(source: impl AsRef<Path>, runtime: &Runtime) -> Result<LockedPackage> {
    snapshot(source.as_ref(), runtime, true).map(|(p, _)| p)
}
/// Inspect bytes/engine compatibility without claiming host capability support.
pub fn inspect_for_install(
    source: impl AsRef<Path>,
    engine_version: Version,
) -> Result<LockedPackage> {
    snapshot(
        source.as_ref(),
        &Runtime {
            engine_version,
            capabilities: BTreeSet::new(),
        },
        false,
    )
    .map(|(p, _)| p)
}
fn snapshot(
    source: &Path,
    runtime: &Runtime,
    enforce_capabilities: bool,
) -> Result<(LockedPackage, BTreeMap<String, Vec<u8>>)> {
    no_links(source)?;
    let manifest: Manifest = json(&source.join(MANIFEST))?;
    validate_manifest(&manifest, runtime, enforce_capabilities)?;
    let mut files = BTreeMap::new();
    let mut hashes = BTreeMap::new();
    let mut total = 0;
    for path in &manifest.files {
        let bytes = read_bounded(&source.join(path), MAX_FILE)?;
        total += bytes.len() as u64;
        if total > MAX_TOTAL {
            return fail("package byte limit exceeded");
        }
        hashes.insert(path.clone(), hash(&bytes));
        files.insert(path.clone(), bytes);
    }
    let mut package = LockedPackage {
        manifest,
        digest: String::new(),
        files: hashes,
    };
    package.digest = identity(&package)?;
    Ok((package, files))
}
fn identity(p: &LockedPackage) -> Result<String> {
    // Struct field order and BTree collections give one canonical JSON encoding.
    let bytes = serde_json::to_vec(&("orr-package-v1", &p.manifest, &p.files))?;
    Ok(hash(&bytes))
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn hex_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn compatible(engine: &str, runtime: &Runtime) -> Result<()> {
    let req =
        VersionReq::parse(engine).map_err(|e| Error(format!("invalid engine requirement: {e}")))?;
    if !req.matches(&runtime.engine_version) {
        return fail("incompatible engine version");
    }
    Ok(())
}
fn name(s: &str) -> Result<()> {
    if s.is_empty()
        || s.len() > 64
        || !s
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
    {
        return fail("invalid package/capability name");
    }
    Ok(())
}
fn exact(v: &str) -> Result<()> {
    let parsed =
        Version::parse(v).map_err(|e| Error(format!("exact semantic version required: {e}")))?;
    if parsed.to_string() != v {
        return fail("noncanonical version");
    }
    Ok(())
}
fn validate_manifest(m: &Manifest, runtime: &Runtime, enforce_capabilities: bool) -> Result<()> {
    if m.schema != 1
        || m.files.len() > MAX_FILES
        || m.dependencies.len() > MAX_PACKAGES
        || m.capabilities.len() > 128
    {
        return fail("unsupported manifest schema or bounds");
    }
    name(&m.name)?;
    exact(&m.version)?;
    compatible(&m.engine, runtime)?;
    for cap in &m.capabilities {
        name(cap)?;
        if enforce_capabilities && !runtime.capabilities.contains(cap) {
            return fail(format!("missing compiled capability: {cap}"));
        }
    }
    for (dep, version) in &m.dependencies {
        name(dep)?;
        exact(version)?;
    }
    let mut folded = BTreeSet::new();
    for path in &m.files {
        portable(path)?;
        if path
            .split('/')
            .next()
            .is_some_and(|part| part.eq_ignore_ascii_case(MANIFEST))
            || !folded.insert(path.to_ascii_lowercase())
        {
            return fail("reserved or case-colliding asset path");
        }
    }
    for path in &folded {
        for (index, _) in path.match_indices('/') {
            if folded.contains(&path[..index]) {
                return fail("file/directory path collision");
            }
        }
    }
    Ok(())
}
fn portable(path: &str) -> Result<()> {
    if path.is_empty() || path.len() > 240 || !path.is_ascii() || path.split('/').count() > 16 {
        return fail("invalid portable asset path");
    }
    for part in path.split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with('.')
            || part.len() > 100
            || !part
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return fail(format!("nonportable asset path: {path}"));
        }
    }
    Ok(())
}
fn resolve(
    direct: &BTreeMap<String, String>,
    available: &BTreeMap<String, LockedPackage>,
) -> Result<BTreeMap<String, LockedPackage>> {
    fn visit(
        name: &str,
        version: &str,
        available: &BTreeMap<String, LockedPackage>,
        active: &mut BTreeSet<String>,
        output: &mut BTreeMap<String, LockedPackage>,
    ) -> Result<()> {
        let p = available
            .get(name)
            .ok_or_else(|| Error(format!("missing dependency/package: {name}@{version}")))?;
        if p.manifest.version != version {
            return fail(format!(
                "version conflict: {name} requires {version}, available {}",
                p.manifest.version
            ));
        }
        if active.contains(name) {
            return fail(format!("dependency cycle: {name}"));
        }
        if output.contains_key(name) {
            return Ok(());
        }
        if active.len() + output.len() >= MAX_PACKAGES {
            return fail("package graph limit exceeded");
        }
        active.insert(name.to_string());
        for (dep, version) in &p.manifest.dependencies {
            visit(dep, version, available, active, output)?;
        }
        active.remove(name);
        output.insert(name.to_string(), p.clone());
        Ok(())
    }
    let mut output = BTreeMap::new();
    let mut active = BTreeSet::new();
    for (name, version) in direct {
        visit(name, version, available, &mut active, &mut output)?;
    }
    Ok(output)
}
fn no_links(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    if !absolute.is_absolute() {
        return fail("absolute path required after resolving current directory");
    }
    let mut current = PathBuf::new();
    let mut components = absolute.components().peekable();
    while let Some(component) = components.next() {
        if component == std::path::Component::ParentDir {
            return fail("parent traversal rejected");
        }
        current.push(component);
        // A Windows prefix is not a directory: canonicalize produces verbatim
        // paths whose bare prefix (e.g. \\?\C:) names a device, not its root.
        // Wait for RootDir, then check the root and every actual ancestor.
        // A verbatim UNC share can itself be the root with no RootDir component;
        // in that case the prefix must be checked rather than skipped.
        if matches!(component, std::path::Component::Prefix(_))
            && components.peek() == Some(&std::path::Component::RootDir)
        {
            continue;
        }
        let m = fs::symlink_metadata(&current)?;
        if m.file_type().is_symlink() {
            return fail(format!("symlink rejected: {}", current.display()));
        }
    }
    Ok(())
}
fn safe_mkdir(path: &Path) -> Result<()> {
    if path.exists() {
        no_links(path)?;
        if !path.is_dir() {
            return fail("directory expected");
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        safe_mkdir(parent)?;
    }
    fs::create_dir(path)?;
    no_links(path)
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    no_links(path)?;
    // Reject FIFOs/devices before opening: opening a FIFO for reading can block
    // indefinitely even when we would reject its descriptor metadata afterward.
    // Concurrent hostile path replacement remains outside the documented boundary.
    let before_open = fs::symlink_metadata(path)?;
    if !before_open.is_file() || before_open.len() > limit {
        return fail("regular file required within size bound");
    }
    let mut file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return fail("regular file required within size bound");
    }
    let mut bytes = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return fail("file grew beyond size limit");
    }
    Ok(bytes)
}
fn json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&read_bounded(path, MAX_JSON)?)?)
}
fn write_sync(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(test)]
thread_local! {
    static FAILURE_POINT: std::cell::Cell<&'static str> = const { std::cell::Cell::new("") };
}
#[cfg(test)]
fn injected_failure(point: &str) -> Result<()> {
    FAILURE_POINT.with(|current| {
        if current.get() == point {
            current.set("");
            fail(format!("injected failure: {point}"))
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir() -> tempfile::TempDir {
        // Resolve only the trusted system-temp root before creating any fixture
        // content. macOS temp paths may traverse /var -> /private/var, while
        // production deliberately rejects symlinks in every input ancestor.
        let root = std::env::temp_dir();
        #[cfg(unix)]
        let root = fs::canonicalize(root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }

    #[test]
    fn project_schemas_preserve_metadata_and_only_describe_entry_content() {
        let tmp = fixture_dir();
        let path = tmp.path().join("orr.project.json");
        let legacy = br#"{"schema":1,"engine":"^0.0.1"}"#;
        fs::write(&path, legacy).unwrap();
        let project = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        assert_eq!(project.manifest().unwrap().schema, 1);
        assert!(project.manifest().unwrap().entry.is_none());
        assert_eq!(fs::read(&path).unwrap(), legacy);
        assert_eq!(project.list().unwrap(), Lock::default());
        assert!(!tmp.path().join(LOCK).exists());

        let saved = br#"{"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"scenes/arena.scene.yaml","sprites":"arena.sprites.json"}}"#;
        fs::write(&path, saved).unwrap();
        let project = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        let entry = project.manifest().unwrap().entry.as_ref().unwrap();
        assert_eq!(entry.game, ProjectGame::Arena);
        assert_eq!(entry.scene, "scenes/arena.scene.yaml");
        assert_eq!(entry.sprites.as_deref(), Some("arena.sprites.json"));
        assert_eq!(fs::read(&path).unwrap(), saved);
        assert!(!tmp.path().join(LOCK).exists());
        // Package metadata tools do not need the optional presentation feature
        // or existing scene files merely to install/remove content.
        let src_root = fixture_dir();
        let src = source(src_root.path(), "art", &[]);
        let installer =
            Project::open_for_install(tmp.path(), Runtime::content_only().engine_version).unwrap();
        let installed = installer.install(&[src]).unwrap();
        assert_eq!(
            installed.direct.get("art").map(String::as_str),
            Some("1.0.0")
        );
        installer.verify().unwrap();
        installer.remove("art").unwrap();
        assert_eq!(fs::read(&path).unwrap(), saved);
    }

    #[test]
    fn project_entry_contract_rejects_ambiguous_unsupported_and_unsafe_metadata() {
        let tmp = fixture_dir();
        let path = tmp.path().join("orr.project.json");
        let invalid = [
            r#"{"schema":0,"engine":"^0.0.1"}"#,
            r#"{"schema":3,"engine":"^0.0.1"}"#,
            r#"{"schema":2,"engine":"^0.0.1"}"#,
            r#"{"schema":1,"engine":"^0.0.1","entry":null}"#,
            r#"{"schema":1,"engine":"^0.0.1","entry":{"game":"arena","scene":"a.yaml"}}"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"physics","scene":"a.yaml"}}"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"Arena","scene":"a.yaml"}}"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"scene":"a.yaml"}}"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"../a.yaml"}}"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"/a.yaml"}}"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"a.yaml","sprites":"../b.json"}}"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"a.yaml","sprites":"A.yaml"}}"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"a.yaml","sprites":null}}"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"a.yaml","unknown":0}}"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"a.yaml"},"packages":{}}"#,
            r#"{"schema":2,"schema":1,"engine":"^0.0.1","entry":{"game":"arena","scene":"a.yaml"}}"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"a.yaml","scene":"b.yaml"}}"#,
            r#"{"schema":2,"engine":">=99.0.0","entry":{"game":"arena","scene":"a.yaml"}}"#,
        ];
        for text in invalid {
            fs::write(&path, text).unwrap();
            assert!(
                Project::open(tmp.path(), Runtime::content_only()).is_err(),
                "accepted {text}"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
            assert!(!tmp.path().join(LOCK).exists());
        }
        fs::write(
            &path,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"a.yaml"}}"#,
        )
        .unwrap();
        assert!(Project::open(tmp.path(), Runtime::content_only())
            .unwrap()
            .manifest()
            .unwrap()
            .entry
            .as_ref()
            .unwrap()
            .sprites
            .is_none());
    }

    #[test]
    fn project_ui_is_strict_optional_metadata_without_activation_authority() {
        let tmp = fixture_dir();
        let path = tmp.path().join("orr.project.json");
        let base = r#"{"schema":2,"engine":"*","entry":{"game":"arena","scene":"arena.yaml"}}"#;
        let without: ProjectManifest = serde_json::from_str(base).unwrap();
        assert!(without.entry.as_ref().unwrap().ui.is_none());
        assert_eq!(serde_json::to_string(&without).unwrap(), base);
        let valid_ui = r#"{"profile":"arena-korean-v1","font":{"package":"korean-game-ui","asset":"OrreryKoreanUI.otf"}}"#;
        let with = |ui: &str| {
            format!("{{\"schema\":2,\"engine\":\"*\",\"entry\":{{\"game\":\"arena\",\"scene\":\"arena.yaml\",\"ui\":{ui}}}}}")
        };
        fs::write(&path, with(valid_ui)).unwrap();
        let project = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        let ui = project
            .manifest()
            .unwrap()
            .entry
            .as_ref()
            .unwrap()
            .ui
            .as_ref()
            .unwrap();
        assert_eq!(ui.profile, ProjectUiProfile::ArenaKoreanV1);
        assert_eq!(ui.font.package, "korean-game-ui");
        assert!(!tmp.path().join(LOCK).exists());
        let invalid = [
            "null".to_owned(),
            "{}".to_owned(),
            valid_ui.replace("arena-korean-v1", "arena-korean-v2"),
            valid_ui.replace("korean-game-ui", "../font"),
            valid_ui.replace("OrreryKoreanUI.otf", "../font.otf"),
            valid_ui.replace("OrreryKoreanUI.otf", "/font.otf"),
            valid_ui.replace("OrreryKoreanUI.otf", ""),
            valid_ui.replace("\"font\":", "\"digest\":\"bad\",\"font\":"),
            valid_ui.replace("\"package\":", "\"version\":\"1.0.0\",\"package\":"),
            valid_ui.replace(
                "\"profile\":",
                "\"profile\":\"arena-korean-v1\",\"profile\":",
            ),
            valid_ui.replace("\"asset\":", "\"asset\":\"font.otf\",\"asset\":"),
            valid_ui.replace(
                "{\"package\":\"korean-game-ui\",\"asset\":\"OrreryKoreanUI.otf\"}",
                "null",
            ),
        ];
        for ui in invalid {
            fs::write(&path, with(&ui)).unwrap();
            assert!(
                Project::open(tmp.path(), Runtime::content_only()).is_err(),
                "accepted {ui}"
            );
            assert!(!tmp.path().join(LOCK).exists());
        }
        fs::write(
            &path,
            with(valid_ui).replace("\"ui\":", "\"ui\":null,\"ui\":"),
        )
        .unwrap();
        assert!(Project::open(tmp.path(), Runtime::content_only()).is_err());
    }

    #[test]
    fn canonical_paths_check_root_and_each_existing_component() {
        let tmp = fixture_dir();
        let root = fs::canonicalize(tmp.path()).unwrap();
        // On Windows this is the verbatim path returned by Project::open.
        no_links(&root).unwrap();
        let volume_root = root.ancestors().last().unwrap();
        no_links(volume_root).unwrap();
        let nested = root.join("nested");
        safe_mkdir(&nested).unwrap();
        write_sync(&nested.join("asset.txt"), b"checked").unwrap();
        no_links(&nested.join("asset.txt")).unwrap();
        assert!(no_links(&nested.join("missing")).is_err());
        // Appending text preserves ParentDir even for verbatim Windows paths;
        // PathBuf::push would normalize it away before no_links sees it.
        let mut traversal = nested.as_os_str().to_os_string();
        traversal.push(format!("{}..", std::path::MAIN_SEPARATOR));
        assert!(no_links(Path::new(&traversal))
            .unwrap_err()
            .to_string()
            .contains("parent traversal"));
    }

    #[test]
    fn sha256_uses_canonical_lowercase_hex() {
        assert_eq!(
            hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
    fn source(root: &Path, name: &str, dependencies: &[(&str, &str)]) -> PathBuf {
        let path = root.join(name);
        fs::create_dir(&path).unwrap();
        let manifest = Manifest {
            schema: 1,
            name: name.into(),
            version: "1.0.0".into(),
            engine: "^0.0.1".into(),
            capabilities: BTreeSet::new(),
            dependencies: dependencies
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            files: ["asset.txt".into()].into(),
        };
        fs::write(path.join(MANIFEST), serde_json::to_vec(&manifest).unwrap()).unwrap();
        fs::write(path.join("asset.txt"), b"original").unwrap();
        path
    }
    fn edit(path: &Path, f: impl FnOnce(&mut Manifest)) {
        let mut m: Manifest = json(&path.join(MANIFEST)).unwrap();
        f(&mut m);
        fs::write(path.join(MANIFEST), serde_json::to_vec(&m).unwrap()).unwrap();
    }
    #[test]
    fn install_load_remove_reinstall_identity_and_source_isolation() {
        let tmp = fixture_dir();
        let src = source(tmp.path(), "art", &[]);
        let project_root = tmp.path().join("game");
        fs::create_dir(&project_root).unwrap();
        let p = Project::open(&project_root, Runtime::content_only()).unwrap();
        let first = p.install(std::slice::from_ref(&src)).unwrap();
        let lock_bytes = fs::read(project_root.join(LOCK)).unwrap();
        p.install(std::slice::from_ref(&src)).unwrap();
        assert_eq!(lock_bytes, fs::read(project_root.join(LOCK)).unwrap());
        fs::write(src.join("asset.txt"), b"changed").unwrap();
        assert_eq!(p.read_asset("art", "asset.txt").unwrap(), b"original");
        assert!(p.install(std::slice::from_ref(&src)).is_err());
        assert_eq!(lock_bytes, fs::read(project_root.join(LOCK)).unwrap());
        p.remove("art").unwrap();
        assert!(p
            .read_asset("art", "asset.txt")
            .unwrap_err()
            .to_string()
            .contains("missing package"));
        fs::write(src.join("asset.txt"), b"original").unwrap();
        assert_eq!(first, p.install(&[src]).unwrap());
        p.verify().unwrap();
    }
    #[test]
    fn dependency_conflict_cycle_and_failure_preserve_lock() {
        let tmp = fixture_dir();
        let a = source(tmp.path(), "a", &[("b", "1.0.0")]);
        let b = source(tmp.path(), "b", &[]);
        let p = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        assert!(p.install(std::slice::from_ref(&a)).is_err());
        assert!(!tmp.path().join(LOCK).exists());
        let good = p.install(&[a.clone(), b.clone()]).unwrap();
        let b_only = p.remove("a").unwrap();
        assert_eq!(b_only.packages.len(), 1);
        p.install(std::slice::from_ref(&a)).unwrap();
        let old = fs::read(tmp.path().join(LOCK)).unwrap();
        edit(&a, |m| {
            m.version = "2.0.0".into();
            m.dependencies.insert("b".into(), "2.0.0".into());
        });
        assert!(p.install(std::slice::from_ref(&a)).is_err());
        assert_eq!(old, fs::read(tmp.path().join(LOCK)).unwrap());
        edit(&a, |m| {
            m.dependencies.insert("b".into(), "1.0.0".into());
        });
        edit(&b, |m| {
            m.version = "2.0.0".into();
            m.dependencies.insert("a".into(), "2.0.0".into());
        });
        edit(&a, |m| {
            m.dependencies.insert("b".into(), "2.0.0".into());
        });
        assert!(p
            .install(&[a, b])
            .unwrap_err()
            .to_string()
            .contains("cycle"));
        assert_eq!(good, p.verify().unwrap());
    }
    #[test]
    fn candidate_dependencies_are_transitive_and_pruned() {
        let tmp = fixture_dir();
        let a = source(tmp.path(), "a", &[("b", "1.0.0")]);
        let b = source(tmp.path(), "b", &[]);
        let p = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        let lock = p.install_with_dependencies(&[a], &[b]).unwrap();
        assert_eq!(lock.direct.len(), 1);
        assert!(lock.direct.contains_key("a"));
        assert_eq!(lock.packages.len(), 2);
        assert!(p.remove("b").is_err());
        let object = p.object(&lock.packages["b"]);
        assert!(p.remove("a").unwrap().packages.is_empty());
        assert!(object.exists());
    }
    #[test]
    fn host_capability_inventory_is_required_for_asset_reads() {
        let tmp = fixture_dir();
        let src = source(tmp.path(), "art", &[]);
        edit(&src, |m| {
            m.capabilities.insert("sprite".into());
        });
        let tool =
            Project::open_for_install(tmp.path(), Runtime::content_only().engine_version).unwrap();
        tool.install(&[src]).unwrap();
        assert!(tool.read_asset("art", "asset.txt").is_err());
        let host = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        assert!(host
            .read_asset("art", "asset.txt")
            .unwrap_err()
            .to_string()
            .contains("compiled capability"));
        let mut runtime = Runtime::content_only();
        runtime.capabilities.insert("sprite".into());
        assert_eq!(
            Project::open(tmp.path(), runtime)
                .unwrap()
                .read_asset("art", "asset.txt")
                .unwrap(),
            b"original"
        );
    }
    #[test]
    fn tamper_and_writer_failure_preserve_authority() {
        let tmp = fixture_dir();
        let src = source(tmp.path(), "art", &[]);
        let p = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        let lock = p.install(std::slice::from_ref(&src)).unwrap();
        let old = fs::read(tmp.path().join(LOCK)).unwrap();
        fs::write(tmp.path().join(".orr/packages/writer.lock"), b"stale").unwrap();
        assert!(p.remove("art").is_err());
        fs::remove_file(tmp.path().join(".orr/packages/writer.lock")).unwrap();
        fs::write(
            p.object(&lock.packages["art"]).join("asset.txt"),
            b"tampered",
        )
        .unwrap();
        assert!(p.verify().is_err());
        assert!(p.read_asset("art", "asset.txt").is_err());
        assert!(p.install(&[src]).is_err());
        assert_eq!(old, fs::read(tmp.path().join(LOCK)).unwrap());
    }
    #[test]
    fn reject_nonportable_colliding_and_hook_manifests() {
        let tmp = fixture_dir();
        let src = source(tmp.path(), "art", &[]);
        for path in [
            "../escape",
            "/absolute",
            "a\\b",
            "C:drive",
            "CON.png",
            "a/../b",
            "a//b",
            "a.",
            "a/./b",
        ] {
            assert!(portable(path).is_err(), "{path}");
        }
        edit(&src, |m| {
            m.files.insert("ASSET.txt".into());
        });
        assert!(inspect(&src, &Runtime::content_only()).is_err());
        edit(&src, |m| {
            m.files = ["a".into(), "a/b".into()].into();
        });
        assert!(inspect(&src, &Runtime::content_only()).is_err());
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(src.join(MANIFEST)).unwrap()).unwrap();
        value["hooks"] = serde_json::json!({"install":"echo unsafe"});
        fs::write(src.join(MANIFEST), serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(inspect(&src, &Runtime::content_only()).is_err());
    }
    #[test]
    fn digest_ignores_json_layout_and_source_location() {
        let tmp = fixture_dir();
        let a = source(tmp.path(), "art", &[]);
        let first = inspect(&a, &Runtime::content_only()).unwrap();
        let m: Manifest = json(&a.join(MANIFEST)).unwrap();
        fs::write(a.join(MANIFEST), serde_json::to_vec_pretty(&m).unwrap()).unwrap();
        assert_eq!(first, inspect(&a, &Runtime::content_only()).unwrap());
        let moved = tmp.path().join("moved");
        fs::rename(&a, &moved).unwrap();
        assert_eq!(first, inspect(moved, &Runtime::content_only()).unwrap());
    }
    #[test]
    fn exact_versions_and_engine_rejected_before_mutation() {
        let tmp = fixture_dir();
        let src = source(tmp.path(), "art", &[]);
        let p = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        edit(&src, |m| {
            m.dependencies.insert("other".into(), "^1.0".into());
        });
        assert!(p.install(std::slice::from_ref(&src)).is_err());
        edit(&src, |m| {
            m.dependencies.clear();
            m.engine = ">=99".into();
        });
        assert!(p.install(&[src]).is_err());
        assert!(!tmp.path().join(LOCK).exists());
    }
    #[test]
    fn staged_object_and_lock_publish_failures_preserve_active_state() {
        let tmp = fixture_dir();
        let src = source(tmp.path(), "art", &[]);
        let p = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        let old = p.install(std::slice::from_ref(&src)).unwrap();
        let original = fs::read(tmp.path().join(LOCK)).unwrap();
        edit(&src, |m| m.version = "2.0.0".into());
        for point in ["object-stage", "lock-publish"] {
            FAILURE_POINT.with(|current| current.set(point));
            assert!(p
                .install(std::slice::from_ref(&src))
                .unwrap_err()
                .to_string()
                .contains("injected failure"));
            assert_eq!(original, fs::read(tmp.path().join(LOCK)).unwrap());
            assert_eq!(old, p.verify().unwrap());
            assert!(!tmp.path().join(".orr/packages/writer.lock").exists());
        }
        p.install(&[src]).unwrap();
    }
    #[test]
    fn checked_install_requires_both_exact_locks_and_keeps_callback_failures_atomic() {
        let tmp = fixture_dir();
        let src = source(tmp.path(), "art", &[]);
        let p = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        let old = p.install(std::slice::from_ref(&src)).unwrap();
        let bytes = fs::read(tmp.path().join(LOCK)).unwrap();
        edit(&src, |m| m.version = "2.0.0".into());
        let candidate = inspect(&src, &Runtime::content_only()).unwrap();
        let mut approved = old.clone();
        approved.direct.insert("art".into(), "2.0.0".into());
        approved.packages.insert("art".into(), candidate);
        assert!(p.install_checked(std::slice::from_ref(&src), &Lock::default(), &approved, || panic!("stale callback")).is_err());
        assert!(p.install_checked(std::slice::from_ref(&src), &old, &old, || panic!("unapproved callback")).is_err());
        let failure = p.install_checked(std::slice::from_ref(&src), &old, &approved, || {
            assert_eq!(p.verify().unwrap(), old);
            assert!(p.object(&approved.packages["art"]).exists());
            assert!(p.writer().is_err());
            Err("document changed".into())
        }).unwrap_err();
        assert!(failure.to_string().contains("document changed"));
        assert_eq!(fs::read(tmp.path().join(LOCK)).unwrap(), bytes);
        assert_eq!(p.verify().unwrap(), old);
        assert_eq!(p.install_checked(&[src], &old, &approved, || Ok(())).unwrap(), approved);
    }
    #[test]
    fn checked_install_injected_failures_keep_old_lock() {
        for point in ["object-stage", "lock-publish"] {
            let tmp = fixture_dir();
            let src = source(tmp.path(), "art", &[]);
            let p = Project::open(tmp.path(), Runtime::content_only()).unwrap();
            let old = p.install(std::slice::from_ref(&src)).unwrap();
            let bytes = fs::read(tmp.path().join(LOCK)).unwrap();
            edit(&src, |m| m.version = "2.0.0".into());
            let mut approved = old.clone();
            approved.direct.insert("art".into(), "2.0.0".into());
            approved.packages.insert("art".into(), inspect(&src, &Runtime::content_only()).unwrap());
            FAILURE_POINT.with(|current| current.set(point));
            assert!(p.install_checked(&[src], &old, &approved, || Ok(())).unwrap_err().to_string().contains("injected failure"));
            assert_eq!(fs::read(tmp.path().join(LOCK)).unwrap(), bytes);
            assert_eq!(p.verify().unwrap(), old);
        }
    }
    #[test]
    fn competing_writer_cannot_modify_authority() {
        let tmp = fixture_dir();
        let src = source(tmp.path(), "art", &[]);
        let p = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        p.install(std::slice::from_ref(&src)).unwrap();
        let old = fs::read(tmp.path().join(LOCK)).unwrap();
        let guard = p.writer().unwrap();
        let competitor = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        assert!(competitor.install(&[src]).is_err());
        assert!(competitor.remove("art").is_err());
        assert_eq!(old, fs::read(tmp.path().join(LOCK)).unwrap());
        drop(guard);
        competitor.remove("art").unwrap();
    }
    #[test]
    fn forged_lock_digest_file_map_and_unreachable_package_are_rejected() {
        let tmp = fixture_dir();
        let src = source(tmp.path(), "art", &[]);
        let extra = source(tmp.path(), "unused", &[]);
        let p = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        let old = p.install(&[src]).unwrap();
        let mut changed = old.clone();
        changed.packages.get_mut("art").unwrap().digest = "0".repeat(64);
        assert!(p.validate_lock(&changed).is_err());
        let mut changed = old.clone();
        changed.packages.get_mut("art").unwrap().files.clear();
        assert!(p.validate_lock(&changed).is_err());
        let mut changed = old.clone();
        changed.packages.insert(
            "unused".into(),
            inspect(extra, &Runtime::content_only()).unwrap(),
        );
        assert!(p
            .validate_lock(&changed)
            .unwrap_err()
            .to_string()
            .contains("unreachable"));
        assert_eq!(old, p.verify().unwrap());
    }
    #[test]
    fn bounds_reject_before_large_reads_and_graph_walks() {
        let tmp = fixture_dir();
        let src = source(tmp.path(), "art", &[]);
        let runtime = Runtime::content_only();
        let mut m: Manifest = json(&src.join(MANIFEST)).unwrap();
        m.files = (0..=MAX_FILES).map(|i| format!("f{i}")).collect();
        assert!(validate_manifest(&m, &runtime, true).is_err());
        assert!(portable(&"a".repeat(101)).is_err());
        assert!(portable(&["abcdefghij"; 17].join("/")).is_err());
        assert!(portable(&["a".repeat(100), "b".repeat(100), "c".repeat(39)].join("/")).is_err());
        assert!(portable(&["a".repeat(100), "b".repeat(100), "c".repeat(38)].join("/")).is_ok());
        fs::OpenOptions::new()
            .write(true)
            .open(src.join("asset.txt"))
            .unwrap()
            .set_len(MAX_FILE + 1)
            .unwrap();
        assert!(inspect(&src, &runtime).is_err());
        fs::OpenOptions::new()
            .write(true)
            .open(src.join(MANIFEST))
            .unwrap()
            .set_len(MAX_JSON + 1)
            .unwrap();
        assert!(inspect(&src, &runtime).is_err());
        let mut packages = BTreeMap::new();
        for i in 0..=MAX_PACKAGES {
            let name = format!("p{i}");
            let manifest = Manifest {
                schema: 1,
                name: name.clone(),
                version: "1.0.0".into(),
                engine: "*".into(),
                capabilities: BTreeSet::new(),
                files: BTreeSet::new(),
                dependencies: if i < MAX_PACKAGES {
                    [(format!("p{}", i + 1), "1.0.0".into())].into()
                } else {
                    BTreeMap::new()
                },
            };
            packages.insert(
                name,
                LockedPackage {
                    manifest,
                    digest: String::new(),
                    files: BTreeMap::new(),
                },
            );
        }
        assert!(resolve(&[("p0".into(), "1.0.0".into())].into(), &packages).is_err());
        assert_eq!(
            resolve(&[("p1".into(), "1.0.0".into())].into(), &packages)
                .unwrap()
                .len(),
            MAX_PACKAGES
        );
    }
    #[cfg(unix)]
    #[test]
    fn ancestor_links_and_reserved_manifest_subtrees_are_rejected() {
        use std::os::unix::fs::symlink;
        let tmp = fixture_dir();
        let src = source(tmp.path(), "art", &[]);
        let alias = tmp.path().join("alias");
        symlink(&src, &alias).unwrap();
        assert!(inspect(alias, &Runtime::content_only()).is_err());
        edit(&src, |m| {
            m.files = ["orr.package.json/asset".into()].into();
        });
        assert!(inspect(&src, &Runtime::content_only())
            .unwrap_err()
            .to_string()
            .contains("reserved"));
        let project = tmp.path().join("game");
        fs::create_dir(&project).unwrap();
        symlink(tmp.path(), project.join(".orr")).unwrap();
        let p = Project::open(project, Runtime::content_only()).unwrap();
        assert!(p
            .install(&[src])
            .unwrap_err()
            .to_string()
            .contains("symlink"));
    }
    #[cfg(unix)]
    #[test]
    #[expect(
        clippy::disallowed_types,
        reason = "wall-clock timeout only for isolated CLI/file-system regression test"
    )]
    fn fifo_inputs_fail_without_blocking() {
        use std::{
            process::Command,
            thread,
            time::{Duration, Instant},
        };
        const CHILD_ROOT: &str = "ORR_PACKAGE_FIFO_TEST_ROOT";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let root = PathBuf::from(root);
            let p = Project::open(&root, Runtime::content_only()).unwrap();
            for name in ["fifo-asset", "fifo-manifest"] {
                let err = p.install(&[root.join(name)]).unwrap_err();
                assert!(err.to_string().contains("regular file"), "{err}");
                assert!(!root.join(".orr/packages/writer.lock").exists());
                assert!(!root.join(LOCK).exists());
            }
            return;
        }
        let tmp = fixture_dir();
        for (name, file) in [("fifo-asset", "asset.txt"), ("fifo-manifest", MANIFEST)] {
            let path = source(tmp.path(), name, &[]).join(file);
            fs::remove_file(&path).unwrap();
            assert!(Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success());
        }
        // A regression must fail rather than hang the entire test process. Only
        // the child touches FIFO reads, and the parent kills/reaps it on timeout.
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::fifo_inputs_fail_without_blocking",
                "--nocapture",
            ])
            .env(CHILD_ROOT, tmp.path())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("FIFO input blocked package validation");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_at_source_object_and_lock_are_rejected() {
        use std::os::unix::fs::symlink;
        let tmp = fixture_dir();
        let src = source(tmp.path(), "art", &[]);
        let p = Project::open(tmp.path(), Runtime::content_only()).unwrap();
        let lock = p.install(std::slice::from_ref(&src)).unwrap();
        fs::remove_file(src.join("asset.txt")).unwrap();
        symlink("../orr.packages.lock.json", src.join("asset.txt")).unwrap();
        assert!(inspect(&src, &Runtime::content_only()).is_err());
        let asset = p.object(&lock.packages["art"]).join("asset.txt");
        fs::remove_file(&asset).unwrap();
        symlink(tmp.path().join(LOCK), asset).unwrap();
        assert!(p.read_asset("art", "asset.txt").is_err());
        let locked = tmp.path().join(LOCK);
        let saved = tmp.path().join("saved.json");
        fs::rename(&locked, &saved).unwrap();
        symlink(saved, locked).unwrap();
        assert!(p.list().is_err());
    }
    #[test]
    fn schema_three_requires_valid_explicit_collect_progress() {
        let valid = serde_json::json!({"schema":3,"engine":"*","entry":{"game":"collect-dodge-v1","scene":"level.yaml"},"progress":{"schema":1,"game_id":"12345678-1234-4234-8234-123456789abc","profile":"collect-dodge-highscore-v1"}});
        let runtime = Runtime::content_only();
        let manifest: ProjectManifest = serde_json::from_value(valid.clone()).unwrap();
        manifest.validate(&runtime).unwrap();
        for schema in [1, 2, 4] {
            let mut bad = valid.clone();
            bad["schema"] = schema.into();
            assert!(serde_json::from_value::<ProjectManifest>(bad)
                .unwrap()
                .validate(&runtime)
                .is_err());
        }
        let mut bad = valid.clone();
        bad.as_object_mut().unwrap().remove("progress");
        assert!(serde_json::from_value::<ProjectManifest>(bad)
            .unwrap()
            .validate(&runtime)
            .is_err());
        let mut bad = valid.clone();
        bad["entry"]["game"] = "arena".into();
        assert!(serde_json::from_value::<ProjectManifest>(bad)
            .unwrap()
            .validate(&runtime)
            .is_err());
        let mut bad = valid;
        bad["progress"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<ProjectManifest>(bad).is_err());
    }
}
