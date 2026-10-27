//! Source-built editor. Explicit registration, not plugin discovery.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: editor <delivery|switchboard> <scene.yaml> | editor --project <registered-project.json>".into());
    }
    let editor = if args[0] == "--project" {
        #[cfg(feature = "gameplay")]
        {
            let prepared = orr_editor::registered_project::PreparedRegisteredProject::open(
                &args[1],
                orr_static_starter::gameplay::registry(),
            )?;
            orr_editor::Editor::open_registered_project(prepared)?
        }
        #[cfg(not(feature = "gameplay"))]
        {
            return Err("--project requires the gameplay feature".into());
        }
    } else {
        let game = orr_static_starter::integration::registry().handle(&args[0])?;
        orr_editor::Editor::open_registered(std::path::Path::new(&args[1]), game)?
    };
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
