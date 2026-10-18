use crate::{backend::HostSpec, editor::Editor, game::EditorGame, mesh_art::Snapshot, model::Mode};
use egui::Ui;
use orr_remote::mesh_navigation::mesh_from_view;
use std::path::PathBuf;

const MAX_HISTORY: usize = 4;

struct HistoryEntry {
    id: u64,
    snapshot: Option<Snapshot>,
}

pub(crate) struct Panel {
    importer: crate::mesh_art_import_panel::Panel,
    package_root: String,
    asset: String,
    license: String,
    offset: [String; 3],
    scale: String,
    rotation: [String; 3],
    face_route_direction: bool,
    tint: [String; 4],
    surface_enabled: bool,
    surface_material: String,
    surface_uv_offset: [String; 2],
    surface_uv_scale: [String; 2],
    surface_quarter_turns: String,
    surface_pivot: [String; 3],
    license_acknowledged: bool,
    confirm_remove: bool,
    confirm_discard_invalid: bool,
    candidate: Option<Snapshot>,
    scene: Option<PathBuf>,
    scene_epoch: Option<u64>,
    graph_revision: Option<[u8; 32]>,
    document_pending: bool,
    context_stale: bool,
    sidecar_sha: Option<String>,
    sidecar_error: Option<String>,
    history: Vec<HistoryEntry>,
    cursor: usize,
    next_id: u64,
    saved_id: Option<u64>,
    presentation_error: Option<String>,
    status: String,
    error: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            importer: crate::mesh_art_import_panel::Panel::default(),
            package_root: String::new(),
            asset: "model.glb".into(),
            license: "LICENSE.txt".into(),
            offset: ["0".into(), "0".into(), "0".into()],
            scale: "1".into(),
            rotation: ["0".into(), "0".into(), "0".into()],
            face_route_direction: false,
            tint: ["1".into(), "1".into(), "1".into(), "1".into()],
            surface_enabled: false,
            surface_material: "0".into(),
            surface_uv_offset: ["0".into(), "0".into()],
            surface_uv_scale: ["1".into(), "1".into()],
            surface_quarter_turns: "0".into(),
            surface_pivot: ["0".into(), "0".into(), "0".into()],
            license_acknowledged: false,
            confirm_remove: false,
            confirm_discard_invalid: false,
            candidate: None,
            scene: None,
            scene_epoch: None,
            graph_revision: None,
            document_pending: false,
            context_stale: false,
            sidecar_sha: None,
            sidecar_error: None,
            history: vec![HistoryEntry {
                id: 0,
                snapshot: None,
            }],
            cursor: 0,
            next_id: 1,
            saved_id: Some(0),
            presentation_error: None,
            status: String::new(),
            error: None,
        }
    }
}

impl Panel {
    pub(crate) fn sync(&mut self, editor: &Editor) {
        if editor.game() != EditorGame::MeshNavigation || !editor.spec().is_local() {
            return;
        }
        if !document_frame_ready(
            editor.game(),
            editor.spec(),
            editor.path().as_deref(),
            editor.mode(),
            editor.snapshot().is_some(),
            editor.checksum(),
            editor.sim().checksum,
        ) {
            self.document_pending = true;
            self.status =
                "Waiting for the requested scene to become the authoritative editor document."
                    .into();
            return;
        }
        let Some(scene) = editor.path() else {
            self.document_pending = true;
            return;
        };
        let Some(snapshot) = editor.snapshot() else {
            self.document_pending = true;
            return;
        };
        self.document_pending = false;
        let mesh_view = mesh_from_view(snapshot.predicted()).ok();
        let revision = mesh_view.as_ref().map(|view| view.graph.revision());
        self.refresh_presentation_error(mesh_view.as_ref());
        let epoch = editor.scene_load_epoch();
        let unchanged = self.scene.as_ref() == Some(&scene)
            && self.scene_epoch == Some(epoch)
            && self.graph_revision == revision;
        if unchanged {
            return;
        }
        let same_epoch_graph_context = self.scene.is_some()
            && self.scene_epoch == Some(epoch)
            && self.graph_revision == revision;
        if same_epoch_graph_context {
            self.scene = Some(scene);
            self.sidecar_sha = None;
            self.context_stale = false;
            if self.current_snapshot().is_some() {
                self.saved_id = None;
                self.status =
                    "Scene path changed; save agent art to create its new sidecar explicitly."
                        .into();
            } else {
                self.saved_id = self.current_entry().map(|entry| entry.id);
                self.status = "Scene path changed; no agent art is present.".into();
            }
            return;
        }
        if self.dirty() {
            self.context_stale = true;
            self.status = "Unsaved agent art belongs to a different scene or mesh revision; undo to the saved state before continuing.".into();
            return;
        }

        self.scene = Some(scene.clone());
        self.scene_epoch = Some(epoch);
        self.graph_revision = revision;
        self.context_stale = false;
        self.candidate = None;
        self.confirm_remove = false;
        self.error = None;
        self.sidecar_error = None;
        self.sidecar_sha = None;
        self.confirm_discard_invalid = false;
        self.reset_history(None, Some(0));
        let Some(revision) = revision else {
            self.status.clear();
            return;
        };
        match crate::mesh_art::load(&scene, revision) {
            Ok(Some((snapshot, sidecar_sha))) => {
                self.copy_transform(&snapshot);
                self.push_initial(Some(snapshot));
                self.sidecar_sha = Some(sidecar_sha);
                self.status = "Loaded saved agent art for this mesh revision.".into();
            }
            Ok(None) => {
                self.status = "No saved agent art for this mesh revision.".into();
            }
            Err(error) => {
                self.sidecar_error = Some(error.clone());
                self.error = Some(error.clone());
                self.sidecar_sha = match crate::mesh_art::sidecar_stamp(&scene) {
                    Ok(stamp) => stamp,
                    Err(stamp_error) => {
                        self.error = Some(format!(
                            "{error}; sidecar cannot be safely removed: {stamp_error}"
                        ));
                        None
                    }
                };
                self.status = format!("Saved agent art could not be admitted: {error}");
            }
        }
        self.refresh_presentation_error(mesh_view.as_ref());
    }

    pub(crate) fn show(&mut self, ui: &mut Ui, editor: &Editor) {
        if editor.game() != EditorGame::MeshNavigation {
            return;
        }
        self.sync(editor);
        let import_enabled = !self.document_pending && self.can_edit_context(editor);
        let import_scene = import_enabled.then(|| editor.path()).flatten();
        if let Some(report) = self.importer.show(
            ui,
            import_scene.as_deref(),
            editor.scene_load_epoch(),
            import_scene.as_ref().and(self.graph_revision),
            import_enabled,
        ) {
            self.package_root = report.root.display().to_string();
            self.asset = report.asset;
            self.license = report.license;
            self.copy_transform(&report.snapshot);
            self.candidate = Some(report.snapshot);
            self.license_acknowledged = false;
            self.error = None;
            self.status = "Imported raw agent art is selected but not applied; use Apply agent art and Save agent art explicitly.".into();
        }
        ui.collapsing("Mesh agent art", |ui| {
            if self.document_pending {
                ui.label("Waiting for the requested scene to become the authoritative editor document.");
            }
            ui.label("Optional package-backed static art follows the admitted mesh agent. Scene data and physics are unchanged.");
            ui.label("The sidecar stores a bounded cooked model snapshot; source package files are not needed after loading.");
            ui.weak("Use trusted, stable local directories; concurrent writers are not isolated.");
            let source_enabled = self.can_edit(editor);
            ui.add_enabled_ui(source_enabled, |ui| {
                let package_label = ui.label("Art package source directory");
                let package_changed = ui
                    .add(egui::TextEdit::singleline(&mut self.package_root).desired_width(f32::INFINITY))
                    .labelled_by(package_label.id)
                    .changed();
                let asset_label = ui.label("Art model asset");
                let asset_changed = ui
                    .add(egui::TextEdit::singleline(&mut self.asset).desired_width(f32::INFINITY))
                    .labelled_by(asset_label.id)
                    .changed();
                let license_label = ui.label("Art license asset");
                let license_changed = ui
                    .add(egui::TextEdit::singleline(&mut self.license).desired_width(f32::INFINITY))
                    .labelled_by(license_label.id)
                    .changed();
                if package_changed || asset_changed || license_changed {
                    self.candidate = None;
                    self.license_acknowledged = false;
                    self.error = None;
                    self.status = "Art source changed; load the package again before applying.".into();
                }
                ui.checkbox(
                    &mut self.license_acknowledged,
                    "I have reviewed rights to use this agent art",
                );
                if ui
                    .add_enabled(
                        source_enabled
                            && self.license_acknowledged
                            && !self.package_root.trim().is_empty()
                            && !self.asset.trim().is_empty()
                            && !self.license.trim().is_empty(),
                        egui::Button::new("Load agent art"),
                    )
                    .clicked()
                {
                    self.load_candidate();
                }
            });

            ui.separator();
            ui.add_enabled_ui(source_enabled, |ui| {
                for (axis, label) in [
                    "Agent art offset X",
                    "Agent art offset Y",
                    "Agent art offset Z",
                ]
                .into_iter()
                .enumerate()
                {
                    text_field(ui, label, &mut self.offset[axis]);
                }
                text_field(ui, "Agent art scale", &mut self.scale);
                for (axis, label) in [
                    "Agent art rotation X (degrees)",
                    "Agent art rotation Y (degrees)",
                    "Agent art rotation Z (degrees)",
                ]
                .into_iter()
                .enumerate()
                {
                    text_field(ui, label, &mut self.rotation[axis]);
                }
                ui.checkbox(&mut self.face_route_direction, "Face route direction");
                for (channel, label) in ["Art tint R", "Art tint G", "Art tint B", "Art tint A"]
                    .into_iter()
                    .enumerate()
                {
                    text_field(ui, label, &mut self.tint[channel]);
                }
                ui.checkbox(&mut self.surface_enabled, "Edit art UV and pivot");
                if self.surface_enabled {
                    text_field(ui, "Art UV material index", &mut self.surface_material);
                    if let Some(snapshot) = self.candidate.as_ref().or_else(|| self.current_snapshot()) {
                        let choices = snapshot
                            .material_choices()
                            .into_iter()
                            .map(|index| index.to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                        ui.label(format!("UV materials used by this model: {choices}"));
                    }
                    text_field(ui, "Art UV offset U", &mut self.surface_uv_offset[0]);
                    text_field(ui, "Art UV offset V", &mut self.surface_uv_offset[1]);
                    text_field(ui, "Art UV scale U", &mut self.surface_uv_scale[0]);
                    text_field(ui, "Art UV scale V", &mut self.surface_uv_scale[1]);
                    text_field(ui, "Art UV quarter turns", &mut self.surface_quarter_turns);
                    for (axis, label) in ["Art pivot X", "Art pivot Y", "Art pivot Z"].into_iter().enumerate() {
                        text_field(ui, label, &mut self.surface_pivot[axis]);
                    }
                }
                if ui.button("Apply agent art").clicked() {
                    self.apply(editor);
                }
                if ui
                    .add_enabled(self.dirty() && !self.facing_form_dirty(), egui::Button::new("Save agent art"))
                    .clicked()
                {
                    self.save(editor);
                }
                ui.checkbox(&mut self.confirm_remove, "I confirm removing saved agent art");
                if ui
                    .add_enabled(
                        self.confirm_remove && self.current_snapshot().is_some(),
                        egui::Button::new("Remove agent art"),
                    )
                    .clicked()
                {
                    self.push_state(None);
                    self.confirm_remove = false;
                    self.status = "Agent art removed in memory; save explicitly to persist.".into();
                    self.error = None;
                }
            });
            let history_enabled = self.can_change_history(editor);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(history_enabled && self.cursor > 0, egui::Button::new("Undo agent art"))
                    .clicked()
                {
                    self.undo();
                }
                if ui
                    .add_enabled(
                        history_enabled && self.cursor + 1 < self.history.len(),
                        egui::Button::new("Redo agent art"),
                    )
                    .clicked()
                {
                    self.redo();
                }
            });
            if self.sidecar_error.is_some()
                && ui
                    .add_enabled(
                        history_enabled && !self.dirty(),
                        egui::Button::new("Reload agent art sidecar"),
                    )
                    .clicked()
            {
                self.reload_sidecar(editor);
            }
            if self.sidecar_error.is_some() {
                ui.checkbox(
                    &mut self.confirm_discard_invalid,
                    "I confirm discarding invalid agent art",
                );
                if ui
                    .add_enabled(
                        self.can_discard_invalid(editor) && self.confirm_discard_invalid,
                        egui::Button::new("Discard invalid agent art sidecar"),
                    )
                    .clicked()
                {
                    self.discard_invalid_sidecar(editor);
                }
            }

            if let Some(snapshot) = self.current_snapshot() {
                ui.label(format!("Current agent art: {}", snapshot.label()));
            }
            if self.dirty() {
                ui.colored_label(egui::Color32::YELLOW, "Unsaved agent art changes block Play.");
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            } else if let Some(error) = &self.presentation_error {
                ui.colored_label(egui::Color32::RED, error);
            } else if !self.status.is_empty() {
                ui.label(&self.status);
            }
        });
    }

    pub(crate) fn placement(&self, editor: &Editor) -> Option<crate::viewport3d::SceneModel> {
        if editor.game() != EditorGame::MeshNavigation
            || self.document_pending
            || self.context_stale
            || self.sidecar_error.is_some()
            || self.presentation_error.is_some()
        {
            return None;
        }
        let (scene, revision) = self.current_identity(editor)?;
        if self.scene.as_ref() != Some(&scene) {
            return None;
        }
        let snapshot = self.current_snapshot()?;
        if snapshot.graph_revision() != revision {
            return None;
        }
        let frame = editor.snapshot()?.predicted();
        let view = mesh_from_view(frame).ok()?;
        snapshot.placement_for_view(&view).ok()
    }

    pub(crate) fn play_blocker(&self, editor: &Editor) -> Option<String> {
        if editor.game() != EditorGame::MeshNavigation {
            return None;
        }
        if self.import_busy() {
            return Some("Finish the raw agent-art import before Play".into());
        }
        if self.document_pending
            || !document_frame_ready(
                editor.game(),
                editor.spec(),
                editor.path().as_deref(),
                editor.mode(),
                editor.snapshot().is_some(),
                editor.checksum(),
                editor.sim().checksum,
            )
        {
            return Some(
                "Wait for the requested scene to become the authoritative editor document".into(),
            );
        }
        if self.context_stale {
            return Some("Resolve unsaved agent art from the prior scene or mesh revision".into());
        }
        if let Some(error) = &self.sidecar_error {
            return Some(format!("Saved agent art is invalid: {error}"));
        }
        if let Some(error) = &self.presentation_error {
            return Some(format!("Agent art cannot be presented: {error}"));
        }
        if self.facing_form_dirty() {
            return Some("Apply or reset the agent art facing option before Play".into());
        }
        if self.dirty() {
            return Some("Save or undo agent art changes before Play".into());
        }
        if self.current_snapshot().is_some() && self.placement(editor).is_none() {
            return Some("Wait for agent art matching the current admitted mesh revision".into());
        }
        None
    }

    pub(crate) fn dirty(&self) -> bool {
        self.current_entry().map(|entry| entry.id) != self.saved_id || self.facing_form_dirty()
    }

    fn facing_form_dirty(&self) -> bool {
        self.current_snapshot().is_some_and(|snapshot| {
            (snapshot.facing() == crate::mesh_facing::Facing::Route) != self.face_route_direction
        })
    }

    pub(crate) fn import_busy(&self) -> bool {
        self.importer.busy()
    }

    fn load_candidate(&mut self) {
        let Some(revision) = self.graph_revision else {
            self.error = Some("Place an admitted mesh agent before loading art".into());
            return;
        };
        let result = Snapshot::from_package(
            std::path::Path::new(self.package_root.trim()),
            self.asset.trim(),
            self.license.trim(),
            revision,
        );
        match result {
            Ok(snapshot) => {
                self.copy_transform(&snapshot);
                self.candidate = Some(snapshot.clone());
                self.status = format!(
                    "Loaded static package art {}; not applied.",
                    snapshot.label()
                );
                self.error = None;
                self.license_acknowledged = false;
            }
            Err(error) => {
                self.candidate = None;
                self.error = Some(format!("Agent art load failed: {error}"));
                self.status = self.error.clone().unwrap_or_default();
            }
        }
    }

    fn apply(&mut self, editor: &Editor) {
        if !self.can_edit(editor) {
            self.error = Some(
                "Agent art can only be applied in the matching editable local mesh scene".into(),
            );
            return;
        }
        let base = self
            .candidate
            .as_ref()
            .or_else(|| self.current_snapshot())
            .cloned();
        let Some(base) = base else {
            self.error = Some("Load a verified package model before applying agent art".into());
            return;
        };
        let result = parse_transform(&self.offset, &self.scale)
            .and_then(|(offset, scale)| base.with_transform(offset, scale))
            .and_then(|snapshot| {
                parse_rotation(&self.rotation).and_then(|rotation| snapshot.with_rotation(rotation))
            })
            .and_then(|snapshot| {
                let facing = if self.face_route_direction {
                    crate::mesh_facing::Facing::Route
                } else {
                    crate::mesh_facing::Facing::Fixed
                };
                snapshot.with_facing(facing)
            })
            .and_then(|snapshot| parse_tint(&self.tint).and_then(|tint| snapshot.with_tint(tint)))
            .and_then(|snapshot| {
                parse_surface(
                    self.surface_enabled,
                    &self.surface_material,
                    &self.surface_uv_offset,
                    &self.surface_uv_scale,
                    &self.surface_quarter_turns,
                    &self.surface_pivot,
                )
                .and_then(|settings| snapshot.with_surface(settings))
            });
        match result {
            Ok(snapshot) if Some(snapshot.graph_revision()) == self.graph_revision => {
                self.push_state(Some(snapshot.clone()));
                self.candidate = Some(snapshot);
                self.status = "Agent art applied in memory; save explicitly to persist.".into();
                self.error = None;
            }
            Ok(_) => {
                self.error =
                    Some("Loaded art belongs to a different mesh revision; load it again".into());
            }
            Err(error) => self.error = Some(format!("Agent art transform rejected: {error}")),
        }
    }

    fn save(&mut self, editor: &Editor) {
        if self.facing_form_dirty() {
            self.error = Some("Apply the agent art facing option before saving".into());
            return;
        }
        if !self.can_edit(editor) {
            self.error = Some("Save agent art only for its matching editable local scene".into());
            return;
        }
        let Some(scene) = self.scene.as_deref() else {
            self.error = Some("Save the mesh scene before saving agent art".into());
            return;
        };
        let current = self.current_snapshot().cloned();
        if current
            .as_ref()
            .is_some_and(|snapshot| Some(snapshot.graph_revision()) != self.graph_revision)
        {
            self.error = Some("Agent art is stale for the current mesh revision".into());
            return;
        }
        match crate::mesh_art::save(scene, current.as_ref(), self.sidecar_sha.as_deref()) {
            Ok(sidecar_sha) => {
                self.sidecar_sha = sidecar_sha;
                self.saved_id = self.current_entry().map(|entry| entry.id);
                self.sidecar_error = None;
                self.status = if current.is_some() {
                    "Agent art saved for this scene.".into()
                } else {
                    "Agent art removed from the scene sidecar.".into()
                };
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Save agent art failed: {error}")),
        }
    }

    fn can_edit(&self, editor: &Editor) -> bool {
        !self.import_busy() && self.can_edit_context(editor)
    }

    fn can_edit_context(&self, editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && !self.context_stale
            && self.sidecar_error.is_none()
            && self.scene.is_some()
            && self.graph_revision.is_some()
            && self
                .current_identity(editor)
                .is_some_and(|(scene, revision)| {
                    self.scene.as_ref() == Some(&scene) && self.graph_revision == Some(revision)
                })
    }

    fn can_change_history(&self, editor: &Editor) -> bool {
        !self.import_busy()
            && editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && !self.document_pending
            && document_frame_ready(
                editor.game(),
                editor.spec(),
                editor.path().as_deref(),
                editor.mode(),
                editor.snapshot().is_some(),
                editor.checksum(),
                editor.sim().checksum,
            )
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
    }

    fn can_discard_invalid(&self, editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && self.sidecar_error.is_some()
            && self.sidecar_sha.is_some()
            && !self.dirty()
            && self
                .current_identity(editor)
                .is_some_and(|(scene, revision)| {
                    self.scene.as_ref() == Some(&scene) && self.graph_revision == Some(revision)
                })
    }

    fn discard_invalid_sidecar(&mut self, editor: &Editor) {
        if !self.can_discard_invalid(editor) {
            self.error = Some("Discard is available only for the unchanged invalid sidecar of this editable local scene".into());
            return;
        }
        let (Some(scene), Some(sidecar_sha)) = (self.scene.as_deref(), self.sidecar_sha.as_deref())
        else {
            self.error = Some("No safely verified invalid sidecar is available to discard".into());
            return;
        };
        match crate::mesh_art::save(scene, None, Some(sidecar_sha)) {
            Ok(_) => {
                self.confirm_discard_invalid = false;
                self.sidecar_sha = None;
                self.sidecar_error = None;
                self.error = None;
                self.status =
                    "Invalid agent art sidecar discarded; load or create art again explicitly."
                        .into();
                self.reset_history(None, Some(0));
                self.copy_current_transform();
                self.refresh_presentation_error(None);
            }
            Err(error) => {
                self.error = Some(format!("Discard invalid agent art sidecar failed: {error}"));
            }
        }
    }

    fn undo(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.copy_current_transform();
            self.status = "Undid agent art edit; save explicitly to persist.".into();
            self.error = None;
        }
    }

    fn redo(&mut self) {
        if self.cursor + 1 < self.history.len() {
            self.cursor += 1;
            self.copy_current_transform();
            self.status = "Redid agent art edit; save explicitly to persist.".into();
            self.error = None;
        }
    }

    fn reload_sidecar(&mut self, editor: &Editor) {
        let Some((scene, revision)) = self.current_identity(editor) else {
            self.error =
                Some("Wait for an admitted local mesh scene before reloading agent art".into());
            return;
        };
        self.scene = Some(scene.clone());
        self.scene_epoch = Some(editor.scene_load_epoch());
        self.graph_revision = Some(revision);
        self.context_stale = false;
        self.sidecar_error = None;
        self.error = None;
        self.sidecar_sha = None;
        self.confirm_discard_invalid = false;
        self.reset_history(None, Some(0));
        match crate::mesh_art::load(&scene, revision) {
            Ok(Some((snapshot, sidecar_sha))) => {
                self.copy_transform(&snapshot);
                self.push_initial(Some(snapshot));
                self.sidecar_sha = Some(sidecar_sha);
                self.status = "Loaded saved agent art for this mesh revision.".into();
            }
            Ok(None) => self.status = "No saved agent art for this mesh revision.".into(),
            Err(error) => {
                self.sidecar_error = Some(error.clone());
                self.error = Some(error.clone());
                self.sidecar_sha = match crate::mesh_art::sidecar_stamp(&scene) {
                    Ok(stamp) => stamp,
                    Err(stamp_error) => {
                        self.error = Some(format!(
                            "{error}; sidecar cannot be safely removed: {stamp_error}"
                        ));
                        None
                    }
                };
                self.status = format!("Saved agent art could not be admitted: {error}");
            }
        }
    }

    fn current_identity(&self, editor: &Editor) -> Option<(PathBuf, [u8; 32])> {
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

    fn refresh_presentation_error(&mut self, view: Option<&orr_remote::mesh_navigation::MeshView>) {
        self.presentation_error = match (self.current_snapshot(), view) {
            (Some(art), Some(view)) if art.graph_revision() == view.graph.revision() => {
                art.placement_for_view(view).err()
            }
            (Some(_), Some(_)) => Some("Agent art belongs to a different mesh revision".into()),
            (Some(_), None) => {
                Some("The current Frame has no admitted mesh agent for agent art".into())
            }
            (None, _) => None,
        };
    }

    fn reset_history(&mut self, snapshot: Option<Snapshot>, saved_id: Option<u64>) {
        self.history.clear();
        self.history.push(HistoryEntry { id: 0, snapshot });
        self.cursor = 0;
        self.next_id = 1;
        self.saved_id = saved_id;
    }

    fn push_initial(&mut self, snapshot: Option<Snapshot>) {
        self.reset_history(snapshot, Some(0));
    }

    fn push_state(&mut self, snapshot: Option<Snapshot>) {
        self.history.truncate(self.cursor + 1);
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.history.push(HistoryEntry { id, snapshot });
        self.cursor = self.history.len() - 1;
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
            self.cursor = self.cursor.saturating_sub(1);
        }
        self.copy_current_transform();
    }

    fn current_entry(&self) -> Option<&HistoryEntry> {
        self.history.get(self.cursor)
    }

    fn current_snapshot(&self) -> Option<&Snapshot> {
        self.current_entry()?.snapshot.as_ref()
    }

    fn copy_transform(&mut self, snapshot: &Snapshot) {
        self.offset = snapshot.offset().map(format_number);
        self.scale = format_number(snapshot.scale());
        self.rotation = crate::mesh_rotation::to_degrees(snapshot.rotation())
            .map(|angles| angles.map(format_number))
            .unwrap_or_else(|_| ["invalid".into(), "invalid".into(), "invalid".into()]);
        self.face_route_direction = snapshot.facing() == crate::mesh_facing::Facing::Route;
        self.tint = snapshot.tint().map(format_number);
        self.copy_surface(snapshot);
    }

    fn copy_current_transform(&mut self) {
        if let Some(snapshot) = self.current_snapshot().cloned() {
            self.copy_transform(&snapshot);
        } else {
            self.offset = ["0".into(), "0".into(), "0".into()];
            self.scale = "1".into();
            self.rotation = ["0".into(), "0".into(), "0".into()];
            self.face_route_direction = false;
            self.tint = ["1".into(), "1".into(), "1".into(), "1".into()];
            self.surface_enabled = false;
            self.surface_material = "0".into();
            self.surface_uv_offset = ["0".into(), "0".into()];
            self.surface_uv_scale = ["1".into(), "1".into()];
            self.surface_quarter_turns = "0".into();
            self.surface_pivot = ["0".into(), "0".into(), "0".into()];
        }
    }

    fn copy_surface(&mut self, snapshot: &Snapshot) {
        if let Some(settings) = snapshot.surface() {
            self.surface_enabled = true;
            self.surface_material = settings.material.to_string();
            self.surface_uv_offset = settings.uv_offset.map(format_number);
            self.surface_uv_scale = settings.uv_scale.map(format_number);
            self.surface_quarter_turns = settings.uv_quarter_turns.to_string();
            self.surface_pivot = settings.pivot.map(format_number);
        } else {
            self.surface_enabled = false;
            self.surface_material = "0".into();
            self.surface_uv_offset = ["0".into(), "0".into()];
            self.surface_uv_scale = ["1".into(), "1".into()];
            self.surface_quarter_turns = "0".into();
            self.surface_pivot = ["0".into(), "0".into(), "0".into()];
        }
    }
}

fn parse_surface(
    enabled: bool,
    material: &str,
    uv_offset: &[String; 2],
    uv_scale: &[String; 2],
    uv_quarter_turns: &str,
    pivot: &[String; 3],
) -> Result<Option<crate::mesh_surface::Settings>, String> {
    if !enabled {
        return Ok(None);
    }
    let material = material
        .trim()
        .parse::<u32>()
        .map_err(|_| "UV material index must be a nonnegative integer")?;
    let parse_pair = |values: &[String; 2], name: &str| -> Result<[f32; 2], String> {
        Ok([
            values[0]
                .trim()
                .parse::<f32>()
                .map_err(|_| format!("{name} U must be a number"))?,
            values[1]
                .trim()
                .parse::<f32>()
                .map_err(|_| format!("{name} V must be a number"))?,
        ])
    };
    let uv_offset = parse_pair(uv_offset, "UV offset")?;
    let uv_scale = parse_pair(uv_scale, "UV scale")?;
    let uv_quarter_turns = uv_quarter_turns
        .trim()
        .parse::<u32>()
        .map_err(|_| "UV quarter turns must be an integer from 0 through 3")?;
    let pivot = [
        pivot[0]
            .trim()
            .parse::<f32>()
            .map_err(|_| "pivot X must be a number")?,
        pivot[1]
            .trim()
            .parse::<f32>()
            .map_err(|_| "pivot Y must be a number")?,
        pivot[2]
            .trim()
            .parse::<f32>()
            .map_err(|_| "pivot Z must be a number")?,
    ];
    if uv_offset
        .iter()
        .chain(uv_scale.iter())
        .chain(pivot.iter())
        .any(|value| !value.is_finite())
    {
        return Err("UV and pivot values must be finite".into());
    }
    if uv_quarter_turns > 3 {
        return Err("UV quarter turns must be from 0 through 3".into());
    }
    Ok(Some(crate::mesh_surface::Settings {
        material,
        uv_offset,
        uv_scale,
        uv_quarter_turns,
        pivot,
    }))
}

fn parse_tint(values: &[String; 4]) -> Result<[f32; 4], String> {
    let mut tint = [0.0; 4];
    for (index, value) in values.iter().enumerate() {
        tint[index] = value
            .trim()
            .parse::<f32>()
            .map_err(|_| "Art tint values must be finite RGBA multipliers in 0..=1")?;
    }
    crate::mesh_material::validate(tint)
}

fn reported_scene_matches_request(
    game: EditorGame,
    spec: &HostSpec,
    reported: Option<&std::path::Path>,
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
    reported: Option<&std::path::Path>,
    mode: Mode,
    has_snapshot: bool,
    frame_checksum: u64,
    state_checksum: u64,
) -> bool {
    reported_scene_matches_request(game, spec, reported)
        && has_snapshot
        && (mode == Mode::Play || frame_checksum == state_checksum)
}

fn text_field(ui: &mut Ui, label: &str, value: &mut String) {
    let label_response = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(label_response.id);
}

fn parse_transform(offset: &[String; 3], scale: &str) -> Result<([f32; 3], f32), String> {
    let parsed_offset = [
        offset[0]
            .parse::<f32>()
            .map_err(|_| "offset X must be a number")?,
        offset[1]
            .parse::<f32>()
            .map_err(|_| "offset Y must be a number")?,
        offset[2]
            .parse::<f32>()
            .map_err(|_| "offset Z must be a number")?,
    ];
    let parsed_scale = scale.parse::<f32>().map_err(|_| "scale must be a number")?;
    if parsed_offset.iter().any(|value| !value.is_finite()) || !parsed_scale.is_finite() {
        return Err("agent art transform values must be finite".into());
    }
    Ok((parsed_offset, parsed_scale))
}

fn parse_rotation(rotation: &[String; 3]) -> Result<[f32; 4], String> {
    let angles = [
        rotation[0]
            .parse::<f32>()
            .map_err(|_| "rotation X must be a number")?,
        rotation[1]
            .parse::<f32>()
            .map_err(|_| "rotation Y must be a number")?,
        rotation[2]
            .parse::<f32>()
            .map_err(|_| "rotation Z must be a number")?,
    ];
    crate::mesh_rotation::from_degrees(angles)
}

fn format_number(value: f32) -> String {
    value.to_string()
}

#[cfg(test)]
#[path = "mesh_art_panel/tests.rs"]
mod tests;
