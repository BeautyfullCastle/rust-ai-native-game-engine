//! Offline, opt-in authoring of one ordinary Room wall spawn operation.

#[cfg(feature = "room-project")]
#[path = "room_wall/authoring.rs"]
mod authoring;

#[cfg(feature = "room-project")]
fn main() -> std::process::ExitCode {
    // Six options and eight values. Bound collection before parsing, including
    // the extra argument which makes an overlong invocation fail closed.
    let args = std::env::args_os().skip(1).take(15).collect();
    match authoring::Options::parse(args).and_then(authoring::run) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("room_wall: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "room-project"))]
fn main() -> std::process::ExitCode {
    eprintln!("room_wall requires --features room-project; no project or output was opened");
    std::process::ExitCode::FAILURE
}
