use std::process::ExitCode;
fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!("{}", orr_sample::particle_demo::HELP);
        return ExitCode::SUCCESS;
    }
    match orr_sample::particle_demo::Options::parse(args).and_then(orr_sample::particle_demo::run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("particle_demo: {e}");
            ExitCode::FAILURE
        }
    }
}
