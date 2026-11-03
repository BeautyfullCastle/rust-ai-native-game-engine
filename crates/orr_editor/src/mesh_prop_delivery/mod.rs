//! Linux deterministic headless delivery of saved Mesh props, not a native game window.
//! SHA-256 provides integrity only. Runtime trust is asserted explicitly by the operator.
pub mod export;
pub(crate) mod io;
pub mod manifest;
pub mod player;
pub mod source;
use manifest::*;
pub use source::SavedSource;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
pub struct VerifiedBundle {
    pub scene: PathBuf,
    pub manifest: Manifest,
    pub manifest_sha256: String,
    source: SavedSource,
    snapshots: Vec<io::Snapshot>,
    root: PathBuf,
}
impl VerifiedBundle {
    pub fn recheck(&self) -> Result<(), String> {
        self.source.recheck()?;
        for s in &self.snapshots {
            s.recheck()?;
        }
        io::inventory(&self.root, &inventory(&self.manifest))
    }
}
fn inventory(m: &Manifest) -> BTreeSet<PathBuf> {
    m.files
        .iter()
        .map(|f| PathBuf::from(&f.name))
        .chain(std::iter::once(MANIFEST_NAME.into()))
        .collect()
}
pub fn verify_bundle(root: &Path, expected_exe: &Path) -> Result<VerifiedBundle, String> {
    verify(root, Some(expected_exe))
}
pub(crate) fn verify(root: &Path, expected_exe: Option<&Path>) -> Result<VerifiedBundle, String> {
    let root = io::checked_path(root)?;
    let record = io::Snapshot::read(&root.join(MANIFEST_NAME), MAX_MANIFEST)?;
    if record.mode != 0o644 {
        return Err("manifest mode must be 0644".into());
    }
    let manifest = Manifest::parse(&record.bytes)?;
    let manifest_sha256 = hash(&record.bytes);
    io::inventory(&root, &inventory(&manifest))?;
    let mut snapshots = vec![record];
    for f in &manifest.files {
        let (limit, _) = manifest.limit(&f.name)?;
        let snapshot = io::Snapshot::read(&root.join(&f.name), limit)?;
        if snapshot.bytes.len() != f.bytes
            || snapshot.mode != f.mode
            || hash(&snapshot.bytes) != f.sha256
        {
            return Err(format!("bundle descriptor mismatch: {}", f.name));
        }
        if f.name == RUNTIME_NAME {
            orr_sample::trusted_runtime::validate_elf(&snapshot.bytes)?;
        }
        snapshots.push(snapshot);
    }
    if let Some(exe) = expected_exe {
        if io::checked_path(exe)? != root.join(RUNTIME_NAME) {
            return Err("running executable is not the bundled runtime".into());
        }
    }
    let scene = root.join(&manifest.scene);
    let source = SavedSource::capture(&scene)?;
    if source.initial_checksum() != manifest.checksum
        || source.graph_revision() != manifest.graph_revision
    {
        return Err("admitted scene differs from manifest".into());
    }
    let result = VerifiedBundle {
        scene,
        manifest,
        manifest_sha256,
        source,
        snapshots,
        root,
    };
    result.recheck()?;
    Ok(result)
}

#[cfg(test)]
mod tests;
