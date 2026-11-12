//! One optional unshadowed point light, staged alongside existing lighting.
use super::*;
fn defaults() -> orr_render::PointLight {
    orr_render::PointLight {
        position: [0.0, 4.0, 0.0],
        color: [1.0, 0.9, 0.7],
        intensity: 3.0,
        range: 12.0,
    }
}
fn number(ui: &mut Ui, label: &str, value: &mut f32, min: f32, max: f32) -> bool {
    ui.horizontal(|ui| {
        let label = ui.label(label);
        ui.add(egui::DragValue::new(value).speed(0.05).range(min..=max))
            .labelled_by(label.id)
            .changed()
    })
    .inner
}
impl Panel {
    pub(super) fn show_point_light_preview_controls(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        can_edit: bool,
    ) {
        self.sync_preview(editor);
        let focused = ui.input(|input| {
            input.focused
                && !input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::WindowFocused(false)))
        });
        let ready = can_edit
            && self.can_edit(editor)
            && focused
            && self
                .scene
                .as_deref()
                .zip(self.graph_revision)
                .is_some_and(|(scene, revision)| self.parse_document(scene, revision).is_ok());
        let mut enabled = self.point_light_preview_enabled();
        let response = ui.add_enabled(
            ready || enabled,
            egui::Checkbox::new(&mut enabled, "Preview point light draft"),
        );
        if response.changed() && (!enabled || ready) {
            if let Err(error) = self.set_point_light_preview(editor, enabled) {
                self.error = Some(error);
            }
        }
        ui.weak("Preview only: shows the validated point light draft in this viewport. Apply and Save explicitly to persist it; reopening starts with preview off.");
    }
    pub(super) fn show_point_light_pick_controls(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        can_edit: bool,
    ) {
        let focused = ui.input(|input| {
            input.focused
                && !input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::WindowFocused(false)))
        });
        let ready = can_edit
            && self.can_edit(editor)
            && focused
            && self.point_light.is_some()
            && !self.pick_pending()
            && !self.viewport_pick_busy();
        if ui
            .add_enabled(ready, egui::Button::new("Pick point light X/Z"))
            .clicked()
            && ready
        {
            if let Err(error) = self.begin_viewport_pick(editor) {
                self.error = Some(error);
            }
        }
        if self.pick_pending() {
            ui.weak("Click the viewport Y=0 plane to stage X/Z only; height and other light settings stay unchanged. Apply and Save explicitly.");
            if ui.button("Cancel point light pick").clicked() {
                self.cancel_pick();
            }
        }
    }
    pub(super) fn show_point_light_controls(&mut self, ui: &mut Ui) -> bool {
        let mut enabled = self.point_light.is_some();
        let mut changed = ui.checkbox(&mut enabled, "Enable point light").changed();
        if changed {
            self.point_light = enabled.then(defaults);
        }
        if let Some(light) = self.point_light.as_mut() {
            ui.collapsing("Point light settings",|ui| {
                ui.weak("One unshadowed point light in world coordinates. Changes are drafts until Apply game lighting and Save; they never change physics.");
                for(axis,value)in ["X","Y","Z"].into_iter().zip(&mut light.position) {changed|=number(ui,&format!("Point light position {axis}"),value,-1e9,1e9);}
                for(channel,value)in ["R","G","B"].into_iter().zip(&mut light.color) {changed|=number(ui,&format!("Point light linear {channel}"),value,0.0,1e4);}
                changed|=number(ui,"Point light intensity",&mut light.intensity,0.0,1e4);
                changed|=number(ui,"Point light range",&mut light.range,1e-6,1e9);
                if ui.button("Reset point light defaults").clicked(){*light=defaults();changed=true;}
            });
        }
        changed
    }
}
#[cfg(test)]
#[path = "point_light_ui_tests.rs"]
mod tests;
