//! Transient Mesh point-light picking, fenced through physical pointer release.
use super::*;
impl EditorApp {
    pub(super) fn mesh_light_pick_pending(&self) -> bool {
        self.mesh_lighting.pick_pending()
    }
    pub(super) fn mesh_light_pick_owns_input(&self) -> bool {
        self.mesh_light_pointer_owned || self.mesh_light_pick_pending()
    }
    /// A wheel event can outlive pointer ownership: egui smooths its hidden
    /// remainder on later passes. Suppress viewport zoom until the public scroll
    /// status and this pass's delta both report that it has drained.
    pub(super) fn mesh_light_pick_blocks_scroll(&mut self, ctx: &egui::Context) -> bool {
        let scrolling =
            ctx.input(|i| i.is_scrolling() || i.smooth_scroll_delta != egui::Vec2::ZERO);
        if self.mesh_light_pick_owns_input() && scrolling {
            self.mesh_light_scroll_quarantined = true;
        } else if !scrolling {
            self.mesh_light_scroll_quarantined = false;
        }
        self.mesh_light_scroll_quarantined
    }
    pub(super) fn cancel_mesh_light_pick(&mut self) {
        self.mesh_lighting.cancel_pick();
    }
    fn mesh_light_pick_blocked(&self, ctx: &egui::Context) -> bool {
        if self.mesh_camera_preview_owns_input() { return true; }
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
            || self.mesh_navigation.geometry_preview_active()
            || self.mesh_navigation.agent_pick_busy()
            || self.mesh_navigation.art_import_busy()
            || self.mesh_navigation.has_unplaced_draft()
            || self.mesh_mission_pick_busy()
            || self.hazard_pick_busy()
        {
            return true;
        }
        #[cfg(feature = "room-lighting")]
        if self.room_light_pick_owns_input() {
            return true;
        }
        #[cfg(feature = "mesh-mission")]
        if self.mesh_game_creator.busy() {
            return true;
        }
        if self.mesh_animation_import_busy() {
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
        if self.navigation_endpoint_owns_input() {
            return true;
        }
        #[cfg(feature = "terrain")]
        if self.terrain_ramp.active() {
            return true;
        }
        !ctx.input(|i| i.focused)
            || ctx.input(|i| i.key_pressed(Key::Escape))
            || ctx.input(|i| {
                i.events
                    .iter()
                    .any(|event| matches!(event, egui::Event::WindowFocused(false)))
            })
            || ctx.text_edit_focused()
            || self.ui.dialog.is_some()
            || !self.editor.game().has_mesh_view()
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
    pub(super) fn sync_mesh_light_pick(&mut self, ctx: &egui::Context) {
        let pending = self.mesh_light_pick_pending();
        let (pressed, released, down) = ctx.input(|i| {
            (
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::PointerButton {
                            button: PointerButton::Primary,
                            pressed: true,
                            ..
                        }
                    )
                }),
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::PointerButton {
                            button: PointerButton::Primary,
                            pressed: false,
                            ..
                        }
                    )
                }),
                i.pointer.primary_down(),
            )
        });
        if (pending || self.mesh_light_pointer_owned) && (pressed || down) {
            self.mesh_light_waiting_release = true;
        }
        if released {
            self.mesh_light_waiting_release = false;
        }
        self.mesh_light_pointer_owned = pending
            || (self.mesh_light_pointer_owned
                && (self.mesh_light_waiting_release || released || down));
        let blocked = self.mesh_light_pick_blocked(ctx);
        if blocked {
            self.cancel_mesh_light_pick();
        }
        self.mesh_lighting.sync_viewport_pick(&self.editor);
        let owned = self.mesh_light_pick_owns_input() || self.mesh_camera_preview_owns_input();
        self.mesh_lighting.set_viewport_pick_busy(owned);
        self.mesh_light_pick_blocks_scroll(ctx);
    }
    /// Receives the final presentation camera and physical size used by this
    /// viewport. Outside controls retain their own input; only a viewport click
    /// can stage a position. The production caller requires a freshly drawn frame.
    pub(super) fn mesh_light_viewport_pick(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        camera: &orr_render::Camera3D,
        size: (u32, u32),
        frame_available: bool,
    ) -> bool {
        let pending = self.mesh_light_pick_pending();
        self.mesh_light_pointer_owned |= pending;
        let position = response
            .interact_pointer_pos()
            .or_else(|| response.hover_pos())
            .filter(|p| response.rect.contains(*p));
        let clicked = position.is_some() && response.clicked_by(PointerButton::Primary);
        if !frame_available || self.mesh_light_pick_blocked(ctx) {
            self.cancel_mesh_light_pick();
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
        let panel = &mut self.mesh_lighting;
        let consumed =
            panel.handle_viewport_pick(&self.editor, camera, size, pixel, clicked, false);
        clicked && consumed
    }
}

#[cfg(all(test, target_os = "linux", feature = "mesh-navigation"))]
#[path = "mesh_light_pick_tests.rs"]
mod tests;
