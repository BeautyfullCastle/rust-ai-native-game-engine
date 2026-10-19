//! Explicit default-off standalone particle preset authoring.
#[path = "../particle_preset_authoring.rs"]
mod particle_preset_authoring;
fn main() -> eframe::Result {
    eframe::run_native(
        "Particle preset authoring",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 820.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::<particle_preset_authoring::App>::default())),
    )
}
