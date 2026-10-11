//! Optional installed-package clip authoring. No implicit install or activation.
use orr_editor::sprite_clip_panel::ClipPanel;

struct App { panel: ClipPanel }
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.panel.show(ui));
        });
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os().skip(1).take(6)
        .map(|arg| arg.into_string().map_err(|_| std::io::Error::other("arguments must be UTF-8")))
        .collect::<Result<Vec<_>, _>>()?;
    if args.len() != 5 || args.iter().any(|arg| arg.is_empty() || arg.len() > 4096) {
        return Err(std::io::Error::other("usage: orr_sprite_clip PROJECT PACKAGE DOCUMENT CLIP NEW_VERSION").into());
    }
    let panel = ClipPanel::open_installed(std::path::Path::new(&args[0]), &args[1], &args[2], &args[3], &args[4])
        .map_err(std::io::Error::other)?;
    eframe::run_native(
        "Sprite clip authoring",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 720.0]),
            ..Default::default()
        },
        Box::new(move |_| Ok(Box::new(App { panel }))),
    )?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("sprite clip authoring: {error}");
        std::process::exit(2);
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "../sprite_clip_app_tests.rs"]
mod tests;
