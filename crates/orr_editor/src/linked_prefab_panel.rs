//! Explicit, host-authoritative linked-prefab authoring. No watcher or executable content.
//! Source authoring is Linux-first; platforms unable to pin directories fail closed.
//! Rechecks detect path replacement, but do not promise hostile-race isolation.
use crate::{game::EditorGame, Editor};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_BYTES: usize = 32 * 1024;

/// A pinned scene directory. This first profile deliberately accepts only flat,
/// portable source filenames beside the scene, never arbitrary filesystem paths.
pub struct SourceFiles {
    root: PathBuf,
    ancestors: Vec<orr_fsid::PinnedDirectory>,
    protected_scene: Option<(String, PathBuf)>,
}
impl SourceFiles {
    pub fn new(root: &Path) -> Result<Self, String> {
        let absolute = if root.is_absolute() {
            root.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(root)
        };
        let mut current = PathBuf::new();
        let mut ancestors = Vec::new();
        for component in absolute.components() {
            if component == Component::CurDir {
                continue;
            }
            if component == Component::ParentDir {
                return Err("Source directory must not contain parent traversal".into());
            }
            current.push(component);
            if matches!(component, Component::Prefix(_)) {
                continue;
            }
            let pinned = orr_fsid::PinnedDirectory::open(&current).map_err(|error| {
                if error.kind() == std::io::ErrorKind::InvalidInput {
                    "Source directory must contain only real directories".to_string()
                } else {
                    error.to_string()
                }
            })?;
            ancestors.push(pinned);
        }
        Ok(Self {
            root: current,
            ancestors,
            protected_scene: None,
        })
    }
    /// Bind source I/O to a local scene while excluding the active scene itself,
    /// including case aliases and regular-file aliases of its current identity.
    pub fn new_for_scene(scene: &Path) -> Result<Self, String> {
        let mut files = Self::new(scene.parent().unwrap_or(Path::new(".")))?;
        let name = scene
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Active scene must have a valid filename")?
            .to_owned();
        let path = files.root.join(&name);
        let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Active scene must remain a regular, non-symlink file".into());
        }
        files.protected_scene = Some((name, path));
        Ok(files)
    }
    fn recheck(&self) -> Result<(), String> {
        for directory in &self.ancestors {
            if directory.verify_unchanged().is_err() {
                return Err("Source directory changed; reopen the scene".into());
            }
        }
        Ok(())
    }
    fn path(&self, name: &str) -> Result<PathBuf, String> {
        self.recheck()?;
        if name.is_empty()
            || name.len() > 120
            || name.starts_with('.')
            || name.ends_with('.')
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        {
            return Err(
                "Use a portable filename beside the scene (letters, digits, dash, underscore, dot)"
                    .into(),
            );
        }
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err("Reserved filename".into());
        }
        let path = self.root.join(name);
        if let Some((scene_name, scene_path)) = &self.protected_scene {
            if name.eq_ignore_ascii_case(scene_name) {
                return Err("The active scene cannot be used as a prefab source".into());
            }
            let scene = orr_fsid::open_file(scene_path).map_err(|_| {
                "Active scene changed; reopen it before editing prefab sources".to_string()
            })?;
            // Identity comes from open handles, so a hard link to the scene is an alias.
            // A missing or non-regular candidate is not an alias; the read refuses it.
            match orr_fsid::same_file(&path, &scene) {
                Ok(true) => {
                    return Err(
                        "A file alias of the active scene cannot be used as a prefab source".into(),
                    );
                }
                Ok(false) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidInput
                    ) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(path)
    }
    pub fn read(&self, name: &str) -> Result<String, String> {
        let path = self.path(name)?;
        // `open_file` refuses a link, a folder or a FIFO without waiting for a writer.
        let mut file = orr_fsid::open_file(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidInput {
                "Source must be a regular, non-symlink file of at most 32 KiB".to_string()
            } else {
                error.to_string()
            }
        })?;
        let opened = file.metadata().map_err(|e| e.to_string())?;
        if opened.len() > MAX_BYTES as u64 {
            return Err("Source must be a regular, non-symlink file of at most 32 KiB".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        self.recheck()?;
        let after = file.metadata().map_err(|e| e.to_string())?;
        if !matches!(orr_fsid::same_file(&path, &file), Ok(true))
            || after.len() != bytes.len() as u64
            || opened.modified().ok() != after.modified().ok()
            || bytes.len() > MAX_BYTES
        {
            return Err("Source file changed or exceeds 32 KiB".into());
        }
        String::from_utf8(bytes).map_err(|e| e.to_string())
    }
    /// Explicit replacement only for prefab filenames, guarded by the exact
    /// bytes read when the operation started. No overwrite is implied by capture.
    pub fn replace_source(&self, name: &str, expected: &str, text: &str) -> Result<(), String> {
        if !name.ends_with(".prefab.yaml") || name.len() <= ".prefab.yaml".len() {
            return Err("Replacement requires a .prefab.yaml source filename".into());
        }
        if text.len() > MAX_BYTES || expected.len() > MAX_BYTES {
            return Err("Source exceeds 32 KiB".into());
        }
        let path = self.path(name)?;
        let refused =
            || "Replacement requires an existing writable regular, non-symlink source".to_string();
        let pinned = orr_fsid::open_file(&path).map_err(|_| refused())?;
        let before = pinned.metadata().map_err(|e| e.to_string())?;
        if before.permissions().readonly() {
            return Err(refused());
        }
        if self.read(name)? != expected {
            return Err("Source changed on disk; replacement canceled".into());
        }
        let mut temp = tempfile::NamedTempFile::new_in(&self.root).map_err(|e| e.to_string())?;
        temp.as_file()
            .set_permissions(before.permissions())
            .map_err(|e| e.to_string())?;
        temp.write_all(text.as_bytes())
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        self.recheck()?;
        if self.read(name)? != expected {
            return Err("Source changed while preparing replacement; replacement canceled".into());
        }
        let after = pinned.metadata().map_err(|e| e.to_string())?;
        if after.permissions().readonly()
            || !matches!(orr_fsid::same_file(&path, &pinned), Ok(true))
        {
            return Err("Source identity or permissions changed; replacement canceled".into());
        }
        // Release the handle before the replacement so it cannot block the rename.
        drop(pinned);
        self.path(name)?;
        // Same checked-path race limitations as reads; the actual replacement is atomic.
        temp.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn save_new(&self, name: &str, text: &str) -> Result<(), String> {
        if text.len() > MAX_BYTES {
            return Err("Source exceeds 32 KiB".into());
        }
        let path = self.path(name)?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.root).map_err(|e| e.to_string())?;
        temp.write_all(text.as_bytes())
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        self.recheck()?;
        // Atomic no-replace: never overwrites a scene, source, symlink or other file.
        temp.persist_noclobber(path).map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub struct Panel {
    scene: Option<PathBuf>,
    files: Option<Result<SourceFiles, String>>,
    pub source: String,
    pub position: [String; 2],
    state: Option<Value>,
    error: Option<String>,
    notice: Option<String>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            scene: None,
            files: None,
            source: "actors.prefab.yaml".into(),
            position: ["0".into(), "0".into()],
            state: None,
            error: None,
            notice: None,
        }
    }
}
impl Panel {
    pub fn state(&self) -> Option<&Value> {
        self.state.as_ref()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    fn files(&self) -> Result<&SourceFiles, String> {
        self.files
            .as_ref()
            .ok_or("Open a local scene first")?
            .as_ref()
            .map_err(Clone::clone)
    }
    fn refresh(&mut self, editor: &mut Editor) -> Result<(), String> {
        let candidate = editor.host_call("prefab.list", json!({}))?;
        if candidate["revision"].as_u64().is_none() || !candidate["instances"].is_array() {
            return Err("Malformed prefab response".into());
        }
        self.state = Some(candidate);
        Ok(())
    }
    fn mutate(
        &mut self,
        editor: &mut Editor,
        method: &str,
        mut params: Value,
    ) -> Result<(), String> {
        params["expected_revision"] =
            self.state.as_ref().ok_or("Refresh linked prefabs first")?["revision"].clone();
        let candidate = editor.host_call(method, params)?;
        if candidate["revision"].as_u64().is_none() || !candidate["instances"].is_array() {
            return Err("Malformed prefab response; refresh before retrying".into());
        }
        self.state = Some(candidate);
        editor.prefab_mark_edited();
        self.notice = Some("Linked prefab change applied; save the scene to persist".into());
        Ok(())
    }
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        if editor.game() != EditorGame::CollectDodge
            || editor.is_playing_mode()
            || !editor.spec().is_local()
        {
            return;
        }
        let scene = editor.path();
        if self.scene != scene {
            self.scene = scene.clone();
            self.files = scene.as_ref().map(|p| SourceFiles::new_for_scene(p));
            self.state = None;
            self.error = None;
            self.notice = None;
        }
        crate::theme::header("Linked prefabs").default_open(true).show(ui, |ui| {
            ui.label("CollectDodgeV1 · 1–8 non-player actors · fixed topology");
            ui.label("Source filename (beside scene)");
            ui.text_edit_singleline(&mut self.source);
            if let Some(Err(error)) = &self.files { ui.colored_label(egui::Color32::RED, error); }
            ui.add_enabled_ui(editor.can_mutate() && editor.spec().is_local(), |ui| {
                if ui.button("Refresh linked prefabs").clicked() { self.error = self.refresh(editor).err(); }
                if ui.button("Capture selected to new source").clicked() {
                    let result = (|| {
                        self.files()?.path(&self.source)?;
                        let selected: Vec<String> = editor.selected_guids().iter().map(ToString::to_string).collect();
                        let result = editor.host_call("prefab.capture", json!({"selected":selected}))?;
                        let text = result["text"].as_str().ok_or("Malformed capture response")?;
                        self.files()?.save_new(&self.source, text)?;
                        self.notice = Some("Canonical source saved; existing files are never overwritten".into());
                        self.refresh(editor)
                    })();
                    self.error = result.err();
                }
                if ui.button("Replace source from selection").on_hover_text("Explicitly replace this .prefab.yaml source with the selected unlinked actors. Instances change only after Apply source update.").clicked() {
                    let result = (|| {
                        let expected = self.files()?.read(&self.source)?;
                        let selected: Vec<String> = editor.selected_guids().iter().map(ToString::to_string).collect();
                        let result = editor.host_call("prefab.capture", json!({"selected":selected}))?;
                        let text = result["text"].as_str().ok_or("Malformed capture response")?;
                        self.files()?.replace_source(&self.source, &expected, text)?;
                        self.notice = Some("Source explicitly replaced; use Apply source update for each instance".into());
                        self.refresh(editor)
                    })();
                    self.error = result.err();
                }
                if ui.add_enabled(self.state.is_some(), egui::Button::new("Instantiate source")).clicked() {
                    let result = (|| { let text = self.files()?.read(&self.source)?;
                        self.mutate(editor, "prefab.instantiate", json!({"source":self.source,"text":text})) })();
                    self.error = result.err();
                }
                ui.horizontal(|ui| { ui.label("Position override x/y"); ui.add(egui::TextEdit::singleline(&mut self.position[0]).desired_width(55.0)); ui.add(egui::TextEdit::singleline(&mut self.position[1]).desired_width(55.0)); });
                let instances = self.state.as_ref().and_then(|v| v["instances"].as_array()).cloned().unwrap_or_default();
                for instance in instances {
                    let id = instance["instance"].clone();
                    ui.push_id(id.to_string(), |ui| {
                        ui.separator();
                        ui.label(format!("Instance {id}"));
                        ui.label(format!("Source: {}", instance["source"].as_str().unwrap_or("?")));
                        ui.label(format!("Digest: {}", instance["digest"]));
                        if ui.button("Apply source update").clicked() {
                            let result = (|| {
                                let source = instance["source"].as_str().ok_or("Missing source identity")?;
                                let text = self.files()?.read(source)?;
                                self.mutate(editor, "prefab.update", json!({"instance":id,"source":source,"digest":instance["digest"],"text":text}))
                            })();
                            self.error = result.err();
                        }
                        if ui.button("Detach instance").on_hover_text("Keep these actors and all current values as independent scene entities. Remove only this source link; the source file stays untouched. Undo restores the link.").clicked() {
                            self.error = self.mutate(editor, "prefab.detach", json!({"instance":id})).err();
                        }
                        if let Some(guids) = instance["guids"].as_object() {
                            for (source_guid, target) in guids {
                                ui.push_id(source_guid, |ui| {
                                    let overridden = instance["position_overrides"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(source_guid)));
                                    ui.label(format!("{source_guid} → {target}; ordinal {}; {}", instance["ordinals"][source_guid], if overridden { "position override" } else { "inherited position" }));
                                    if ui.button("Override position").clicked() {
                                        let result = (|| {
                                            let values = self.position.iter().map(|text| {
                                                let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
                                                if !value.is_number() { return Err("Position requires decimal numbers".to_string()); }
                                                Ok(value)
                                            }).collect::<Result<Vec<_>, String>>()?;
                                            self.mutate(editor, "prefab.position", json!({"instance":id,"source_guid":source_guid,"value":values}))
                                        })();
                                        self.error = result.err();
                                    }
                                    if ui.add_enabled(overridden, egui::Button::new("Revert position")).clicked() {
                                        self.error = self.mutate(editor, "prefab.revert", json!({"instance":id,"source_guid":source_guid})).err();
                                    }
                                });
                            }
                        }
                    });
                }
            });
            if let Some(error) = &self.error { ui.colored_label(egui::Color32::RED, format!("Prefab conflict/error: {error}")); }
            if let Some(notice) = &self.notice { ui.label(notice); }
        });
    }
}

#[cfg(test)]
mod source_folder_tests {
    use super::*;

    #[test]
    fn source_files_detect_a_replaced_scene_folder() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap().join("scene folder");
        std::fs::create_dir(&root).unwrap();
        let files = SourceFiles::new(&root).unwrap();
        files.path("prefab.json").unwrap();
        std::fs::rename(&root, root.with_file_name("moved")).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert!(files.path("prefab.json").is_err());
    }

    #[test]
    fn source_files_refuse_a_file_as_the_scene_folder() {
        let temp = tempfile::tempdir().unwrap();
        let file = std::fs::canonicalize(temp.path()).unwrap().join("not a folder");
        std::fs::write(&file, b"x").unwrap();
        assert!(SourceFiles::new(&file).is_err());
    }
}
