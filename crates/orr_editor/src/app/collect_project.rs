//! Main-editor entry points into the existing closed Collect project creator.
//! All fallible work precedes replacement; no authored data is silently discarded.
#[path = "collect_project/panel.rs"]
mod panel;
#[cfg(test)]
#[path = "collect_project/tests.rs"]
mod tests;

use super::EditorApp;
use crate::{editor::input::Phase, Editor, HostSpec, Mode};
use orr_sample::collect_project::{PreparedProject, ProgressSupport, SpriteSupport};
pub(super) use panel::Panel;
use panel::{Kind, Request};

impl EditorApp {
    pub(super) fn collect_project_dialog_open(&self) -> bool {
        self.ui.collect_project.is_open()
    }

    pub(super) fn collect_project_menu(&mut self, ui: &mut egui::Ui) {
        let blocked = self.collect_project_blocker();
        for (label, kind) in [
            ("New Collect project…", Kind::New),
            ("Open Collect project…", Kind::Open),
        ] {
            if ui
                .add_enabled(blocked.is_none(), egui::Button::new(label))
                .on_disabled_hover_text(
                    blocked
                        .as_deref()
                        .unwrap_or("Finish the current project operation"),
                )
                .clicked()
            {
                self.ui.collect_project.open(kind);
                ui.close();
            }
        }
    }

    /// Returns true for the entire modal frame, including cancellation/adoption.
    /// This prevents queued keyboard/accessibility actions from editing the old
    /// document or immediately acting on the replacement behind the dialog.
    pub(super) fn collect_project_modal(&mut self, ctx: &egui::Context) -> bool {
        if !self.collect_project_dialog_open() {
            return false;
        }
        self.editor.release_control();
        let blocked = self.collect_project_blocker();
        if let Some(request) = self.ui.collect_project.show(ctx, blocked.as_deref()) {
            self.switch_collect_project_with(ctx, request, |_, project| {
                Editor::start(&HostSpec::PreparedCollect {
                    scene: project.path().to_owned(),
                    text: project.scene().text().into(),
                    listen: None,
                    debug_hooks: false,
                })
            });
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(40));
        true
    }

    fn collect_project_blocker(&self) -> Option<String> {
        if !(self.editor.game() == crate::game::EditorGame::PhysGame
            || self.editor.game().is_collect())
            || !self.editor.spec().is_local()
            || self.editor.mode() != Mode::Edit
            || !self.editor.can_mutate()
            || self.editor.is_viewer()
            || self.editor.down().is_some()
            || self.editor.previewing().is_some()
            || self.editor.in_gesture()
            || self.editor.history().in_tx
            || self.editor.sim().in_tx
            || self.editor.input_phase() != Phase::Off
            || self.editor.input_cleanup_pending()
            || self.ui.take_control
            || self.ui.body_drag.is_some()
        {
            return Some("Collect project changes require an idle local Physics or Collect editor in Edit mode with released control.".into());
        }
        if self.editor.erp_status().is_some() {
            return Some("Reopen without an ERP listener before changing projects; the listener is never silently replaced.".into());
        }
        if self.editor.is_dirty() || !self.editor.proposals().is_empty() {
            return Some("Save or explicitly undo scene edits and resolve proposals before changing projects.".into());
        }
        if self.shot.is_some()
            || self.remote_capture.is_some()
            || self.encoder.is_some()
            || self.ui.dialog.is_some()
            || self.package_work_pending()
            || self.hazard_export_busy()
        {
            return Some(
                "Finish content operations, captures and path dialogs before changing projects."
                    .into(),
            );
        }
        #[cfg(feature = "mesh-mission")]
        if self.mesh_game_creator.busy() {
            return Some(
                "Finish or cancel new mesh game creation before changing projects.".into(),
            );
        }
        #[cfg(feature = "mesh-export")]
        if self.mesh_export.busy() {
            return Some("Finish mesh export before changing projects.".into());
        }
        #[cfg(feature = "mesh-navigation")]
        if self.mesh_navigation.art_import_busy()
            || self.mesh_navigation.prop_gesture_busy()
            || self.mesh_navigation.agent_pick_busy()
            || self.mesh_navigation.geometry_vertex_busy()
            || self.mesh_mission_pick_busy()
            || self.hazard_pick_busy()
            || self.mesh_navigation.has_unplaced_draft()
            || self.mesh_camera.dirty()
            || self.mesh_lighting.dirty()
            || self.mesh_hud.dirty()
            || self.mesh_mission_dirty()
            || self.mesh_hazards_dirty()
            || self.mesh_animation_dirty()
        {
            return Some(
                "Finish or explicitly discard mesh authoring work before changing projects.".into(),
            );
        }
        #[cfg(feature = "models")]
        if self.models.bindings.as_ref().is_some_and(|b| b.dirty()) {
            return Some(
                "Save or explicitly discard model bindings before changing projects.".into(),
            );
        }
        #[cfg(feature = "animated-models")]
        if self.ui.animated_window
            || self
                .animated_models
                .bindings
                .as_ref()
                .is_some_and(|b| b.dirty())
        {
            return Some("Close animation authoring and save or discard its bindings before changing projects.".into());
        }
        #[cfg(feature = "terrain")]
        if self.terrain.document.dirty() {
            return Some(
                "Save or explicitly discard terrain edits before changing projects.".into(),
            );
        }
        #[cfg(feature = "irradiance-probes")]
        if self.irradiance.is_baking()
            || self.irradiance.is_validating_bake()
            || self.irradiance.bindings.as_ref().is_some_and(|b| b.dirty())
        {
            return Some(
                "Finish irradiance work and save or discard its bindings before changing projects."
                    .into(),
            );
        }
        #[cfg(feature = "room-project")]
        if self
            .room_camera
            .as_ref()
            .is_some_and(|camera| camera.dirty())
        {
            return Some(
                "Save or explicitly discard Room camera edits before changing projects.".into(),
            );
        }
        if let Some(bindings) = &self.sprites.bindings {
            if bindings.dirty() || !self.sprites.scene_matches(&self.editor) {
                return Some("Save or explicitly discard sprite edits and open matching bindings before changing projects.".into());
            }
            match crate::sprite_bindings::Bindings::open(bindings.path.clone()) {
                Ok(saved) if saved.document() == bindings.document() => {},
                Ok(_) => return Some("Sprite bindings changed on disk; reload or resolve them before changing projects.".into()),
                Err(error) => return Some(format!("Cannot verify saved sprite bindings: {error}")),
            }
        }
        self.collect_ui_project_switch_guard().err()
    }

    /// The starter seam keeps fault injection local to tests. It cannot publish a
    /// replacement: every caller goes through the final current-document guards.
    fn switch_collect_project_with(
        &mut self,
        ctx: &egui::Context,
        request: Request,
        start: impl FnOnce(&mut Self, &PreparedProject) -> Result<Editor, String>,
    ) -> bool {
        let candidate = (|| -> Result<(Editor, PreparedProject), String> {
            if let Some(blocked) = self.collect_project_blocker() {
                return Err(blocked);
            }
            let original = self.editor.collect_project_source_stamp()?;
            if let Some((seed, game_id)) = &request.create {
                // Once this succeeds, failure recovery is Open, never Create.
                let created = orr_sample::project_create::create_collect(
                    &orr_sample::project_create::CreateOptions {
                        output: request.path.clone(),
                        template: orr_sample::project_create::COLLECT_UI_TEMPLATE.into(),
                        seed: seed.clone(),
                    },
                    game_id,
                )?;
                self.ui.collect_project.created(created.output);
            }
            let project = PreparedProject::open_with_ui(
                &request.path,
                ProgressSupport::MetadataOnly,
                SpriteSupport::Supported,
                true,
            )?;
            if project.ui().is_none() || project.sprites().is_none() || project.progress().is_none()
            {
                return Err("Open requires a complete Collect project with sprites, UI and progress identity.".into());
            }
            let mut editor = start(self, &project)?;
            editor.sync();
            if !editor.game().is_collect()
                || editor.path().as_deref() != Some(project.path())
                || editor.checksum() != project.scene().frame().checksum()
            {
                return Err(
                    "Prepared Collect host does not match the admitted project scene.".into(),
                );
            }
            editor
                .collect_project_source_stamp()
                .map_err(|error| format!("Prepared Collect host is not ready: {error}"))?;
            // Read current app state again only after candidate startup. No fonts,
            // texture caches, selection, history or capture ownership changed yet.
            if let Some(blocked) = self.collect_project_blocker() {
                return Err(blocked);
            }
            if self.editor.collect_project_source_stamp()? != original {
                return Err("Current document changed during project preparation; save or resolve it before opening the project.".into());
            }
            Ok((editor, project))
        })();
        match candidate {
            Ok((editor, project)) => {
                let path = project.root().to_owned();
                let mut replacement = Self::new(editor, self.render_state.clone());
                replacement.viewport_hdr_disabled = self.viewport_hdr_disabled;
                // This changes shared egui fonts/textures, so it is deliberately
                // after the last fallible guard and immediately before the swap.
                crate::project::install_collect_presentation(&mut replacement, project, ctx);
                replacement.gpu = self.gpu.take();
                replacement.gpu3d = self.gpu3d.take();
                replacement
                    .editor
                    .info(format!("Collect project ready: {}", path.display()));
                *self = replacement;
                true
            }
            Err(error) => {
                let message = format!(
                    "Current project retained; cannot open {}: {error}",
                    request.path.display()
                );
                self.ui.collect_project.failed(message.clone());
                self.editor.error(message);
                false
            }
        }
    }
}
