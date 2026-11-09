//! Explicit attachment and guarded revision for plain Room projects on a stable local filesystem.
//! This detects ordinary conflicts, not hostile concurrent directory replacement.
use crate::project_export::{
    admission::{ProjectSnapshot, SnapshotFile},
    profile::Profile,
};
use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

const MANIFEST: &str = "orr.project.json";
const BACKUP: &str = "orr.project.before-controls.json";

/// A committed manifest is never rolled back automatically, even on sync failure.
#[derive(Debug)]
pub struct Outcome {
    pub backup: PathBuf,
    pub durability_warning: Option<String>,
}

/// Prepared, validated bytes. Commit rechecks all captured project inputs.
pub struct Attachment {
    original: ProjectSnapshot,
    controls: SnapshotFile,
    candidate: Vec<u8>,
    mode: u32,
    backup: String,
}

pub fn prepare(root: &Path, name: &str) -> Result<Attachment, String> {
    prepare_change(root, name, None)
}

/// Switch to a different existing sidecar only when the current reference matches.
/// The caller supplies a fresh backup filename for this revision; neither sidecar
/// is ever written. Close all other writers before preparing and committing.
pub fn prepare_update(
    root: &Path,
    name: &str,
    expected_old: &str,
    backup: &str,
) -> Result<Attachment, String> {
    validate_name(expected_old)?;
    validate_name(backup)?;
    prepare_change(root, name, Some((expected_old, backup)))
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.len() > 100
        || !name.ends_with(".json")
        || !name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        || [MANIFEST, BACKUP, "orr.packages.lock.json"]
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
    {
        return Err(
            "controls must be a non-reserved simple ASCII .json filename at the project root"
                .into(),
        );
    }
    Ok(())
}

fn prepare_change(
    root: &Path,
    name: &str,
    revision: Option<(&str, &str)>,
) -> Result<Attachment, String> {
    validate_name(name)?;
    let original = ProjectSnapshot::open_profile(root, Profile::Room)?;
    let manifest = original
        .files
        .iter()
        .find(|f| f.relative == MANIFEST)
        .ok_or("missing manifest")?;
    let typed: orr_package::ProjectManifest =
        serde_json::from_slice(&manifest.bytes).map_err(err)?;
    if typed.schema != 2
        || typed.progress.is_some()
        || original.entry.lighting.is_some()
        || original.entry.character.is_some()
    {
        return Err(
            "attachment/revision requires plain schema-2 Room, no lighting/character/progress"
                .into(),
        );
    }
    // Do not reinterpret legacy sequence forms or silently rewrite null fields.
    let mut value: serde_json::Value = serde_json::from_slice(&manifest.bytes).map_err(err)?;
    let entry = value
        .as_object_mut()
        .and_then(|v| v.get_mut("entry"))
        .and_then(|v| v.as_object_mut())
        .ok_or("manifest and entry must be JSON objects")?;
    let backup = if let Some((expected_old, backup)) = revision {
        if original.entry.controls.as_deref() != Some(expected_old) {
            return Err("current controls reference does not match expected old reference; inspect the manifest".into());
        }
        if backup.eq_ignore_ascii_case(name)
            || original
                .files
                .iter()
                .any(|f| f.relative.eq_ignore_ascii_case(backup))
        {
            return Err("backup filename aliases project content or new controls".into());
        }
        if fs::symlink_metadata(original.root.join(expected_old))
            .map_err(err)?
            .nlink()
            != 1
        {
            return Err("old controls must not be hardlinked".into());
        }
        backup
    } else {
        if entry.contains_key("controls") {
            return Err(
                "controls already present; first attachment cannot replace a mapping".into(),
            );
        }
        BACKUP
    };
    if original
        .files
        .iter()
        .any(|f| f.relative.eq_ignore_ascii_case(name))
    {
        return Err("controls filename aliases existing project content".into());
    }
    let controls = SnapshotFile::read(
        &original.root.join(name),
        name,
        orr_input::MAX_FILE_BYTES as u64,
    )?;
    crate::room_controls::RoomControls::load(&controls.bytes)?;
    let metadata = fs::symlink_metadata(original.root.join(MANIFEST)).map_err(err)?;
    if metadata.nlink() != 1
        || fs::symlink_metadata(original.root.join(name))
            .map_err(err)?
            .nlink()
            != 1
    {
        return Err("manifest and controls must not be hardlinked".into());
    }
    entry.insert("controls".into(), name.into());
    let mut candidate = serde_json::to_vec_pretty(&value).map_err(err)?;
    candidate.push(b'\n');
    if candidate.len() > 1024 * 1024 {
        return Err("candidate manifest exceeds 1 MiB".into());
    }
    // Validate the complete new project privately before touching live metadata.
    let stage = tempfile::tempdir().map_err(err)?;
    for file in &original.files {
        let path = stage.path().join(&file.relative);
        fs::create_dir_all(path.parent().ok_or("missing parent")?).map_err(err)?;
        fs::write(
            path,
            if file.relative == MANIFEST {
                &candidate
            } else {
                &file.bytes
            },
        )
        .map_err(err)?;
    }
    fs::write(stage.path().join(name), &controls.bytes).map_err(err)?;
    let admitted = crate::room_project::PreparedProject::open(stage.path())?;
    if admitted.scene().frame().checksum() != original.initial_checksum
        || admitted
            .controls()
            .ok_or("candidate controls absent")?
            .bytes
            != controls.bytes
    {
        return Err("candidate changed scene or controls".into());
    }
    original.recheck()?;
    controls.recheck()?;
    Ok(Attachment {
        original,
        controls,
        candidate,
        mode: metadata.permissions().mode() & 0o777,
        backup: backup.into(),
    })
}

impl Attachment {
    pub fn commit(self) -> Result<Outcome, String> {
        self.commit_with(|_| Ok(()))
    }

    fn commit_with(
        self,
        mut checkpoint: impl FnMut(&str) -> Result<(), String>,
    ) -> Result<Outcome, String> {
        let root = &self.original.root;
        let manifest = root.join(MANIFEST);
        let backup = root.join(&self.backup);
        let original_bytes = &self
            .original
            .files
            .iter()
            .find(|f| f.relative == MANIFEST)
            .ok_or("missing captured manifest")?
            .bytes;
        checkpoint("stage")?;
        let mut stage = tempfile::NamedTempFile::new_in(root).map_err(err)?;
        stage
            .as_file()
            .set_permissions(fs::Permissions::from_mode(self.mode))
            .map_err(err)?;
        stage.write_all(&self.candidate).map_err(err)?;
        checkpoint("stage_sync")?;
        stage.as_file().sync_all().map_err(err)?;
        // A complete, synced backup is published without replacing anything.
        let mut saved = tempfile::NamedTempFile::new_in(root).map_err(err)?;
        saved.write_all(original_bytes).map_err(err)?;
        checkpoint("backup_sync")?;
        saved.as_file().sync_all().map_err(err)?;
        checkpoint("conflict")?;
        self.original.recheck()?;
        self.controls.recheck()?;
        checkpoint("backup")?;
        saved
            .persist_noclobber(&backup)
            .map_err(|e| format!("backup not published; manifest unchanged: {e}"))?;
        let mut before_commit = || -> Result<(), String> {
            checkpoint("backup_directory_sync")?;
            fs::File::open(root)
                .and_then(|f| f.sync_all())
                .map_err(err)?;
            self.original.recheck()?;
            self.controls.recheck()?;
            Ok(())
        };
        before_commit().map_err(|e| {
            format!(
                "manifest unchanged; backup retained at {}: {e}",
                backup.display()
            )
        })?;
        checkpoint("rename").map_err(|e| {
            format!(
                "manifest unchanged; backup retained at {}: {e}",
                backup.display()
            )
        })?;
        stage.persist(&manifest).map_err(|e| {
            format!(
                "manifest unchanged; backup retained at {}: {e}",
                backup.display()
            )
        })?;
        let durability_warning = checkpoint("directory_sync")
            .and_then(|_| fs::File::open(root).and_then(|f| f.sync_all()).map_err(err))
            .err();
        Ok(Outcome {
            backup,
            durability_warning,
        })
    }
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[cfg(all(test, feature = "project-create"))]
mod tests;
