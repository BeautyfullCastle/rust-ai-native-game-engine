//! Session-only collider framing. Imported model bounds are intentionally excluded.
use super::*;
impl EditorApp {
    fn frame_selection_allowed(&self, ctx: &egui::Context) -> bool {
        if !(self.editor.game().is_room() || self.editor.game() == crate::game::EditorGame::Yard3D)
            || self.editor.mode() != Mode::Edit
            || self.editor.previewing().is_some()
            || self.editor.in_gesture()
            || self.editor.selection().is_none()
            || self.ui.dialog.is_some()
            || ctx.text_edit_focused()
            || !ctx.input(|input| input.focused)
            || ctx.input(|input| {
                input.pointer.secondary_down()
                    || input.pointer.middle_down()
                    || (input.pointer.primary_down()
                        && input.pointer.press_origin().is_some_and(|point| {
                            self.ui
                                .viewport_rect
                                .is_some_and(|rect| rect.contains(point))
                        }))
            })
            || self.package_work_pending()
            || self.hazard_export_busy()
            || self.shot.is_some()
            || self.remote_capture.is_some()
            || self.encoder.is_some()
        {
            return false;
        }
        #[cfg(feature = "room-authoring")]
        if self.room_authoring.wall_footprint_busy()
            || self.room_authoring.actor_placement_busy()
            || self.room_wall_pointer_owned
            || self.room_actor_pointer_owned
        {
            return false;
        }
        #[cfg(feature = "mesh-navigation")]
        if self.mesh_navigation.prop_gesture_busy()
            || self.mesh_navigation.geometry_vertex_busy()
            || self.mesh_navigation.agent_pick_busy()
            || self.mesh_navigation.art_import_busy()
            || self.mesh_navigation.has_unplaced_draft()
            || self.mesh_mission_pick_busy()
            || self.hazard_pick_busy()
        {
            return false;
        }
        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-projects"))]
        if self.windows_projects.busy() {
            return false;
        }
        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-export"))]
        if self.windows_export.open || self.windows_export.busy() {
            return false;
        }
        true
    }
    fn frame_selection_now(&mut self, ctx: &egui::Context, size: (u32, u32)) {
        if self.frame_selection_allowed(ctx) {
            if let Err(error) = self.editor.frame_selected_collider(size) {
                self.editor.error(error);
            }
        }
    }
    pub(super) fn frame_selection_button(&mut self, ui: &mut Ui) {
        if !(self.editor.game().is_room() || self.editor.game() == crate::game::EditorGame::Yard3D)
        {
            return;
        }
        let allowed = self.frame_selection_allowed(ui.ctx());
        if ui.add_enabled(allowed,egui::Button::new("Frame selected collider")).on_hover_text("Session camera only · collider bounds, not imported model bounds · F with viewport focus").clicked() {
            self.frame_selection_now(ui.ctx(),self.ui.viewport_px);
        }
    }
    pub(super) fn frame_selection_shortcut(
        &mut self,
        ui: &mut Ui,
        response: &egui::Response,
        size: (u32, u32),
    ) {
        if response.clicked_by(PointerButton::Secondary)
            || response.clicked_by(PointerButton::Middle)
        {
            response.request_focus();
        }
        if response.dragged() || response.drag_started() || response.drag_stopped() {
            return;
        }
        if response.has_focus()
            && !ui.input(|input| input.pointer.any_down())
            && ui.input(|input| input.key_pressed(Key::F) && input.modifiers == Modifiers::NONE)
        {
            self.frame_selection_now(ui.ctx(), size);
        }
    }
}
