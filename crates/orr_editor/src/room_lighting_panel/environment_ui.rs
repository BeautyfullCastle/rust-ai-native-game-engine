//! Optional environment values stage into the same Room lighting document.
use super::*;
use orr_sample::room_lighting::Environment;
impl Panel {
    pub(super) fn show_environment_controls(&mut self, ui: &mut egui::Ui) {
        let mut enabled = self.candidate.environment.is_some();
        if ui
            .checkbox(&mut enabled, "Environment lighting enabled")
            .changed()
        {
            self.candidate.environment = enabled.then(Environment::default);
        }
        let Some(environment) = self.candidate.environment.as_mut() else {
            return;
        };
        ui.collapsing("Environment lighting controls",|ui| {
            ui.weak("Sun and sky values are linear-light presentation settings. Apply and Save remain explicit; they do not change game physics.");
            for (axis,value) in ["X","Y","Z"].into_iter().zip(&mut environment.direction) {number_bounded(ui,&format!("Sun direction {axis}"),value,-1.0,1.0);}
            for (channel,value) in ["R","G","B"].into_iter().zip(&mut environment.color) {number_bounded(ui,&format!("Sun linear {channel}"),value,0.0,1.0);}
            number_bounded(ui,"Sun intensity",&mut environment.intensity,0.0,8.0);
            for (channel,value) in ["R","G","B"].into_iter().zip(&mut environment.sky) {number_bounded(ui,&format!("Sky linear {channel}"),value,0.0,1.0);}
            for (channel,value) in ["R","G","B"].into_iter().zip(&mut environment.ground) {number_bounded(ui,&format!("Ground linear {channel}"),value,0.0,1.0);}
            number_bounded(ui,"Environment ambient",&mut environment.ambient,0.0,8.0);
            if ui.button("Reset environment defaults").clicked() {*environment=Environment::default();}
        });
    }
}
#[cfg(test)]
#[path = "environment_ui_tests.rs"]
mod tests;
