//! Dedicated closed-project terrain point-route player, with no demo fallback.
use std::process::ExitCode;
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!("{}", orr_sample::navigation_app::HELP);
        return ExitCode::SUCCESS;
    }
    match orr_sample::navigation_app::Options::parse(args).and_then(orr_sample::navigation_app::run)
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("navigation_playground: {error}");
            ExitCode::FAILURE
        }
    }
}
