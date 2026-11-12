//! Render-local static Mesh camera previews and their transient input fence.
use super::*;

impl EditorApp {
    pub(super) fn mesh_camera_preview_owns_input(&self) -> bool {
        self.mesh_camera.static_preview_enabled() || self.mesh_camera_preview_owned
    }
    fn mesh_camera_preview_busy(&self) -> bool {
        let blocked = self.ui.dialog.is_some()
            || self.package_work_pending()
            || self.hazard_export_busy()
            || self.navigation_export_busy()
            || self.editor.in_gesture()
            || self.editor.previewing().is_some()
            || self.mesh_navigation.prop_gesture_busy()
            || self.mesh_navigation.geometry_vertex_busy()
            || self.mesh_navigation.geometry_preview_active()
            || self.mesh_navigation.agent_pick_busy()
            || self.mesh_navigation.art_import_busy()
            || self.mesh_navigation.has_unplaced_draft()
            || self.mesh_animation_import_busy()
            || self.mesh_mission_pick_busy()
            || self.hazard_pick_busy()
            || self.mesh_light_pick_owns_input()
            || self.shot.is_some()
            || self.remote_capture.is_some()
            || self.encoder.is_some();
        #[cfg(feature = "mesh-mission")]
        let blocked = blocked || self.mesh_game_creator.busy();
        #[cfg(feature = "mesh-export")]
        let blocked = blocked
            || self.mesh_export.busy()
            || self.mesh_player_session.dialog_open()
            || self.mesh_recorded_playback.active();
        #[cfg(feature = "navigation")]
        let blocked = blocked || self.navigation_endpoint_owns_input();
        #[cfg(feature = "terrain")]
        let blocked = blocked || self.terrain_ramp.active();
        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-projects"))]
        let blocked = blocked || self.windows_projects.busy();
        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-export"))]
        let blocked = blocked || self.windows_export.open || self.windows_export.busy();
        blocked
    }
    pub(super) fn sync_mesh_camera_preview(&mut self, ctx: &egui::Context) {
        let active = self.mesh_camera.static_preview_enabled();
        let owned = active || self.mesh_camera_preview_owned;
        let (pressed, released, down, scrolling) = ctx.input(|i| {
            let mut pressed = 0_u8;
            let mut released = 0_u8;
            let mut down = 0_u8;
            for (index, button) in [
                PointerButton::Primary,
                PointerButton::Secondary,
                PointerButton::Middle,
                PointerButton::Extra1,
                PointerButton::Extra2,
            ]
            .into_iter()
            .enumerate()
            {
                if i.pointer.button_down(button) {
                    down |= 1 << index;
                }
                for event in &i.events {
                    if let egui::Event::PointerButton {
                        button: event_button,
                        pressed: event_pressed,
                        ..
                    } = event
                    {
                        if *event_button == button {
                            if *event_pressed {
                                pressed |= 1 << index;
                            } else {
                                released |= 1 << index;
                            }
                        }
                    }
                }
            }
            (
                pressed,
                released,
                down,
                i.is_scrolling() || i.smooth_scroll_delta != egui::Vec2::ZERO,
            )
        });
        if owned {
            self.mesh_camera_preview_buttons |= pressed | down;
        }
        self.mesh_camera_preview_buttons &= !released;
        if owned && scrolling {
            self.mesh_camera_preview_scroll = true;
        } else if !scrolling {
            self.mesh_camera_preview_scroll = false;
        }
        let frame = ctx.cumulative_frame_nr();
        if active
            || (self.mesh_camera_preview_owned
                && (self.mesh_camera_preview_buttons != 0 || released != 0 || down != 0))
        {
            self.mesh_camera_preview_fenced_frame = Some(frame);
        }
        self.mesh_camera_preview_owned = self.mesh_camera_preview_fenced_frame == Some(frame);
        let blocked = self.mesh_camera_preview_busy();
        let focused = ctx.input(|i| {
            i.focused
                && !i
                    .events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        });
        if blocked || !focused {
            self.mesh_camera.clear_static_preview();
        } else {
            self.mesh_camera.sync_static_preview(&self.editor);
        }
        let light_busy = self.mesh_light_pick_owns_input() || self.mesh_camera_preview_owns_input();
        self.mesh_lighting.set_viewport_pick_busy(light_busy);
        self.mesh_camera.set_static_preview_busy(
            blocked
                || !focused
                || self.mesh_camera_preview_owned
                || self.mesh_camera_preview_scroll,
        );
    }
    pub(super) fn mesh_camera_for_frame(
        &mut self,
        size: (u32, u32),
    ) -> Result<orr_render::Camera3D, String> {
        if let Some(camera) = self.mesh_camera.static_preview_camera(&self.editor, size) {
            return Ok(camera);
        }
        self.editor.presentation_camera3d(size)
    }
}
#[cfg(all(test, target_os = "linux", feature = "mesh-mission"))]
#[path = "mesh_camera_preview_tests.rs"]
mod tests;
