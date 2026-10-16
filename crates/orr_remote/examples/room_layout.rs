//! Offline, opt-in authoring of the three Room objective positions.

#[cfg(feature = "room-project")]
#[path = "room_layout/authoring.rs"]
mod authoring;

#[cfg(feature = "room-project")]
fn main() -> std::process::ExitCode {
    let args = std::env::args_os().skip(1).take(14).collect();
    match authoring::Options::parse(args).and_then(authoring::run) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("room_layout: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "room-project"))]
fn main() -> std::process::ExitCode {
    eprintln!("room_layout requires --features room-project; no project or output was opened");
    std::process::ExitCode::FAILURE
}
