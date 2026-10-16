//! Dedicated authored CollectDodgeV1 runtime; no Arena/network fallback.
use std::process::ExitCode;
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!("{}", orr_sample::collect_app::HELP);
        return ExitCode::SUCCESS;
    }
    match orr_sample::collect_app::Options::parse(args).and_then(orr_sample::collect_app::run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("collect_dodge: {error}");
            ExitCode::FAILURE
        }
    }
}
