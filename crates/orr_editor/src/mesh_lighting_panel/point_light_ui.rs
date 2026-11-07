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
