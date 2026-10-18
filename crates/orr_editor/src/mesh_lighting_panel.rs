use crate::{
    backend::HostSpec, editor::Editor, game::EditorGame, mesh_lighting::Document, model::Mode,
};
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
    direction: [String; 3],
    color: [String; 3],
    intensity: String,
    sky: [String; 3],
    ground: [String; 3],
    ambient: String,
    shadows: bool,
    shadow_center: [String; 3],
    shadow_radius: String,
    tonemap: bool,
    exposure: String,
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
        let defaults = orr_sample::yard3d_view::yard_lighting();
        Self {
            direction: defaults.direction.map(format_number),
            color: defaults.color.map(format_number),
            intensity: format_number(defaults.intensity),
            sky: defaults.sky.map(format_number),
            ground: defaults.ground.map(format_number),
            ambient: format_number(defaults.ambient),
            shadows: defaults.shadows,
            shadow_center: defaults.shadow_center.map(format_number),
            shadow_radius: format_number(defaults.shadow_radius),
            tonemap: defaults.tonemap,
            exposure: format_number(defaults.exposure),
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
            self.status = "Unsaved game lighting belongs to another scene or graph revision; explicitly discard and reload before continuing.".into();
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
        ui.collapsing("Mesh game lighting", |ui| {
            if self.document_pending {
                ui.label("Waiting for the requested local mesh scene and admitted graph.");
            }
            ui.label("Optional scene-local lighting is view-only: it does not change simulation state.");
            ui.weak("Sidecar saves use bounded local files and optimistic digest checks; concurrent writers are not isolated.");
            let can_edit = self.can_edit(editor);
            ui.add_enabled_ui(can_edit, |ui| {
                let mut changed = false;
                for (axis, label) in ["Lighting direction X", "Lighting direction Y", "Lighting direction Z"]
                    .into_iter().enumerate()
                {
                    changed |= text_field(ui, label, &mut self.direction[axis]);
                }
                for (channel, label) in ["Light color R", "Light color G", "Light color B"]
                    .into_iter().enumerate()
                {
                    changed |= text_field(ui, label, &mut self.color[channel]);
                }
                changed |= text_field(ui, "Light intensity", &mut self.intensity);
                for (channel, label) in ["Sky color R", "Sky color G", "Sky color B"]
                    .into_iter().enumerate()
                {
                    changed |= text_field(ui, label, &mut self.sky[channel]);
                }
                for (channel, label) in ["Ground color R", "Ground color G", "Ground color B"]
                    .into_iter().enumerate()
                {
                    changed |= text_field(ui, label, &mut self.ground[channel]);
                }
                changed |= text_field(ui, "Ambient", &mut self.ambient);
                changed |= ui.checkbox(&mut self.shadows, "Enable shadows").changed();
                for (axis, label) in ["Shadow center X", "Shadow center Y", "Shadow center Z"]
                    .into_iter().enumerate()
                {
                    changed |= text_field(ui, label, &mut self.shadow_center[axis]);
                }
                changed |= text_field(ui, "Shadow radius", &mut self.shadow_radius);
                changed |= ui.checkbox(&mut self.tonemap, "Enable tonemapping").changed();
                changed |= text_field(ui, "Exposure", &mut self.exposure);
                if changed {
                    self.form_dirty = true;
                    self.error = None;
                    self.status = "Lighting form changed; apply it before Play or save.".into();
                }
                if ui
                    .add_enabled(self.form_dirty || self.current_document().is_none(), egui::Button::new("Apply game lighting"))
                    .clicked()
                {
                    self.apply(editor);
                }
            });

            let history_enabled = self.can_change_history(editor);
            ui.horizontal(|ui| {
                if ui.add_enabled(history_enabled && self.cursor > 0, egui::Button::new("Undo game lighting")).clicked() {
                    self.undo();
                }
                if ui.add_enabled(history_enabled && self.cursor + 1 < self.history.len(), egui::Button::new("Redo game lighting")).clicked() {
                    self.redo();
                }
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(self.can_save(editor) && self.history_is_dirty(), egui::Button::new("Save game lighting")).clicked() {
                    self.save();
                }
                if ui.add_enabled(self.can_reload(editor) && !self.dirty(), egui::Button::new("Reload game lighting")).clicked() {
                    self.reload(editor);
                }
                self.confirm_remove = self.confirm_remove && self.current_document().is_some();
                ui.checkbox(&mut self.confirm_remove, "I confirm removing the saved game lighting");
                if ui.add_enabled(self.can_edit(editor) && self.current_document().is_some() && self.confirm_remove, egui::Button::new("Remove game lighting")).clicked() {
                    self.remove();
                }
            });
            if self.context_stale || self.dirty() {
                ui.checkbox(&mut self.confirm_discard_changes, "I confirm discarding unsaved game lighting changes");
                if ui.add_enabled(self.can_discard_changes(editor) && self.confirm_discard_changes, egui::Button::new("Discard game lighting changes and reload")).clicked() {
                    self.discard_changes_and_reload(editor);
                }
            }
            if self.sidecar_error.is_some() {
                ui.checkbox(&mut self.confirm_discard_invalid, "I confirm discarding the invalid game lighting sidecar");
                if ui.add_enabled(self.can_discard_invalid(editor) && self.confirm_discard_invalid, egui::Button::new("Discard invalid game lighting sidecar")).clicked() {
                    self.discard_invalid();
                }
            }
            if self.dirty() {
                ui.colored_label(egui::Color32::YELLOW, "Unsaved lighting form or sidecar history blocks Play and export.");
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            } else if !self.status.is_empty() {
                ui.label(&self.status);
            }
        });
    }

    pub(crate) fn applied_lighting(&self, editor: &Editor) -> Option<orr_render::Lighting> {
        if self.document_pending
            || self.context_stale
            || self.sidecar_error.is_some()
            || !self.identity_matches(editor)
        {
            return None;
        }
        self.current_document()
            .and_then(|document| document.lighting().ok())
    }

    pub(crate) fn play_blocker(&self, editor: &Editor) -> Option<String> {
        if editor.game() != EditorGame::MeshNavigation {
            return None;
        }
        if self.document_pending || current_identity(editor).is_none() {
            return Some("Wait for the requested local mesh scene and admitted graph".into());
        }
        if self.context_stale {
            return Some(
                "Discard or resolve lighting from the previous scene or graph revision".into(),
            );
        }
        if let Some(error) = &self.sidecar_error {
            return Some(format!("Saved game lighting is invalid: {error}"));
        }
        if self.dirty() {
            return Some("Apply, save, or discard game lighting changes before Play".into());
        }
        if self.scene_epoch != Some(editor.scene_load_epoch()) || !self.identity_matches(editor) {
            return Some("Game lighting does not match the current mesh scene and graph".into());
        }
        None
    }

    pub(crate) fn dirty(&self) -> bool {
        self.form_dirty || self.context_stale || self.history_is_dirty()
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
        let mut lighting = self.current_document().map_or_else(
            || Ok(orr_sample::yard3d_view::yard_lighting()),
            Document::lighting,
        )?;
        for (axis, label) in ["direction X", "direction Y", "direction Z"]
            .into_iter()
            .enumerate()
        {
            lighting.direction[axis] = parse_number(&self.direction[axis], label)?;
        }
        for (channel, label) in ["color R", "color G", "color B"].into_iter().enumerate() {
            lighting.color[channel] = parse_number(&self.color[channel], label)?;
            lighting.sky[channel] =
                parse_number(&self.sky[channel], ["sky R", "sky G", "sky B"][channel])?;
            lighting.ground[channel] = parse_number(
                &self.ground[channel],
                ["ground R", "ground G", "ground B"][channel],
            )?;
        }
        lighting.intensity = parse_number(&self.intensity, "intensity")?;
        lighting.ambient = parse_number(&self.ambient, "ambient")?;
        lighting.shadows = self.shadows;
        for (axis, label) in ["shadow center X", "shadow center Y", "shadow center Z"]
            .into_iter()
            .enumerate()
        {
            lighting.shadow_center[axis] = parse_number(&self.shadow_center[axis], label)?;
        }
        lighting.shadow_radius = parse_number(&self.shadow_radius, "shadow radius")?;
        lighting.tonemap = self.tonemap;
        lighting.exposure = parse_number(&self.exposure, "exposure")?;
        let document = Document::from_lighting(lighting)?;
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
            self.error = Some("The current mesh scene is not ready for lighting changes".into());
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
        self.status = "Game lighting applied in memory; save explicitly to persist.".into();
    }

    fn save(&mut self) {
        let (Some(scene), Some(graph_revision)) = (self.scene.as_deref(), self.graph_revision)
        else {
            self.error = Some("Save the mesh scene before saving its lighting".into());
            return;
        };
        match crate::mesh_lighting_store::save(
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
                    "Game lighting saved for this scene and graph.".into()
                } else {
                    "Game lighting removed from the scene sidecar.".into()
                };
            }
            Err(error) => self.error = Some(format!("Save game lighting failed: {error}")),
        }
    }

    fn reload(&mut self, editor: &Editor) {
        if !self.can_reload(editor) || self.dirty() {
            self.error = Some(
                "Reload requires a clean lighting state in the current editable local scene".into(),
            );
            return;
        }
        let Some((scene, graph_revision)) = current_identity(editor) else {
            self.error = Some("The current mesh scene is not ready for lighting reload".into());
            return;
        };
        self.load_context(&scene, graph_revision, editor.scene_load_epoch());
    }

    fn remove(&mut self) {
        if self.current_document().is_none() || !self.confirm_remove {
            self.error =
                Some("Remove requires confirmation and a saved or applied game lighting".into());
            return;
        }
        self.push_state(None);
        self.copy_current_fields();
        self.form_dirty = false;
        self.confirm_remove = false;
        self.error = None;
        self.status = "Game lighting removed in memory; save explicitly to persist.".into();
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
        match crate::mesh_lighting_store::save(scene, None, Some(sidecar_sha), graph_revision) {
            Ok(_) => {
                self.sidecar_sha = None;
                self.sidecar_error = None;
                self.confirm_discard_invalid = false;
                self.error = None;
                self.reset_history(None, Some(0));
                self.copy_current_fields();
                self.status =
                    "Invalid game lighting sidecar discarded; apply lighting explicitly.".into();
            }
            Err(error) => {
                self.error = Some(format!(
                    "Discard invalid game lighting sidecar failed: {error}"
                ))
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
        self.status = "Undid game lighting edit; save explicitly to persist.".into();
    }

    fn redo(&mut self) {
        if self.cursor + 1 >= self.history.len() {
            return;
        }
        self.cursor += 1;
        self.copy_current_fields();
        self.form_dirty = false;
        self.error = None;
        self.status = "Redid game lighting edit; save explicitly to persist.".into();
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
        match crate::mesh_lighting_store::load(scene, graph_revision) {
            Ok(Some((document, sidecar_sha))) => {
                if let Err(error) = self.copy_document_fields(&document) {
                    self.record_invalid_sidecar(scene, error);
                    return;
                }
                self.push_initial(Some(document));
                self.sidecar_sha = Some(sidecar_sha);
                self.status = "Loaded saved game lighting for this scene and graph.".into();
            }
            Ok(None) => {
                self.copy_lighting_fields(orr_sample::yard3d_view::yard_lighting());
                self.status = "No saved game lighting; default renderer lighting is active.".into();
            }
            Err(error) => self.record_invalid_sidecar(scene, error),
        }
    }

    fn record_invalid_sidecar(&mut self, scene: &Path, error: String) {
        self.sidecar_error = Some(error.clone());
        self.error = Some(error.clone());
        self.sidecar_sha = match crate::mesh_lighting_store::sidecar_stamp(scene) {
            Ok(stamp) => stamp,
            Err(stamp_error) => {
                self.error = Some(format!(
                    "{error}; sidecar cannot be safely removed: {stamp_error}"
                ));
                None
            }
        };
        self.status = format!("Saved game lighting could not be admitted: {error}");
    }

    fn copy_document_fields(&mut self, document: &Document) -> Result<(), String> {
        self.copy_lighting_fields(document.lighting()?);
        Ok(())
    }

    fn copy_current_fields(&mut self) {
        match self.current_document().map(Document::lighting) {
            Some(Ok(lighting)) => self.copy_lighting_fields(lighting),
            Some(Err(error)) => {
                self.error = Some(format!("Current game lighting is invalid: {error}"))
            }
            None => self.copy_lighting_fields(orr_sample::yard3d_view::yard_lighting()),
        }
    }

    fn copy_lighting_fields(&mut self, lighting: orr_render::Lighting) {
        self.direction = lighting.direction.map(format_number);
        self.color = lighting.color.map(format_number);
        self.intensity = format_number(lighting.intensity);
        self.sky = lighting.sky.map(format_number);
        self.ground = lighting.ground.map(format_number);
        self.ambient = format_number(lighting.ambient);
        self.shadows = lighting.shadows;
        self.shadow_center = lighting.shadow_center.map(format_number);
        self.shadow_radius = format_number(lighting.shadow_radius);
        self.tonemap = lighting.tonemap;
        self.exposure = format_number(lighting.exposure);
        self.form_dirty = false;
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

fn parse_number(value: &str, label: &str) -> Result<f32, String> {
    if value.len() > 48 {
        return Err(format!("game lighting {label} is too long"));
    }
    value
        .trim()
        .parse::<f32>()
        .map_err(|_| format!("game lighting {label} must be a number"))
}

fn format_number(value: f32) -> String {
    value.to_string()
}

fn text_field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let label_response = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(label_response.id)
        .changed()
}
