//! Read-only admission checks for replacing the current editor with a Collect
//! project. A caller must obtain and compare a fresh stamp immediately before
//! adoption. These observations do not lock the host or the filesystem.
use super::*;
use orr_sample::authored_ui::{Document, MAX_BYTES};
use std::{fs, io::Read, path::Component};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CollectProjectSourceStamp {
    game: EditorGame,
    path: PathBuf,
    checksum: u64,
    load_epoch: u64,
    control_generation: u64,
    history: J,
    scene: String,
}

impl Editor {
    fn collect_project_source_idle(&self) -> Result<(), String> {
        if !matches!(self.game(), EditorGame::PhysGame | EditorGame::CollectDodge)
            || !self.spec().is_local()
            || self.erp_status().is_some()
            || !self.can_mutate()
            || !self.scene_edit_allowed
            || self.mode() != Mode::Edit
            || self.previewing().is_some()
            || self.in_gesture()
            || self.batch_uncertain
            || self.history.in_tx
            || self.sim.in_tx
            || self.input_phase() != input::Phase::Off
            || self.input_cleanup_pending()
            || self
                .pending
                .values()
                .any(|pending| matches!(pending.kind, Pend::Gesture(..) | Pend::Input(..)))
        {
            return Err("Collect project changes require an idle local Physics or Collect editor in Edit mode, without an ERP listener, active control, transaction or pending edit".into());
        }
        if self.is_dirty() {
            return Err("Save the current scene or explicitly undo its edits before changing Collect projects".into());
        }
        Ok(())
    }

    /// Read authoritative host state and history without accepting, cancelling,
    /// saving or stopping anything. Cached toolbar availability is not authority.
    pub(crate) fn collect_project_source_stamp(
        &mut self,
    ) -> Result<CollectProjectSourceStamp, String> {
        self.collect_project_source_idle()?;
        let before = self.call("sim.state", J::Null)?;
        let (path, checksum) = clean_state(&before)?;
        let history = self.call("history.list", J::Null)?;
        clean_history(&history)?;
        let proposals = self.call("proposal.list", J::Null)?;
        if proposals
            .get("proposals")
            .and_then(J::as_array)
            .is_none_or(|open| !open.is_empty())
        {
            return Err("Resolve open proposals before changing Collect projects".into());
        }
        let scene = self.call("scene.save", json!({"write": false, "source": "doc"}))?;
        let scene_text = scene
            .get("text")
            .and_then(J::as_str)
            .ok_or("Host did not return the current scene document")?;
        let limit = if self.game().is_collect() {
            orr_sample::collect_project::MAX_BYTES
        } else {
            orr_sample::project::MAX_SCENE_BYTES
        };
        if scene_text.len() as u64 > limit
            || scene.get("dirty").and_then(J::as_bool) != Some(false)
            || scene
                .get("checksum")
                .and_then(orr_remote::wire::parse_checksum)
                != Some(checksum)
        {
            return Err(
                "Current scene document is dirty, stale or exceeds the project source limit".into(),
            );
        }
        let absolute = std::path::absolute(&path).map_err(|error| error.to_string())?;
        let saved = read_saved_file(&absolute, limit)?;
        let saved_text = std::str::from_utf8(&saved).map_err(|error| error.to_string())?;
        let saved_scene = orr_reflect::Scene::parse(saved_text, &self.types)
            .map_err(|error| format!("Saved source scene: {error}"))?;
        let current_scene = orr_reflect::Scene::parse(scene_text, &self.types)
            .map_err(|error| format!("Current source scene: {error}"))?;
        if saved_scene != current_scene {
            return Err("The source scene changed on disk; resolve the difference before changing Collect projects".into());
        }
        let after = self.call("sim.state", J::Null)?;
        clean_state(&after)?;
        self.collect_project_source_idle()?;
        if before != after {
            return Err(
                "Current host changed while checking the project source; retry after it settles"
                    .into(),
            );
        }
        if self.path().as_ref() != Some(&path)
            || self.sim.checksum != checksum
            || self.checksum != checksum
            || self.snapshot.as_ref().is_none_or(|snapshot| {
                snapshot.seq() == 0
                    || snapshot.tick() != 0
                    || snapshot.timeline().is_some()
                    || snapshot.predicted().checksum() != checksum
            })
        {
            return Err("Wait for the current authoritative scene and Frame before changing Collect projects".into());
        }
        Ok(CollectProjectSourceStamp {
            game: self.game(),
            path,
            checksum,
            load_epoch: self.scene_load_epoch(),
            control_generation: self.state_generation,
            history,
            scene: scene_text.to_owned(),
        })
    }
}

fn clean_state(state: &J) -> Result<(PathBuf, u64), String> {
    if state.get("mode").and_then(J::as_str) != Some("edit")
        || state.get("playing").and_then(J::as_bool) != Some(false)
        || state.get("dirty").and_then(J::as_bool) != Some(false)
        || state.get("in_tx").and_then(J::as_bool) != Some(false)
        || state.get("head_tick").and_then(J::as_u64) != Some(0)
        || state.get("epoch").and_then(J::as_u64) != Some(0)
        || state.get("session_mode").is_some()
    {
        return Err("Save the current scene and finish Play, control and transactions before changing Collect projects".into());
    }
    let path = state
        .get("scene_path")
        .and_then(J::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or("Save the current scene before changing Collect projects")?;
    let checksum = state
        .get("checksum")
        .and_then(orr_remote::wire::parse_checksum)
        .ok_or("Host did not return an authoritative scene checksum")?;
    if state
        .get("doc_checksum")
        .and_then(orr_remote::wire::parse_checksum)
        != Some(checksum)
    {
        return Err("Host document and visible scene checksums do not agree".into());
    }
    Ok((path, checksum))
}

fn clean_history(history: &J) -> Result<(), String> {
    if history.get("dirty").and_then(J::as_bool) != Some(false)
        || history.get("in_tx").and_then(J::as_bool) != Some(false)
        || history.get("can_undo").and_then(J::as_bool).is_none()
        || history.get("can_redo").and_then(J::as_bool).is_none()
        || history
            .get("entries")
            .and_then(J::as_array)
            .is_none_or(|entries| {
                entries.iter().any(|entry| {
                    entry.get("id").and_then(J::as_u64).is_none()
                        || entry.get("label").and_then(J::as_str).is_none()
                        || entry.get("origin").and_then(J::as_str).is_none()
                        || entry.get("op_count").and_then(J::as_u64).is_none()
                        || entry.get("undone").and_then(J::as_bool).is_none()
                })
            })
    {
        return Err("Current history is dirty, in a transaction or unavailable; resolve it before changing Collect projects".into());
    }
    Ok(())
}

/// The held authoring panel has no dirty accessor. Compare its accepted document
/// with the bounded, regular saved file instead. This never saves or clears undo.
pub(crate) fn saved_ui_matches(path: &Path, document: &Document) -> Result<(), String> {
    let bytes = read_saved_file(path, MAX_BYTES as u64)?;
    if &Document::parse(&bytes)? != document {
        return Err("Save the authored Collect UI or explicitly undo its edits before changing projects; if its file changed externally, reopen it first".into());
    }
    Ok(())
}

fn read_saved_file(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let parents = checked_parents(path)?;
    // The first handle pins the identity of the inspected file. `open_file` refuses a
    // link, a folder or a FIFO without waiting for a writer.
    let inspected =
        orr_fsid::open_file(path).map_err(|error| format!("Project source file: {error}"))?;
    let before = inspected.metadata().map_err(|error| error.to_string())?;
    regular_file(&before, limit)?;
    let mut file =
        orr_fsid::open_file(path).map_err(|error| format!("Project source file: {error}"))?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    regular_file(&opened, limit)?;
    let same_handle = orr_fsid::file_id(&file)
        .and_then(|opened_id| orr_fsid::file_id(&inspected).map(|first| opened_id == first))
        .map_err(|error| error.to_string())?;
    if !same_handle || !same_stamp(&before, &opened) {
        return Err("Project source file changed before reading; reopen the project".into());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let handle_after = file.metadata().map_err(|error| error.to_string())?;
    regular_file(&handle_after, limit)?;
    if bytes.len() as u64 != opened.len()
        || bytes.len() as u64 > limit
        || !same_stamp(&opened, &handle_after)
        || !matches!(orr_fsid::same_file(path, &file), Ok(true))
    {
        return Err("Project source file changed while reading; reopen the project".into());
    }
    for parent in &parents {
        if parent.verify_unchanged().is_err() {
            return Err(
                "Project source directory changed while reading; reopen the project".into(),
            );
        }
    }
    Ok(bytes)
}

fn checked_parents(path: &Path) -> Result<Vec<orr_fsid::PinnedDirectory>, String> {
    if !path.is_absolute() {
        return Err("Project source path is not absolute".into());
    }
    let parent = path.parent().ok_or("Project source file has no parent")?;
    let mut current = PathBuf::new();
    let mut result = Vec::new();
    for component in parent.components() {
        current.push(component.as_os_str());
        // Windows prefixes are not complete directory paths until RootDir.
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        // One open checks the type, refuses a link or reparse point and pins the identity.
        let directory = orr_fsid::PinnedDirectory::open(&current).map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidInput {
                "Project source parents must be non-symlink, non-reparse directories".to_string()
            } else {
                error.to_string()
            }
        })?;
        result.push(directory);
    }
    Ok(result)
}

fn regular_file(metadata: &fs::Metadata, limit: u64) -> Result<(), String> {
    if !orr_fsid::is_link(metadata) && metadata.is_file() && metadata.len() <= limit {
        return Ok(());
    }
    Err(format!(
        "Project source must remain a regular, non-symlink file within {limit} bytes"
    ))
}

/// Size and times of one file. Identity is compared by handle, not here.
fn same_stamp(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if first.ctime() != second.ctime() || first.ctime_nsec() != second.ctime_nsec() {
            return false;
        }
    }
    first.len() == second.len()
        && matches!((first.modified(), second.modified()), (Ok(first), Ok(second)) if first == second)
}

#[cfg(test)]
#[path = "collect_project_tests.rs"]
mod tests;
