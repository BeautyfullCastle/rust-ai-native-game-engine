use crate::{
    backend::HostSpec,
    editor::Editor,
    game::EditorGame,
    mesh_camera::{Document, Projection},
    model::Mode,
};
use egui::Ui;
use orr_remote::mesh_navigation::mesh_from_view;
use std::path::{Path, PathBuf};

const MAX_HISTORY: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProjectionMode {
    Perspective,
    Orthographic,
}

#[derive(Clone)]
struct HistoryEntry {
    id: u64,
    document: Option<Document>,
}

pub(crate) struct Panel {
    target: [String; 3],
    follow_agent: bool,
    follow_offset: [String; 3],
    yaw_degrees: String,
    pitch_degrees: String,
    distance: String,
    fov_y_degrees: String,
    near: String,
    far: String,
    half_height: String,
    projection: ProjectionMode,
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
        Self {
            target: ["0".into(), "2".into(), "0".into()],
            follow_agent: false,
            follow_offset: ["0".into(), "0".into(), "0".into()],
            yaw_degrees: "31.51268".into(),
            pitch_degrees: "28.64789".into(),
            distance: "16".into(),
            fov_y_degrees: "50".into(),
            near: "0.1".into(),
            far: "500".into(),
            half_height: "8".into(),
            projection: ProjectionMode::Perspective,
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
    pub(crate) fn sync(&mut self, editor: &mut Editor) {
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
            self.status = "Unsaved game camera belongs to another scene or graph revision; explicitly discard and reload before continuing.".into();
            return;
        }
        if editor.has_mesh_camera() {
            editor.clear_mesh_camera();
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
        match crate::mesh_camera_store::load(&scene, graph_revision) {
            Ok(Some((document, sidecar_sha))) => {
                self.copy_document_fields(&document);
                match editor.install_mesh_camera(document.clone()) {
                    Ok(()) => {
                        self.push_initial(Some(document));
                        self.sidecar_sha = Some(sidecar_sha);
                        self.status =
                            "Loaded the saved game camera for this scene and graph.".into();
                    }
                    Err(error) => self.record_invalid_sidecar(&scene, error),
                }
            }
            Ok(None) => {
                self.copy_orbit_fields(&editor.camera3d);
                self.status = "No saved game camera; viewport navigation remains transient.".into();
            }
            Err(error) => self.record_invalid_sidecar(&scene, error),
        }
    }

    pub(crate) fn show(&mut self, ui: &mut Ui, editor: &mut Editor) {
        if editor.game() != EditorGame::MeshNavigation {
            return;
        }
        self.sync(editor);
        ui.collapsing("Mesh game camera", |ui| {
            if self.document_pending {
                ui.label("Waiting for the requested local mesh scene and admitted graph.");
            }
            ui.label("The optional camera sidecar is view-only; viewport navigation stays transient until captured and applied.");
            ui.weak("Sidecar saves use bounded local files and optimistic digest checks; concurrent writers are not isolated.");
            let can_edit = self.can_edit(editor);
            ui.add_enabled_ui(can_edit, |ui| {
                let mut changed = false;
                for (axis, label) in [
                    "Game camera target X",
                    "Game camera target Y",
                    "Game camera target Z",
                ]
                .into_iter()
                .enumerate()
                {
                    changed |= text_field(ui, label, &mut self.target[axis]);
                }
                changed |= text_field(ui, "Game camera yaw (degrees)", &mut self.yaw_degrees);
                changed |= text_field(ui, "Game camera pitch (degrees)", &mut self.pitch_degrees);
                changed |= text_field(ui, "Game camera distance", &mut self.distance);
                changed |= text_field(
                    ui,
                    "Game camera field of view (degrees)",
                    &mut self.fov_y_degrees,
                );
                changed |= text_field(ui, "Game camera near plane", &mut self.near);
                changed |= text_field(ui, "Game camera far plane", &mut self.far);
                changed |= text_field(
                    ui,
                    "Game camera orthographic half height",
                    &mut self.half_height,
                );
                changed |= ui.checkbox(&mut self.follow_agent, "Follow mesh agent").changed();
                if self.follow_agent {
                    for (axis, label) in [
                        "Camera follow offset X",
                        "Camera follow offset Y",
                        "Camera follow offset Z",
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        changed |= text_field(ui, label, &mut self.follow_offset[axis]);
                    }
                }
                ui.horizontal(|ui| {
                    changed |= ui
                        .radio_value(
                            &mut self.projection,
                            ProjectionMode::Perspective,
                            "Perspective game camera",
                        )
                        .changed();
                    changed |= ui
                        .radio_value(
                            &mut self.projection,
                            ProjectionMode::Orthographic,
                            "Orthographic game camera",
                        )
                        .changed();
                });
                if changed {
                    self.form_dirty = true;
                    self.error = None;
                    self.status = "Game camera form changed; apply it before Play or save.".into();
                }
                ui.horizontal(|ui| {
                    if ui
                        .button("Capture viewport as game camera")
                        .clicked()
                    {
                        self.capture_viewport(editor);
                    }
                    let can_apply = self.form_dirty || self.current_document().is_none();
                    if ui
                        .add_enabled(can_apply, egui::Button::new("Apply game camera"))
                        .clicked()
                    {
                        self.apply(editor);
                    }
                });
            });

            let history_enabled = self.can_change_history(editor);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(history_enabled && self.cursor > 0, egui::Button::new("Undo game camera"))
                    .clicked()
                {
                    self.undo(editor);
                }
                if ui
                    .add_enabled(
                        history_enabled && self.cursor + 1 < self.history.len(),
                        egui::Button::new("Redo game camera"),
                    )
                    .clicked()
                {
                    self.redo(editor);
                }
                if ui
                    .add_enabled(
                        self.can_reset_viewport(editor),
                        egui::Button::new("Reset viewport to game camera"),
                    )
                    .clicked()
                {
                    self.reset_viewport(editor);
                }
            });

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.can_save(editor) && self.history_is_dirty(),
                        egui::Button::new("Save game camera"),
                    )
                    .clicked()
                {
                    self.save(editor);
                }
                if ui
                    .add_enabled(self.can_reload(editor) && !self.dirty(), egui::Button::new("Reload game camera"))
                    .clicked()
                {
                    self.reload(editor);
                }
                self.confirm_remove = self.confirm_remove && self.current_document().is_some();
                ui.checkbox(
                    &mut self.confirm_remove,
                    "I confirm removing the saved game camera",
                );
                if ui
                    .add_enabled(
                        self.can_edit(editor)
                            && self.current_document().is_some()
                            && self.confirm_remove,
                        egui::Button::new("Remove game camera"),
                    )
                    .clicked()
                {
                    self.remove(editor);
                }
            });

            if self.context_stale || self.dirty() {
                ui.checkbox(
                    &mut self.confirm_discard_changes,
                    "I confirm discarding unsaved game camera changes",
                );
                if ui
                    .add_enabled(
                        self.can_discard_changes(editor) && self.confirm_discard_changes,
                        egui::Button::new("Discard game camera changes and reload"),
                    )
                    .clicked()
                {
                    self.discard_changes_and_reload(editor);
                }
            }

            if self.sidecar_error.is_some() {
                ui.checkbox(
                    &mut self.confirm_discard_invalid,
                    "I confirm discarding the invalid game camera sidecar",
                );
                if ui
                    .add_enabled(
                        self.can_discard_invalid(editor) && self.confirm_discard_invalid,
                        egui::Button::new("Discard invalid game camera sidecar"),
                    )
                    .clicked()
                {
                    self.discard_invalid(editor);
                }
            }

            if self.dirty() {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Unsaved camera form or sidecar history blocks Play and export.",
                );
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            } else if !self.status.is_empty() {
                ui.label(&self.status);
            }
        });
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
                "Discard or resolve the game camera from the previous scene or graph revision"
                    .into(),
            );
        }
        if let Some(error) = &self.sidecar_error {
            return Some(format!("Saved game camera is invalid: {error}"));
        }
        if self.dirty() {
            return Some("Apply, save, or discard game camera changes before Play".into());
        }
        if self.current_document().is_some()
            && (self.scene_epoch != Some(editor.scene_load_epoch())
                || current_identity(editor).is_none_or(|(scene, revision)| {
                    self.scene.as_ref() != Some(&scene) || self.graph_revision != Some(revision)
                }))
        {
            return Some("Game camera does not match the current mesh scene and graph".into());
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
        self.can_edit_scene(editor)
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

    fn can_edit_scene(&self, editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation && editor.spec().is_local()
    }

    fn can_change_history(&self, editor: &Editor) -> bool {
        self.can_edit(editor)
    }

    fn can_save(&self, editor: &Editor) -> bool {
        self.can_edit(editor) && !self.form_dirty
    }

    fn can_reload(&self, editor: &Editor) -> bool {
        self.can_edit_scene(editor)
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && !editor.in_gesture()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && !self.document_pending
            && current_identity(editor).is_some()
    }

    fn can_reset_viewport(&self, editor: &Editor) -> bool {
        self.can_edit_scene(editor)
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && !editor.in_gesture()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && !self.document_pending
            && !self.context_stale
            && self.sidecar_error.is_none()
            && self.identity_matches(editor)
            && self.current_document().is_some()
    }

    fn can_discard_changes(&self, editor: &Editor) -> bool {
        self.can_reload(editor)
            && (self.context_stale || self.dirty())
            && current_identity(editor).is_some()
    }

    fn can_discard_invalid(&self, editor: &Editor) -> bool {
        self.can_edit_scene(editor)
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

    fn capture_viewport(&mut self, editor: &Editor) {
        if !self.can_edit(editor) {
            self.error = Some("Capture requires the matching editable local mesh scene".into());
            return;
        }
        match capture_document(editor.camera3d, self.current_document()) {
            Ok(document) => {
                self.copy_document_fields(&document);
                self.form_dirty = true;
                self.error = None;
                self.status =
                    "Viewport pose captured in the form; Apply and Save remain explicit.".into();
            }
            Err(error) => self.error = Some(format!("Capture viewport camera failed: {error}")),
        }
    }

    fn apply(&mut self, editor: &mut Editor) {
        if !self.can_edit(editor) {
            self.error = Some("Apply requires the matching editable local mesh scene".into());
            return;
        }
        let Some((scene, graph_revision)) = current_identity(editor) else {
            self.error = Some("The current mesh scene is not ready for camera changes".into());
            return;
        };
        let document = match self.parse_form(&scene, graph_revision) {
            Ok(document) => document,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        if let Err(error) = editor.install_mesh_camera(document.clone()) {
            self.error = Some(format!("Apply game camera failed: {error}"));
            return;
        }
        self.push_state(Some(document));
        self.copy_current_fields();
        self.form_dirty = false;
        self.error = None;
        self.status = "Game camera applied in memory; save explicitly to persist.".into();
    }

    fn save(&mut self, editor: &Editor) {
        if !self.can_save(editor) {
            self.error = Some(
                "Save requires a clean camera form in its matching editable local scene".into(),
            );
            return;
        }
        let (Some(scene), Some(graph_revision)) = (self.scene.as_deref(), self.graph_revision)
        else {
            self.error = Some("Save the mesh scene before saving its camera".into());
            return;
        };
        match crate::mesh_camera_store::save(
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
                    "Game camera saved for this scene and graph.".into()
                } else {
                    "Game camera removed from the scene sidecar.".into()
                };
            }
            Err(error) => self.error = Some(format!("Save game camera failed: {error}")),
        }
    }

    fn reload(&mut self, editor: &mut Editor) {
        if !self.can_reload(editor) || self.dirty() {
            self.error = Some(
                "Reload requires a clean camera state in the current editable local scene".into(),
            );
            return;
        }
        let Some((scene, graph_revision)) = current_identity(editor) else {
            self.error = Some("The current mesh scene is not ready for camera reload".into());
            return;
        };
        self.load_context(&scene, graph_revision, editor);
    }

    fn remove(&mut self, editor: &mut Editor) {
        if !self.can_edit(editor) || self.current_document().is_none() || !self.confirm_remove {
            self.error =
                Some("Remove requires confirmation and a saved or applied game camera".into());
            return;
        }
        editor.clear_mesh_camera();
        self.push_state(None);
        self.copy_current_fields();
        self.form_dirty = false;
        self.confirm_remove = false;
        self.error = None;
        self.status = "Game camera removed in memory; save explicitly to persist.".into();
    }

    fn reset_viewport(&mut self, editor: &mut Editor) {
        let Some(document) = self.current_document().cloned() else {
            return;
        };
        if !self.can_reset_viewport(editor) {
            self.error = Some("Reset requires the current admitted game camera".into());
            return;
        }
        match editor.install_mesh_camera(document) {
            Ok(()) => {
                self.error = None;
                self.status = "Viewport reset to the saved or applied game camera.".into();
            }
            Err(error) => self.error = Some(format!("Reset viewport failed: {error}")),
        }
    }

    fn discard_changes_and_reload(&mut self, editor: &mut Editor) {
        if !self.can_discard_changes(editor) || !self.confirm_discard_changes {
            self.error = Some(
                "Discard requires explicit confirmation in the current editable local scene".into(),
            );
            return;
        }
        let Some((scene, graph_revision)) = current_identity(editor) else {
            self.error = Some("The current mesh scene is not ready for camera reload".into());
            return;
        };
        self.scene = None;
        self.scene_epoch = None;
        self.graph_revision = None;
        self.context_stale = false;
        self.form_dirty = false;
        self.confirm_discard_changes = false;
        self.reset_history(None, Some(0));
        self.load_context(&scene, graph_revision, editor);
    }

    fn discard_invalid(&mut self, editor: &mut Editor) {
        if !self.can_discard_invalid(editor) || !self.confirm_discard_invalid {
            self.error = Some(
                "Discard requires explicit confirmation for the unchanged invalid camera sidecar"
                    .into(),
            );
            return;
        }
        let (Some(scene), Some(sidecar_sha), Some(graph_revision)) = (
            self.scene.as_deref(),
            self.sidecar_sha.as_deref(),
            self.graph_revision,
        ) else {
            self.error =
                Some("No safely identified invalid camera sidecar is available to remove".into());
            return;
        };
        match crate::mesh_camera_store::save(scene, None, Some(sidecar_sha), graph_revision) {
            Ok(_) => {
                if editor.has_mesh_camera() {
                    editor.clear_mesh_camera();
                }
                self.sidecar_sha = None;
                self.sidecar_error = None;
                self.confirm_discard_invalid = false;
                self.error = None;
                self.reset_history(None, Some(0));
                self.copy_orbit_fields(&editor.camera3d);
                self.status =
                    "Invalid game camera sidecar discarded; capture or apply a camera explicitly."
                        .into();
            }
            Err(error) => {
                self.error = Some(format!(
                    "Discard invalid game camera sidecar failed: {error}"
                ))
            }
        }
    }

    fn undo(&mut self, editor: &mut Editor) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        self.restore_current(editor);
        self.status = "Undid game camera edit; save explicitly to persist.".into();
    }

    fn redo(&mut self, editor: &mut Editor) {
        if self.cursor + 1 >= self.history.len() {
            return;
        }
        self.cursor += 1;
        self.restore_current(editor);
        self.status = "Redid game camera edit; save explicitly to persist.".into();
    }

    fn restore_current(&mut self, editor: &mut Editor) {
        let document = self.current_document().cloned();
        let result = match document {
            Some(document) => editor.install_mesh_camera(document),
            None => {
                editor.clear_mesh_camera();
                Ok(())
            }
        };
        self.copy_current_fields();
        self.form_dirty = false;
        self.error = result
            .err()
            .map(|error| format!("Restore game camera failed: {error}"));
    }

    fn load_context(&mut self, scene: &Path, graph_revision: [u8; 32], editor: &mut Editor) {
        if editor.has_mesh_camera() {
            editor.clear_mesh_camera();
        }
        self.scene = Some(scene.to_path_buf());
        self.scene_epoch = Some(editor.scene_load_epoch());
        self.graph_revision = Some(graph_revision);
        self.document_pending = false;
        self.context_stale = false;
        self.sidecar_sha = None;
        self.sidecar_error = None;
        self.error = None;
        self.confirm_discard_changes = false;
        self.confirm_discard_invalid = false;
        self.reset_history(None, Some(0));
        match crate::mesh_camera_store::load(scene, graph_revision) {
            Ok(Some((document, sidecar_sha))) => match editor.install_mesh_camera(document.clone())
            {
                Ok(()) => {
                    self.copy_document_fields(&document);
                    self.push_initial(Some(document));
                    self.sidecar_sha = Some(sidecar_sha);
                    self.status = "Loaded the saved game camera for this scene and graph.".into();
                }
                Err(error) => self.record_invalid_sidecar(scene, error),
            },
            Ok(None) => {
                if editor.has_mesh_camera() {
                    editor.clear_mesh_camera();
                }
                self.copy_orbit_fields(&editor.camera3d);
                self.status = "No saved game camera; viewport navigation remains transient.".into();
            }
            Err(error) => self.record_invalid_sidecar(scene, error),
        }
    }

    fn record_invalid_sidecar(&mut self, scene: &Path, error: String) {
        self.sidecar_error = Some(error.clone());
        self.error = Some(error.clone());
        self.sidecar_sha = match crate::mesh_camera_store::sidecar_stamp(scene) {
            Ok(stamp) => stamp,
            Err(stamp_error) => {
                self.error = Some(format!(
                    "{error}; sidecar cannot be safely removed: {stamp_error}"
                ));
                None
            }
        };
        self.status = format!("Saved game camera could not be admitted: {error}");
    }

    fn parse_form(&self, scene: &Path, graph_revision: [u8; 32]) -> Result<Document, String> {
        let target = [
            parse_number(&self.target[0], "target X")?,
            parse_number(&self.target[1], "target Y")?,
            parse_number(&self.target[2], "target Z")?,
        ];
        let yaw_degrees = parse_number(&self.yaw_degrees, "yaw")?;
        let pitch_degrees = parse_number(&self.pitch_degrees, "pitch")?;
        let distance = parse_number(&self.distance, "distance")?;
        let fov_y_degrees = parse_number(&self.fov_y_degrees, "field of view")?;
        let near = parse_number(&self.near, "near plane")?;
        let far = parse_number(&self.far, "far plane")?;
        let projection = match self.projection {
            ProjectionMode::Perspective => Projection::Perspective,
            ProjectionMode::Orthographic => Projection::Orthographic {
                half_height: parse_number(&self.half_height, "orthographic half height")?,
            },
        };
        let follow_offset = if self.follow_agent {
            Some([
                parse_number(&self.follow_offset[0], "follow offset X")?,
                parse_number(&self.follow_offset[1], "follow offset Y")?,
                parse_number(&self.follow_offset[2], "follow offset Z")?,
            ])
        } else {
            None
        };
        let document = Document::from_orbit(orr_render::OrbitCamera {
            target,
            yaw: yaw_degrees.to_radians(),
            pitch: pitch_degrees.to_radians(),
            distance,
            fov_y_degrees,
        })?
        .with_projection(projection, near, far)?
        .with_follow_offset(follow_offset)?;
        let filename = scene
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("scene filename must be Unicode")?;
        document.encode(filename, graph_revision)?;
        for size in [(1, 16_384), (16_384, 1), (800, 600)] {
            document.camera(&document.orbit(), size)?;
        }
        Ok(document)
    }

    fn copy_document_fields(&mut self, document: &Document) {
        self.target = document.target.map(format_number);
        self.follow_agent = document.follow_offset.is_some();
        self.follow_offset = document
            .follow_offset
            .unwrap_or([0.0; 3])
            .map(format_number);
        self.yaw_degrees = format_number(document.yaw.to_degrees());
        self.pitch_degrees = format_number(document.pitch.to_degrees());
        self.distance = format_number(document.distance);
        self.fov_y_degrees = format_number(document.fov_y_degrees);
        self.near = format_number(document.near);
        self.far = format_number(document.far);
        self.projection = match document.projection {
            Projection::Perspective => ProjectionMode::Perspective,
            Projection::Orthographic { half_height } => {
                self.half_height = format_number(half_height);
                ProjectionMode::Orthographic
            }
        };
        self.form_dirty = false;
    }

    fn copy_orbit_fields(&mut self, orbit: &orr_render::OrbitCamera) {
        self.target = orbit.target.map(format_number);
        self.follow_agent = false;
        self.follow_offset = ["0".into(), "0".into(), "0".into()];
        self.yaw_degrees = format_number(orbit.yaw.to_degrees());
        self.pitch_degrees = format_number(orbit.pitch.to_degrees());
        self.distance = format_number(orbit.distance);
        self.fov_y_degrees = format_number(orbit.fov_y_degrees);
        self.projection = ProjectionMode::Perspective;
        self.form_dirty = false;
    }

    fn copy_current_fields(&mut self) {
        if let Some(document) = self.current_document().cloned() {
            self.copy_document_fields(&document);
        } else {
            self.target = ["0".into(), "2".into(), "0".into()];
            self.follow_agent = false;
            self.follow_offset = ["0".into(), "0".into(), "0".into()];
            self.yaw_degrees = "31.51268".into();
            self.pitch_degrees = "28.64789".into();
            self.distance = "16".into();
            self.fov_y_degrees = "50".into();
            self.near = "0.1".into();
            self.far = "500".into();
            self.half_height = "8".into();
            self.projection = ProjectionMode::Perspective;
            self.form_dirty = false;
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
        self.reset_history(document, Some(0));
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

fn capture_document(
    orbit: orr_render::OrbitCamera,
    current: Option<&Document>,
) -> Result<Document, String> {
    let (projection, near, far) = current
        .map_or((Projection::Perspective, 0.1, 500.0), |document| {
            (document.projection, document.near, document.far)
        });
    let projection = match (projection, current) {
        (Projection::Orthographic { half_height }, Some(document)) => Projection::Orthographic {
            half_height: half_height * orbit.distance / document.distance,
        },
        (projection, _) => projection,
    };
    let follow_offset = current.and_then(|document| {
        document.follow_offset.map(|offset| {
            std::array::from_fn(|axis| offset[axis] + orbit.target[axis] - document.target[axis])
        })
    });
    Document::from_orbit(orbit)?
        .with_projection(projection, near, far)?
        .with_follow_offset(follow_offset)
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
        return Err(format!("game camera {label} is too long"));
    }
    value
        .trim()
        .parse::<f32>()
        .map_err(|_| format!("game camera {label} must be a number"))
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

#[cfg(test)]
#[path = "mesh_camera_panel/tests.rs"]
mod tests;
