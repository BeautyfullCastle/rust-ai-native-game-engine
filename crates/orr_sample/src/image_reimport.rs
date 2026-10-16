//! Explicit, same-layout PNG replacement. Package objects are immutable; only
//! the expected active lock is atomically replaced. No source watcher, child
//! process, ORAM image format, scene Undo or power-loss durability is claimed.
use crate::{
    project_export::{
        admission::{ProjectSnapshot, SnapshotFile, MAX_FILE_BYTES},
        profile::{Prepared, Profile},
    },
    project_sprites::{decode_atlas, Asset, AssetKey, Document},
};
use orr_package::{Lock, Project, ProjectGame};
use semver::Version;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// The calling host's actual consuming route, never inferred from Cargo feature
/// unification. Progress is metadata-only and never opens player storage.
#[derive(Clone, Copy)]
pub struct Consumer {
    pub collect: bool,
    pub ui: bool,
}
pub struct Options {
    pub project: PathBuf,
    pub package: String,
    pub document: String,
    pub image: PathBuf,
    pub version: String,
    pub consumer: Consumer,
}
/// Owns every byte needed to commit. Dropping an uncommitted transaction removes
/// only its private stage. No active source file or package is modified.
pub struct Transaction {
    original: ProjectSnapshot,
    replacement: SnapshotFile,
    _stage: tempfile::TempDir,
    source: PathBuf,
    expected: Lock,
    approved: Lock,
    runtime: orr_package::Runtime,
    assets: BTreeMap<AssetKey, Asset>,
    document: Document,
    sidecar_path: PathBuf,
}
impl Transaction {
    pub fn assets(&self) -> &BTreeMap<AssetKey, Asset> {
        &self.assets
    }
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn sidecar_path(&self) -> &Path {
        &self.sidecar_path
    }
    pub fn scene_path(&self) -> PathBuf {
        self.original.root.join(&self.original.entry.scene)
    }
    pub fn initial_checksum(&self) -> u64 {
        self.original.initial_checksum
    }
    pub fn candidate_lock(&self) -> &Lock {
        &self.approved
    }
    /// One-shot CAS. All fallible validation precedes the active lock rename.
    /// The caller swaps its already-prepared view cache after this returns Ok.
    pub fn commit(self) -> Result<Lock, String> {
        let project =
            Project::open(&self.original.root, self.runtime.clone()).map_err(|e| e.to_string())?;
        project
            .install_checked(
                std::slice::from_ref(&self.source),
                &self.expected,
                &self.approved,
                || {
                    self.original.recheck()?;
                    self.replacement.recheck()
                },
            )
            .map_err(|e| e.to_string())
    }
}

pub fn prepare(options: &Options) -> Result<Transaction, String> {
    let profile = if options.consumer.collect {
        Profile::Collect
    } else {
        Profile::Arena
    };
    let original = ProjectSnapshot::open_profile(&options.project, profile)?;
    let manifest = original
        .files
        .iter()
        .find(|f| f.role == "project_manifest")
        .ok_or("missing project manifest")?;
    let manifest: orr_package::ProjectManifest =
        serde_json::from_slice(&manifest.bytes).map_err(|e| e.to_string())?;
    if original.entry.game
        != if options.consumer.collect {
            ProjectGame::CollectDodgeV1
        } else {
            ProjectGame::Arena
        }
    {
        return Err("reimport consuming game differs from project entry".into());
    }
    if original.entry.ui.is_some() && !options.consumer.ui {
        return Err("reimport consumer does not support authored UI".into());
    }
    let runtime = profile.runtime();
    let project = Project::open(&original.root, runtime.clone()).map_err(|e| e.to_string())?;
    let expected = project.verify().map_err(|e| e.to_string())?;
    let old = expected
        .packages
        .get(&options.package)
        .ok_or("selected package is not active")?;
    if !expected.direct.contains_key(&options.package) {
        return Err("reimport requires a directly selected package".into());
    }
    if old.manifest.capabilities != BTreeSet::from(["sprite".into()])
        || !old.manifest.dependencies.is_empty()
    {
        return Err("reimport requires a sprite-only package without dependencies".into());
    }
    let previous = Version::parse(&old.manifest.version).map_err(|e| e.to_string())?;
    let next = Version::parse(&options.version).map_err(|e| e.to_string())?;
    if next.cmp_precedence(&previous) != std::cmp::Ordering::Greater {
        return Err("replacement requires a strictly higher package version".into());
    }
    let document_bytes = project
        .read_asset(&options.package, &options.document)
        .map_err(|e| e.to_string())?;
    let sprite = orr_sprite::SpriteDocument::from_json(
        std::str::from_utf8(&document_bytes).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let image = &sprite.atlas().image;
    if !image.ends_with(".png") {
        return Err("reimport atlas must use a .png path".into());
    }
    let raw = format!("{}.rgba", image.strip_suffix(".png").unwrap());
    let required = BTreeSet::from([
        options.document.clone(),
        image.clone(),
        "LICENSE.txt".into(),
    ]);
    let mut with_raw = required.clone();
    with_raw.insert(raw.clone());
    if required.len() != 3 || (old.manifest.files != required && old.manifest.files != with_raw) {
        return Err("reimport package must contain only its sprite document, PNG, optional same-stem RGBA and LICENSE.txt".into());
    }
    let replacement = SnapshotFile::read(&options.image, "replacement.png", MAX_FILE_BYTES)?;
    let rgba = decode_atlas(
        &replacement.bytes,
        sprite.atlas().width,
        sprite.atlas().height,
    )?;
    // Runtime decoding obtains pixels; authoring additionally consumes through
    // IEND so a truncated/corrupt trailing PNG chunk cannot become a new asset.
    let mut reader = png17::Decoder::new(std::io::Cursor::new(&replacement.bytes))
        .read_info()
        .map_err(|e| e.to_string())?;
    reader.finish().map_err(|e| e.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix("orr-image-reimport-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let root = stage.path().join("project");
    fs::create_dir(&root).map_err(|e| e.to_string())?;
    for file in &original.files {
        write_new(&root.join(&file.relative), &file.bytes)?;
    }
    let source = stage.path().join("replacement");
    fs::create_dir(&source).map_err(|e| e.to_string())?;
    let mut new_manifest = old.manifest.clone();
    new_manifest.version = next.to_string();
    write_new(
        &source.join("orr.package.json"),
        &serde_json::to_vec_pretty(&new_manifest).map_err(|e| e.to_string())?,
    )?;
    for path in &old.manifest.files {
        let bytes = if path == image {
            replacement.bytes.clone()
        } else if path == &raw {
            rgba.clone()
        } else {
            project
                .read_asset(&options.package, path)
                .map_err(|e| e.to_string())?
        };
        write_new(&source.join(path), &bytes)?;
    }
    let staged_project = Project::open(&root, runtime.clone()).map_err(|e| e.to_string())?;
    let approved = staged_project
        .install(std::slice::from_ref(&source))
        .map_err(|e| e.to_string())?;
    let admitted = ProjectSnapshot::open_profile(&root, profile)?;
    if admitted.initial_checksum != original.initial_checksum {
        return Err("image replacement changed simulation checksum".into());
    }
    let staged_manifest = staged_project.manifest().ok_or("missing staged manifest")?;
    if staged_manifest != &manifest {
        return Err("image replacement changed project or progress identity".into());
    }
    let prepared = Prepared::open(&root, profile)?;
    let sprites = prepared
        .sprites()
        .ok_or("reimport requires saved sprite bindings")?;
    if !sprites
        .document
        .bindings
        .values()
        .any(|b| b.package == options.package && b.document == options.document)
    {
        return Err("selected sprite document is not referenced by this project".into());
    }
    let document = sprites.document.clone();
    let assets = sprites.assets.clone();
    let sidecar_path = original.root.join(
        original
            .entry
            .sprites
            .as_ref()
            .ok_or("missing sprite sidecar")?,
    );
    original.recheck()?;
    replacement.recheck()?;
    Ok(Transaction {
        original,
        replacement,
        _stage: stage,
        source,
        expected,
        approved,
        runtime,
        assets,
        document,
        sidecar_path,
    })
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    fs::create_dir_all(path.parent().ok_or("missing parent")?).map_err(|e| e.to_string())?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
