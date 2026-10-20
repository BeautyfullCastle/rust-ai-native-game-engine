//! Explicit read-only Arena source receipt. This is not a lock or trust token.
use super::*;
use orr_sample::project_export::{ArenaExportSource, ArenaSourceDigest, ArenaSourceFile};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Component};

impl Editor {
    fn arena_export_idle(&self) -> Result<(), String> {
        if self.game() != EditorGame::Arena
            || !self.spec().is_local()
            || self.erp_status().is_some()
            || !self.can_mutate()
            || !self.scene_edit_allowed
            || self.mode() != Mode::Edit
            || self.previewing().is_some()
            || self.in_gesture()
            || self.batch_uncertain
            || self.down.is_some()
            || self.history.in_tx
            || self.sim.in_tx
            || self.input_phase() != input::Phase::Off
            || self.input_cleanup_pending()
            || self.is_dirty()
            || self
                .pending
                .values()
                .any(|p| matches!(p.kind, Pend::Gesture(..) | Pend::Input(..)))
        {
            return Err("Export requires a saved, idle local Arena in Edit mode without ERP listener, control, transaction, preview or pending mutation".into());
        }
        Ok(())
    }

    pub(crate) fn arena_export_source(
        &mut self,
        root: &Path,
        sprites: &crate::sprite_panel::SpritePanel,
    ) -> Result<ArenaExportSource, String> {
        self.arena_export_idle()?;
        let generations = (
            self.change_gen,
            self.state_generation,
            self.scene_load_epoch(),
        );
        let before = self.call("sim.state", J::Null)?;
        let (path, checksum) = clean_state(&before)?;
        let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
        let history = self.call("history.list", J::Null)?;
        clean_history(&history)?;
        let proposals = self.call("proposal.list", J::Null)?;
        if proposals
            .get("proposals")
            .and_then(J::as_array)
            .is_none_or(|v| !v.is_empty())
        {
            return Err("Resolve open proposals before exporting".into());
        }
        let doc = self.call("scene.save", json!({"write":false,"source":"doc"}))?;
        let text = doc
            .get("text")
            .and_then(J::as_str)
            .ok_or("Missing authoritative document")?;
        if text.len() as u64 > orr_sample::project::MAX_SCENE_BYTES
            || doc.get("dirty").and_then(J::as_bool) != Some(false)
            || doc
                .get("checksum")
                .and_then(orr_remote::wire::parse_checksum)
                != Some(checksum)
        {
            return Err("Authoritative document is dirty, oversized or incoherent".into());
        }
        let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
        let manifest_bytes = read_saved_file(&root.join("orr.project.json"), 1024 * 1024)?;
        let manifest: orr_package::ProjectManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
        let entry = manifest
            .entry
            .as_ref()
            .ok_or("Project has no authored entry")?;
        if entry.game != orr_package::ProjectGame::Arena
            || entry.ui.is_some()
            || entry.tilemap.is_some()
            || entry.models.is_some()
            || entry.camera.is_some()
        {
            return Err("Editor Arena export supports scenes and sprites only; declared UI/tilemap or other presentation is refused".into());
        }
        let mut files = vec![("orr.project.json".to_owned(), Some(manifest_bytes))];
        let lock = match fs::symlink_metadata(root.join("orr.packages.lock.json")) {
            Ok(_) => Some(read_saved_file(
                &root.join("orr.packages.lock.json"),
                1024 * 1024,
            )?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        files.push(("orr.packages.lock.json".into(), lock));
        // Validate bounded relative paths before joining or reading caller-controlled entries.
        ArenaSourceFile::new(entry.scene.clone(), None)?;
        let scene_bytes = read_saved_file(
            &root.join(&entry.scene),
            orr_sample::project::MAX_SCENE_BYTES,
        )?;
        if fs::canonicalize(root.join(&entry.scene)).map_err(|e| e.to_string())? != path {
            return Err("Current scene is not this project's canonical entry scene".into());
        }
        let saved = orr_reflect::Scene::parse(
            std::str::from_utf8(&scene_bytes).map_err(|e| e.to_string())?,
            &self.types,
        )
        .map_err(|e| e.to_string())?;
        let current = orr_reflect::Scene::parse(text, &self.types).map_err(|e| e.to_string())?;
        if saved != current {
            return Err(
                "Saved scene and authoritative document differ; save or reopen explicitly".into(),
            );
        }
        files.push((entry.scene.clone(), Some(scene_bytes)));
        if let Some(name) = &entry.sprites {
            ArenaSourceFile::new(name.clone(), None)?;
            files.push((
                name.clone(),
                Some(read_saved_file(&root.join(name), 1024 * 1024)?),
            ));
        }
        // Shared admission verifies the complete active package lock and decodes referenced assets.
        let prepared = orr_sample::project::PreparedProject::open_with_capabilities(
            &root,
            crate::sprite_bindings::compiled_runtime(),
            orr_sample::project::UiSupport::Unsupported,
            orr_sample::project::TilemapSupport::Unsupported,
        )?;
        if prepared.scene().path() != path || prepared.scene().scene() != &current {
            return Err("Project source changed during admission".into());
        }
        match (prepared.sprites(), sprites.bindings.as_ref()) {
            (None, None) => {}
            (Some(admitted), Some(loaded)) => {
                if loaded.memory_dirty()
                    || fs::canonicalize(&loaded.path).map_err(|e| e.to_string())? != admitted.path
                    || loaded.document() != &admitted.document
                    || !loaded.matches_scene(path.to_str().ok_or("Scene path must be UTF-8")?)
                {
                    return Err(
                        "Saved declared sprite bindings differ from loaded accepted bindings"
                            .into(),
                    );
                }
                sprites.arena_export_assets_match(&admitted.assets)?;
            }
            _ => return Err("Declared and loaded sprite bindings must match exactly".into()),
        }
        let mut inventory = Vec::new();
        for (name, bytes) in files {
            let actual = match fs::symlink_metadata(root.join(&name)) {
                Ok(_) => Some(read_saved_file(
                    &root.join(&name),
                    if name == entry.scene {
                        orr_sample::project::MAX_SCENE_BYTES
                    } else {
                        1024 * 1024
                    },
                )?),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.to_string()),
            };
            if actual != bytes {
                return Err("Source file changed while capturing export receipt".into());
            }
            let digest = bytes
                .map(|b| {
                    ArenaSourceDigest::new(b.len() as u64, format!("{:x}", Sha256::digest(&b)))
                })
                .transpose()?;
            inventory.push(ArenaSourceFile::new(name, digest)?);
        }
        let after_history = self.call("history.list", J::Null)?;
        let after = self.call("sim.state", J::Null)?;
        self.arena_export_idle()?;
        if before != after
            || history != after_history
            || generations
                != (
                    self.change_gen,
                    self.state_generation,
                    self.scene_load_epoch(),
                )
            || self.sim.checksum != checksum
            || self.checksum != checksum
            || self.snapshot.as_ref().is_none_or(|s| {
                s.seq() == 0
                    || s.tick() != 0
                    || s.timeline().is_some()
                    || s.predicted().checksum() != checksum
            })
        {
            return Err("Host/history/frame changed while capturing export source; retry explicitly after it settles".into());
        }
        ArenaExportSource::new(path, checksum, inventory)
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
        return Err("Save the current scene and finish Play, control and transactions before exporting Arena".into());
    }
    let path = state
        .get("scene_path")
        .and_then(J::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or("Save the current scene before exporting Arena")?;
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
        return Err("Current history is dirty, in a transaction or unavailable; resolve it before exporting Arena".into());
    }
    Ok(())
}

fn read_saved_file(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let parents = checked_parents(path)?;
    let before =
        fs::symlink_metadata(path).map_err(|error| format!("Project source file: {error}"))?;
    regular_file(&before, limit)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000 | 0x800); // O_NOFOLLOW | O_NONBLOCK
    }
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x100 | 0x4); // O_NOFOLLOW | O_NONBLOCK
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    if !cfg!(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        windows
    )) {
        return Err("Safe Project source file checks are unsupported on this platform".into());
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("Project source file: {error}"))?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    regular_file(&opened, limit)?;
    if !same_file(&before, &opened) {
        return Err("Project source file changed before reading; reopen the project".into());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let after = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let handle_after = file.metadata().map_err(|error| error.to_string())?;
    regular_file(&after, limit)?;
    regular_file(&handle_after, limit)?;
    if bytes.len() as u64 != opened.len()
        || bytes.len() as u64 > limit
        || !same_file(&before, &after)
        || !same_file(&opened, &handle_after)
    {
        return Err("Project source file changed while reading; reopen the project".into());
    }
    for (parent, before) in parents {
        let after = fs::symlink_metadata(parent).map_err(|error| error.to_string())?;
        if !safe_directory(&after) || !same_identity(&before, &after) {
            return Err(
                "Project source directory changed while reading; reopen the project".into(),
            );
        }
    }
    Ok(bytes)
}

fn checked_parents(path: &Path) -> Result<Vec<(PathBuf, fs::Metadata)>, String> {
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
        let metadata = fs::symlink_metadata(&current).map_err(|error| error.to_string())?;
        if !safe_directory(&metadata) {
            return Err(
                "Project source parents must be non-symlink, non-reparse directories".into(),
            );
        }
        result.push((current.clone(), metadata));
    }
    Ok(result)
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x0400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

fn safe_directory(metadata: &fs::Metadata) -> bool {
    metadata.is_dir() && !is_link(metadata)
}

fn regular_file(metadata: &fs::Metadata, limit: u64) -> Result<(), String> {
    if is_link(metadata) || !metadata.is_file() || metadata.len() > limit {
        return Err(format!(
            "Project source must remain a regular, non-symlink file within {limit} bytes"
        ));
    }
    Ok(())
}

fn same_identity(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        first.dev() == second.dev() && first.ino() == second.ino()
    }
    #[cfg(not(unix))]
    {
        matches!((first.created(), second.created()), (Ok(first), Ok(second)) if first == second)
    }
}

fn same_file(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if first.ctime() != second.ctime() || first.ctime_nsec() != second.ctime_nsec() {
            return false;
        }
    }
    same_identity(first, second)
        && first.len() == second.len()
        && matches!((first.modified(), second.modified()), (Ok(first), Ok(second)) if first == second)
}

#[cfg(test)]
#[path = "arena_export_tests.rs"]
mod tests;
