use super::EditorApp;
use crate::{game::EditorGame, Mode};

impl EditorApp {
    pub(super) fn mesh_workspace_save_blocker(&self) -> Option<String> {
        if self.editor.game() != EditorGame::MeshNavigation
            || !self.editor.spec().is_local()
            || self.editor.mode() != Mode::Edit
            || !self.editor.can_mutate()
            || self.editor.down().is_some()
            || self.editor.previewing().is_some()
            || self.editor.in_gesture()
            || self.editor.history().in_tx
            || self.editor.erp_status().is_some()
            || !self.editor.proposals().is_empty()
        {
            return Some("Saving a mesh workspace requires an idle local Edit host without open proposals or an ERP listener".into());
        }
        if self.editor.path().is_none() {
            return Some("Save the mesh scene to a file before saving its workspace".into());
        }
        if self.package_work_pending()
            || self.shot.is_some()
            || self.remote_capture.is_some()
            || self.encoder.is_some()
            || self.ui.dialog.is_some()
        {
            return Some("Finish content operations, captures and path dialogs before saving the mesh workspace".into());
        }
        #[cfg(feature = "mesh-mission")]
        if self.mesh_game_creator.busy() {
            return Some("Finish or cancel mesh game creation before saving the workspace".into());
        }
        #[cfg(all(windows, target_arch = "x86_64"))]
        if self.mesh_copy.busy() {
            return Some("Finish duplicating the mesh workspace before saving it".into());
        }
        #[cfg(feature = "mesh-export")]
        if self.mesh_export.busy() {
            return Some("Finish exporting the mesh game before saving its workspace".into());
        }
        if self.mesh_navigation.agent_pick_busy()
            || self.mesh_navigation.geometry_vertex_busy()
            || self.mesh_navigation.prop_gesture_busy()
            || self.mesh_mission_pick_busy()
        {
            return Some(
                "Finish or cancel viewport authoring before saving the mesh workspace".into(),
            );
        }
        self.mesh_navigation
            .workspace_save_blocker(&self.editor)
            .or_else(|| self.mesh_camera.workspace_save_blocker(&self.editor))
            .or_else(|| self.mesh_lighting.workspace_save_blocker(&self.editor))
            .or_else(|| self.mesh_hud.workspace_save_blocker(&self.editor))
            .or_else(|| self.mesh_mission_blocker())
            .or_else(|| {
                #[cfg(feature = "mesh-animated-models")]
                {
                    self.mesh_animation.workspace_save_blocker(&self.editor)
                }
                #[cfg(not(feature = "mesh-animated-models"))]
                {
                    self.mesh_animation_blocker()
                }
            })
    }

    pub(super) fn save_mesh_workspace(&mut self) -> bool {
        self.sync_mesh_presentation();
        if let Some(reason) = self.mesh_workspace_save_blocker() {
            self.editor
                .error(format!("Mesh workspace was not saved: {reason}"));
            return false;
        }
        if !self.editor.save() {
            return false;
        }
        let mut saved = vec!["scene"];
        let result = (|| -> Result<(), String> {
            if self
                .mesh_navigation
                .save_workspace_agent_art_changes(&self.editor)?
            {
                saved.push("agent art");
            }
            if self
                .mesh_navigation
                .save_workspace_prop_changes(&self.editor)?
            {
                saved.push("props");
            }
            if self.mesh_camera.save_workspace_changes(&self.editor)? {
                saved.push("camera");
            }
            if self.mesh_lighting.save_workspace_changes(&self.editor)? {
                saved.push("lighting");
            }
            if self.mesh_hud.save_workspace_changes(&self.editor)? {
                saved.push("HUD");
            }
            #[cfg(feature = "mesh-animated-models")]
            if self.mesh_animation.save_workspace_changes(&self.editor)? {
                saved.push("animation");
            }
            Ok(())
        })();
        self.sync_mesh_presentation();
        match result {
            Ok(()) => {
                self.editor.info(format!("Saved applied mesh workspace changes: {}. Unapplied forms are never applied by Save.", saved.join(", ")));
                true
            }
            Err(error) => {
                self.editor.error(format!("Mesh workspace save stopped after saving {}: {error}. Earlier saves remain on disk; remaining changes stay pending.", saved.join(", ")));
                false
            }
        }
    }
}

#[cfg(all(test, feature = "mesh-mission"))]
#[path = "mesh_workspace_save/tests.rs"]
mod tests;
