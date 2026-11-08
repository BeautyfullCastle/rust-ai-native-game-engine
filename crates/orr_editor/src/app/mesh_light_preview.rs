//! Frame-level retirement for the opt-in Mesh point-light draft preview.
use super::*;

impl EditorApp {
    pub(super) fn sync_mesh_light_preview(&mut self, ctx: &egui::Context) {
        // Validate even when the inspector/viewport is hidden, so a context ABA
        // cannot restore a preview that was invalid in an intervening frame.
        self.mesh_lighting.sync_preview(&self.editor);
        if !self.mesh_lighting.point_light_preview_enabled() {
            return;
        }
        let blocked = !ctx.input(|i| i.focused)
            || ctx.input(|i| {
                i.events
                    .iter()
                    .any(|event| matches!(event, egui::Event::WindowFocused(false)))
            })
            || self.editor.mode() != Mode::Edit
            || self.editor.previewing().is_some()
            || self.editor.in_gesture()
            || self.ui.dialog.is_some()
            || self.package_work_pending()
            || self.hazard_export_busy()
            || self.navigation_export_busy()
            || self.mesh_navigation.prop_gesture_busy()
            || self.mesh_navigation.geometry_vertex_busy()
            || self.mesh_navigation.geometry_preview_active()
            || self.mesh_navigation.agent_pick_busy()
            || self.mesh_navigation.art_import_busy()
            || self.mesh_navigation.has_unplaced_draft()
            || self.mesh_animation_import_busy()
            || self.mesh_mission_pick_busy()
            || self.hazard_pick_busy()
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
        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-projects"))]
        let blocked = blocked || self.windows_projects.busy();
        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-export"))]
        let blocked = blocked || self.windows_export.open || self.windows_export.busy();
        if blocked {
            let _ = self
                .mesh_lighting
                .set_point_light_preview(&self.editor, false);
        }
    }
}

#[cfg(all(test, target_os = "linux", feature = "mesh-mission"))]
#[path = "mesh_light_preview_tests.rs"]
mod tests;
