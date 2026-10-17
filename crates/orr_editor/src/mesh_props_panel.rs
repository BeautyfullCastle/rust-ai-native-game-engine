use crate::{
    backend::HostSpec,
    editor::Editor,
    game::EditorGame,
    mesh_art::Snapshot,
    mesh_props::{self, Document, Prop, MAX_PROPS},
    model::Mode,
    viewport3d::SceneModel,
};
use egui::Ui;
use orr_remote::mesh_navigation::mesh_from_view;
use std::path::{Path, PathBuf};

const MAX_HISTORY: usize = 4;

struct HistoryEntry {
    id: u64,
    document: Document,
}

pub(crate) struct Panel {
    package_root: String,
    asset: String,
    license: String,
    name: String,
    position: [String; 3],
    scale: String,
    license_acknowledged: bool,
    confirm_remove: bool,
    confirm_discard_invalid: bool,
    candidate: Option<Snapshot>,
    selected: Option<u64>,
    document: Document,
    scene: Option<PathBuf>,
    scene_epoch: Option<u64>,
    graph_revision: Option<[u8; 32]>,
    document_pending: bool,
    context_stale: bool,
    sidecar_digest: Option<String>,
    sidecar_error: Option<String>,
    history: Vec<HistoryEntry>,
    cursor: usize,
    next_history_id: u64,
    next_prop_id: u64,
    saved_history_id: Option<u64>,
    status: String,
    error: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            package_root: String::new(),
            asset: String::new(),
            license: String::new(),
            name: "prop".into(),
            position: ["0".into(), "0".into(), "0".into()],
            scale: "1".into(),
            license_acknowledged: false,
            confirm_remove: false,
            confirm_discard_invalid: false,
            candidate: None,
            selected: None,
            document: Document::default(),
            scene: None,
            scene_epoch: None,
            graph_revision: None,
            document_pending: true,
            context_stale: false,
            sidecar_digest: None,
            sidecar_error: None,
            history: vec![HistoryEntry {
                id: 0,
                document: Document::default(),
            }],
            cursor: 0,
            next_history_id: 1,
            next_prop_id: 1,
            saved_history_id: Some(0),
            status: String::new(),
            error: None,
        }
    }
}

impl Panel {
    pub(crate) fn sync(&mut self, editor: &Editor) {
        if editor.game() != EditorGame::MeshNavigation {
            self.document_pending = true;
            return;
        }
        let Some((scene, graph_revision)) = current_identity(editor) else {
            self.document_pending = true;
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
        if self.scene.is_some()
            && self.scene_epoch == Some(scene_epoch)
            && self.graph_revision == Some(graph_revision)
        {
            self.scene = Some(scene);
            self.sidecar_digest = None;
            self.context_stale = false;
            self.sidecar_error = None;
            self.confirm_discard_invalid = false;
            if self.document.props.is_empty() {
                self.saved_history_id = self.current_history_id();
                self.status = "Scene path changed; no mesh props are present.".into();
            } else {
                self.saved_history_id = None;
                self.status =
                    "Scene path changed; save mesh props to create their new sidecar explicitly."
                        .into();
            }
            return;
        }
        if self.dirty() {
            self.context_stale = true;
            self.confirm_discard_invalid = false;
            self.status = "Unsaved mesh props belong to a different scene or graph revision; undo to the saved state before continuing.".into();
            return;
        }

        self.scene = Some(scene.clone());
        self.scene_epoch = Some(scene_epoch);
        self.graph_revision = Some(graph_revision);
        self.context_stale = false;
        self.candidate = None;
        self.selected = None;
        self.confirm_remove = false;
        self.confirm_discard_invalid = false;
        self.error = None;
        self.sidecar_error = None;
        self.sidecar_digest = None;
        self.reset_history(Document::default(), Some(0));
        match mesh_props::load(&scene, graph_revision) {
            Ok(Some((document, digest))) => {
                self.sidecar_digest = Some(digest);
                self.selected = document.props.first().map(|prop| prop.id);
                self.document = document.clone();
                self.copy_selection_fields();
                self.reset_history(document, Some(0));
                self.recompute_next_prop_id();
                self.status = "Loaded saved mesh props for this graph revision.".into();
            }
            Ok(None) => {
                self.document = Document::default();
                self.reset_history(Document::default(), Some(0));
                self.next_prop_id = 1;
                self.status = "No saved mesh props for this graph revision.".into();
            }
            Err(error) => {
                self.sidecar_error = Some(error.clone());
                self.sidecar_digest = mesh_props::sidecar_stamp(&scene).ok().flatten();
                self.error = Some(error.clone());
                self.status = format!("Saved mesh props could not be admitted: {error}");
            }
        }
    }

    pub(crate) fn show(&mut self, ui: &mut Ui, editor: &Editor) {
        if editor.game() != EditorGame::MeshNavigation {
            return;
        }
        self.sync(editor);
        ui.collapsing("Mesh props", |ui| {
            ui.label("Static cosmetic props are saved in a separate scene sidecar and do not affect navigation physics.");
            ui.weak("Up to six props reserve the remaining viewport asset slots for the graph and agent art.");
            let editable = self.can_edit(editor);
            ui.add_enabled_ui(editable, |ui| {
                let package_changed = field(ui, "Prop package source directory", &mut self.package_root);
                let asset_changed = field(ui, "Prop model asset", &mut self.asset);
                let license_changed = field(ui, "Prop license asset", &mut self.license);
                if package_changed || asset_changed || license_changed {
                    self.candidate = None;
                    self.license_acknowledged = false;
                    self.error = None;
                    self.status = "Prop source changed; load the package again before adding or updating props.".into();
                }
                if ui
                    .checkbox(
                        &mut self.license_acknowledged,
                        "I have reviewed rights to use these props",
                    )
                    .changed()
                {
                    self.candidate = None;
                }
                if ui
                    .add_enabled(
                        self.license_acknowledged
                            && !self.package_root.trim().is_empty()
                            && !self.asset.trim().is_empty()
                            && !self.license.trim().is_empty(),
                        egui::Button::new("Load prop candidate"),
                    )
                    .clicked()
                {
                    self.load_candidate();
                }
            });

            ui.add_enabled_ui(editable, |ui| {
                field(ui, "Prop name", &mut self.name);
                field(ui, "Prop position X", &mut self.position[0]);
                field(ui, "Prop position Y", &mut self.position[1]);
                field(ui, "Prop position Z", &mut self.position[2]);
                field(ui, "Prop scale", &mut self.scale);
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        editable
                            && self.candidate.is_some()
                            && self.document.props.len() < MAX_PROPS,
                        egui::Button::new("Add prop"),
                    )
                    .clicked()
                {
                    self.add_prop();
                }
                if ui
                    .add_enabled(editable && self.selected_prop().is_some(), egui::Button::new("Update selected prop"))
                    .clicked()
                {
                    self.update_selected();
                }
            });

            ui.label(format!("Props: {}/{}", self.document.props.len(), MAX_PROPS));
            let mut selected_prop = None;
            for prop in &self.document.props {
                let label = format!("{} · prop-{:016x}", prop.name, prop.id);
                if ui
                    .selectable_label(self.selected == Some(prop.id), label)
                    .clicked()
                {
                    selected_prop = Some(prop.id);
                }
            }
            if let Some(identifier) = selected_prop {
                self.selected = Some(identifier);
                self.copy_selection_fields();
                self.confirm_remove = false;
            }
            ui.add_enabled_ui(editable, |ui| {
                ui.checkbox(&mut self.confirm_remove, "I confirm removing this prop");
            });
            if ui
                .add_enabled(
                    editable && self.confirm_remove && self.selected_prop().is_some(),
                    egui::Button::new("Remove selected prop"),
                )
                .clicked()
            {
                self.remove_selected();
            }
            if self.sidecar_error.is_some() {
                ui.checkbox(
                    &mut self.confirm_discard_invalid,
                    "I confirm discarding invalid mesh props",
                );
                if ui
                    .add_enabled(
                        self.can_discard_invalid(editor),
                        egui::Button::new("Discard invalid props sidecar"),
                    )
                    .clicked()
                {
                    self.discard_invalid_sidecar(editor);
                }
            }
            ui.horizontal(|ui| {
                let history_enabled = self.can_change_history(editor);
                if ui
                    .add_enabled(history_enabled && self.cursor > 0, egui::Button::new("Undo prop edit"))
                    .clicked()
                {
                    self.undo();
                }
                if ui
                    .add_enabled(
                        history_enabled && self.cursor + 1 < self.history.len(),
                        egui::Button::new("Redo prop edit"),
                    )
                    .clicked()
                {
                    self.redo();
                }
                if ui
                    .add_enabled(editable && self.dirty(), egui::Button::new("Save props"))
                    .clicked()
                {
                    self.save(editor);
                }
                if ui
                    .add_enabled(self.can_reload(editor), egui::Button::new("Reload props"))
                    .clicked()
                {
                    self.reload(editor);
                }
            });
            if self.document_pending {
                ui.label("Waiting for an admitted local mesh scene and coherent Frame.");
            }
            if self.context_stale {
                ui.colored_label(egui::Color32::RED, "Mesh props belong to an earlier scene or graph revision.");
            }
            if self.dirty() {
                ui.colored_label(egui::Color32::YELLOW, "Unsaved mesh props changes block Play.");
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            } else if !self.status.is_empty() {
                ui.label(&self.status);
            }
        });
    }

    pub(crate) fn placements(&self, editor: &Editor) -> Vec<SceneModel> {
        if self.document_pending
            || self.context_stale
            || self.sidecar_error.is_some()
            || !current_identity(editor).is_some_and(|(scene, revision)| {
                self.scene.as_ref() == Some(&scene)
                    && self.graph_revision == Some(revision)
                    && self.scene_epoch == Some(editor.scene_load_epoch())
            })
        {
            return Vec::new();
        }
        self.document.placements().unwrap_or_default()
    }

    pub(crate) fn play_blocker(&self, editor: &Editor) -> Option<String> {
        if editor.game() != EditorGame::MeshNavigation {
            return None;
        }
        if self.document_pending
            || current_identity(editor).is_none_or(|(scene, revision)| {
                self.scene.as_ref() != Some(&scene)
                    || self.graph_revision != Some(revision)
                    || self.scene_epoch != Some(editor.scene_load_epoch())
            })
        {
            return Some("Wait for the requested admitted mesh scene and coherent Frame".into());
        }
        if self.context_stale {
            return Some(
                "Resolve unsaved mesh props from the prior scene or graph revision".into(),
            );
        }
        if let Some(error) = &self.sidecar_error {
            return Some(format!("Saved mesh props are invalid: {error}"));
        }
        if self.dirty() {
            return Some("Save or undo mesh prop changes before Play".into());
        }
        if self.document.placements().is_err() {
            return Some("Mesh props cannot be presented for the current graph".into());
        }
        None
    }

    pub(crate) fn dirty(&self) -> bool {
        self.current_history_id() != self.saved_history_id
    }

    fn load_candidate(&mut self) {
        let Some(graph_revision) = self.graph_revision else {
            self.error = Some("Wait for an admitted mesh graph before loading a prop".into());
            return;
        };
        match Snapshot::from_package(
            Path::new(self.package_root.trim()),
            self.asset.trim(),
            self.license.trim(),
            graph_revision,
        ) {
            Ok(snapshot) => {
                self.copy_snapshot_fields(&snapshot);
                self.candidate = Some(snapshot);
                self.error = None;
                self.status = "Loaded prop candidate; add or update a prop explicitly.".into();
                self.license_acknowledged = false;
            }
            Err(error) => {
                self.candidate = None;
                self.error = Some(format!("Prop package load failed: {error}"));
            }
        }
    }

    fn add_prop(&mut self) {
        let Some(candidate) = self.candidate.clone() else {
            self.error = Some("Load a verified package before adding a prop".into());
            return;
        };
        if self.document.props.len() >= MAX_PROPS {
            self.error = Some(format!("A mesh scene supports at most {MAX_PROPS} props"));
            return;
        }
        let name = match validate_name(&self.name) {
            Ok(name) => name,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        if self.document.props.iter().any(|prop| prop.name == name) {
            self.error = Some("Prop names must be unique in the mesh scene".into());
            return;
        }
        let Some(id) = self.available_prop_id() else {
            self.error = Some("No more stable prop identifiers are available".into());
            return;
        };
        let art = match self.transformed(candidate) {
            Ok(art) => art,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let mut document = self.document.clone();
        document.props.push(Prop { id, name, art });
        let Some(graph_revision) = self.graph_revision else {
            self.error = Some("Wait for an admitted mesh graph before adding a prop".into());
            return;
        };
        if let Err(error) = validate_document(&document, graph_revision) {
            self.error = Some(error);
            return;
        }
        self.push_document(document);
        self.next_prop_id = id.checked_add(1).unwrap_or(1);
        self.selected = Some(id);
        self.confirm_remove = false;
        self.error = None;
        self.status = "Added mesh prop in memory; save explicitly to persist.".into();
    }

    fn update_selected(&mut self) {
        let Some(selected) = self.selected else {
            self.error = Some("Select a mesh prop before updating it".into());
            return;
        };
        let Some(index) = self
            .document
            .props
            .iter()
            .position(|prop| prop.id == selected)
        else {
            self.error = Some("The selected mesh prop is no longer present".into());
            return;
        };
        let name = match validate_name(&self.name) {
            Ok(name) => name,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        if self
            .document
            .props
            .iter()
            .enumerate()
            .any(|(prop_index, prop)| prop_index != index && prop.name == name)
        {
            self.error = Some("Prop names must be unique in the mesh scene".into());
            return;
        }
        let base = self
            .candidate
            .as_ref()
            .unwrap_or(&self.document.props[index].art)
            .clone();
        let art = match self.transformed(base) {
            Ok(art) => art,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let mut document = self.document.clone();
        document.props[index].name = name;
        document.props[index].art = art;
        let Some(graph_revision) = self.graph_revision else {
            self.error = Some("Wait for an admitted mesh graph before updating a prop".into());
            return;
        };
        if let Err(error) = validate_document(&document, graph_revision) {
            self.error = Some(error);
            return;
        }
        self.push_document(document);
        self.error = None;
        self.status = "Updated mesh prop in memory; save explicitly to persist.".into();
    }

    fn remove_selected(&mut self) {
        let Some(selected) = self.selected else {
            self.error = Some("Select a mesh prop before removing it".into());
            return;
        };
        let mut document = self.document.clone();
        let original_count = document.props.len();
        document.props.retain(|prop| prop.id != selected);
        if document.props.len() == original_count {
            self.error = Some("The selected mesh prop is no longer present".into());
            return;
        }
        self.push_document(document);
        self.selected = self.document.props.first().map(|prop| prop.id);
        self.confirm_remove = false;
        self.copy_selection_fields();
        self.error = None;
        self.status = "Removed mesh prop in memory; save explicitly to persist.".into();
    }

    fn save(&mut self, editor: &Editor) {
        let Some((scene, revision)) = current_identity(editor) else {
            self.error = Some("Save props only for the matching admitted local mesh scene".into());
            return;
        };
        if !self.can_edit(editor) || self.scene.as_ref() != Some(&scene) {
            self.error = Some("Save props only for the matching editable local mesh scene".into());
            return;
        }
        if let Err(error) = validate_document(&self.document, revision) {
            self.error = Some(error);
            return;
        }
        let document = (!self.document.props.is_empty()).then_some(&self.document);
        match mesh_props::save(&scene, document, self.sidecar_digest.as_deref(), revision) {
            Ok(digest) => {
                self.sidecar_digest = digest;
                self.saved_history_id = self.current_history_id();
                self.sidecar_error = None;
                self.context_stale = false;
                self.confirm_discard_invalid = false;
                self.error = None;
                self.status = "Mesh props saved for this scene.".into();
            }
            Err(error) => self.error = Some(format!("Save mesh props failed: {error}")),
        }
    }

    fn reload(&mut self, editor: &Editor) {
        if self.dirty() {
            self.error = Some("Save or undo mesh prop changes before reloading".into());
            return;
        }
        let Some((scene, revision)) = current_identity(editor) else {
            self.error =
                Some("Wait for an admitted local mesh scene before reloading props".into());
            return;
        };
        self.scene = Some(scene.clone());
        self.scene_epoch = Some(editor.scene_load_epoch());
        self.graph_revision = Some(revision);
        self.sidecar_digest = None;
        self.sidecar_error = None;
        self.confirm_discard_invalid = false;
        self.error = None;
        match mesh_props::load(&scene, revision) {
            Ok(Some((document, digest))) => {
                self.sidecar_digest = Some(digest);
                self.selected = document.props.first().map(|prop| prop.id);
                self.document = document.clone();
                self.copy_selection_fields();
                self.reset_history(document, Some(0));
                self.recompute_next_prop_id();
                self.status = "Reloaded saved mesh props.".into();
            }
            Ok(None) => {
                self.document = Document::default();
                self.selected = None;
                self.reset_history(Document::default(), Some(0));
                self.next_prop_id = 1;
                self.status = "No saved mesh props for this graph revision.".into();
            }
            Err(error) => {
                self.sidecar_error = Some(error.clone());
                self.sidecar_digest = mesh_props::sidecar_stamp(&scene).ok().flatten();
                self.error = Some(error.clone());
                self.status = format!("Saved mesh props could not be admitted: {error}");
            }
        }
    }

    fn can_discard_invalid(&self, editor: &Editor) -> bool {
        let current_scene = editor.path();
        let scene_matches = current_identity(editor).is_some_and(|(scene, revision)| {
            Some(scene.clone()) == current_scene
                && self.scene.as_ref() == Some(&scene)
                && self.scene_epoch == Some(editor.scene_load_epoch())
                && self.graph_revision == Some(revision)
        });
        discard_invalid_ready(
            self.confirm_discard_invalid,
            editor.game() == EditorGame::MeshNavigation
                && editor.spec().is_local()
                && editor.mode() == Mode::Edit
                && editor.can_mutate()
                && editor.previewing().is_none()
                && editor.proposals().is_empty()
                && !self.document_pending,
            self.sidecar_error.is_some(),
            self.sidecar_digest.is_some(),
            !self.dirty(),
            scene_matches,
        )
    }

    fn discard_invalid_sidecar(&mut self, editor: &Editor) {
        if !self.can_discard_invalid(editor) {
            self.error = Some(
                "Discard is available only for the unchanged invalid sidecar of this editable local mesh scene".into(),
            );
            return;
        }
        let (Some(scene), Some(revision), Some(expected_digest)) = (
            self.scene.as_deref(),
            self.graph_revision,
            self.sidecar_digest.as_deref(),
        ) else {
            self.error = Some("No safely observed invalid mesh props sidecar is available".into());
            return;
        };
        match mesh_props::save(scene, None, Some(expected_digest), revision) {
            Ok(_) => {
                self.document = Document::default();
                self.selected = None;
                self.candidate = None;
                self.sidecar_digest = None;
                self.sidecar_error = None;
                self.context_stale = false;
                self.confirm_discard_invalid = false;
                self.error = None;
                self.status =
                    "Invalid mesh props sidecar discarded; load or add props again explicitly."
                        .into();
                self.reset_history(Document::default(), Some(0));
                self.next_prop_id = 1;
            }
            Err(error) => {
                self.confirm_discard_invalid = false;
                self.error = Some(format!(
                    "Discard invalid mesh props sidecar failed: {error}"
                ));
            }
        }
    }

    fn can_edit(&self, editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && !self.document_pending
            && !self.context_stale
            && self.sidecar_error.is_none()
            && self.scene_epoch == Some(editor.scene_load_epoch())
            && current_identity(editor).is_some_and(|(scene, revision)| {
                self.scene.as_ref() == Some(&scene) && self.graph_revision == Some(revision)
            })
    }

    fn can_change_history(&self, editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && document_frame_ready(
                editor.game(),
                editor.spec(),
                editor.path().as_deref(),
                editor.mode(),
                editor.snapshot().is_some(),
                editor.checksum(),
                editor.sim().checksum,
            )
    }

    fn can_reload(&self, editor: &Editor) -> bool {
        !self.dirty()
            && editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && current_identity(editor).is_some()
    }

    fn transformed(&self, snapshot: Snapshot) -> Result<Snapshot, String> {
        let (position, scale) = parse_transform(&self.position, &self.scale)?;
        snapshot.with_transform(position, scale)
    }

    fn selected_prop(&self) -> Option<&Prop> {
        self.document
            .props
            .iter()
            .find(|prop| Some(prop.id) == self.selected)
    }

    fn available_prop_id(&self) -> Option<u64> {
        let mut candidate = self.next_prop_id.max(1);
        for _ in 0..=MAX_PROPS {
            if self.document.props.iter().all(|prop| prop.id != candidate) {
                return Some(candidate);
            }
            candidate = candidate.checked_add(1).unwrap_or(1);
        }
        None
    }

    fn push_document(&mut self, document: Document) {
        self.history.truncate(self.cursor + 1);
        let id = self.next_history_id;
        self.next_history_id = self.next_history_id.wrapping_add(1).max(1);
        self.history.push(HistoryEntry {
            id,
            document: document.clone(),
        });
        self.cursor = self.history.len() - 1;
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
            self.cursor = self.cursor.saturating_sub(1);
        }
        self.document = document;
    }

    fn undo(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.document = self.history[self.cursor].document.clone();
            self.selected = self
                .selected
                .filter(|selected| self.document.props.iter().any(|prop| prop.id == *selected))
                .or_else(|| self.document.props.first().map(|prop| prop.id));
            self.copy_selection_fields();
            self.status = "Undid mesh prop edit; save explicitly to persist.".into();
            self.error = None;
        }
    }

    fn redo(&mut self) {
        if self.cursor + 1 < self.history.len() {
            self.cursor += 1;
            self.document = self.history[self.cursor].document.clone();
            self.selected = self
                .selected
                .filter(|selected| self.document.props.iter().any(|prop| prop.id == *selected))
                .or_else(|| self.document.props.first().map(|prop| prop.id));
            self.copy_selection_fields();
            self.status = "Redid mesh prop edit; save explicitly to persist.".into();
            self.error = None;
        }
    }

    fn reset_history(&mut self, document: Document, saved_id: Option<u64>) {
        self.document = document.clone();
        self.history.clear();
        self.history.push(HistoryEntry { id: 0, document });
        self.cursor = 0;
        self.next_history_id = 1;
        self.saved_history_id = saved_id;
    }

    fn current_history_id(&self) -> Option<u64> {
        self.history.get(self.cursor).map(|entry| entry.id)
    }

    fn recompute_next_prop_id(&mut self) {
        self.next_prop_id = self
            .document
            .props
            .iter()
            .map(|prop| prop.id)
            .max()
            .and_then(|id| id.checked_add(1))
            .unwrap_or(1);
    }

    fn copy_selection_fields(&mut self) {
        if let Some((name, snapshot)) = self
            .selected_prop()
            .map(|prop| (prop.name.clone(), prop.art.clone()))
        {
            self.name = name;
            self.copy_snapshot_fields(&snapshot);
        }
    }

    fn copy_snapshot_fields(&mut self, snapshot: &Snapshot) {
        self.position = snapshot.offset().map(format_number);
        self.scale = format_number(snapshot.scale());
    }
}

fn current_identity(editor: &Editor) -> Option<(PathBuf, [u8; 32])> {
    if editor.game() != EditorGame::MeshNavigation || !editor.spec().is_local() {
        return None;
    }
    let scene = editor.path()?;
    if !document_frame_ready(
        editor.game(),
        editor.spec(),
        Some(&scene),
        editor.mode(),
        editor.snapshot().is_some(),
        editor.checksum(),
        editor.sim().checksum,
    ) {
        return None;
    }
    let view = mesh_from_view(editor.snapshot()?.predicted()).ok()?;
    Some((scene, view.graph.revision()))
}

fn discard_invalid_ready(
    confirmed: bool,
    editable_local_scene: bool,
    invalid_sidecar: bool,
    expected_digest_present: bool,
    clean_document: bool,
    matching_scene_epoch_and_revision: bool,
) -> bool {
    confirmed
        && editable_local_scene
        && invalid_sidecar
        && expected_digest_present
        && clean_document
        && matching_scene_epoch_and_revision
}

fn reported_scene_matches_request(
    game: EditorGame,
    spec: &HostSpec,
    reported: Option<&Path>,
) -> bool {
    if game != EditorGame::MeshNavigation {
        return false;
    }
    let requested = match spec {
        HostSpec::Local { scene, .. } => scene.as_path(),
        HostSpec::LocalGame {
            scene,
            game: requested_game,
            ..
        } if *requested_game == EditorGame::MeshNavigation => scene.as_path(),
        _ => return false,
    };
    let Some(reported) = reported else {
        return false;
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

fn document_frame_ready(
    game: EditorGame,
    spec: &HostSpec,
    reported: Option<&Path>,
    mode: Mode,
    has_snapshot: bool,
    frame_checksum: u64,
    state_checksum: u64,
) -> bool {
    reported_scene_matches_request(game, spec, reported)
        && has_snapshot
        && (mode == Mode::Play || frame_checksum == state_checksum)
}

fn parse_transform(position: &[String; 3], scale: &str) -> Result<([f32; 3], f32), String> {
    let parsed_position = [
        position[0]
            .trim()
            .parse::<f32>()
            .map_err(|_| "prop position X must be a number")?,
        position[1]
            .trim()
            .parse::<f32>()
            .map_err(|_| "prop position Y must be a number")?,
        position[2]
            .trim()
            .parse::<f32>()
            .map_err(|_| "prop position Z must be a number")?,
    ];
    let parsed_scale = scale
        .trim()
        .parse::<f32>()
        .map_err(|_| "prop scale must be a number")?;
    if parsed_position.iter().any(|value| !value.is_finite()) || !parsed_scale.is_finite() {
        return Err("prop transform values must be finite".into());
    }
    Ok((parsed_position, parsed_scale))
}

fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
        return Err("prop name must be 1-64 printable UTF-8 bytes".into());
    }
    Ok(name.to_owned())
}

fn validate_document(document: &Document, graph_revision: [u8; 32]) -> Result<(), String> {
    if document.props.len() > MAX_PROPS {
        return Err(format!("A mesh scene supports at most {MAX_PROPS} props"));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    for prop in &document.props {
        if prop.id == 0 || !ids.insert(prop.id) {
            return Err("mesh prop identifiers must be nonzero and unique".into());
        }
        let name = validate_name(&prop.name)?;
        if name != prop.name || !names.insert(name) {
            return Err("mesh prop names must be trimmed and unique".into());
        }
        if prop.art.graph_revision() != graph_revision {
            return Err("mesh prop art belongs to a different graph revision".into());
        }
        prop.art.placement([orr_fp::FP::ZERO; 3])?;
    }
    Ok(())
}

fn field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let response = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width()))
        .labelled_by(response.id)
        .changed()
}

fn format_number(value: f32) -> String {
    value.to_string()
}

#[cfg(test)]
#[path = "mesh_props_panel/tests.rs"]
mod tests;
