//! Explicit, local Room viewport camera capture. The existing camera document
//! panel owns validation, persistence and undo; this child only coordinates UI.
use super::EditorApp;
use crate::{
    room_camera_capture::{Context, State},
    Mode,
};
use orr_sample::room_camera::Document;

struct Edit {
    before: Context,
    after: Document,
    undone: bool,
}
#[derive(Default)]
pub(super) struct Capture {
    staged: State,
    edit: Option<Edit>,
    message: String,
}
impl Capture {
    pub(super) fn owns_history(&self) -> bool {
        self.edit.is_some()
    }
    pub(super) fn invalidate(&mut self) {
        self.staged.cancel();
        self.edit = None;
    }
}
impl EditorApp {
    fn room_capture_context(&self, allow_owned_camera_dirty: bool) -> Option<Context> {
        let panel = self.room_camera.as_ref()?;
        if !self.editor.game().is_room()
            || !self.editor.spec().is_local()
            || self.editor.mode() != Mode::Edit
            || !self.editor.can_mutate()
            || !self.editor.has_room_camera()
            || !self.editor.yard_rows_coherent()
            || !self.editor.room_camera_capture_coherent()
            || !self.editor.proposals().is_empty()
            || !self.models.scene_matches(&self.editor)
            || self.editor.is_dirty()
            || self.editor.in_gesture()
            || self.editor.history().in_tx
            || self.editor.sim().in_tx
            || self.editor.previewing().is_some()
            || self.editor.erp_status().is_some()
            || self
                .models
                .bindings
                .as_ref()
                .is_none_or(|bindings| bindings.dirty())
            || (!allow_owned_camera_dirty && panel.dirty())
            || self.ui.dialog.is_some()
            || self.shot.is_some()
            || self.remote_capture.is_some()
            || self.encoder.is_some()
            || self.package_work_pending()
            || self.hazard_export_busy()
        {
            return None;
        }
        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-projects"))]
        if self.windows_projects.busy() {
            return None;
        }
        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-export"))]
        if self.windows_export.open || self.windows_export.busy() {
            return None;
        }
        Some(Context {
            scene: self.editor.path()?,
            epoch: self.editor.scene_load_epoch(),
            checksum: self.editor.checksum(),
            applied: panel.document().clone(),
        })
    }
    pub(super) fn sync_room_camera_capture(&mut self) {
        let context = self.room_capture_context(false);
        if self.room_camera_capture.staged.sync(context.as_ref()) {
            self.room_camera_capture.message =
                "Staged camera cleared because the Room context changed.".into();
        }
        let current = self.room_capture_context(true);
        let keep = self.room_camera_capture.edit.as_ref().is_none_or(|edit| {
            current.as_ref().is_some_and(|now| {
                now.scene == edit.before.scene
                    && now.epoch == edit.before.epoch
                    && now.checksum == edit.before.checksum
                    && now.applied
                        == if edit.undone {
                            edit.before.applied.clone()
                        } else {
                            edit.after.clone()
                        }
            })
        });
        if !keep {
            self.room_camera_capture.edit = None;
        }
    }
    pub(super) fn show_room_camera_capture(&mut self, ui: &mut egui::Ui) {
        self.sync_room_camera_capture();
        if !self.editor.game().is_room() || self.room_camera.is_none() {
            return;
        }
        ui.separator();
        ui.label("Capture Room viewport camera");
        let current = self.room_capture_context(false);
        let owned = self.room_camera_capture.edit.is_some();
        if ui
            .add_enabled(
                current.is_some() && !owned && !self.room_camera_capture.staged.busy(),
                egui::Button::new("Stage viewport camera"),
            )
            .clicked()
        {
            if let Some(context) = self.room_capture_context(false) {
                self.room_camera_capture.message = match self.room_camera_capture.staged.stage(context,&self.editor.camera3d) {
                    Ok(()) => "Viewport camera staged only. Apply to author it, or Cancel to keep the saved camera.".into(),
                    Err(error) => error,
                };
            }
        }
        let candidate = current
            .as_ref()
            .and_then(|context| self.room_camera_capture.staged.candidate(context))
            .cloned();
        if ui
            .add_enabled(
                candidate.is_some(),
                egui::Button::new("Apply staged viewport camera"),
            )
            .clicked()
        {
            if let Some(context) = self.room_capture_context(false) {
                if let Some(candidate) =
                    self.room_camera_capture.staged.candidate(&context).cloned()
                {
                    if candidate == context.applied {
                        self.room_camera_capture.staged.cancel();
                        self.room_camera_capture.message =
                            "Viewport already matches the authored camera; no history entry added."
                                .into();
                        return;
                    }
                    let result = self
                        .room_camera
                        .as_mut()
                        .unwrap()
                        .apply_document(candidate.clone())
                        .and_then(|()| self.editor.install_room_camera(candidate.clone()));
                    self.room_camera_capture.staged.cancel();
                    match result {
                        Ok(()) => {
                            self.room_camera_capture.edit = Some(Edit {
                                before: context,
                                after: candidate,
                                undone: false,
                            });
                            self.room_camera_capture.message = "Captured camera applied in memory. Save captured camera explicitly to persist it.".into();
                        }
                        Err(error) => self.room_camera_capture.message = error,
                    }
                }
            }
        }
        if ui
            .add_enabled(
                self.room_camera_capture.staged.busy(),
                egui::Button::new("Cancel staged viewport camera"),
            )
            .clicked()
        {
            self.room_camera_capture.staged.cancel();
            self.room_camera_capture.message =
                "Staged camera cancelled; authored camera unchanged.".into();
        }
        if owned {
            ui.label("Capture-specific history is active. Finish it before using other camera authoring controls.");
            let undone = self
                .room_camera_capture
                .edit
                .as_ref()
                .is_some_and(|edit| edit.undone);
            if ui
                .add_enabled(!undone, egui::Button::new("Undo captured camera"))
                .clicked()
            {
                self.sync_room_camera_capture();
                if self
                    .room_camera_capture
                    .edit
                    .as_ref()
                    .is_some_and(|edit| !edit.undone)
                {
                    if self.room_camera.as_mut().unwrap().undo() {
                        let document = self.room_camera.as_ref().unwrap().document().clone();
                        if let Err(error) = self.editor.install_room_camera(document) {
                            self.room_camera_capture.message = error;
                        }
                        self.room_camera_capture.edit.as_mut().unwrap().undone = true;
                    } else {
                        self.room_camera_capture.edit = None;
                    }
                }
            }
            if ui
                .add_enabled(undone, egui::Button::new("Redo captured camera"))
                .clicked()
            {
                self.sync_room_camera_capture();
                if let Some(edit) = self
                    .room_camera_capture
                    .edit
                    .as_ref()
                    .filter(|edit| edit.undone)
                {
                    let document = edit.after.clone();
                    let result = self
                        .room_camera
                        .as_mut()
                        .unwrap()
                        .apply_document(document.clone())
                        .and_then(|()| self.editor.install_room_camera(document));
                    match result {
                        Ok(()) => self.room_camera_capture.edit.as_mut().unwrap().undone = false,
                        Err(error) => self.room_camera_capture.message = error,
                    }
                }
            }
            if ui.button("Save captured camera").clicked() {
                self.sync_room_camera_capture();
                if self.room_camera_capture.edit.is_some() {
                    self.room_camera_capture.message =
                        match self.room_camera.as_mut().unwrap().save() {
                            Ok(()) => "Captured camera saved.".into(),
                            Err(error) => error,
                        };
                }
            }
            if ui.button("Finish captured camera history").clicked() {
                self.room_camera_capture.edit = None;
            }
        }
        if current.is_none() && !owned {
            ui.label("Capture requires a saved, clean local Room in Edit mode with idle authoring and no active jobs.");
        }
        if !self.room_camera_capture.message.is_empty() {
            ui.label(&self.room_camera_capture.message);
        }
    }
}
