//! Native registered-project editor for the Collect and deliver user game.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--project") {
        return Err("usage: collect-delivery-editor --project PROJECT.json".into());
    }
    let path = args.next().ok_or("missing project path")?;
    if args.next().is_some() {
        return Err("unexpected editor argument".into());
    }
    let prepared = orr_editor::registered_project::PreparedRegisteredProject::open(
        &path,
        orr_collect_delivery_game::registration::registry(),
    )?;
    let editor = orr_editor::Editor::start(&prepared.host_spec())?;
    eframe::run_native(
        "Collect and deliver",
        eframe::NativeOptions::default(),
        Box::new(move |context| {
            Ok(Box::new(orr_editor::EditorApp::new(
                editor,
                context.wgpu_render_state.clone(),
            )))
        }),
    )?;
    Ok(())
}
