//! Optional standalone Room map authoring. No project or runtime lifecycle.
#[path = "../room_input_map_authoring.rs"]
mod room_input_map_authoring;

fn main() -> eframe::Result {
    eframe::run_native(
        "Room input-map authoring",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([720.0, 650.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::<room_input_map_authoring::App>::default())),
    )
}
