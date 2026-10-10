//! Dedicated authored RoomEscapeV1 runtime; no demo or network fallback.
use std::process::ExitCode;
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!("{}", orr_sample::room_app::HELP);
        return ExitCode::SUCCESS;
    }
    match orr_sample::room_app::Options::parse(args).and_then(orr_sample::room_app::run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("room_escape: {error}");
            ExitCode::FAILURE
        }
    }
}
