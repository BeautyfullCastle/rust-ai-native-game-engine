use crate::{backend::HostSpec, editor::Editor, game::EditorGame, mesh_hud::Document, model::Mode};
use egui::Ui;
use orr_remote::mesh_navigation::mesh_from_view;
use std::path::{Path, PathBuf};

const MAX_HISTORY: usize = 16;

#[derive(Clone)]
struct HistoryEntry {
    id: u64,
    document: Option<Document>,
}

pub(crate) struct Panel {
    enabled: bool,
    title: String,
    idle_text: String,
    moving_text: String,
    arrived_text: String,
    show_tick: bool,
    font_size: String,
    text_color: [String; 4],
    background_color: [String; 4],
    form_dirty: bool,
    confirm_remove: bool,
    confirm_discard_changes: bool,
    confirm_discard_invalid: bool,
    history: Vec<HistoryEntry>,
    cursor: usize,
    next_id: u64,
    saved_id: Option<u64>,
    scene: Option<PathBuf>,
    scene_epoch: Option<u64>,
    graph_revision: Option<[u8; 32]>,
    document_pending: bool,
    context_stale: bool,
    sidecar_sha: Option<String>,
    sidecar_error: Option<String>,
    status: String,
    error: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        let document = Document::default();
        Self {
            enabled: document.enabled,
            title: document.title.clone(),
            idle_text: document.idle_text.clone(),
            moving_text: document.moving_text.clone(),
            arrived_text: document.arrived_text.clone(),
            show_tick: document.show_tick,
            font_size: format_number(document.font_size),
            text_color: document.text_color.map(format_number),
            background_color: document.background_color.map(format_number),
            form_dirty: false,
            confirm_remove: false,
            confirm_discard_changes: false,
            confirm_discard_invalid: false,
            history: vec![HistoryEntry {
                id: 0,
                document: None,
            }],
            cursor: 0,
            next_id: 1,
            saved_id: Some(0),
            scene: None,
            scene_epoch: None,
            graph_revision: None,
            document_pending: true,
            context_stale: false,
            sidecar_sha: None,
            sidecar_error: None,
            status: String::new(),
            error: None,
        }
    }
}

impl Panel {
    pub(crate) fn sync(&mut self, editor: &Editor) {
        if editor.game() != EditorGame::MeshNavigation || !editor.spec().is_local() {
            self.document_pending = true;
            return;
        }
        let Some((scene, graph_revision)) = current_identity(editor) else {
            self.document_pending = true;
            self.status = "Waiting for the requested local mesh scene and admitted graph.".into();
            return;
        };
        self.document_pending = false;
        let scene_epoch = editor.scene_load_epoch();
        if self.scene.as_ref() == Some(&scene)
            && self.scene_epoch == Some(scene_epoch)
            && self.graph_revision == Some(graph_revision)
        {
            return;
        }
        if self.dirty() {
            self.context_stale = true;
            self.status = "Unsaved game HUD belongs to another scene or graph revision; explicitly discard and reload before continuing.".into();
            return;
        }
        self.scene = Some(scene.clone());
        self.scene_epoch = Some(scene_epoch);
        self.graph_revision = Some(graph_revision);
        self.context_stale = false;
        self.sidecar_sha = None;
        self.sidecar_error = None;
        self.error = None;
        self.confirm_remove = false;
        self.confirm_discard_changes = false;
        self.confirm_discard_invalid = false;
        self.reset_history(None, Some(0));
        self.load_context(&scene, graph_revision, scene_epoch);
    }

    pub(crate) fn show(&mut self, ui: &mut Ui, editor: &Editor) {
        if editor.game() != EditorGame::MeshNavigation {
            return;
        }
        self.sync(editor);
        ui.collapsing("Mesh game HUD", |ui| {
            if self.document_pending {
                ui.label("Waiting for the requested local mesh scene and admitted graph.");
            }
            ui.label("Optional scene-local HUD presents route state only; it does not change simulation state or declare victory.");
            let can_edit = self.can_edit(editor);
            ui.add_enabled_ui(can_edit, |ui| {
                let mut changed = false;
                changed |= ui.checkbox(&mut self.enabled, "Show mesh game HUD").changed();
                changed |= text_field(ui, "HUD title", &mut self.title);
                changed |= text_field(ui, "HUD idle text", &mut self.idle_text);
                changed |= text_field(ui, "HUD moving text", &mut self.moving_text);
                changed |= text_field(ui, "HUD arrived text", &mut self.arrived_text);
                changed |= text_field(ui, "HUD font size", &mut self.font_size);
                changed |= ui.checkbox(&mut self.show_tick, "Show HUD tick").changed();
                for (channel, label) in ["HUD text color R", "HUD text color G", "HUD text color B", "HUD text color A"]
                    .into_iter()
                    .enumerate()
                {
                    changed |= text_field(ui, label, &mut self.text_color[channel]);
                }
                for (channel, label) in ["HUD background color R", "HUD background color G", "HUD background color B", "HUD background color A"]
                    .into_iter()
                    .enumerate()
                {
                    changed |= text_field(ui, label, &mut self.background_color[channel]);
                }
                if changed {
                    self.form_dirty = true;
                    self.error = None;
                    self.status = "HUD form changed; apply it before Play or save.".into();
                }
                if ui.add_enabled(self.form_dirty || self.current_document().is_none(), egui::Button::new("Apply game HUD")).clicked() {
                    self.apply(editor);
                }
            });

            let history_enabled = self.can_change_history(editor);
            ui.horizontal(|ui| {
                if ui.add_enabled(history_enabled && self.cursor > 0, egui::Button::new("Undo HUD edit")).clicked() {
                    self.undo();
                }
                if ui.add_enabled(history_enabled && self.cursor + 1 < self.history.len(), egui::Button::new("Redo HUD edit")).clicked() {
                    self.redo();
                }
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(self.can_save(editor) && self.history_is_dirty(), egui::Button::new("Save game HUD")).clicked() {
                    self.save();
                }
                if ui.add_enabled(self.can_reload(editor) && !self.dirty(), egui::Button::new("Reload game HUD")).clicked() {
                    self.reload(editor);
                }
            });
            ui.vertical(|ui| {
                self.confirm_remove = self.confirm_remove && self.current_document().is_some();
                ui.checkbox(&mut self.confirm_remove, "I confirm removing the saved game HUD");
                if ui.add_enabled(self.can_edit(editor) && self.current_document().is_some() && self.confirm_remove, egui::Button::new("Remove game HUD")).clicked() {
                    self.remove();
                }
            });
            if self.context_stale || self.dirty() {
                ui.checkbox(&mut self.confirm_discard_changes, "I confirm discarding unsaved game HUD changes");
                if ui.add_enabled(self.can_discard_changes(editor) && self.confirm_discard_changes, egui::Button::new("Discard game HUD changes and reload")).clicked() {
                    self.discard_changes_and_reload(editor);
                }
            }
            if self.sidecar_error.is_some() {
                ui.checkbox(&mut self.confirm_discard_invalid, "I confirm discarding the invalid game HUD sidecar");
                if ui.add_enabled(self.can_discard_invalid(editor) && self.confirm_discard_invalid, egui::Button::new("Discard invalid game HUD sidecar")).clicked() {
                    self.discard_invalid();
                }
            }
            if self.dirty() {
                ui.colored_label(egui::Color32::YELLOW, "Unsaved HUD form or sidecar history blocks Play and export.");
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            } else if !self.status.is_empty() {
                ui.label(&self.status);
            }
        });
    }

    pub(crate) fn dirty(&self) -> bool {
        self.form_dirty || self.context_stale || self.history_is_dirty()
    }

    pub(crate) fn applied_document(&self, editor: &Editor) -> Option<Document> {
        if self.document_pending
            || self.context_stale
            || self.sidecar_error.is_some()
            || self.form_dirty
            || !self.identity_matches(editor)
        {
            return None;
        }
        self.current_document().cloned()
    }

    pub(crate) fn play_blocker(&self, editor: &Editor) -> Option<String> {
        if editor.game() != EditorGame::MeshNavigation {
            return None;
        }
        if self.document_pending || current_identity(editor).is_none() {
            return Some("Wait for the requested local mesh scene and admitted graph".into());
        }
        if self.context_stale {
            return Some("Discard or resolve HUD from the previous scene or graph revision".into());
        }
        if let Some(error) = &self.sidecar_error {
            return Some(format!("Saved game HUD is invalid: {error}"));
        }
        if self.dirty() {
            return Some("Apply, save, or discard game HUD changes before Play".into());
        }
        if self.scene_epoch != Some(editor.scene_load_epoch()) || !self.identity_matches(editor) {
            return Some("Game HUD does not match the current mesh scene and graph".into());
        }
        None
    }

    fn history_is_dirty(&self) -> bool {
        self.history.get(self.cursor).map(|entry| entry.id) != self.saved_id
    }

    fn current_document(&self) -> Option<&Document> {
        self.history.get(self.cursor)?.document.as_ref()
    }

    fn can_edit(&self, editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && !editor.in_gesture()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && !self.context_stale
            && self.sidecar_error.is_none()
            && !self.document_pending
            && self.scene.is_some()
            && self.graph_revision.is_some()
            && self.identity_matches(editor)
    }

    fn can_change_history(&self, editor: &Editor) -> bool {
        self.can_edit(editor)
    }

    fn can_save(&self, editor: &Editor) -> bool {
        self.can_edit(editor) && !self.form_dirty
    }

    fn can_reload(&self, editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && !editor.in_gesture()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && !self.document_pending
            && current_identity(editor).is_some()
    }

    fn can_discard_changes(&self, editor: &Editor) -> bool {
        self.can_reload(editor) && (self.context_stale || self.dirty())
    }

    fn can_discard_invalid(&self, editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && !editor.in_gesture()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && !self.document_pending
            && self.sidecar_sha.is_some()
            && self.sidecar_error.is_some()
            && self.identity_matches(editor)
    }

    fn identity_matches(&self, editor: &Editor) -> bool {
        current_identity(editor).is_some_and(|(scene, revision)| {
            self.scene.as_ref() == Some(&scene)
                && self.scene_epoch == Some(editor.scene_load_epoch())
                && self.graph_revision == Some(revision)
        })
    }

    fn parse_document(&self, scene: &Path, graph_revision: [u8; 32]) -> Result<Document, String> {
        let document = Document {
            enabled: self.enabled,
            title: self.title.clone(),
            idle_text: self.idle_text.clone(),
            moving_text: self.moving_text.clone(),
            arrived_text: self.arrived_text.clone(),
            show_tick: self.show_tick,
            font_size: parse_f32(&self.font_size, "font size")?,
            text_color: parse_color(&self.text_color, "text color")?,
            background_color: parse_color(&self.background_color, "background color")?,
        };
        document.validate()?;
        let filename = scene
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("scene filename must be Unicode")?;
        document.encode(filename, graph_revision)?;
        Ok(document)
    }

    fn apply(&mut self, editor: &Editor) {
        if !self.can_edit(editor) {
            self.error = Some("Apply requires the matching editable local mesh scene".into());
            return;
        }
        let (Some(scene), Some(graph_revision)) = (self.scene.as_deref(), self.graph_revision)
        else {
            self.error = Some("The current mesh scene is not ready for HUD changes".into());
            return;
        };
        let document = match self.parse_document(scene, graph_revision) {
            Ok(document) => document,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        self.push_state(Some(document));
        self.copy_current_fields();
        self.form_dirty = false;
        self.error = None;
        self.status = "Game HUD applied in memory; save explicitly to persist.".into();
    }

    fn save(&mut self) {
        let (Some(scene), Some(graph_revision)) = (self.scene.as_deref(), self.graph_revision)
        else {
            self.error = Some("Save the mesh scene before saving its HUD".into());
            return;
        };
        match crate::mesh_hud_store::save(
            scene,
            self.current_document(),
            self.sidecar_sha.as_deref(),
            graph_revision,
        ) {
            Ok(sidecar_sha) => {
                self.sidecar_sha = sidecar_sha;
                self.saved_id = self.history.get(self.cursor).map(|entry| entry.id);
                self.sidecar_error = None;
                self.confirm_remove = false;
                self.error = None;
                self.status = if self.current_document().is_some() {
                    "Game HUD saved for this scene and graph.".into()
                } else {
                    "Game HUD removed from the scene sidecar.".into()
                };
            }
            Err(error) => self.error = Some(format!("Save game HUD failed: {error}")),
        }
    }

    fn reload(&mut self, editor: &Editor) {
        if !self.can_reload(editor) || self.dirty() {
            self.error = Some(
                "Reload requires a clean HUD state in the current editable local scene".into(),
            );
            return;
        }
        let Some((scene, graph_revision)) = current_identity(editor) else {
            self.error = Some("The current mesh scene is not ready for HUD reload".into());
            return;
        };
        self.load_context(&scene, graph_revision, editor.scene_load_epoch());
    }

    fn remove(&mut self) {
        if self.current_document().is_none() || !self.confirm_remove {
            self.error =
                Some("Remove requires confirmation and a saved or applied game HUD".into());
            return;
        }
        self.push_state(None);
        self.copy_current_fields();
        self.form_dirty = false;
        self.confirm_remove = false;
        self.error = None;
        self.status = "Game HUD removed in memory; save explicitly to persist.".into();
    }

    fn discard_changes_and_reload(&mut self, editor: &Editor) {
        if !self.confirm_discard_changes {
            return;
        }
        let Some((scene, graph_revision)) = current_identity(editor) else {
            return;
        };
        self.scene = None;
        self.scene_epoch = None;
        self.graph_revision = None;
        self.context_stale = false;
        self.form_dirty = false;
        self.confirm_discard_changes = false;
        self.reset_history(None, Some(0));
        self.load_context(&scene, graph_revision, editor.scene_load_epoch());
    }

    fn discard_invalid(&mut self) {
        let (Some(scene), Some(sidecar_sha), Some(graph_revision)) = (
            self.scene.as_deref(),
            self.sidecar_sha.as_deref(),
            self.graph_revision,
        ) else {
            return;
        };
        if !self.confirm_discard_invalid {
            return;
        }
        match crate::mesh_hud_store::discard_invalid(scene, sidecar_sha, graph_revision) {
            Ok(_) => {
                self.sidecar_sha = None;
                self.sidecar_error = None;
                self.confirm_discard_invalid = false;
                self.error = None;
                self.reset_history(None, Some(0));
                self.copy_current_fields();
                self.status = "Invalid game HUD sidecar discarded; apply a HUD explicitly.".into();
            }
            Err(error) => {
                self.error = Some(format!("Discard invalid game HUD sidecar failed: {error}"))
            }
        }
    }

    fn undo(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        self.copy_current_fields();
        self.form_dirty = false;
        self.error = None;
        self.status = "Undid game HUD edit; save explicitly to persist.".into();
    }

    fn redo(&mut self) {
        if self.cursor + 1 >= self.history.len() {
            return;
        }
        self.cursor += 1;
        self.copy_current_fields();
        self.form_dirty = false;
        self.error = None;
        self.status = "Redid game HUD edit; save explicitly to persist.".into();
    }

    fn load_context(&mut self, scene: &Path, graph_revision: [u8; 32], scene_epoch: u64) {
        self.scene = Some(scene.to_path_buf());
        self.scene_epoch = Some(scene_epoch);
        self.graph_revision = Some(graph_revision);
        self.document_pending = false;
        self.context_stale = false;
        self.sidecar_sha = None;
        self.sidecar_error = None;
        self.error = None;
        self.confirm_discard_changes = false;
        self.confirm_discard_invalid = false;
        self.reset_history(None, Some(0));
        match crate::mesh_hud_store::load(scene, graph_revision) {
            Ok(Some((document, sidecar_sha))) => {
                self.copy_document_fields(&document);
                self.push_initial(Some(document));
                self.sidecar_sha = Some(sidecar_sha);
                self.status = "Loaded saved game HUD for this scene and graph.".into();
            }
            Ok(None) => {
                self.copy_document_fields(&Document::default());
                self.status = "No saved game HUD; no Mesh game HUD overlay is active.".into();
            }
            Err(error) => self.record_invalid_sidecar(scene, error),
        }
    }

    fn record_invalid_sidecar(&mut self, scene: &Path, error: String) {
        self.sidecar_error = Some(error.clone());
        self.error = Some(error.clone());
        self.sidecar_sha = match crate::mesh_hud_store::sidecar_stamp(scene) {
            Ok(stamp) => stamp,
            Err(stamp_error) => {
                self.error = Some(format!(
                    "{error}; sidecar cannot be safely removed: {stamp_error}"
                ));
                None
            }
        };
        self.status = format!("Saved game HUD could not be admitted: {error}");
    }

    fn copy_document_fields(&mut self, document: &Document) {
        self.enabled = document.enabled;
        self.title.clone_from(&document.title);
        self.idle_text.clone_from(&document.idle_text);
        self.moving_text.clone_from(&document.moving_text);
        self.arrived_text.clone_from(&document.arrived_text);
        self.show_tick = document.show_tick;
        self.font_size = format_number(document.font_size);
        self.text_color = document.text_color.map(format_number);
        self.background_color = document.background_color.map(format_number);
        self.form_dirty = false;
    }

    fn copy_current_fields(&mut self) {
        if let Some(document) = self.current_document().cloned() {
            self.copy_document_fields(&document);
        } else {
            self.copy_document_fields(&Document::default());
        }
    }

    fn reset_history(&mut self, document: Option<Document>, saved_id: Option<u64>) {
        self.history.clear();
        self.history.push(HistoryEntry { id: 0, document });
        self.cursor = 0;
        self.next_id = 1;
        self.saved_id = saved_id;
    }

    fn push_initial(&mut self, document: Option<Document>) {
        self.history[0].document = document;
    }

    fn push_state(&mut self, document: Option<Document>) {
        if self.current_document() == document.as_ref() {
            return;
        }
        self.history.truncate(self.cursor + 1);
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.history.push(HistoryEntry { id, document });
        self.cursor = self.history.len() - 1;
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
            self.cursor = self.cursor.saturating_sub(1);
        }
    }
}

fn current_identity(editor: &Editor) -> Option<(PathBuf, [u8; 32])> {
    if editor.game() != EditorGame::MeshNavigation
        || !editor.spec().is_local()
        || editor.down().is_some()
        || editor.previewing().is_some()
    {
        return None;
    }
    let scene = editor.path()?;
    if !reported_scene_matches_request(editor.spec(), &scene) {
        return None;
    }
    let snapshot = editor.snapshot()?;
    let frame = snapshot.predicted();
    if editor.mode() != Mode::Play && frame.checksum() != editor.sim().checksum {
        return None;
    }
    let view = mesh_from_view(frame).ok()?;
    Some((scene, view.graph.revision()))
}

fn reported_scene_matches_request(spec: &HostSpec, reported: &Path) -> bool {
    let requested = match spec {
        HostSpec::Local { scene, .. } => scene.as_path(),
        HostSpec::LocalGame {
            scene,
            game: EditorGame::MeshNavigation,
            ..
        } => scene.as_path(),
        _ => return false,
    };
    if requested == reported {
        return true;
    }
    match (
        std::fs::canonicalize(requested),
        std::fs::canonicalize(reported),
    ) {
        (Ok(requested), Ok(reported)) => requested == reported,
        _ => false,
    }
}

fn parse_f32(value: &str, label: &str) -> Result<f32, String> {
    if value.len() > 48 {
        return Err(format!("HUD {label} is too long"));
    }
    value
        .trim()
        .parse::<f32>()
        .map_err(|_| format!("HUD {label} must be a number"))
}

fn parse_color(values: &[String; 4], label: &str) -> Result<[u8; 4], String> {
    let mut color = [0; 4];
    for (channel, value) in values.iter().enumerate() {
        if value.len() > 3 {
            return Err(format!("HUD {label} channel must be an unsigned byte"));
        }
        color[channel] = value
            .trim()
            .parse::<u8>()
            .map_err(|_| format!("HUD {label} channel must be an unsigned byte"))?;
    }
    Ok(color)
}

fn format_number(value: impl ToString) -> String {
    value.to_string()
}

fn text_field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let response = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(response.id)
        .changed()
}

#[cfg(test)]
#[path = "mesh_hud_panel/tests.rs"]
mod tests;
