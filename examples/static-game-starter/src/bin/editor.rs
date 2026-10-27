//! Source-built editor. Explicit registration, not plugin discovery.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = orr_static_starter::editor_args::EditorArgs::parse(std::env::args().skip(1))?;
    let editor = if args.source == "--project" {
        #[cfg(feature = "gameplay")]
        {
            #[cfg(feature = "stormglass")]
            let registry = orr_static_starter::stormglass::registry();
            #[cfg(not(feature = "stormglass"))]
            let registry = orr_static_starter::gameplay::registry();
            let prepared = orr_editor::registered_project::PreparedRegisteredProject::open(
                &args.path,
                registry,
            )?;
            let spec = prepared.host_spec();
            let spec = match args.listener {
                Some(config) => spec.with_listener(config),
                None => spec,
            };
            orr_editor::Editor::start(&spec)?
        }
        #[cfg(not(feature = "gameplay"))]
        {
            return Err("--project requires the gameplay feature".into());
        }
    } else {
        let game = orr_static_starter::integration::registry().handle(&args.source)?;
        orr_editor::Editor::open_registered(std::path::Path::new(&args.path), game)?
    };
    if let Some((url, _)) = editor.erp_status() {
        println!("ERP: {url}");
    }
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
