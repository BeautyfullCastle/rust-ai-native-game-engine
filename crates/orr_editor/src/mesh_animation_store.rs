use crate::mesh_art::checked_scene;
#[cfg(feature = "mesh-animated-models")]
use crate::{
    mesh_animation::Snapshot,
    mesh_art::{digest, read_optional_file, recheck_scene, validate_expected},
};
use std::path::{Path, PathBuf};
#[cfg(feature = "mesh-animated-models")]
use std::{fs, io::Write};

const MAX_PATH_BYTES: usize = 4096;
#[cfg(feature = "mesh-animated-models")]
const MAX_SIDECAR_BYTES: u64 = 32 * 1024 * 1024;
const SIDECAR_SUFFIX: &str = ".mesh-animation.json";

pub(crate) fn sidecar_path(scene: &Path) -> Result<PathBuf, String> {
    let (canonical_scene, _) = checked_scene(scene)?;
    sidecar_path_for(&canonical_scene)
}

pub(crate) fn has_sidecar(scene: &Path) -> Result<bool, String> {
    let sidecar = sidecar_path(scene)?;
    match std::fs::symlink_metadata(&sidecar) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("inspect mesh animation sidecar: {error}")),
    }
}

#[cfg(feature = "mesh-animated-models")]
pub(crate) fn load(
    scene: &Path,
    graph_revision: [u8; 32],
) -> Result<Option<(Snapshot, String)>, String> {
    if !has_sidecar(scene)? {
        return Ok(None);
    }
    let (canonical_scene, filename) = checked_scene(scene)?;
    let sidecar = sidecar_path_for(&canonical_scene)?;
    let Some(bytes) = read_optional_file(&sidecar, MAX_SIDECAR_BYTES, "mesh animation sidecar")?
    else {
        return Ok(None);
    };
    let snapshot = Snapshot::decode_embedded(&bytes, &filename, graph_revision)?;
    Ok(Some((snapshot, digest(&bytes))))
}

#[cfg(feature = "mesh-animated-models")]
pub(crate) fn sidecar_stamp(scene: &Path) -> Result<Option<String>, String> {
    let sidecar = sidecar_path(scene)?;
    Ok(
        read_optional_file(&sidecar, MAX_SIDECAR_BYTES, "mesh animation sidecar")?
            .as_deref()
            .map(digest),
    )
}

#[cfg(feature = "mesh-animated-models")]
pub(crate) fn save(
    scene: &Path,
    snapshot: Option<&Snapshot>,
    expected: Option<&str>,
    graph_revision: [u8; 32],
) -> Result<Option<String>, String> {
    let (canonical_scene, filename) = checked_scene(scene)?;
    let sidecar = sidecar_path_for(&canonical_scene)?;
    validate_expected(expected)?;
    let current = read_optional_file(&sidecar, MAX_SIDECAR_BYTES, "mesh animation sidecar")?;
    match (current.as_deref().map(digest), expected) {
        (None, None) => {}
        (Some(current_digest), Some(expected_digest)) if current_digest == expected_digest => {}
        (None, Some(_)) => {
            return Err("mesh animation sidecar was removed by another editor".into())
        }
        (Some(_), None) => {
            return Err("mesh animation sidecar already exists; reload before saving".into())
        }
        (Some(_), Some(_)) => {
            return Err("mesh animation sidecar changed; reload before saving".into())
        }
    }
    let Some(snapshot) = snapshot else {
        if current.is_some() {
            recheck_scene(&canonical_scene)?;
            let final_current =
                read_optional_file(&sidecar, MAX_SIDECAR_BYTES, "mesh animation sidecar")?;
            if final_current.as_deref().map(digest).as_deref() != expected {
                return Err("mesh animation sidecar changed before removal".into());
            }
            fs::remove_file(&sidecar)
                .map_err(|error| format!("remove mesh animation sidecar: {error}"))?;
        }
        return Ok(None);
    };
    if snapshot.graph_revision() != graph_revision {
        return Err("mesh animation snapshot belongs to a different graph revision".into());
    }
    let bytes = snapshot.encode_embedded(&filename)?;
    if bytes.len() as u64 > MAX_SIDECAR_BYTES {
        return Err(format!(
            "mesh animation sidecar exceeds {MAX_SIDECAR_BYTES} bytes"
        ));
    }
    let parent = canonical_scene.parent().ok_or("scene parent missing")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("create mesh animation staging file: {error}"))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("write mesh animation sidecar: {error}"))?;
    recheck_scene(&canonical_scene)?;
    let final_current = read_optional_file(&sidecar, MAX_SIDECAR_BYTES, "mesh animation sidecar")?;
    if final_current.as_deref().map(digest).as_deref() != expected {
        return Err("mesh animation sidecar changed before save; reload before saving".into());
    }
    if expected.is_some() {
        temporary
            .persist(&sidecar)
            .map_err(|error| format!("publish mesh animation sidecar: {}", error.error))?;
    } else {
        temporary.persist_noclobber(&sidecar).map_err(|error| {
            format!(
                "publish mesh animation sidecar without replacement: {}",
                error.error
            )
        })?;
    }
    Ok(Some(digest(&bytes)))
}

fn sidecar_path_for(scene: &Path) -> Result<PathBuf, String> {
    let filename = scene
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("scene filename must be Unicode")?;
    let sidecar = scene.with_file_name(format!("{filename}{SIDECAR_SUFFIX}"));
    let text = sidecar
        .to_str()
        .ok_or("mesh animation sidecar path must be Unicode")?;
    if text.len() > MAX_PATH_BYTES {
        return Err("mesh animation sidecar path exceeds its byte limit".into());
    }
    Ok(sidecar)
}

#[cfg(test)]
#[path = "mesh_animation_store/tests.rs"]
mod tests;
