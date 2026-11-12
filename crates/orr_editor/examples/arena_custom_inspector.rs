//! Run with --features custom-inspector --example arena_custom_inspector -- SCENE.
#[path = "arena_custom_inspector/panels.rs"]
mod panels;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scene = std::env::args_os()
        .nth(1)
        .ok_or("provide an Arena .scene.yaml path")?;
    let editor = orr_editor::Editor::open_game(
        std::path::Path::new(&scene),
        orr_editor::game::EditorGame::Arena,
    )?;
    let registry = panels::panels()?;
    eframe::run_native(
        "Arena custom Inspectors",
        eframe::NativeOptions::default(),
        Box::new(move |cc| {
            Ok(Box::new(
                orr_editor::EditorApp::new(editor, cc.wgpu_render_state.clone())
                    .with_custom_inspectors(registry),
            ))
        }),
    )?;
    Ok(())
}
