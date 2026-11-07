//! New lit Room projects assembled in an owned stage, never modifying an existing project.
use crate::{
    project_create::{self, CreateOptions, CreateReport},
    room_lighting::Document,
};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Component, Path, PathBuf},
};
pub const TEMPLATE: &str = "room-escape-lit-3d-v1";

pub fn read_lighting(path: &Path) -> Result<Document, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)
        .map_err(err)?;
    if !file.metadata().map_err(err)?.is_file() {
        return Err("lighting input must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(crate::room_lighting::MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    Document::parse(&bytes)
}

/// Linux only. Existing destination objects are never replaced. Ordinary trusted
/// byte changes and destination collisions reject; parent inode replacement,
/// hostile races and power-loss durability are not covered.
pub fn create(output: &Path, seed: &str, lighting: &Document) -> Result<CreateReport, String> {
    create_with(
        output,
        seed,
        lighting,
        |_, _| Ok(()),
        crate::project_publish::publish_no_replace,
    )
}
fn create_with(
    output: &Path,
    seed: &str,
    lighting: &Document,
    mut checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
    publish: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<CreateReport, String> {
    let bytes = lighting.to_bytes()?;
    let output = destination(output)?;
    let parent = output.parent().ok_or("output parent required")?;
    let transaction = tempfile::Builder::new()
        .prefix(".orr-lit-room-")
        .tempdir_in(parent)
        .map_err(err)?;
    let root = transaction.path().join("project");
    let mut report = project_create::create(&CreateOptions {
        output: root.clone(),
        template: project_create::ROOM_TEMPLATE.into(),
        seed: seed.into(),
    })?;
    checkpoint("base-created", &root)?;
    let manifest_path = root.join("orr.project.json");
    let mut manifest: orr_package::ProjectManifest =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(err)?).map_err(err)?;
    manifest
        .entry
        .as_mut()
        .ok_or("starter entry missing")?
        .lighting = Some("room.lighting.json".into());
    let mut light = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("room.lighting.json"))
        .map_err(err)?;
    light
        .write_all(&bytes)
        .and_then(|()| light.sync_all())
        .map_err(err)?;
    let mut manifest_file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&manifest_path)
        .map_err(err)?;
    manifest_file
        .write_all(&serde_json::to_vec_pretty(&manifest).map_err(err)?)
        .and_then(|()| manifest_file.sync_all())
        .map_err(err)?;
    let mut readme = OpenOptions::new()
        .append(true)
        .open(root.join("README.md"))
        .map_err(err)?;
    writeln!(readme,"\nLit starter: {TEMPLATE}. This project declares room.lighting.json. Build the editor/runtime/exporter with room-lighting; an unconsenting consumer rejects this project. The supplied validated light policy is copied by value; its source file is not needed. Edit Room lighting, Apply and Save independently. No shadows or extra lights are added.").map_err(err)?;
    readme.sync_all().map_err(err)?;
    let expected = snapshot(&root)?;
    checkpoint("before-validation", &root)?;
    let prepared = crate::room_project::PreparedProject::open_with_lighting(&root, true)?;
    if prepared.lighting().map(|v| &v.document) != Some(lighting)
        || prepared.scene().frame().checksum() != report.initial_checksum
    {
        return Err("lit starter admission differs from authored lighting or initial Frame".into());
    }
    checkpoint("before-publish", &root)?;
    if snapshot(&root)? != expected {
        return Err("lit starter bytes changed before publication".into());
    }
    // Re-admit after the final checkpoint, before publishing the complete closure.
    crate::room_project::PreparedProject::open_with_lighting(&root, true)?;
    if destination(&output)? != output {
        return Err("output parent changed".into());
    }
    publish(&root, &output)?;
    report.output = output;
    report.template = TEMPLATE.into();
    Ok(report)
}
fn snapshot(root: &Path) -> Result<std::collections::BTreeMap<PathBuf, Option<Vec<u8>>>, String> {
    let mut entries = std::collections::BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    let mut total = 0;
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).map_err(err)? {
            let path = entry.map_err(err)?.path();
            let relative = path.strip_prefix(root).map_err(err)?.to_path_buf();
            let metadata = fs::symlink_metadata(&path).map_err(err)?;
            if entries.len() >= 64 {
                return Err("lit starter closure exceeds64 entries".into());
            }
            let bytes = if metadata.is_dir() {
                pending.push(path);
                None
            } else if metadata.is_file() {
                let mut bytes = Vec::new();
                OpenOptions::new()
                    .read(true)
                    .custom_flags(
                        (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
                    )
                    .open(path)
                    .map_err(err)?
                    .take(1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(err)?;
                total += bytes.len();
                if bytes.len() > 1024 * 1024 || total > 3 * 1024 * 1024 {
                    return Err("lit starter closure exceeds byte bounds".into());
                }
                Some(bytes)
            } else {
                return Err("lit starter closure contains a special file or symlink".into());
            };
            entries.insert(relative, bytes);
        }
    }
    Ok(entries)
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn destination(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("output must be UTF-8")?;
    if !path.is_absolute()
        || text.len() > 4096
        || text.chars().any(char::is_control)
        || text.split('/').any(|s| matches!(s, "." | ".."))
        || path.components().count() > 64
    {
        return Err("output requires an absolute bounded path without traversal".into());
    }
    let leaf = path
        .file_name()
        .filter(|s| s.len() <= 200)
        .ok_or("bounded output leaf required")?;
    let parent = path.parent().ok_or("existing output parent required")?;
    let mut current = PathBuf::new();
    for c in parent.components() {
        if !matches!(c, Component::RootDir | Component::Normal(_)) {
            return Err("invalid output component".into());
        }
        current.push(c);
        let m = fs::symlink_metadata(&current).map_err(err)?;
        if m.file_type().is_symlink() || !m.is_dir() {
            return Err("output ancestors must be existing nonsymlink directories".into());
        }
    }
    let out = fs::canonicalize(parent).map_err(err)?.join(leaf);
    match fs::symlink_metadata(&out) {
        Ok(_) => Err("output destination already exists".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(out),
        Err(e) => Err(err(e)),
    }
}
#[cfg(test)]
mod tests;
