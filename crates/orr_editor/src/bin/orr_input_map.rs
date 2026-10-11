//! Standalone optional authoring tool; no EditorApp/project/simulation lifecycle.
#[path = "../input_map_authoring.rs"]
mod input_map_authoring;

#[derive(Default)]
struct App { panel: input_map_authoring::Panel }
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.panel.show(ui));
        });
    }
}
fn main() -> eframe::Result {
    eframe::run_native(
        "Arena input-map authoring",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([720.0, 650.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::<App>::default())),
    )
}
