//! Source-built editor. Explicit registration, not plugin discovery.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: editor <delivery|switchboard> <scene.yaml>".into());
    }
    let game = orr_static_starter::integration::registry().handle(&args[0])?;
    let editor = orr_editor::Editor::open_registered(std::path::Path::new(&args[1]), game)?;
    eframe::run_native(
        "Static Game Starter",
        eframe::NativeOptions::default(),
        Box::new(move |cc| {
            Ok(Box::new(orr_editor::EditorApp::new(
                editor,
                cc.wgpu_render_state.clone(),
            )))
        }),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
