//! Explicit inspected-route framing; transient session camera only.
use super::*;
#[derive(Default)]
pub(super) struct State {
    frame: Option<u64>,
    buttons: u8,
    scroll: bool,
}
impl State {
    fn sync(&mut self, ctx: &egui::Context, arm: bool) {
        let owned = self.frame.is_some();
        let mut released = 0_u8;
        let mut down = 0_u8;
        ctx.input(|i| {
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
                        button: b, pressed, ..
                    } = event
                    {
                        if *b == button {
                            if *pressed && (owned || arm) {
                                self.buttons |= 1 << index;
                            } else if !*pressed {
                                released |= 1 << index;
                            }
                        }
                    }
                }
            }
            let scrolling = i.is_scrolling() || i.smooth_scroll_delta != egui::Vec2::ZERO;
            if (owned || arm) && scrolling {
                self.scroll = true;
            } else if !scrolling {
                self.scroll = false;
            }
        });
        if owned || arm {
            self.buttons |= down;
        }
        self.buttons &= !released;
        let frame = ctx.cumulative_frame_nr();
        if arm || (owned && (self.buttons != 0 || released != 0 || down != 0)) {
            self.frame = Some(frame);
        } else if self.frame != Some(frame) {
            self.frame = None;
        }
    }
}
impl EditorApp {
    pub(super) fn terrain_patrol_frame_blocks_scroll(&self) -> bool {
        self.terrain_patrol_frame.scroll
    }
    pub(super) fn terrain_patrol_frame_owns_input(&self) -> bool {
        self.terrain_patrol_frame.frame.is_some()
    }
    fn terrain_patrol_frame_blocker(&self, ctx: &egui::Context) -> Option<&'static str> {
        let blocked = !ctx.input(|i| {
            i.focused
                && !i
                    .events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        }) || !self.editor.game().is_navigation()
            || self.editor.mode() != Mode::Edit
            || self.editor.previewing().is_some()
            || self.editor.in_gesture()
            || self.ui.dialog.is_some()
            || self.package_work_pending()
            || self.hazard_export_busy()
            || self.navigation_export_busy()
            || self.navigation_endpoint_owns_input()
            || self.terrain_ramp.active()
            || self.shot.is_some()
            || self.remote_capture.is_some()
            || self.encoder.is_some();
        #[cfg(feature = "mesh-navigation")]
        let blocked = blocked
            || self.mesh_camera_preview_owns_input()
            || self.mesh_camera_preview_scroll
            || self.mesh_light_pick_owns_input()
            || self.mesh_light_scroll_quarantined
            || self.mesh_navigation.prop_gesture_busy()
            || self.mesh_navigation.geometry_vertex_busy()
            || self.mesh_navigation.geometry_preview_active()
            || self.mesh_navigation.agent_pick_busy()
            || self.mesh_navigation.art_import_busy()
            || self.mesh_navigation.has_unplaced_draft()
            || self.mesh_animation_import_busy()
            || self.mesh_mission_pick_busy()
            || self.hazard_pick_busy();
        #[cfg(feature = "mesh-patrol")]
        let blocked = blocked
            || self.mesh_patrol_frame_owns_input()
            || self.mesh_patrol_frame_blocks_scroll();
        #[cfg(feature = "room-lighting")]
        let blocked = blocked || self.room_light_pick_owns_input();
        #[cfg(feature = "room-authoring")]
        let blocked = blocked
            || self.room_authoring.wall_footprint_busy()
            || self.room_authoring.actor_placement_busy()
            || self.room_authoring.wall_placement_busy();
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
            Some(
                "Finish other work and use the current idle Terrain viewport before framing the inspected patrol route",
            )
        } else {
            None
        }
    }
    pub(super) fn sync_terrain_patrol_frame(&mut self, ctx: &egui::Context) {
        self.terrain_patrol_frame.sync(ctx, false);
        let busy = self.terrain_patrol_frame_blocker(ctx).is_some()
            || self.terrain_patrol_frame_owns_input()
            || self.terrain_patrol_frame.scroll;
        self.navigation.set_patrol_frame_busy(busy);
    }
    pub(super) fn dispatch_terrain_patrol_frame(&mut self, ctx: &egui::Context, size: (u32, u32)) {
        let points = match self
            .navigation
            .take_patrol_frame_request(&self.editor, &self.terrain)
        {
            Ok(Some(points)) => points,
            Ok(None) => return,
            Err(error) => {
                self.terrain_patrol_frame.sync(ctx, true);
                self.editor.error(error);
                return;
            }
        };
        let blocked = self.terrain_patrol_frame_blocker(ctx);
        let residual = self.terrain_patrol_frame.scroll
            || ctx.input(|i| i.is_scrolling() || i.smooth_scroll_delta != egui::Vec2::ZERO);
        // Even a rejected action owns this input batch; never cancel then route
        // its simultaneous drag/click into a different authoring tool.
        self.terrain_patrol_frame.sync(ctx, true);
        if let Some(error) = blocked {
            self.editor.error(error);
            return;
        }
        if residual || ctx.input(|i| i.pointer.any_down()) {
            self.editor
                .error("Release viewport gestures and scrolling before framing the patrol route");
            return;
        }
        let valid = self.ui.viewport_rect.is_some_and(|rect| {
            let width = rect.width();
            let height = rect.height();
            let ppp = ctx.pixels_per_point();
            width.is_finite()
                && height.is_finite()
                && width > 0.0
                && height > 0.0
                && ppp.is_finite()
                && ppp > 0.0
                && size.0 > 0
                && size.1 > 0
                && size.0 <= 8192
                && size.1 <= 8192
                && size == self.ui.viewport_px
                && size
                    == (
                        ((width * ppp).round() as u32).clamp(1, 8192),
                        ((height * ppp).round() as u32).clamp(1, 8192),
                    )
        });
        if !valid {
            self.editor
                .error("Framing requires the current visible viewport size");
            return;
        }
        if let Err(error) = self.editor.frame_inspected_terrain_patrol(&points, size) {
            self.editor.error(error);
        }
    }
    pub(super) fn reject_undispatched_terrain_patrol_frame(&mut self, ctx: &egui::Context) {
        match self
            .navigation
            .take_patrol_frame_request(&self.editor, &self.terrain)
        {
            Ok(None) => {}
            Ok(Some(_)) => {
                self.terrain_patrol_frame.sync(ctx, true);
                self.editor
                    .error("Framing requires a visible current viewport");
            }
            Err(error) => {
                self.terrain_patrol_frame.sync(ctx, true);
                self.editor.error(error);
            }
        }
    }
}
#[cfg(all(test, target_os = "linux"))]
#[path = "terrain_patrol_frame_tests.rs"]
mod tests;
