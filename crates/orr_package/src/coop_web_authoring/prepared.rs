//! Structural/pixel proof only. This type is not a JS sampler verification receipt.
use super::{absolute, manifest, png};
use crate::authoring_io::{directory, read_at, recheck_directory};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[derive(Debug, Clone, Serialize)]
pub(super) struct ImageReceipt {
    pub name: String,
    pub bytes: usize,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub id: String,
}
pub(super) struct Prepared {
    bytes: Vec<u8>,
    images: Vec<ImageReceipt>,
    roles: Vec<String>,
    snapshots: BTreeMap<String, Vec<u8>>,
    root: PathBuf,
}
impl Prepared {
    pub(super) fn recheck(&self) -> Result<(), String> {
        for (name, bytes) in &self.snapshots {
            if snapshot(&self.root.join(name), 8 * 1024 * 1024)? != *bytes {
                return Err("animation image changed during verification".into());
            }
        }
        Ok(())
    }
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(super) fn images(&self) -> &[ImageReceipt] {
        &self.images
    }
    pub(super) fn roles(&self) -> &[String] {
        &self.roles
    }
}
pub(super) fn snapshot(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let p = absolute(path)?;
    let root = p.parent().ok_or("missing input parent")?;
    let fd = directory(root)?;
    let data = read_at(&fd, p.file_name().ok_or("missing input filename")?, max)?;
    recheck_directory(root, &fd)?;
    Ok(data)
}
// Offline web-format admission uses the authored f64 rectangle contract, never simulation state.
#[allow(clippy::float_arithmetic)]
pub(super) fn prepare(bytes: Vec<u8>, assets: &Path) -> Result<Prepared, String> {
    let value = manifest::parse(&bytes)?;
    let names = manifest::names(&value)?;
    let root = absolute(assets)?;
    let fd = directory(&root)?;
    let mut snapshots = BTreeMap::new();
    let mut total = 0usize;
    for name in &names {
        let data = snapshot(&root.join(name), 8 * 1024 * 1024)?;
        total = total.checked_add(data.len()).ok_or("image byte overflow")?;
        if total > 32 * 1024 * 1024 {
            return Err("animation images exceed32MiB".into());
        }
        snapshots.insert(name.clone(), data);
    }
    recheck_directory(&root, &fd)?;
    let mut images = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let data = &snapshots[name];
        let (width, height) = png::validate(data)?;
        for definition in value
            .as_object()
            .ok_or("manifest object required")?
            .values()
        {
            if definition["image"].as_str() == Some(name.as_str()) {
                for clip in definition["clips"]
                    .as_object()
                    .ok_or("clips required")?
                    .values()
                {
                    for frame in clip["frames"].as_array().ok_or("frames required")? {
                        let r = frame.as_array().ok_or("frame required")?;
                        if r[0].as_f64().ok_or("x")? + r[2].as_f64().ok_or("width")?
                            > f64::from(width)
                            || r[1].as_f64().ok_or("y")? + r[3].as_f64().ok_or("height")?
                                > f64::from(height)
                        {
                            return Err("animation frame outside PNG".into());
                        }
                    }
                }
            }
        }
        images.push(ImageReceipt {
            name: name.clone(),
            bytes: data.len(),
            sha256: digest(data),
            width,
            height,
            id: format!("authored-animation-image-{i}"),
        });
    }
    let roles = value
        .as_object()
        .ok_or("manifest object required")?
        .keys()
        .cloned()
        .collect();
    Ok(Prepared {
        bytes,
        images,
        roles,
        snapshots,
        root,
    })
}
