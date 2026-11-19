//! Generic prefab authoring for every registered game.
//!
//! A prefab is a `.prefab.yaml` scene fragment beside the open scene. The panel
//! creates one from the selection, places instances, shows what an instance
//! overrides (the inspector marks those fields), reverts overrides, writes
//! overrides back into the prefab file and updates instances when the file
//! changes. All scene changes go through the host's `prefab_instance.*`
//! methods, so each one is one undo step. File access is limited to flat
//! `.prefab.yaml` names in the scene's folder; it is checked local I/O, not a
//! sandbox against a hostile file system.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value as J};

use crate::{Editor, Target};

/// Largest prefab file the panel reads or writes.
const MAX_BYTES: u64 = 1_048_576;
const POLL_EVERY: Duration = Duration::from_millis(400);
const SUFFIX: &str = ".prefab.yaml";

/// The scene's folder, as far as prefab files are concerned.
struct PrefabFiles {
    dir: PathBuf,
    scene_name: String,
}

type Stamp = (u64, Option<SystemTime>);

impl PrefabFiles {
    fn new(scene: &Path) -> Result<Self, String> {
        let dir = scene.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
        let scene_name = scene.file_name().and_then(|n| n.to_str()).ok_or("The open scene has no usable file name")?.to_string();
        Ok(Self { dir, scene_name })
    }

    fn path(&self, name: &str) -> Result<PathBuf, String> {
        let stem = name.strip_suffix(SUFFIX).unwrap_or("");
        if stem.is_empty()
            || name.len() > 120
            || !stem.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            || stem.starts_with('.')
            || stem.ends_with('.')
        {
            return Err("Use a flat file name like boxes.prefab.yaml (letters, digits, dash, underscore, dot)".into());
        }
        let upper = stem.split('.').next().unwrap_or("").to_ascii_uppercase();
        if ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str()) || (upper.len() == 4 && (upper.starts_with("COM") || upper.starts_with("LPT")) && upper.as_bytes()[3].is_ascii_digit()) {
            return Err("Reserved file name".into());
        }
        if name.eq_ignore_ascii_case(&self.scene_name) {
            return Err("The open scene cannot be a prefab file".into());
        }
        Ok(self.dir.join(name))
    }

    fn regular(path: &Path) -> Result<std::fs::Metadata, String> {
        let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err("A prefab file must be a regular file, not a link or folder".into());
        }
        if meta.len() > MAX_BYTES {
            return Err("A prefab file must be at most 1 MiB".into());
        }
        Ok(meta)
    }

    fn stamp(&self, name: &str) -> Option<Stamp> {
        let meta = Self::regular(&self.path(name).ok()?).ok()?;
        Some((meta.len(), meta.modified().ok()))
    }

    fn read(&self, name: &str) -> Result<String, String> {
        let path = self.path(name)?;
        Self::regular(&path)?;
        String::from_utf8(std::fs::read(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }

    /// Creates a new file. An existing file is never replaced.
    fn create_new(&self, name: &str, text: &str) -> Result<(), String> {
        use std::io::Write;
        let path = self.path(name)?;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists { format!("{name} already exists; choose another name") } else { e.to_string() }
        })?;
        file.write_all(text.as_bytes()).and_then(|()| file.sync_all()).map_err(|e| e.to_string())
    }

    /// Replaces an existing regular file through a temporary file in the same folder.
    fn replace(&self, name: &str, text: &str) -> Result<(), String> {
        use std::io::Write;
        let path = self.path(name)?;
        Self::regular(&path)?;
        let temp = self.dir.join(format!(".{name}.{}.tmp", std::process::id()));
        let result = (|| {
            let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
            file.write_all(text.as_bytes()).and_then(|()| file.sync_all()).map_err(|e| e.to_string())?;
            drop(file);
            std::fs::rename(&temp, &path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }

    fn list(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| self.path(n).is_ok() && Self::regular(&self.dir.join(n)).is_ok())
            .collect();
        names.sort();
        names
    }
}

/// What the panel last read from the host, and when it must read again.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    selected: Option<String>,
    entries: usize,
    undone: usize,
    last: Option<u64>,
    checksum: u64,
}

/// The prefab panel of the main editor.
pub struct Panel {
    scene: Option<PathBuf>,
    files: Option<Result<PrefabFiles, String>>,
    /// File name for the next "Create prefab".
    pub new_name: String,
    /// Link the selection to the prefab it is saved as.
    pub link_selection: bool,
    choice: String,
    entity: Option<J>,
    instance_count: usize,
    sources: BTreeSet<String>,
    key: Option<Key>,
    force: bool,
    notice: Option<String>,
    error: Option<String>,
    conflicts: Vec<String>,
    observed: BTreeMap<String, (Stamp, String)>,
    polled: Option<Instant>,
    listed: Vec<String>,
    listed_at: Option<Instant>,
    marks: BTreeMap<String, Arc<Vec<String>>>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            scene: None,
            files: None,
            new_name: "prefab.prefab.yaml".into(),
            link_selection: true,
            choice: String::new(),
            entity: None,
            instance_count: 0,
            sources: BTreeSet::new(),
            key: None,
            force: true,
            notice: None,
            error: None,
            conflicts: Vec::new(),
            observed: BTreeMap::new(),
            polled: None,
            listed: Vec::new(),
            listed_at: None,
            marks: BTreeMap::new(),
        }
    }
}

impl Panel {
    /// Overridden field paths of the selected entity for one component.
    pub fn marks(&self, component: &str) -> Arc<Vec<String>> {
        self.marks.get(component).cloned().unwrap_or_default()
    }

    /// Conflicts reported by the last update, if any.
    pub fn conflicts(&self) -> &[String] {
        &self.conflicts
    }

    /// The last non-modal notice.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// The last error text.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn files(&self) -> Result<&PrefabFiles, String> {
        self.files.as_ref().ok_or("Open a local scene first")?.as_ref().map_err(Clone::clone)
    }

    fn available(&self, editor: &Editor) -> bool {
        !editor.game().is_collect() && !editor.is_playing_mode() && editor.spec().is_local() && editor.path().is_some()
    }

    /// Reads what the panel shows from the host when the document or selection changed.
    fn refresh(&mut self, editor: &mut Editor) {
        let history = editor.history();
        let key = Key {
            selected: editor.selected_guid().map(|g| g.to_string()),
            entries: history.entries.len(),
            undone: history.entries.iter().filter(|e| e.undone).count(),
            last: history.entries.last().map(|e| e.id),
            checksum: editor.sim().checksum,
        };
        if !self.force && self.key.as_ref() == Some(&key) {
            return;
        }
        self.force = false;
        self.key = Some(key.clone());
        self.marks.clear();
        self.entity = None;
        match editor.host_call("prefab_instance.list", json!({})) {
            Ok(list) => {
                let instances = list["instances"].as_array().cloned().unwrap_or_default();
                self.instance_count = instances.len();
                self.sources = instances.iter().filter_map(|i| i["source"].as_str().map(str::to_string)).collect();
            }
            Err(error) => {
                self.error = Some(error);
                return;
            }
        }
        if let Some(selected) = key.selected {
            if let Ok(entity) = editor.host_call("prefab_instance.entity", json!({"entity": selected})) {
                let mut by_component: BTreeMap<String, Vec<String>> = BTreeMap::new();
                for o in entity["overrides"].as_array().into_iter().flatten() {
                    if o["kind"] == "field" {
                        if let (Some(c), Some(p)) = (o["component"].as_str(), o["path"].as_str()) {
                            by_component.entry(c.to_string()).or_default().push(p.to_string());
                        }
                    }
                }
                self.marks = by_component.into_iter().map(|(c, p)| (c, Arc::new(p))).collect();
                self.entity = Some(entity);
            }
        }
    }

    /// One mutating host call with the current revision. Marks the document edited.
    fn call(&mut self, editor: &mut Editor, method: &str, mut params: J) -> Result<J, String> {
        let list = editor.host_call("prefab_instance.list", json!({}))?;
        params["expected_revision"] = list["revision"].clone();
        let result = editor.host_call(method, params)?;
        editor.prefab_mark_edited();
        self.force = true;
        Ok(result)
    }

    fn remember(&mut self, source: &str, text: &str) {
        if let Some(stamp) = self.files.as_ref().and_then(|f| f.as_ref().ok()).and_then(|f| f.stamp(source)) {
            self.observed.insert(source.to_string(), (stamp, text.to_string()));
        }
    }

    /// Updates every instance of `source` to `text` and reports what happened.
    fn update(&mut self, editor: &mut Editor, source: &str, text: &str, why: &str) -> Result<(), String> {
        self.remember(source, text);
        let result = self.call(editor, "prefab_instance.update", json!({"source": source, "text": text}))?;
        let report = &result["report"];
        let count = |k: &str| report[k].as_u64().unwrap_or(0);
        self.conflicts = report["conflicts"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_string)).collect();
        let updated = report["updated"].as_array().map_or(0, Vec::len);
        let message = if updated == 0 {
            format!("{source}: instances already match the prefab")
        } else {
            let mut m = format!("{source} {why}: updated {updated} instance(s), {} entity(s) added, {} removed", count("spawned"), count("removed"));
            if !self.conflicts.is_empty() {
                m.push_str(&format!("; {} conflict(s), the instance value was kept", self.conflicts.len()));
            }
            m
        };
        editor.info(message.clone());
        self.notice = Some(message);
        Ok(())
    }

    /// Reads changed prefab files and updates the instances of each.
    fn poll(&mut self, editor: &mut Editor) {
        if self.polled.is_some_and(|t| t.elapsed() < POLL_EVERY) || !editor.can_mutate() || editor.previewing().is_some() || editor.sim().in_tx {
            return;
        }
        self.polled = Some(Instant::now());
        for source in self.sources.clone() {
            let Ok(files) = self.files() else { return };
            let stamp = files.stamp(&source);
            if stamp.is_some() && self.observed.get(&source).map(|o| o.0) == stamp {
                continue;
            }
            let Some(stamp) = stamp else {
                if !self.observed.contains_key(&source) {
                    self.error = Some(format!("Prefab file {source} is missing; its instances keep their last values"));
                    self.observed.insert(source, ((0, None), String::new()));
                }
                continue;
            };
            let text = match files.read(&source) {
                Ok(text) => text,
                Err(error) => {
                    self.error = Some(format!("{source}: {error}"));
                    self.observed.insert(source, (stamp, String::new()));
                    continue;
                }
            };
            if self.observed.get(&source).is_some_and(|o| o.1 == text) {
                self.observed.insert(source, (stamp, text));
                continue;
            }
            self.observed.insert(source.clone(), (stamp, text.clone()));
            match self.update(editor, &source, &text, "changed on disk") {
                Ok(()) => self.error = None,
                Err(error) => self.error = Some(format!("{source}: {error}")),
            }
        }
    }

    /// Restores the prefab value of one overridden field of the selected entity.
    pub fn revert_field(&mut self, editor: &mut Editor, component: &str, path: &str) {
        let Some(entity) = editor.selected_guid() else { return };
        match self.call(editor, "prefab_instance.revert", json!({"entity": entity.to_string(), "component": component, "path": path})) {
            Ok(_) => self.error = None,
            Err(error) => self.error = Some(error),
        }
    }

    fn create(&mut self, editor: &mut Editor) -> Result<(), String> {
        let name = self.new_name.clone();
        self.files()?.path(&name)?;
        let selected: Vec<String> = editor.selected_guids().iter().map(ToString::to_string).collect();
        let made = editor.host_call("prefab_instance.create", json!({"selected": selected}))?;
        let text = made["text"].as_str().ok_or("Malformed create response")?.to_string();
        self.files()?.create_new(&name, &text)?;
        self.remember(&name, &text);
        if self.link_selection {
            self.call(editor, "prefab_instance.link", json!({"source": name, "text": text, "guids": made["guids"]}))?;
        }
        self.listed_at = None;
        self.notice = Some(format!("Saved {name}{}", if self.link_selection { "; the selection is now an instance of it" } else { "" }));
        Ok(())
    }

    fn instantiate(&mut self, editor: &mut Editor, name: &str) -> Result<(), String> {
        let text = self.files()?.read(name)?;
        let result = self.call(editor, "prefab_instance.instantiate", json!({"source": name, "text": text}))?;
        self.remember(name, &text);
        if let Some(root) = result["instance"].as_str().and_then(|g| orr_reflect::Guid::parse(g).ok()) {
            editor.select(Some(Target::Guid(root)));
        }
        self.notice = Some(format!("Placed an instance of {name}"));
        Ok(())
    }

    fn apply(&mut self, editor: &mut Editor, entity: &str) -> Result<(), String> {
        let made = editor.host_call("prefab_instance.apply_text", json!({"entity": entity}))?;
        let source = made["source"].as_str().ok_or("Malformed apply response")?.to_string();
        let text = made["text"].as_str().ok_or("Malformed apply response")?.to_string();
        let files = self.files()?;
        let on_disk = files.read(&source)?;
        if self.observed.get(&source).is_some_and(|o| o.1 != on_disk) {
            return Err(format!("{source} changed on disk; update the instances from it before applying"));
        }
        files.replace(&source, &text)?;
        self.update(editor, &source, &text, "applied")
    }

    fn update_from_file(&mut self, editor: &mut Editor, source: &str) -> Result<(), String> {
        let text = self.files()?.read(source)?;
        self.update(editor, source, &text, "updated from file")
    }

    /// Draws the panel.
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        if !self.available(editor) {
            self.marks.clear();
            return;
        }
        let scene = editor.path();
        if self.scene != scene {
            self.scene = scene.clone();
            self.files = scene.as_deref().map(PrefabFiles::new);
            self.key = None;
            self.force = true;
            self.observed.clear();
            self.notice = None;
            self.error = None;
            self.conflicts.clear();
            self.listed_at = None;
        }
        self.refresh(editor);
        self.poll(editor);
        if self.listed_at.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
            self.listed = self.files.as_ref().and_then(|f| f.as_ref().ok()).map(PrefabFiles::list).unwrap_or_default();
            self.listed_at = Some(Instant::now());
        }
        let editable = editor.can_mutate() && editor.previewing().is_none();
        crate::theme::header("Prefabs").default_open(true).show(ui, |ui| {
            if let Some(Err(error)) = &self.files {
                ui.colored_label(crate::theme::ERROR, error);
            }
            ui.add_enabled_ui(editable, |ui| {
                let member = self.entity.as_ref().filter(|e| e["member"] == true).cloned();
                if let Some(member) = member {
                    let source = member["source"].as_str().unwrap_or("?").to_string();
                    let entity = editor.selected_guid().map(|g| g.to_string()).unwrap_or_default();
                    let overrides = member["instance_override_count"].as_u64().unwrap_or(0);
                    ui.label(format!("Instance of {source}"));
                    ui.label(format!("{overrides} override(s) on this instance"));
                    if ui.add_enabled(overrides > 0, egui::Button::new("Revert all overrides")).on_hover_text("Restore every prefab value of this instance (one undo step)").clicked() {
                        self.error = self.call(editor, "prefab_instance.revert", json!({"entity": entity})).err();
                    }
                    if ui.add_enabled(overrides > 0, egui::Button::new("Apply to prefab")).on_hover_text("Write this instance's overrides into the prefab file; every instance of it updates").clicked() {
                        self.error = self.apply(editor, &entity).err();
                    }
                    if ui.button("Update from prefab file").on_hover_text("Read the prefab file again and update the instances").clicked() {
                        self.error = self.update_from_file(editor, &source).err();
                    }
                    if ui.button("Detach instance").on_hover_text("Keep the entities and values, remove the link to the prefab (one undo step)").clicked() {
                        self.error = self.call(editor, "prefab_instance.detach", json!({"entity": entity})).err();
                    }
                } else if !editor.selected_guids().is_empty() {
                    ui.label("Create a prefab file from the selected entities");
                    ui.text_edit_singleline(&mut self.new_name);
                    ui.checkbox(&mut self.link_selection, "Link the selection to the new prefab");
                    if ui.button("Create prefab from selection").on_hover_text("Saves the selection as a new .prefab.yaml beside the scene. An entity that refers to an unselected entity is refused. Existing files are never replaced.").clicked() {
                        self.error = self.create(editor).err();
                    }
                }
                ui.separator();
                if self.choice.is_empty() || !self.listed.contains(&self.choice) {
                    self.choice = self.listed.first().cloned().unwrap_or_default();
                }
                egui::ComboBox::from_id_salt("prefab_choice").selected_text(if self.choice.is_empty() { "(no prefab files)" } else { self.choice.as_str() }).show_ui(ui, |ui| {
                    for name in &self.listed {
                        ui.selectable_value(&mut self.choice, name.clone(), name);
                    }
                });
                if ui.add_enabled(!self.choice.is_empty(), egui::Button::new("Instantiate prefab")).on_hover_text("Place a new instance with fresh entity ids").clicked() {
                    let name = self.choice.clone();
                    self.error = self.instantiate(editor, &name).err();
                }
                ui.weak(format!("{} prefab instance(s) in this scene", self.instance_count));
            });
            for conflict in &self.conflicts {
                ui.colored_label(crate::theme::WARN, conflict);
            }
            if let Some(notice) = &self.notice {
                ui.label(notice);
            }
            if let Some(error) = &self.error {
                ui.colored_label(crate::theme::ERROR, error);
            }
        });
    }
}
