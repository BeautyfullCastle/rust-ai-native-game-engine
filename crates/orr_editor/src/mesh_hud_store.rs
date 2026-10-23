use crate::{
    mesh_art::{checked_scene, digest, read_optional_file, recheck_scene, validate_expected},
    mesh_hud::{Document, MAX_BYTES},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const MAX_PATH_BYTES: usize = 4096;
const SIDECAR_SUFFIX: &str = ".mesh-hud.json";

pub(crate) fn sidecar_path(scene: &Path) -> Result<PathBuf, String> {
    let (canonical_scene, _) = checked_scene(scene)?;
    sidecar_path_for(&canonical_scene)
}

pub(crate) fn load(
    scene: &Path,
    graph_revision: [u8; 32],
) -> Result<Option<(Document, String)>, String> {
    let (canonical_scene, filename) = checked_scene(scene)?;
    let sidecar = sidecar_path_for(&canonical_scene)?;
    let bytes = match read_optional_file(&sidecar, MAX_BYTES as u64, "mesh HUD sidecar")? {
        Some(bytes) => bytes,
        None => return Ok(None),
    };
    let document = Document::decode(&bytes, &filename, graph_revision)?;
    Ok(Some((document, digest(&bytes))))
}

pub(crate) fn sidecar_stamp(scene: &Path) -> Result<Option<String>, String> {
    let sidecar = sidecar_path(scene)?;
    Ok(
        read_optional_file(&sidecar, MAX_BYTES as u64, "mesh HUD sidecar")?
            .as_deref()
            .map(digest),
    )
}

pub(crate) fn has_sidecar(scene: &Path) -> Result<bool, String> {
    Ok(sidecar_stamp(scene)?.is_some())
}

pub(crate) fn save(
    scene: &Path,
    document: Option<&Document>,
    expected: Option<&str>,
    graph_revision: [u8; 32],
) -> Result<Option<String>, String> {
    let (canonical_scene, filename) = checked_scene(scene)?;
    let sidecar = sidecar_path_for(&canonical_scene)?;
    validate_expected(expected)?;
    let current = read_optional_file(&sidecar, MAX_BYTES as u64, "mesh HUD sidecar")?;
    match (current.as_deref().map(digest), expected) {
        (None, None) => {}
        (Some(current), Some(expected)) if current == expected => {}
        (None, Some(_)) => return Err("mesh HUD sidecar was removed by another editor".into()),
        (Some(_), None) => {
            return Err("mesh HUD sidecar already exists; reload before saving".into())
        }
        (Some(_), Some(_)) => return Err("mesh HUD sidecar changed; reload before saving".into()),
    }

    let Some(document) = document else {
        if current.is_some() {
            recheck_scene(&canonical_scene)?;
            let final_current = read_optional_file(&sidecar, MAX_BYTES as u64, "mesh HUD sidecar")?;
            if final_current.as_deref().map(digest).as_deref() != expected {
                return Err(
                    "mesh HUD sidecar changed before removal; reload before removing".into(),
                );
            }
            fs::remove_file(&sidecar)
                .map_err(|error| format!("remove mesh HUD sidecar: {error}"))?;
        }
        return Ok(None);
    };

    if let Some(existing) = current.as_deref() {
        Document::decode(existing, &filename, graph_revision)?;
    }
    let bytes = document.encode(&filename, graph_revision)?;
    if bytes.len() > MAX_BYTES {
        return Err(format!("mesh HUD sidecar exceeds {MAX_BYTES} bytes"));
    }
    let parent = canonical_scene.parent().ok_or("scene parent missing")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("create mesh HUD staging file: {error}"))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("write mesh HUD sidecar: {error}"))?;
    recheck_scene(&canonical_scene)?;
    let final_current = read_optional_file(&sidecar, MAX_BYTES as u64, "mesh HUD sidecar")?;
    if final_current.as_deref().map(digest).as_deref() != expected {
        return Err("mesh HUD sidecar changed before save; reload before saving".into());
    }
    if expected.is_some() {
        temporary
            .persist(&sidecar)
            .map_err(|error| format!("publish mesh HUD sidecar: {}", error.error))?;
    } else {
        temporary.persist_noclobber(&sidecar).map_err(|error| {
            format!(
                "publish mesh HUD sidecar without replacement: {}",
                error.error
            )
        })?;
    }
    Ok(Some(digest(&bytes)))
}

pub(crate) fn discard_invalid(
    scene: &Path,
    expected: &str,
    graph_revision: [u8; 32],
) -> Result<(), String> {
    let (canonical_scene, filename) = checked_scene(scene)?;
    let sidecar = sidecar_path_for(&canonical_scene)?;
    validate_expected(Some(expected))?;
    let bytes = read_optional_file(&sidecar, MAX_BYTES as u64, "mesh HUD sidecar")?
        .ok_or("mesh HUD sidecar is already absent")?;
    if digest(&bytes) != expected {
        return Err("mesh HUD sidecar changed; reload before discarding".into());
    }
    if Document::decode(&bytes, &filename, graph_revision).is_ok() {
        return Err("valid mesh HUD sidecars cannot be discarded as invalid".into());
    }
    recheck_scene(&canonical_scene)?;
    let final_bytes = read_optional_file(&sidecar, MAX_BYTES as u64, "mesh HUD sidecar")?
        .ok_or("mesh HUD sidecar was removed before discard")?;
    if digest(&final_bytes) != expected {
        return Err("mesh HUD sidecar changed before discard; reload before discarding".into());
    }
    fs::remove_file(&sidecar).map_err(|error| format!("discard invalid mesh HUD sidecar: {error}"))
}

fn sidecar_path_for(scene: &Path) -> Result<PathBuf, String> {
    let filename = scene
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("scene filename must be Unicode")?;
    let sidecar = scene.with_file_name(format!("{filename}{SIDECAR_SUFFIX}"));
    let text = sidecar
        .to_str()
        .ok_or("mesh HUD sidecar path must be Unicode")?;
    if text.len() > MAX_PATH_BYTES {
        return Err("mesh HUD sidecar path exceeds its byte limit".into());
    }
    Ok(sidecar)
}

#[cfg(test)]
#[path = "mesh_hud_store/tests.rs"]
mod tests;
