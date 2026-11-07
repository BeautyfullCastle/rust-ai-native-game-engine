//! App routing for the lighting panel's transient fixed-light placement.
use super::*;
impl EditorApp {
    pub(super) fn room_light_pick_pending(&self) -> bool {
        self.room_lighting
            .as_ref()
            .is_some_and(|panel| panel.pick_pending())
    }
    pub(super) fn room_light_pick_owns_input(&self) -> bool {
        self.room_light_pointer_owned || self.room_light_pick_pending()
    }
    pub(super) fn cancel_room_light_pick(&mut self) {
        if let Some(panel) = self.room_lighting.as_mut().filter(|p| p.pick_pending()) {
            panel.cancel_pick();
        }
    }
    fn room_light_pick_blocked(&self, ctx: &egui::Context) -> bool {
        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-projects"))]
        if self.windows_projects.busy() {
            return true;
        }
        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-export"))]
        if self.windows_export.open || self.windows_export.busy() {
            return true;
        }
        #[cfg(feature = "room-authoring")]
        if self.room_authoring.wall_footprint_busy()
            || self.room_authoring.actor_placement_busy()
            || self.room_authoring.wall_placement_busy()
            || self.room_wall_pointer_owned
            || self.room_actor_pointer_owned
            || self.room_wall_placement_pointer_owned
        {
            return true;
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
            return true;
        }
        #[cfg(feature = "mesh-hazards")]
        if self.mesh_hazard_pointer_owned {
            return true;
        }
        #[cfg(feature = "mesh-export")]
        if self.mesh_export.busy()
            || self.mesh_player_session.dialog_open()
            || self.mesh_recorded_playback.active()
        {
            return true;
        }
        #[cfg(feature = "navigation")]
        if self.navigation_endpoint.armed().is_some() {
            return true;
        }
        #[cfg(feature = "terrain")]
        if self.terrain_ramp.active() {
            return true;
        }
        !ctx.input(|i| i.focused)
            || ctx.input(|i| i.key_pressed(Key::Escape))
            || ctx.text_edit_focused()
            || self.ui.dialog.is_some()
            || !self.editor.game().is_room()
            || self.editor.mode() != Mode::Edit
            || self.editor.previewing().is_some()
            || self.editor.in_gesture()
            || self.package_work_pending()
            || self.hazard_export_busy()
            || self.navigation_export_busy()
            || self.shot.is_some()
            || self.remote_capture.is_some()
            || self.encoder.is_some()
    }
    /// Run before global Escape dispatch. A cancelled press remains owned through
    /// its physical release; the following quiet frame retires the fence.
    pub(super) fn sync_room_light_pick(&mut self, ctx: &egui::Context) {
        let pointer_active = ctx.input(|i| i.pointer.any_down() || i.pointer.any_released());
        self.room_light_pointer_owned =
            self.room_light_pick_pending() || (self.room_light_pointer_owned && pointer_active);
        if self.room_light_pick_blocked(ctx) {
            self.cancel_room_light_pick();
        }
    }
    /// Receives the final presentation camera and physical size used by this
    /// viewport. Outside controls retain their own input; only a viewport click
    /// can stage a position. The production caller requires a freshly drawn frame.
    pub(super) fn room_light_viewport_pick(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        camera: &orr_render::Camera3D,
        size: (u32, u32),
    ) -> bool {
        let pending = self.room_light_pick_pending();
        self.room_light_pointer_owned |= pending;
        let position = response
            .interact_pointer_pos()
            .or_else(|| response.hover_pos())
            .filter(|p| response.rect.contains(*p));
        let clicked = position.is_some() && response.clicked_by(PointerButton::Primary);
        if self.room_light_pick_blocked(ctx) {
            self.cancel_room_light_pick();
            return pending && clicked;
        }
        // The rendered physical target may be rounded/clamped independently of
        // the UI scale. Map the complete displayed rectangle to that target.
        let pixel = position.map(|p| {
            [
                (p.x - response.rect.min.x) * size.0 as f32 / response.rect.width(),
                (p.y - response.rect.min.y) * size.1 as f32 / response.rect.height(),
            ]
        });
        let Some(panel) = self.room_lighting.as_mut() else {
            return false;
        };
        let consumed =
            panel.handle_viewport_pick(&self.editor, camera, size, pixel, clicked, false);
        clicked && consumed
    }
}
#[cfg(all(test, target_os = "linux", feature = "project-create"))]
#[path = "room_light_pick_tests.rs"]
mod tests;
