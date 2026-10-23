//! Optional standalone Collect map authoring. No project or runtime lifecycle.
#[path = "../collect_input_map_authoring.rs"]
mod collect_input_map_authoring;

fn main() -> eframe::Result {
    eframe::run_native(
        "Collect input-map authoring",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([720.0, 650.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::<collect_input_map_authoring::App>::default())),
    )
}
