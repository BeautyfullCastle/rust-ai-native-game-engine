use crate::authoring::{
    import_source, validate_payload, validate_source_path, AssetType, Entry, Index,
    MAX_INPUT_BYTES, MAX_SOURCE_BYTES,
};
use crate::{CookError, Result};
use orr_asset::{
    encode_manifest, manifest_encoded_len, AssetRef, Domain, Manifest, ManifestEntry,
    MotionProfileV1, MAX_MANIFEST_BYTES, MAX_PCM_FRAMES,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const COOKER_FORMAT_VERSION: u32 = 1;
pub const IMPORTER_VERSION: u32 = 1;
pub const SIM_MANIFEST: &str = "sim.manifest.bin";
pub const VIEW_MANIFEST: &str = "view.manifest.bin";
const MAX_PAYLOAD_BYTES: usize = 8 + MAX_PCM_FRAMES as usize * 2;
const CACHE_HEADER: usize = 88;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        result.push(DIGITS[usize::from(byte >> 4)] as char);
        result.push(DIGITS[usize::from(byte & 15)] as char);
    }
    result
}

/// Cache identity deliberately excludes GUID and path. V1 has no semantic
/// options or dependencies: their canonical options encoding is a zero count.
pub fn cache_key(asset_type: AssetType, source: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"orr.asset-cook.cache\0");
    hash.update(COOKER_FORMAT_VERSION.to_le_bytes());
    hash.update(IMPORTER_VERSION.to_le_bytes());
    hash.update(asset_type.type_id().to_le_bytes());
    hash.update(1u32.to_le_bytes()); // schema version
    hash.update(0u32.to_le_bytes()); // semantic option count
    hash.update(sha256(source));
    hash.finalize().into()
}

/// Read a regular file with a cap enforced both before allocation and while
/// reading. Checking actual bytes catches a file growing since its metadata.
pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(CookError::invalid(format!(
            "not a bounded regular file: {}",
            path.display()
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&mut file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(CookError::invalid("file byte budget exceeded"));
    }
    Ok(bytes)
}
fn contained(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_source_path(relative)?;
    let resolved = fs::canonicalize(root.join(relative))?;
    if !resolved.starts_with(root) {
        return Err(CookError::invalid(format!(
            "path escapes package root: {relative}"
        )));
    }
    Ok(resolved)
}
fn read_contained(root: &Path, relative: &str, limit: usize) -> Result<Vec<u8>> {
    read_bounded(&contained(root, relative)?, limit)
}

/// Index paths are relative to the directory containing the index, never CWD.
pub fn load_index(path: &Path) -> Result<Index> {
    let (_, bytes) = load_index_bytes(path)?;
    Index::parse(&bytes)
}
fn load_index_bytes(path: &Path) -> Result<(PathBuf, Vec<u8>)> {
    let parent = nonempty_parent(path);
    let root = fs::canonicalize(parent)?;
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| CookError::invalid("index filename must be UTF-8"))?;
    let bytes = read_contained(&root, filename, MAX_INPUT_BYTES)?;
    Ok((root, bytes))
}
// Canonicalize every existing prefix while retaining missing suffixes. Resolve
// symlinks before handling a following '..', matching filesystem path meaning.
// A dangling/unresolvable symlink is rejected, not treated as an absent folder.
fn prospective_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => (),
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(part) => {
                resolved.push(part);
                match fs::canonicalize(&resolved) {
                    Ok(path) => resolved = path,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        if fs::symlink_metadata(&resolved).is_ok() {
                            return Err(CookError::invalid("unresolvable path prefix"));
                        }
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
    }
    Ok(resolved)
}

// Be conservative on every host about case-insensitive Windows aliases in
// missing suffixes. Canonicalized existing prefixes alone cannot resolve them.
fn portable_contains(path: &Path, prefix: &Path) -> bool {
    let mut path = path.components();
    prefix.components().all(|expected| {
        let Some(actual) = path.next() else {
            return false;
        };
        let key = |part: Component<'_>| {
            part.as_os_str()
                .to_string_lossy()
                .trim_end_matches([' ', '.'])
                .to_lowercase()
        };
        key(actual) == key(expected)
    })
}

fn nonempty_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[derive(Clone, Debug)]
pub struct CookOptions {
    pub index: PathBuf,
    pub out: PathBuf,
    pub cache: Option<PathBuf>,
    /// Declared initial fixture references. Other documents are not scanned.
    pub roots: Vec<AssetRef>,
    /// Recompute and compare existing output without writing output or cache.
    pub check: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CookSummary {
    pub sim_manifest_sha256: [u8; 32],
    pub view_manifest_sha256: [u8; 32],
    pub asset_count: usize,
    pub cache_hits: usize,
}
#[derive(Serialize)]
struct Inspection<'a> {
    format: &'static str,
    sim_manifest_sha256: String,
    view_manifest_sha256: String,
    assets: &'a [InspectionEntry],
}
#[derive(Serialize)]
struct InspectionEntry {
    id: String,
    #[serde(rename = "type")]
    asset_type: &'static str,
    schema_version: u32,
    payload_len: u64,
    payload_sha256: String,
}
#[derive(Serialize)]
struct Provenance {
    format: &'static str,
    cooker_format_version: u32,
    importer_version: u32,
    sources: Vec<SourceRecord>,
}
#[derive(Serialize)]
struct SourceRecord {
    id: String,
    source: String,
    source_sha256: String,
    cache_key: String,
}
fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Cook completely before publication. Existing output is never intentionally
/// replaced; a per-output create-new lock serializes cooperating publishers.
/// Output parents must already exist. Crashes can leave an owned staging
/// directory or lock; those are visible and never automatically garbage-collected.
pub fn cook(options: &CookOptions) -> Result<CookSummary> {
    // Validate prospective locations BEFORE any cache directory/file creation.
    // Resolving existing prefixes also covers aliases to a not-yet-created out.
    let mut cache_path = None;
    if !options.check {
        if let Some(cache) = &options.cache {
            let output = prospective_path(&options.out)?;
            let cache = prospective_path(cache)?;
            if portable_contains(&cache, &output) || portable_contains(&output, &cache) {
                return Err(CookError::invalid(
                    "cache and output directories must not overlap",
                ));
            }
            cache_path = Some(cache);
        }
        if fs::symlink_metadata(&options.out).is_ok() {
            return Err(CookError::invalid(
                "output already exists; use a new directory or --check",
            ));
        }
    }
    let (root, index_bytes) = load_index_bytes(&options.index)?;
    let index = Index::parse(&index_bytes)?;
    index.validate_roots(&options.roots)?;
    let mut source_total = index_bytes.len();
    let cache = if options.check {
        None
    } else {
        cache_path
            .as_ref()
            .map(|path| {
                // Create the validated path, never the original spelling: a
                // missing prefix followed by `..` must not create output.
                fs::create_dir_all(path)?;
                fs::canonicalize(path).map_err(CookError::from)
            })
            .transpose()?
    };
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    let mut sim = Vec::new();
    let mut view = Vec::new();
    let mut motion = Vec::new();
    let mut inspected = Vec::new();
    let mut provenance = Provenance {
        format: "orr.asset-cook-report/1",
        cooker_format_version: COOKER_FORMAT_VERSION,
        importer_version: IMPORTER_VERSION,
        sources: Vec::new(),
    };
    let mut paths = BTreeSet::new();
    let mut cache_hits = 0;
    for entry in &index.entries {
        let Entry::Live {
            id,
            asset_type,
            source,
        } = entry
        else {
            continue;
        };
        let path = contained(&root, source)?;
        if !paths.insert(path.clone()) {
            return Err(CookError::invalid("duplicate resolved source path"));
        }
        let remaining = MAX_INPUT_BYTES
            .checked_sub(source_total)
            .ok_or_else(|| CookError::invalid("total input byte budget exceeded"))?;
        let source_bytes = read_bounded(&path, MAX_SOURCE_BYTES.min(remaining))?;
        source_total = source_total
            .checked_add(source_bytes.len())
            .ok_or_else(|| CookError::invalid("total input byte budget exceeded"))?;
        let key = cache_key(*asset_type, &source_bytes);
        let cached = cache
            .as_ref()
            .and_then(|dir| read_cache(dir, key, *asset_type).ok());
        let payload = if let Some(payload) = cached {
            cache_hits += 1;
            payload
        } else {
            let payload = import_source(*asset_type, &source_bytes)?;
            if let Some(dir) = &cache {
                write_cache(dir, key, *asset_type, &payload)?;
            }
            payload
        };
        validate_payload(*asset_type, &payload)?;
        let digest = sha256(&payload);
        let metadata = ManifestEntry {
            id: *id,
            type_id: asset_type.type_id(),
            schema_version: 1,
            payload_len: payload.len() as u64,
            payload_sha256: digest,
        };
        metadata.validate(asset_type.domain())?;
        match asset_type {
            AssetType::Motion => {
                sim.push(metadata);
                motion.push((
                    *id,
                    MotionProfileV1::decode(&payload)?.speed_per_tick().raw(),
                ));
            }
            AssetType::Impact => view.push(metadata),
        }
        // Validate record and cumulative payload budgets as they grow, before
        // retaining another potentially large object in memory.
        let records = if *asset_type == AssetType::Motion {
            &sim
        } else {
            &view
        };
        let mut manifest = vec![0; manifest_encoded_len(asset_type.domain(), records.len())?];
        encode_manifest(asset_type.domain(), records, &mut manifest)?;
        inspected.push(InspectionEntry {
            id: id.to_string(),
            asset_type: asset_type.name(),
            schema_version: 1,
            payload_len: metadata.payload_len,
            payload_sha256: hex(&digest),
        });
        provenance.sources.push(SourceRecord {
            id: id.to_string(),
            source: source.clone(),
            source_sha256: hex(&sha256(&source_bytes)),
            cache_key: hex(&key),
        });
        files.insert(format!("objects/{}.bin", hex(&digest)), payload);
    }
    let sim_bytes = make_manifest(Domain::Sim, &sim)?;
    let view_bytes = make_manifest(Domain::View, &view)?;
    let summary = CookSummary {
        sim_manifest_sha256: sha256(&sim_bytes),
        view_manifest_sha256: sha256(&view_bytes),
        asset_count: sim.len() + view.len(),
        cache_hits,
    };
    files.insert(
        "generated_sim.rs".into(),
        generate_sim(&motion, summary.sim_manifest_sha256).into_bytes(),
    );
    files.insert(
        "inspect.json".into(),
        json_bytes(&Inspection {
            format: "orr.asset-inspect/1",
            sim_manifest_sha256: hex(&summary.sim_manifest_sha256),
            view_manifest_sha256: hex(&summary.view_manifest_sha256),
            assets: &inspected,
        })?,
    );
    files.insert("report.json".into(), json_bytes(&provenance)?);
    files.insert(SIM_MANIFEST.into(), sim_bytes);
    files.insert(VIEW_MANIFEST.into(), view_bytes);
    if options.check {
        compare_output(&options.out, &files)?;
    } else {
        publish(&options.out, &files)?;
    }
    Ok(summary)
}
fn make_manifest(domain: Domain, entries: &[ManifestEntry]) -> Result<Vec<u8>> {
    let mut bytes = vec![0; manifest_encoded_len(domain, entries.len())?];
    encode_manifest(domain, entries, &mut bytes)?;
    Ok(bytes)
}
fn generate_sim(records: &[(AssetRef, i64)], digest: [u8; 32]) -> String {
    use std::fmt::Write;
    let mut out = String::from("// @generated by orr_asset_cook format 1; do not edit.\n");
    out.push_str("pub const SIM_MANIFEST_SHA256: [u8; 32] = [");
    for byte in digest {
        write!(out, "{byte},").expect("String formatting");
    }
    out.push_str("];\nconst fn generated_motion(raw: i64) -> orr_asset::MotionProfileV1 {\n    match orr_asset::MotionProfileV1::new(orr_fp::FP::from_raw(raw)) {\n        Ok(value) => value,\n        Err(_) => panic!(\"invalid generated motion\"),\n    }\n}\n");
    writeln!(
        out,
        "pub static MOTION_PROFILES: [(orr_asset::AssetRef, orr_asset::MotionProfileV1); {}] = [",
        records.len()
    )
    .expect("String formatting");
    for (id, raw) in records {
        writeln!(
            out,
            "    (orr_asset::AssetRef::from_raw({}), generated_motion({raw})),",
            id.get()
        )
        .expect("String formatting");
    }
    out.push_str("];\n");
    out
}

fn read_cache(root: &Path, key: [u8; 32], asset_type: AssetType) -> Result<Vec<u8>> {
    let bytes = read_contained(
        root,
        &format!("{}.bin", hex(&key)),
        CACHE_HEADER + MAX_PAYLOAD_BYTES,
    )?;
    if bytes.len() < CACHE_HEADER
        || &bytes[..4] != b"ORAC"
        || bytes[4..8] != 1u32.to_le_bytes()
        || bytes[8..12] != asset_type.type_id().to_le_bytes()
        || bytes[12..16] != 1u32.to_le_bytes()
        || bytes[16..48] != key
    {
        return Err(CookError::invalid("invalid cache header"));
    }
    let len = u64::from_le_bytes(bytes[48..56].try_into().expect("checked cache header"));
    let payload = &bytes[CACHE_HEADER..];
    if len != payload.len() as u64 || bytes[56..88] != sha256(payload) {
        return Err(CookError::invalid("invalid cache length/hash"));
    }
    validate_payload(asset_type, payload)?;
    Ok(payload.to_vec())
}
fn write_cache(root: &Path, key: [u8; 32], asset_type: AssetType, payload: &[u8]) -> Result<()> {
    let mut bytes = Vec::with_capacity(CACHE_HEADER + payload.len());
    bytes.extend_from_slice(b"ORAC");
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&asset_type.type_id().to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&key);
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&sha256(payload));
    bytes.extend_from_slice(payload);
    let path = root.join(format!("{}.bin", hex(&key)));
    let (temp, mut file) = temporary_file(root, "cache")?;
    let guard = TempFile(temp.clone());
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    // Windows rename does not replace a file. A missing entry is safe: readers
    // always validate a complete entry and otherwise recook. Never follow a
    // possibly corrupt leaf symlink when repairing the cache.
    if fs::symlink_metadata(&path).is_ok() {
        fs::remove_file(&path)?;
    }
    fs::rename(&temp, &path)?;
    drop(guard);
    Ok(())
}

/// Verify both manifests, actual object length/hash/schema, and generated sim
/// records. Does not authenticate the package or bind it to a running binary.
pub fn inspect_bundle(path: &Path) -> Result<Vec<u8>> {
    let root = fs::canonicalize(path)?;
    let sim = read_contained(&root, SIM_MANIFEST, MAX_MANIFEST_BYTES)?;
    let view = read_contained(&root, VIEW_MANIFEST, MAX_MANIFEST_BYTES)?;
    let mut inspected = Vec::new();
    let mut motion = Vec::new();
    let mut ids = BTreeSet::new();
    for (bytes, domain) in [(&sim, Domain::Sim), (&view, Domain::View)] {
        for entry in Manifest::decode(bytes, domain)?.entries() {
            if !ids.insert(entry.id) {
                return Err(CookError::invalid("duplicate ID across manifest domains"));
            }
            let asset_type = match domain {
                Domain::Sim => AssetType::Motion,
                Domain::View => AssetType::Impact,
            };
            let object = read_contained(
                &root,
                &format!("objects/{}.bin", hex(&entry.payload_sha256)),
                MAX_PAYLOAD_BYTES,
            )?;
            if object.len() as u64 != entry.payload_len || sha256(&object) != entry.payload_sha256 {
                return Err(CookError::invalid("object length/hash mismatch"));
            }
            validate_payload(asset_type, &object)?;
            if asset_type == AssetType::Motion {
                motion.push((
                    entry.id,
                    MotionProfileV1::decode(&object)?.speed_per_tick().raw(),
                ));
            }
            inspected.push(InspectionEntry {
                id: entry.id.to_string(),
                asset_type: asset_type.name(),
                schema_version: entry.schema_version,
                payload_len: entry.payload_len,
                payload_sha256: hex(&entry.payload_sha256),
            });
        }
    }
    inspected.sort_by(|a, b| a.id.cmp(&b.id));
    let generated = generate_sim(&motion, sha256(&sim));
    if read_contained(&root, "generated_sim.rs", 64 * 1024)? != generated.as_bytes() {
        return Err(CookError::invalid(
            "generated sim table does not match bundle",
        ));
    }
    json_bytes(&Inspection {
        format: "orr.asset-inspect/1",
        sim_manifest_sha256: hex(&sha256(&sim)),
        view_manifest_sha256: hex(&sha256(&view)),
        assets: &inspected,
    })
}
fn compare_output(path: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    let root = fs::canonicalize(path)?;
    for (name, expected) in files {
        let actual = read_contained(&root, name, expected.len())?;
        if actual != *expected {
            return Err(CookError::invalid(format!(
                "generated output drift: {name}"
            )));
        }
    }
    // No directory traversal drives cooking. Here enumeration is only a bounded
    // exact-output check, so stale objects and accidental extra files also fail.
    let actual = output_names(&root)?;
    if actual != files.keys().cloned().collect() {
        return Err(CookError::invalid(
            "unexpected/missing generated output file",
        ));
    }
    inspect_bundle(&root)?;
    Ok(())
}
fn output_names(root: &Path) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| CookError::invalid("non-UTF-8 output name"))?;
        if name == "objects" && entry.file_type()?.is_dir() {
            for object in fs::read_dir(entry.path())? {
                let object = object?;
                if !object.file_type()?.is_file() {
                    return Err(CookError::invalid("unexpected output entry"));
                }
                let name = object
                    .file_name()
                    .into_string()
                    .map_err(|_| CookError::invalid("non-UTF-8 object name"))?;
                names.insert(format!("objects/{name}"));
                if names.len() > 85 {
                    return Err(CookError::invalid("output record budget exceeded"));
                }
            }
        } else if entry.file_type()?.is_file() {
            names.insert(name);
        } else {
            return Err(CookError::invalid("unexpected output entry"));
        }
        if names.len() > 85 {
            return Err(CookError::invalid("output record budget exceeded"));
        }
    }
    Ok(names)
}
fn temporary_file(parent: &Path, label: &str) -> Result<(PathBuf, File)> {
    for _ in 0..128 {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".orr-asset-{label}-{}-{serial}.tmp",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(CookError::invalid("could not reserve temporary file"))
}
struct TempFile(PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn publish(path: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    let parent = fs::canonicalize(nonempty_parent(path))?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| CookError::invalid("output needs a UTF-8 directory name"))?;
    validate_source_path(name)?;
    let destination = parent.join(name);
    let lock_path = parent.join(format!(".{name}.orr-asset-cook.lock"));
    let lock = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)?;
    drop(lock);
    let lock_guard = TempFile(lock_path);
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(CookError::invalid(
            "output already exists; use a new directory or --check",
        ));
    }
    let mut staging = None;
    for _ in 0..128 {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".orr-asset-bundle-{}-{serial}.tmp",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => {
                staging = Some(path);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    let staging =
        staging.ok_or_else(|| CookError::invalid("could not reserve staging directory"))?;
    let guard = TempDir(staging.clone());
    fs::create_dir(staging.join("objects"))?;
    for (name, bytes) in files {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staging.join(name))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    compare_output(&staging, files)?;
    // Recheck immediately before rename. A cooperating publisher cannot pass
    // the lock. No portable std-only rename-no-replace primitive is available
    // for directories; hostile concurrent filesystem mutation is out of scope.
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(CookError::invalid("output appeared during cook"));
    }
    fs::rename(&staging, destination)?;
    drop(guard);
    drop(lock_guard);
    Ok(())
}

/// Write an authoring index to a new path only. Hard-link publication provides
/// atomic create-if-absent semantics on the destination's own filesystem.
pub fn write_new_index(path: &Path, index: &Index) -> Result<()> {
    let bytes = index.encode()?;
    let parent = fs::canonicalize(nonempty_parent(path))?;
    let (temp, mut file) = temporary_file(&parent, "index")?;
    let guard = TempFile(temp.clone());
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::hard_link(&temp, path)?;
    drop(guard);
    Ok(())
}

/// OS-random candidate IDs are checked against live entries AND tombstones.
pub fn allocate_id(index: &Index) -> Result<AssetRef> {
    for _ in 0..128 {
        let mut bytes = [0; 8];
        getrandom::fill(&mut bytes)
            .map_err(|error| CookError::invalid(format!("OS random source: {error}")))?;
        let id = AssetRef::from_raw(u64::from_le_bytes(bytes));
        if !id.is_null() && !index.entries.iter().any(|entry| entry.id() == id) {
            return Ok(id);
        }
    }
    Err(CookError::invalid("could not allocate an unused GUID"))
}
