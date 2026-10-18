use crate::{
    backend::HostSpec, editor::Editor, game::EditorGame, mesh_animation::Snapshot, model::Mode,
};
use egui::Ui;
use orr_remote::mesh_navigation::mesh_from_view;
use std::path::{Path, PathBuf};

const MAX_HISTORY: usize = 4;

struct HistoryEntry {
    id: u64,
    snapshot: Option<Snapshot>,
}

pub(crate) struct Panel {
    package_root: String,
    asset: String,
    license: String,
    offset: [String; 3],
    scale: String,
    rotation: [String; 3],
    clip: String,
    idle_clip_enabled: bool,
    idle_clip: String,
    rate: String,
    looped: bool,
    face_route: bool,
    form_dirty: bool,
    source_dirty: bool,
    license_acknowledged: bool,
    confirm_remove: bool,
    confirm_discard: bool,
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
    status: String,
    error: Option<String>,
    preview: preview::State,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            package_root: String::new(),
            asset: "animated.glb".into(),
            license: "LICENSE.txt".into(),
            offset: ["0".into(), "0".into(), "0".into()],
            scale: "1".into(),
            rotation: ["0".into(), "0".into(), "0".into()],
            clip: "0".into(),
            idle_clip_enabled: false,
            idle_clip: "0".into(),
            rate: "1".into(),
            looped: true,
            face_route: false,
            form_dirty: false,
            source_dirty: false,
            license_acknowledged: false,
            confirm_remove: false,
            confirm_discard: false,
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
            status: String::new(),
            error: None,
            preview: preview::State::default(),
        }
    }
}

impl Panel {
    pub(crate) fn sync(&mut self, editor: &Editor) {
        preview::sync(self, editor);
        if editor.game() != EditorGame::MeshNavigation || !editor.spec().is_local() {
            return;
        }
        let Some(scene) = editor.path() else {
            self.document_pending = true;
            return;
        };
        if !document_frame_ready(editor, &scene) {
            self.document_pending = true;
            self.status =
                "Waiting for the requested scene to become the authoritative editor document."
                    .into();
            return;
        }
        let Some(frame) = editor.snapshot().map(|snapshot| snapshot.predicted()) else {
            self.document_pending = true;
            return;
        };
        self.document_pending = false;
        let revision = mesh_from_view(frame).ok().map(|view| view.graph.revision());
        let epoch = editor.scene_load_epoch();
        if self.scene.as_ref() == Some(&scene)
            && self.scene_epoch == Some(epoch)
            && self.graph_revision == revision
        {
            return;
        }
        if self.scene.is_some()
            && self.scene_epoch == Some(epoch)
            && self.graph_revision == revision
        {
            self.scene = Some(scene);
            self.sidecar_sha = None;
            self.context_stale = false;
            if self.current_snapshot().is_some() {
                self.saved_id = None;
                self.status =
                    "Scene path changed; save mesh animation to create its sidecar explicitly."
                        .into();
            }
            return;
        }
        if self.dirty() {
            self.context_stale = true;
            self.status = "Unsaved mesh animation belongs to a different scene or mesh revision; undo or explicitly discard before continuing.".into();
            return;
        }
        self.scene = Some(scene.clone());
        self.scene_epoch = Some(epoch);
        self.graph_revision = revision;
        self.context_stale = false;
        self.candidate = None;
        self.form_dirty = false;
        self.source_dirty = false;
        self.confirm_remove = false;
        self.confirm_discard = false;
        self.error = None;
        self.sidecar_error = None;
        self.sidecar_sha = None;
        self.confirm_discard_invalid = false;
        self.reset_history(None, Some(0));
        let Some(revision) = revision else {
            self.status.clear();
            return;
        };
        match crate::mesh_animation_store::load(&scene, revision) {
            Ok(Some((snapshot, digest))) => {
                self.copy_settings(&snapshot);
                self.reset_history(Some(snapshot), Some(0));
                self.sidecar_sha = Some(digest);
                self.status = "Loaded saved mesh animation for this mesh revision.".into();
            }
            Ok(None) => self.status = "No saved mesh animation for this mesh revision.".into(),
            Err(error) => self.record_sidecar_error(&scene, error),
        }
    }

    pub(crate) fn show(&mut self, ui: &mut Ui, editor: &Editor) {
        if editor.game() != EditorGame::MeshNavigation {
            return;
        }
        self.sync(editor);
        ui.collapsing("Mesh agent animation", |ui| {
            ui.label("Optional package-backed skeletal animation follows the admitted mesh agent. It does not mutate the scene or physics.");
            ui.label("A bounded self-contained snapshot is saved beside the scene; source package files are not needed after loading.");
            ui.weak("Animation art overrides only static agent art while active; removing animation restores the preserved static art.");
            ui.weak("Use trusted, stable local directories; concurrent writers are not isolated.");
            if self.document_pending {
                ui.label("Waiting for the requested scene to become the authoritative editor document.");
            }
            let editable = self.can_edit(editor);
            ui.add_enabled_ui(editable, |ui| {
                let source_changed = text_field(ui, "Animation package source directory", &mut self.package_root)
                    | text_field(ui, "Animation model asset", &mut self.asset)
                    | text_field(ui, "Animation license asset", &mut self.license);
                if source_changed {
                    self.preview.reset();
                    self.source_dirty = true;
                    self.candidate = None;
                    self.license_acknowledged = false;
                    self.error = None;
                    self.status = "Animation source changed; load the package again before applying.".into();
                }
                ui.checkbox(&mut self.license_acknowledged, "I have reviewed rights to use this animation");
                if ui.add_enabled(
                    self.license_acknowledged
                        && !self.package_root.trim().is_empty()
                        && !self.asset.trim().is_empty()
                        && !self.license.trim().is_empty(),
                    egui::Button::new("Load mesh animation"),
                ).clicked() {
                    self.load_candidate();
                }
            });
            ui.separator();
            ui.add_enabled_ui(editable, |ui| {
                let mut changed = false;
                for (axis, label) in ["Animation offset X", "Animation offset Y", "Animation offset Z"].into_iter().enumerate() {
                    changed |= text_field(ui, label, &mut self.offset[axis]);
                }
                changed |= text_field(ui, "Animation scale", &mut self.scale);
                for (axis, label) in ["Animation rotation X (degrees)", "Animation rotation Y (degrees)", "Animation rotation Z (degrees)"].into_iter().enumerate() {
                    changed |= text_field(ui, label, &mut self.rotation[axis]);
                }
                changed |= ui.checkbox(&mut self.idle_clip_enabled, "Use idle and moving clips").changed();
                if self.idle_clip_enabled {
                    changed |= text_field(ui, "Animation idle clip index", &mut self.idle_clip);
                }
                changed |= text_field(ui, "Animation clip index", &mut self.clip);
                changed |= text_field(ui, "Animation playback rate", &mut self.rate);
                changed |= ui.checkbox(&mut self.looped, "Loop mesh animation").changed();
                changed |= ui.checkbox(&mut self.face_route, "Animation face route direction").changed();
                self.form_dirty |= changed;
                if changed {
                    self.preview.reset();
                }
                if let Some(snapshot) = self.candidate.as_ref().or_else(|| self.current_snapshot()) {
                    for (index, name, duration) in snapshot.clips() {
                        ui.label(format!("Clip {index}: {name} ({duration:.3}s)"));
                    }
                }
                if let Some((moving, clip)) = self.animation_state(editor) {
                    let state = if moving { "moving" } else { "idle" };
                    ui.label(format!("Mesh animation state: {state} (clip {clip})"));
                }
                preview::show(ui, self, editor);
                if ui.button("Apply mesh animation").clicked() {
                    self.apply(editor);
                }
                if ui.add_enabled(
                    self.can_save(editor) && self.history_is_dirty(),
                    egui::Button::new("Save mesh animation"),
                ).clicked() {
                    self.save(editor);
                }
                ui.checkbox(&mut self.confirm_remove, "I confirm removing saved mesh animation");
                if ui.add_enabled(
                    editable && !self.form_dirty && !self.source_dirty && self.candidate.is_none()
                        && self.current_snapshot().is_some() && self.confirm_remove,
                    egui::Button::new("Remove mesh animation"),
                ).clicked() {
                    self.push_state(None);
                    self.confirm_remove = false;
                    self.status = "Mesh animation removed in memory; save explicitly to persist.".into();
                    self.error = None;
                }
            });
            let history_enabled = self.can_change_history(editor);
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(history_enabled && !self.form_dirty && !self.source_dirty && self.candidate.is_none() && self.cursor > 0, egui::Button::new("Undo mesh animation")).clicked() { self.undo(); }
                if ui.add_enabled(history_enabled && !self.form_dirty && !self.source_dirty && self.candidate.is_none() && self.cursor + 1 < self.history.len(), egui::Button::new("Redo mesh animation")).clicked() { self.redo(); }
                if ui.add_enabled(history_enabled && !self.dirty(), egui::Button::new("Reload mesh animation")).clicked() { self.reload(editor); }
            });
            if self.dirty() {
                ui.checkbox(&mut self.confirm_discard, "I confirm discarding unsaved mesh animation changes");
                if ui.add_enabled(history_enabled && self.confirm_discard, egui::Button::new("Discard unsaved mesh animation changes")).clicked() {
                    self.discard_changes(editor);
                }
            }
            if self.sidecar_error.is_some() {
                ui.checkbox(&mut self.confirm_discard_invalid, "I confirm discarding invalid mesh animation");
                if ui.add_enabled(history_enabled && self.sidecar_sha.is_some() && self.confirm_discard_invalid && !self.dirty(), egui::Button::new("Discard invalid mesh animation sidecar")).clicked() {
                    self.discard_invalid(editor);
                }
            }
            if let Some(snapshot) = self.current_snapshot() {
                ui.label(format!("Current mesh animation: {}", snapshot.label()));
            }
            if self.dirty() { ui.colored_label(egui::Color32::YELLOW, "Unsaved mesh animation changes block Play, export, and workspace changes."); }
            if let Some(error) = &self.error { ui.colored_label(egui::Color32::RED, error); }
            else if let Some(error) = &self.sidecar_error { ui.colored_label(egui::Color32::RED, error); }
            else if !self.status.is_empty() { ui.label(&self.status); }
        });
    }

    pub(crate) fn dirty(&self) -> bool {
        self.form_dirty
            || self.source_dirty
            || self.candidate.is_some()
            || self.history_is_dirty()
            || self.context_stale
    }

    pub(crate) fn workspace_save_blocker(&self, editor: &Editor) -> Option<String> {
        if editor.game() != EditorGame::MeshNavigation {
            return None;
        }
        if self.document_pending {
            return Some("Wait for the requested mesh scene before saving animation".into());
        }
        if self.context_stale {
            return Some(
                "Resolve animation changes from the prior scene or graph revision before saving"
                    .into(),
            );
        }
        if let Some(error) = &self.sidecar_error {
            return Some(format!(
                "Resolve the invalid mesh animation sidecar before saving: {error}"
            ));
        }
        if self.form_dirty || self.source_dirty || self.candidate.is_some() {
            return Some(
                "Apply or reset animation form changes before saving the workspace".into(),
            );
        }
        if self
            .scene_epoch
            .is_some_and(|epoch| epoch != editor.scene_load_epoch())
        {
            return Some("Animation belongs to a prior scene load; reload it before saving".into());
        }
        if !self.history_is_dirty() {
            return None;
        }
        if !self.can_save(editor) {
            return Some("Save animation only from its matching editable local mesh scene".into());
        }
        let (Some(scene), Some(revision)) = (self.scene.as_deref(), self.graph_revision) else {
            return Some("Save the mesh scene before saving animation".into());
        };
        if self
            .current_identity(editor)
            .is_none_or(|(current_scene, current_revision)| {
                current_scene != scene || current_revision != revision
            })
        {
            return Some("Animation no longer matches the current scene or graph revision".into());
        }
        if self
            .current_snapshot()
            .is_some_and(|snapshot| snapshot.graph_revision() != revision)
        {
            return Some("Mesh animation is stale for the current graph revision".into());
        }
        None
    }

    pub(crate) fn save_workspace_changes(&mut self, editor: &Editor) -> Result<bool, String> {
        if let Some(blocker) = self.workspace_save_blocker(editor) {
            return Err(blocker);
        }
        if !self.history_is_dirty() {
            return Ok(false);
        }
        self.error = None;
        self.save(editor);
        match self.error.clone() {
            Some(error) => Err(error),
            None if self.dirty() => Err("Mesh animation did not reach a saved state".into()),
            None => Ok(true),
        }
    }

    pub(crate) fn active(&self, editor: &Editor) -> bool {
        if self.document_pending || self.sidecar_error.is_some() || self.context_stale {
            return false;
        }
        let Some((scene, revision)) = self.current_identity(editor) else {
            return false;
        };
        self.scene.as_ref() == Some(&scene)
            && self.graph_revision == Some(revision)
            && self.current_snapshot().is_some()
    }

    pub(crate) fn placement(
        &self,
        editor: &Editor,
    ) -> Result<Option<crate::viewport3d::AnimatedPlacement>, String> {
        if self.document_pending || self.context_stale {
            return Err("Wait for the requested mesh scene to become authoritative before presenting animation".into());
        }
        if let Some(error) = &self.sidecar_error {
            return Err(format!("Saved mesh animation is invalid: {error}"));
        }
        let Some(snapshot) = self.current_snapshot() else {
            return Ok(None);
        };
        let Some((scene, revision)) = self.current_identity(editor) else {
            return Err(
                "Wait for a coherent local Mesh Navigation frame before presenting animation"
                    .into(),
            );
        };
        if self.scene.as_ref() != Some(&scene)
            || self.graph_revision != Some(revision)
            || snapshot.graph_revision() != revision
        {
            return Err("Mesh animation belongs to a different scene or graph revision".into());
        }
        let frame = editor
            .snapshot()
            .ok_or_else(|| "Mesh animation requires a coherent Frame".to_owned())?
            .predicted();
        let view = mesh_from_view(frame)?;
        if let Some(placement) = self.preview.placement(self, editor, &view)? {
            return Ok(Some(placement));
        }
        snapshot.placement_for_view(&view, frame.tick()).map(Some)
    }

    fn animation_state(&self, editor: &Editor) -> Option<(bool, u32)> {
        let snapshot = self.current_snapshot()?;
        snapshot.settings().idle_clip?;
        let view = mesh_from_view(editor.snapshot()?.predicted()).ok()?;
        if view.graph.revision() != snapshot.graph_revision() {
            return None;
        }
        Some(snapshot.selected_clip_for_view(&view))
    }

    pub(crate) fn play_blocker(&self, editor: &Editor) -> Option<String> {
        if editor.game() != EditorGame::MeshNavigation {
            return None;
        }
        let Some(scene) = editor.path() else {
            return Some("Wait for a saved local Mesh Navigation scene before Play".into());
        };
        if self.document_pending || !document_frame_ready(editor, &scene) {
            return Some("Wait for the requested mesh scene to become authoritative".into());
        }
        if self.context_stale {
            return Some(
                "Resolve mesh animation edits from the prior scene or graph revision".into(),
            );
        }
        if let Some(error) = &self.sidecar_error {
            return Some(format!("Saved mesh animation is invalid: {error}"));
        }
        if self.dirty() {
            return Some(
                "Apply or discard mesh animation form changes, then save or undo before Play"
                    .into(),
            );
        }
        if self.current_snapshot().is_some() {
            if let Err(error) = self.placement(editor) {
                return Some(error);
            }
        }
        None
    }

    fn load_candidate(&mut self) {
        let Some(revision) = self.graph_revision else {
            self.error = Some("Place an admitted mesh agent before loading animation".into());
            return;
        };
        if !self.license_acknowledged {
            self.error = Some("Review animation rights before loading the package".into());
            return;
        }
        match Snapshot::from_package(
            Path::new(self.package_root.trim()),
            self.asset.trim(),
            self.license.trim(),
            revision,
        ) {
            Ok(snapshot) => {
                self.preview.reset();
                self.copy_settings(&snapshot);
                self.candidate = Some(snapshot.clone());
                self.form_dirty = false;
                self.source_dirty = false;
                self.status = format!(
                    "Loaded animation package {}; not applied.",
                    snapshot.label()
                );
                self.error = None;
                self.license_acknowledged = false;
            }
            Err(error) => {
                self.candidate = None;
                self.error = Some(format!("Mesh animation load failed: {error}"));
            }
        }
    }

    fn apply(&mut self, editor: &Editor) {
        if !self.can_edit(editor) {
            self.error = Some(
                "Mesh animation can only be applied in the matching editable local mesh scene"
                    .into(),
            );
            return;
        }
        if self.sidecar_error.is_some() || self.context_stale {
            self.error =
                Some("Resolve the invalid or stale animation sidecar before applying".into());
            return;
        }
        if self.source_dirty {
            self.error = Some("Load the changed animation package source before applying".into());
            return;
        }
        let Some(base) = self
            .candidate
            .as_ref()
            .or_else(|| self.current_snapshot())
            .cloned()
        else {
            self.error =
                Some("Load a verified animated package before applying mesh animation".into());
            return;
        };
        let settings = match parse_settings(
            &self.offset,
            &self.scale,
            &self.rotation,
            PlaybackFields {
                moving_clip: &self.clip,
                idle_clip: self.idle_clip_enabled.then_some(self.idle_clip.as_str()),
                rate: &self.rate,
                looped: self.looped,
                face_route: self.face_route,
            },
        ) {
            Ok(settings) => settings,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        if !base
            .clips()
            .iter()
            .any(|(index, _, _)| *index == settings.clip)
        {
            self.error = Some(format!(
                "Animation clip index {} is not present in this package",
                settings.clip
            ));
            return;
        }
        match base.with_settings(settings) {
            Ok(snapshot) if Some(snapshot.graph_revision()) == self.graph_revision => {
                self.push_state(Some(snapshot));
                self.candidate = None;
                self.form_dirty = false;
                self.source_dirty = false;
                self.status =
                    "Mesh animation applied in memory; save explicitly to persist.".into();
                self.error = None;
            }
            Ok(_) => {
                self.error =
                    Some("Animation belongs to a different mesh revision; load it again".into())
            }
            Err(error) => self.error = Some(format!("Mesh animation settings rejected: {error}")),
        }
    }

    fn save(&mut self, editor: &Editor) {
        if !self.can_save(editor)
            || !self.history_is_dirty()
            || self.form_dirty
            || self.source_dirty
            || self.candidate.is_some()
        {
            self.error = Some("Apply or discard animation form changes before saving".into());
            return;
        }
        let (Some(scene), Some(revision)) = (self.scene.as_deref(), self.graph_revision) else {
            self.error = Some("Save the mesh scene before saving animation".into());
            return;
        };
        let current = self.current_snapshot().cloned();
        if current
            .as_ref()
            .is_some_and(|snapshot| snapshot.graph_revision() != revision)
        {
            self.error = Some("Mesh animation is stale for the current graph revision".into());
            return;
        }
        match crate::mesh_animation_store::save(
            scene,
            current.as_ref(),
            self.sidecar_sha.as_deref(),
            revision,
        ) {
            Ok(digest) => {
                self.sidecar_sha = digest;
                self.saved_id = self.current_entry().map(|entry| entry.id);
                self.sidecar_error = None;
                self.context_stale = false;
                self.confirm_discard = false;
                self.error = None;
                self.status = if current.is_some() {
                    "Mesh animation saved for this scene.".into()
                } else {
                    "Mesh animation removed from the scene sidecar.".into()
                };
            }
            Err(error) => self.error = Some(format!("Save mesh animation failed: {error}")),
        }
    }

    fn reload(&mut self, editor: &Editor) {
        let Some((scene, revision)) = self.current_identity(editor) else {
            self.error =
                Some("Wait for an admitted local mesh scene before reloading animation".into());
            return;
        };
        self.load_saved(&scene, revision, editor.scene_load_epoch());
    }

    fn discard_changes(&mut self, editor: &Editor) {
        if !self.can_change_history(editor) || !self.confirm_discard {
            self.error = Some("Discard is available only after explicit confirmation in the matching local mesh scene".into());
            return;
        }
        self.candidate = None;
        self.form_dirty = false;
        self.source_dirty = false;
        self.context_stale = false;
        self.load_current_or_reset(editor);
        self.confirm_discard = false;
    }

    fn load_current_or_reset(&mut self, editor: &Editor) {
        let Some((scene, revision)) = self.current_identity(editor) else {
            self.error = Some(
                "Wait for a coherent local mesh scene before discarding animation edits".into(),
            );
            return;
        };
        self.load_saved(&scene, revision, editor.scene_load_epoch());
    }

    fn load_saved(&mut self, scene: &Path, revision: [u8; 32], epoch: u64) {
        self.scene = Some(scene.to_path_buf());
        self.scene_epoch = Some(epoch);
        self.graph_revision = Some(revision);
        self.context_stale = false;
        self.sidecar_error = None;
        self.sidecar_sha = None;
        self.candidate = None;
        self.form_dirty = false;
        self.source_dirty = false;
        self.reset_history(None, Some(0));
        match crate::mesh_animation_store::load(scene, revision) {
            Ok(Some((snapshot, digest))) => {
                self.copy_settings(&snapshot);
                self.reset_history(Some(snapshot), Some(0));
                self.sidecar_sha = Some(digest);
                self.status = "Loaded saved mesh animation for this mesh revision.".into();
                self.error = None;
            }
            Ok(None) => {
                self.copy_current_settings();
                self.status = "No saved mesh animation for this mesh revision.".into();
                self.error = None;
            }
            Err(error) => self.record_sidecar_error(scene, error),
        }
    }

    fn discard_invalid(&mut self, editor: &Editor) {
        if !self.can_discard_invalid(editor) {
            self.error = Some("Discard is available only for the unchanged invalid animation sidecar of this editable local scene".into());
            return;
        }
        let (Some(scene), Some(digest), Some(revision)) = (
            self.scene.as_deref(),
            self.sidecar_sha.as_deref(),
            self.graph_revision,
        ) else {
            self.error =
                Some("No safely verified invalid animation sidecar is available to discard".into());
            return;
        };
        match crate::mesh_animation_store::save(scene, None, Some(digest), revision) {
            Ok(_) => {
                self.sidecar_sha = None;
                self.sidecar_error = None;
                self.confirm_discard_invalid = false;
                self.reset_history(None, Some(0));
                self.copy_current_settings();
                self.status = "Invalid mesh animation sidecar discarded; load or create animation again explicitly.".into();
                self.error = None;
            }
            Err(error) => {
                self.error = Some(format!(
                    "Discard invalid mesh animation sidecar failed: {error}"
                ))
            }
        }
    }

    fn record_sidecar_error(&mut self, scene: &Path, error: String) {
        self.sidecar_error = Some(error.clone());
        match crate::mesh_animation_store::sidecar_stamp(scene) {
            Ok(stamp) => self.sidecar_sha = stamp,
            Err(stamp_error) => {
                self.sidecar_sha = None;
                self.error = Some(format!(
                    "{error}; sidecar cannot be safely removed: {stamp_error}"
                ));
            }
        }
        self.status = format!("Saved mesh animation could not be admitted: {error}");
    }

    fn can_edit(&self, editor: &Editor) -> bool {
        !self.document_pending
            && !self.context_stale
            && self.sidecar_error.is_none()
            && editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && !editor.in_gesture()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && self
                .current_identity(editor)
                .is_some_and(|(scene, revision)| {
                    self.scene.as_ref() == Some(&scene) && self.graph_revision == Some(revision)
                })
    }

    fn can_save(&self, editor: &Editor) -> bool {
        self.can_change_history(editor)
            && self.sidecar_error.is_none()
            && !self.context_stale
            && self
                .current_identity(editor)
                .is_some_and(|(scene, revision)| {
                    self.scene.as_ref() == Some(&scene) && self.graph_revision == Some(revision)
                })
    }

    fn can_change_history(&self, editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && !editor.in_gesture()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && editor
                .path()
                .as_deref()
                .is_some_and(|scene| document_frame_ready(editor, scene))
    }

    fn can_discard_invalid(&self, editor: &Editor) -> bool {
        self.can_change_history(editor)
            && self.sidecar_error.is_some()
            && self.sidecar_sha.is_some()
            && !self.dirty()
            && self
                .current_identity(editor)
                .is_some_and(|(scene, revision)| {
                    self.scene.as_ref() == Some(&scene) && self.graph_revision == Some(revision)
                })
    }

    fn current_identity(&self, editor: &Editor) -> Option<(PathBuf, [u8; 32])> {
        if editor.game() != EditorGame::MeshNavigation || !editor.spec().is_local() {
            return None;
        }
        let scene = editor.path()?;
        if !document_frame_ready(editor, &scene) {
            return None;
        }
        let view = mesh_from_view(editor.snapshot()?.predicted()).ok()?;
        Some((scene, view.graph.revision()))
    }

    fn history_is_dirty(&self) -> bool {
        self.current_entry().map(|entry| entry.id) != self.saved_id
    }

    fn reset_history(&mut self, snapshot: Option<Snapshot>, saved_id: Option<u64>) {
        self.preview.reset();
        self.history.clear();
        self.history.push(HistoryEntry { id: 0, snapshot });
        self.cursor = 0;
        self.next_id = 1;
        self.saved_id = saved_id;
    }

    fn push_state(&mut self, snapshot: Option<Snapshot>) {
        self.preview.reset();
        self.history.truncate(self.cursor + 1);
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.history.push(HistoryEntry { id, snapshot });
        self.cursor = self.history.len() - 1;
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
            self.cursor = self.cursor.saturating_sub(1);
        }
        self.form_dirty = false;
        self.copy_current_settings();
    }

    fn current_entry(&self) -> Option<&HistoryEntry> {
        self.history.get(self.cursor)
    }
    fn current_snapshot(&self) -> Option<&Snapshot> {
        self.current_entry()?.snapshot.as_ref()
    }

    fn copy_settings(&mut self, snapshot: &Snapshot) {
        let settings = snapshot.settings();
        self.offset = settings.offset.map(format_number);
        self.scale = format_number(settings.scale);
        self.rotation = crate::mesh_rotation::to_degrees(settings.rotation)
            .map(|angles| angles.map(format_number))
            .unwrap_or_else(|_| ["invalid".into(), "invalid".into(), "invalid".into()]);
        self.clip = settings.clip.to_string();
        self.idle_clip_enabled = settings.idle_clip.is_some();
        self.idle_clip = settings.idle_clip.unwrap_or(0).to_string();
        self.rate = format_number(settings.rate);
        self.looped = settings.looped;
        self.face_route = settings.facing == crate::mesh_facing::Facing::Route;
    }

    fn copy_current_settings(&mut self) {
        if let Some(snapshot) = self.current_snapshot().cloned() {
            self.copy_settings(&snapshot);
        } else {
            self.offset = ["0".into(), "0".into(), "0".into()];
            self.scale = "1".into();
            self.rotation = ["0".into(), "0".into(), "0".into()];
            self.clip = "0".into();
            self.idle_clip_enabled = false;
            self.idle_clip = "0".into();
            self.rate = "1".into();
            self.looped = true;
            self.face_route = false;
        }
    }

    fn undo(&mut self) {
        if self.cursor > 0 {
            self.preview.reset();
            self.cursor -= 1;
            self.copy_current_settings();
            self.candidate = None;
            self.status = "Undid mesh animation edit; save explicitly to persist.".into();
            self.error = None;
        }
    }

    fn redo(&mut self) {
        if self.cursor + 1 < self.history.len() {
            self.preview.reset();
            self.cursor += 1;
            self.copy_current_settings();
            self.candidate = None;
            self.status = "Redid mesh animation edit; save explicitly to persist.".into();
            self.error = None;
        }
    }
}

#[path = "mesh_animation_panel/preview.rs"]
mod preview;

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
    requested == reported
        || matches!((std::fs::canonicalize(requested), std::fs::canonicalize(reported)), (Ok(left), Ok(right)) if left == right)
}

fn frame_gate(
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

fn document_frame_ready(editor: &Editor, scene: &Path) -> bool {
    frame_gate(
        editor.game(),
        editor.spec(),
        Some(scene),
        editor.mode(),
        editor.snapshot().is_some(),
        editor.checksum(),
        editor.sim().checksum,
    )
}

fn text_field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let response = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(response.id)
        .changed()
}

struct PlaybackFields<'a> {
    moving_clip: &'a str,
    idle_clip: Option<&'a str>,
    rate: &'a str,
    looped: bool,
    face_route: bool,
}

fn parse_settings(
    offset: &[String; 3],
    scale: &str,
    rotation: &[String; 3],
    playback: PlaybackFields<'_>,
) -> Result<crate::mesh_animation::Settings, String> {
    let offset = [
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
    let scale = scale.parse::<f32>().map_err(|_| "scale must be a number")?;
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
    let rotation = crate::mesh_rotation::from_degrees(angles)?;
    let clip = playback
        .moving_clip
        .trim()
        .parse::<u32>()
        .map_err(|_| "clip index must be a nonnegative integer")?;
    let idle_clip = playback
        .idle_clip
        .map(|idle_clip| {
            idle_clip
                .trim()
                .parse::<u32>()
                .map_err(|_| "idle clip index must be a nonnegative integer")
        })
        .transpose()?;
    let rate = playback
        .rate
        .parse::<f32>()
        .map_err(|_| "playback rate must be a number")?;
    if offset.iter().any(|value| !value.is_finite()) || !scale.is_finite() || !rate.is_finite() {
        return Err("Animation transform and playback values must be finite".into());
    }
    Ok(crate::mesh_animation::Settings {
        offset,
        scale,
        rotation,
        facing: if playback.face_route {
            crate::mesh_facing::Facing::Route
        } else {
            crate::mesh_facing::Facing::Fixed
        },
        clip,
        idle_clip,
        rate,
        looped: playback.looped,
    })
}

fn format_number(value: f32) -> String {
    value.to_string()
}

#[cfg(test)]
#[path = "mesh_animation_panel/tests.rs"]
mod tests;
