//! Pointer ownership and rendered-view admission for transient Room wall drafts.
use super::*;

impl EditorApp {
    fn room_wall_footprint_blocked(&self, ctx: &egui::Context) -> bool {
        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-projects"))]
        if self.windows_projects.busy() {
            return true;
        }
        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-export"))]
        if self.windows_export.open || self.windows_export.busy() {
            return true;
        }
        !ctx.input(|input| input.focused)
            || ctx.input(|input| input.key_pressed(Key::Escape))
            || ctx.text_edit_focused()
            || self.ui.dialog.is_some()
            || self.editor.mode() != Mode::Edit
            || self.editor.previewing().is_some()
            || self.editor.is_dirty()
            || self.package_work_pending()
            || self.hazard_export_busy()
            || self.shot.is_some()
            || self.remote_capture.is_some()
            || self.encoder.is_some()
            || !self.models.scene_matches(&self.editor)
            || self
                .models
                .bindings
                .as_ref()
                .is_none_or(|bindings| bindings.dirty())
            || self
                .room_camera
                .as_ref()
                .is_some_and(|camera| camera.dirty())
    }

    pub(super) fn sync_room_wall_footprint(&mut self, ctx: &egui::Context) {
        self.room_wall_pointer_owned = self.room_authoring.wall_footprint_busy();
        self.room_authoring.sync_wall_footprint(&self.editor);
        if self.room_wall_footprint_blocked(ctx) {
            self.room_authoring.cancel_wall_footprint();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn room_wall_footprint_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        camera: &orr_render::Camera3D,
        px: (u32, u32),
        rect: Rect,
        pixels_per_point: f32,
        rendered: bool,
    ) {
        let cancel = self.room_wall_footprint_blocked(ui.ctx()) || !rendered;
        let pixel = response
            .interact_pointer_pos()
            .or_else(|| response.hover_pos())
            .map(|point| {
                [
                    (point.x - rect.min.x) * pixels_per_point,
                    (point.y - rect.min.y) * pixels_per_point,
                ]
            });
        self.room_authoring.wall_footprint_input(
            &self.editor,
            camera,
            px,
            crate::room_authoring_panel::ViewInput {
                pixel,
                clicked: rendered && response.clicked_by(PointerButton::Primary),
                cancel,
            },
        );
    }
}
