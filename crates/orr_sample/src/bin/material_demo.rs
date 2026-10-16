use orr_sample::particle_demo::{run_with_materials, MaterialOptions, MATERIAL_HELP};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!("{MATERIAL_HELP}");
        return ExitCode::SUCCESS;
    }
    // Parsing completes before run_with_materials can create an event loop/device.
    match MaterialOptions::parse(args)
        .and_then(|options| run_with_materials(options.particles, options.materials))
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("material_demo: {error}");
            ExitCode::FAILURE
        }
    }
}
