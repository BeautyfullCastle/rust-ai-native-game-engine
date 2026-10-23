use std::process::ExitCode;
fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!("{}", orr_sample::particle_demo::HELP);
        #[cfg(feature = "particle-preset")]
        println!("{}", orr_sample::particle_demo::PRESET_HELP);
        return ExitCode::SUCCESS;
    }
    #[cfg(feature = "particle-preset")]
    let result = orr_sample::particle_demo::PresetOptions::parse(args)
        .and_then(orr_sample::particle_demo::run_with_preset);
    #[cfg(not(feature = "particle-preset"))]
    let result =
        orr_sample::particle_demo::Options::parse(args).and_then(orr_sample::particle_demo::run);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("particle_demo: {e}");
            ExitCode::FAILURE
        }
    }
}
