//! Recenter only the camera draft target, not a frame/fit operation.
use super::{format_number, mesh_from_view, Document, Editor, Panel, Ui};

impl Panel {
    pub(super) fn show_focus_agent(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        projection_match_requested: bool,
    ) {
        // Do not let a second action hide a same-frame projection-match failure.
        if ui
            .add_enabled(
                self.can_edit(editor) && !self.follow_agent && !projection_match_requested,
                egui::Button::new("Aim target at mesh agent"),
            )
            .clicked()
        {
            self.aim_at_agent(editor);
        }
        ui.weak("Aim changes only target fields; not Frame/Fit. Turn Follow off first. Apply and Save remain explicit.");
    }

    fn aim_at_agent(&mut self, editor: &Editor) {
        let result = (|| {
            if !self.can_edit(editor) {
                return Err(
                    "Aim requires the matching editable local mesh scene and graph".to_owned(),
                );
            }
            if self.follow_agent {
                return Err("Turn Follow mesh agent off before aiming a static target".to_owned());
            }
            let snapshot = editor
                .snapshot()
                .ok_or("Wait for a coherent mesh snapshot")?;
            let view = mesh_from_view(snapshot.predicted()).map_err(|error| error.to_string())?;
            // Same view conversion used by the existing follow-camera consumer.
            // Validate target admission without changing or requiring other draft fields.
            let target = view.agent.position.map(orr_fp::FP::to_f32);
            Document::from_orbit(orr_render::OrbitCamera {
                target,
                yaw: 0.0,
                pitch: 0.0,
                distance: 10.0,
                fov_y_degrees: 60.0,
            })?;
            Ok(target)
        })();
        match result {
            Ok(target) => {
                self.target = target.map(format_number);
                self.form_dirty = true;
                self.error = None;
                self.status = "Camera target copied from the mesh agent into the form; Apply and Save remain explicit. This does not fit its bounds.".into();
            }
            Err(error) => self.error = Some(format!("Aim camera target failed: {error}")),
        }
    }
}

#[cfg(test)]
mod tests;
